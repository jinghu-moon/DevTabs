//! Preserve ignore's matching semantics; aggregate privately per visitor.
use super::*;
use std::sync::{Mutex, OnceLock};

static SCAN: Mutex<()> = Mutex::new(());
static SYNTAX: OnceLock<syntect::parsing::SyntaxSet> = OnceLock::new();

#[derive(Default)]
struct Totals {
    files: u64,
    bytes: u64,
    errors: bool,
    languages: BTreeMap<String, u64>,
    categories: BTreeMap<String, u64>,
}

struct Visitor<'a> {
    totals: Totals,
    results: &'a Mutex<Vec<Totals>>,
}
impl Drop for Visitor<'_> {
    fn drop(&mut self) {
        self.results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(std::mem::take(&mut self.totals));
    }
}

pub(super) fn scan(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    threads: usize,
) -> bool {
    let _guard = loop {
        if generation.load(Ordering::Relaxed) != ticket {
            return false;
        }
        match SCAN.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(10))
            }
        }
    };
    let _perf = crate::perf::span(crate::perf::Metric::OverviewSourceScan);
    let syntax = SYNTAX.get_or_init(two_face::syntax::extra_newlines);
    let results = Mutex::new(Vec::new());
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .filter_entry(|e| {
            !e.file_type().is_some_and(|t| t.is_dir())
                || !matches!(
                    e.file_name().to_str(),
                    Some(
                        ".git" | "node_modules" | "target" | "vendor" | "dist" | "build" | ".venv"
                    )
                )
        })
        .threads(threads.clamp(1, 2))
        .build_parallel()
        .run(|| {
            let mut visitor = Visitor {
                totals: Totals::default(),
                results: &results,
            };
            Box::new(move |entry| {
                if generation.load(Ordering::Relaxed) != ticket {
                    return ignore::WalkState::Quit;
                }
                let e = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        visitor.totals.errors = true;
                        return ignore::WalkState::Continue;
                    }
                };
                if e.error().is_some() {
                    visitor.totals.errors = true;
                }
                if !e.file_type().is_some_and(|t| t.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let language = e
                    .path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .and_then(|s| syntax.find_syntax_by_extension(s))
                    .or_else(|| {
                        e.file_name().to_str().and_then(|name| {
                            syntax
                                .find_syntax_by_extension(name)
                                .or_else(|| syntax.find_syntax_by_name(name))
                        })
                    });
                let Some(language) = language else {
                    return ignore::WalkState::Continue;
                };
                let metadata = match e.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        visitor.totals.errors = true;
                        return ignore::WalkState::Continue;
                    }
                };
                let totals = &mut visitor.totals;
                totals.files += 1;
                totals.bytes = totals.bytes.saturating_add(metadata.len());
                let name = if language.name.contains("Separated Values") {
                    "CSV/TSV"
                } else {
                    &language.name
                };
                *totals.languages.entry(name.into()).or_default() += metadata.len();
                let path = e.path().strip_prefix(root).unwrap_or(e.path());
                let category = if path
                    .iter()
                    .any(|s| matches!(s.to_str(), Some("tests" | "test" | "__tests__")))
                    || e.file_name().to_str().is_some_and(|s| {
                        s.contains(".test.") || s.contains(".spec.") || s.starts_with("test_")
                    }) {
                    "tests"
                } else if matches!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("md" | "mdx" | "rst" | "txt")
                ) || path
                    .iter()
                    .any(|s| matches!(s.to_str(), Some("docs" | "doc")))
                {
                    "docs"
                } else {
                    "source"
                };
                *totals.categories.entry(category.into()).or_default() += metadata.len();
                ignore::WalkState::Continue
            })
        });
    if generation.load(Ordering::Relaxed) != ticket {
        return false;
    }
    let mut languages = BTreeMap::<String, u64>::new();
    out.source_files = 0;
    out.source_bytes = 0;
    out.source_issues.clear();
    out.categories.clear();
    for totals in results.into_inner().unwrap_or_else(|e| e.into_inner()) {
        out.source_files += totals.files;
        out.source_bytes = out.source_bytes.saturating_add(totals.bytes);
        for (key, bytes) in totals.languages {
            *languages.entry(key).or_default() += bytes;
        }
        for (key, bytes) in totals.categories {
            *out.categories.entry(key).or_default() += bytes;
        }
        if totals.errors {
            issue(out, true, ScanIssue::ReadError);
        }
    }
    out.languages = languages.into_iter().collect();
    out.languages
        .sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
    out.languages.truncate(8);
    out.source_ready = true;
    out.partial = !out.source_issues.is_empty() || !out.disk_issues.is_empty();
    crate::perf::add(crate::perf::Count::StatisticsSourceFiles, out.source_files);
    true
}
