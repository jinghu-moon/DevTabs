//! Cancelable on-demand statistics, never computed during a UI frame.
mod disk;
mod source;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Hotspot {
    pub path: String,
    pub commits: u32,
    pub added: u64,
    pub removed: u64,
    pub latest: u64,
    pub binary: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScanIssue {
    EntryLimit,
    TimeLimit,
    ReadError,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Calendar {
    pub first_day: u64,
    pub today: u64,
    pub days: Vec<u32>,
}
impl Calendar {
    fn new(today: u64) -> Self {
        Self {
            first_day: today.saturating_sub((today + 3) % 7 + 364),
            today,
            days: vec![0; 371],
        }
    }
    fn record(&mut self, timestamp: u64) {
        let day = timestamp / 86400;
        if day >= self.first_day
            && day <= self.today
            && let Some(value) = self.days.get_mut((day - self.first_day) as usize)
        {
            *value += 1;
        }
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Stats {
    pub files: u64,
    pub bytes: u64,
    pub source_files: u64,
    pub source_bytes: u64,
    pub languages: Vec<(String, u64)>,
    pub directories: Vec<(String, u64)>,
    pub commits: Vec<String>,
    pub activity: u64,
    pub weeks: Vec<u32>,
    pub numstat: Vec<(String, u64, u64)>,
    pub partial: bool,
    pub source_ready: bool,
    pub disk_ready: bool,
    pub disk_scanning: bool,
    pub scanned_entries: u64,
    pub git_error: Option<String>,
    pub source_issues: Vec<ScanIssue>,
    pub disk_issues: Vec<ScanIssue>,
    pub categories: BTreeMap<String, u64>,
    pub artifact_bytes: u64,
    pub dependency_bytes: u64,
    pub artifacts: Vec<(String, u64)>,
    pub days: Vec<u32>,
    pub calendar: Option<Calendar>,
    pub hotspots: Vec<Hotspot>,
    pub directory_hotspots: Vec<Hotspot>,
    pub hotspots_error: Option<String>,
    pub hotspots_limited: bool,
    pub total_commits: Option<u64>,
}
fn scan_ttl(issues: &[ScanIssue]) -> u64 {
    if issues
        .iter()
        .any(|i| matches!(i, ScanIssue::TimeLimit | ScanIssue::ReadError))
    {
        30
    } else {
        600
    }
}
pub fn refresh_secs(s: &Stats) -> u64 {
    scan_ttl(&s.source_issues).min(scan_ttl(&s.disk_issues))
}
pub fn collect(root: &Path, generation: &Arc<AtomicU64>, ticket: u64) -> Stats {
    collect_inner(root, generation, ticket, None)
}
fn collect_inner(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    disk: Option<&Stats>,
) -> Stats {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewStats);
    let mut out = Stats::default();
    if generation.load(Ordering::Relaxed) != ticket {
        return out;
    }
    let start = Instant::now();
    out.disk_ready = disk.is_none() || disk.is_some_and(|s| s.disk_ready);
    if let Some(disk) = disk {
        out.files = disk.files;
        out.bytes = disk.bytes;
        out.directories = disk.directories.clone();
        out.disk_issues = disk.disk_issues.clone();
        out.artifacts = disk.artifacts.clone();
        out.artifact_bytes = disk.artifact_bytes;
        out.dependency_bytes = disk.dependency_bytes;
        out.partial = !disk.disk_issues.is_empty();
    } else if !scan_disk(root, generation, ticket, &mut out, start) {
        return Stats::default();
    }
    if !scan_source(root, generation, ticket, &mut out, Instant::now()) {
        return Stats::default();
    }
    collect_git(root, generation, ticket, &mut out);
    if generation.load(Ordering::Relaxed) == ticket {
        out
    } else {
        Stats::default()
    }
}
fn collect_git(root: &Path, generation: &Arc<AtomicU64>, ticket: u64, out: &mut Stats) {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewStatsGit);
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(root)
    };
    let possible = std::env::var_os("GIT_DIR").is_some()
        || absolute.ancestors().any(|p| p.join(".git").exists())
        || (absolute.join("HEAD").is_file() && absolute.join("objects").is_dir());
    if !possible {
        out.git_error = Some(crate::i18n::t("not_git").into_owned());
        return;
    }
    // An ordinary, freshly initialized repository has no HEAD commit. Its
    // missing loose ref (with no packed refs) is sufficient evidence, so do
    // not launch log/rev-parse/diff processes merely to rediscover that fact.
    if std::env::var_os("GIT_DIR").is_none()
        && let Some(git) = absolute
            .ancestors()
            .map(|p| p.join(".git"))
            .find(|p| p.exists())
        && git.is_dir()
        && git.join("objects").is_dir()
        && git.join("refs").is_dir()
        && !git.join("packed-refs").exists()
        && std::fs::metadata(git.join("HEAD")).is_ok_and(|m| m.len() <= 4096)
        && let Ok(head) = std::fs::read_to_string(git.join("HEAD"))
        && let Some(reference) = head.trim().strip_prefix("ref: refs/heads/")
        && !reference.is_empty()
        && reference.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.contains('\\')
                && !part.contains(':')
        })
        && !git.join("refs/heads").join(reference).exists()
    {
        out.weeks = vec![0; 12];
        out.days = vec![0; 84];
        out.calendar = Some(Calendar::new(crate::config::now() / 86400));
        out.total_commits = Some(0);
        return;
    }
    let now = crate::config::now();
    let mut calendar = Calendar::new(now / 86400);
    let since = format!("--since-as-filter=@{}", calendar.first_day * 86400);
    let recent = crate::explorer::git_output(root, &["log", &since, "--format=%ct%x09%h %cs %s"]);
    if generation.load(Ordering::Relaxed) != ticket {
        return;
    }
    match recent {
        Ok(bytes) => {
            out.weeks = vec![0; 12];
            out.days = vec![0; 84];
            for line in String::from_utf8_lossy(&bytes).lines() {
                let Some((timestamp, summary)) = line.split_once('\t') else {
                    continue;
                };
                if out.commits.len() < 6 {
                    out.commits.push(summary.chars().take(160).collect());
                }
                if let Ok(timestamp) = timestamp.parse::<u64>() {
                    let age = now.saturating_sub(timestamp);
                    if age < 30 * 86400 {
                        out.activity += 1;
                    }
                    if timestamp <= now {
                        calendar.record(timestamp);
                    }
                }
            }
            out.days = calendar.days[287..].to_vec();
            out.weeks = out.days.chunks(7).map(|days| days.iter().sum()).collect();
            out.calendar = Some(calendar);
            // Still show old commits when the project has little recent activity.
            if out.commits.len() < 6
                && let Ok(bytes) =
                    crate::explorer::git_output(root, &["log", "-6", "--format=%h %cs %s"])
            {
                out.commits = String::from_utf8_lossy(&bytes)
                    .lines()
                    .map(|line| line.chars().take(160).collect())
                    .collect();
            }
        }
        Err(_) => {
            out.git_error = Some(crate::i18n::t("overview.stats_git_failed").into_owned());
            // Only the verified unborn-HEAD fast path above may report zero.
            // Timeouts and corrupt repositories must remain unavailable.
            return;
        }
    }
    if generation.load(Ordering::Relaxed) != ticket {
        return;
    }
    out.total_commits = crate::explorer::git_output(root, &["rev-list", "--count", "HEAD"])
        .ok()
        .and_then(|v| String::from_utf8_lossy(&v).trim().parse().ok());
    if generation.load(Ordering::Relaxed) != ticket {
        return;
    }
    match crate::explorer::git_output(
        root,
        &[
            "log",
            "--since=30.days",
            "-1001",
            "--format=%x00DT:%ct",
            "--numstat",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            "--no-merges",
            "--",
            ".",
        ],
    ) {
        Ok(bytes) => {
            let (files, dirs, limited) = parse_hotspots(&bytes);
            out.hotspots = files;
            out.directory_hotspots = dirs;
            out.hotspots_limited = limited;
        }
        Err(_) => {
            out.hotspots_error = Some(crate::i18n::t("overview.hotspots_failed").into_owned())
        }
    }
    if generation.load(Ordering::Relaxed) == ticket
        && let Ok(bytes) =
            crate::explorer::git_output(root, &["diff", "HEAD", "--numstat", "-z", "--no-renames"])
    {
        out.numstat = parse_numstat(&bytes);
    }
}
fn issue(out: &mut Stats, source: bool, reason: ScanIssue) {
    out.partial = true;
    let issues = if source {
        &mut out.source_issues
    } else {
        &mut out.disk_issues
    };
    if !issues.contains(&reason) {
        issues.push(reason);
    }
}
pub fn artifact_directory(name: &str) -> bool {
    matches!(
        name,
        "target" | "build" | "dist" | ".next" | "obj" | "bin" | "vendor" | "node_modules" | ".venv"
    )
}
fn scan_disk(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    _start: Instant,
) -> bool {
    disk::scan(root, generation, ticket, out, 4, &mut |_| {})
}
fn scan_source(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    _start: Instant,
) -> bool {
    source::scan(root, generation, ticket, out, 2)
}
#[cfg(test)]
fn scan_source_serial(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    out: &mut Stats,
    _start: Instant,
) -> bool {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewSourceScan);
    let mut langs = BTreeMap::<String, u64>::new();
    static SYNTAX: std::sync::OnceLock<syntect::parsing::SyntaxSet> = std::sync::OnceLock::new();
    if generation.load(Ordering::Relaxed) != ticket {
        return false;
    }
    let syntax = SYNTAX.get_or_init(two_face::syntax::extra_newlines);
    let walk = ignore::WalkBuilder::new(root)
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
        .build();
    for e in walk {
        if generation.load(Ordering::Relaxed) != ticket {
            return false;
        }
        let Ok(e) = e else {
            issue(out, true, ScanIssue::ReadError);
            continue;
        };
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(language) = e
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
            })
        else {
            continue;
        };
        if let Ok(m) = e.metadata() {
            out.source_files += 1;
            out.source_bytes = out.source_bytes.saturating_add(m.len());
            let name = if language.name.contains("Separated Values") {
                "CSV/TSV".into()
            } else {
                language.name.clone()
            };
            *langs.entry(name).or_default() += m.len();
            let relative = e.path().strip_prefix(root).unwrap_or(e.path());
            let components: Vec<_> = relative.iter().filter_map(|c| c.to_str()).collect();
            let category = if components
                .iter()
                .any(|s| matches!(*s, "tests" | "test" | "__tests__"))
                || components.last().is_some_and(|s| {
                    s.contains(".test.") || s.contains(".spec.") || s.starts_with("test_")
                }) {
                "tests"
            } else if matches!(
                e.path().extension().and_then(|s| s.to_str()),
                Some("md" | "mdx" | "rst" | "txt")
            ) || components.iter().any(|s| matches!(*s, "docs" | "doc"))
            {
                "docs"
            } else {
                "source"
            };
            *out.categories.entry(category.into()).or_default() += m.len();
        } else {
            issue(out, true, ScanIssue::ReadError)
        }
    }
    out.source_ready = true;
    out.languages = langs.into_iter().collect();
    out.languages.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    out.languages.truncate(8);
    if generation.load(Ordering::Relaxed) != ticket {
        return false;
    }
    true
}
/// Source and Git collection do not wait for the full disk traversal.
pub fn source(root: &Path, generation: &Arc<AtomicU64>, ticket: u64) -> Stats {
    source_at(root, &crate::config::cache(), generation, ticket)
}
pub fn source_at(root: &Path, home: &Path, generation: &Arc<AtomicU64>, ticket: u64) -> Stats {
    if generation.load(Ordering::Relaxed) != ticket {
        return Stats::default();
    }
    let key = crate::config::key(root);
    let hash = key.bytes().fold(2166136261u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(16777619)
    });
    let mut stamp = cache_stamp(root);
    stamp.push(6);
    stamp.extend(cache_stamp(&home.join("stats-refresh")));
    let now = crate::config::now();
    for suffix in [".json", "-source.json"] {
        let path = home.join("stats").join(format!("{hash:016x}{suffix}"));
        if let Some(cached) = read_cache(&path, &key, &stamp).filter(|c| {
            c.stats.source_ready
                && now.saturating_sub(c.source_at) < scan_ttl(&c.stats.source_issues)
        }) {
            return cached.stats;
        }
    }
    let result = collect_inner(root, generation, ticket, Some(&Stats::default()));
    if generation.load(Ordering::Relaxed) == ticket
        && let Ok(bytes) = serde_json::to_vec(&Cached {
            path: key,
            source_at: now,
            disk_at: 0,
            stamp,
            stats: result.clone(),
        })
        && bytes.len() <= 128 * 1024
    {
        let _ = crate::config::atomic_write(
            &home.join("stats").join(format!("{hash:016x}-source.json")),
            &bytes,
        );
    }
    result
}
pub fn git(root: &Path) -> anyhow::Result<super::model::GitSummary> {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewGit);
    let bytes = crate::explorer::git_output(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
    )?;
    Ok(parse_git(&bytes))
}
pub fn parse_git(bytes: &[u8]) -> super::model::GitSummary {
    let mut out = super::model::GitSummary::default();
    let mut parts = bytes.split(|b| *b == 0);
    while let Some(part) = parts.next() {
        let text = String::from_utf8_lossy(part);
        if let Some(s) = text.strip_prefix("# branch.head ") {
            out.branch = s.into()
        } else if let Some(s) = text.strip_prefix("# branch.upstream ") {
            out.upstream = s.into()
        } else if let Some(s) = text.strip_prefix("# branch.ab ") {
            let mut s = s.split_whitespace();
            out.ahead = s
                .next()
                .and_then(|s| s.trim_start_matches('+').parse().ok())
                .unwrap_or_default();
            out.behind = s
                .next()
                .and_then(|s| s.trim_start_matches('-').parse().ok())
                .unwrap_or_default()
        } else if matches!(part.first(), Some(b'1' | b'2' | b'u' | b'?')) {
            out.changed += 1;
            if part.first() == Some(&b'u') {
                out.conflicts += 1
            }
            if out.files.len() < 12 {
                let count = match part.first() {
                    Some(b'1') => 8,
                    Some(b'2') => 9,
                    Some(b'u') => 10,
                    _ => 1,
                };
                out.files.push(
                    text.splitn(count + 1, ' ')
                        .last()
                        .unwrap_or_default()
                        .into(),
                );
                if let Some(path) = out.files.last() {
                    out.statuses.insert(
                        path.clone(),
                        if part.first() == Some(&b'?') {
                            "?".into()
                        } else {
                            text.split_whitespace().nth(1).unwrap_or("M").to_owned()
                        },
                    );
                }
            }
            if part.first() == Some(&b'2') {
                parts.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Release source/disk benchmark against unmodified reference stats.rs; temporary fixture only"]
    fn statistics_scan_benchmark() {
        use std::{fs, process::Command};
        let root = std::env::temp_dir().join(format!(
            "DevTabs-statistics-benchmark-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(root.clone());
        for (folder, files, data) in [
            ("src", 3000, &b"fn main() {}\n"[..]),
            ("target", 12000, &b"generated artifact"[..]),
        ] {
            for i in 0..files {
                let parent = root.join(folder).join(format!("part{}", i / 100));
                fs::create_dir_all(&parent).unwrap();
                fs::write(
                    parent.join(format!(
                        "file{i}.{}",
                        if folder == "src" { "rs" } else { "bin" }
                    )),
                    data,
                )
                .unwrap();
            }
        }
        let reference = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/stats-reference/release/devtabs-stats-reference-bench.exe");
        let mut cases = Vec::new();
        for repository in [false, true] {
            if repository {
                assert!(
                    Command::new("git")
                        .args(["init", "--quiet", "--template="])
                        .arg(&root)
                        .status()
                        .unwrap()
                        .success()
                );
                fs::create_dir_all(root.join(".git/info")).unwrap();
                fs::write(root.join(".git/info/exclude"), "target/\n").unwrap();
            }
            let old = Command::new(&reference)
                .arg(&root)
                .output()
                .expect("build artifacts/stats-reference-bench first");
            assert!(
                old.status.success(),
                "{}",
                String::from_utf8_lossy(&old.stderr)
            );
            let old: serde_json::Value = serde_json::from_slice(&old.stdout).unwrap();
            let generation = Arc::new(AtomicU64::new(1));
            let mut source = Vec::new();
            let mut disk = Vec::new();
            let mut full = Vec::new();
            let mut counts = serde_json::Value::Null;
            for i in 0..8 {
                let mut s = Stats::default();
                let start = Instant::now();
                assert!(scan_source(&root, &generation, 1, &mut s, start));
                let us = start.elapsed().as_micros();
                if i > 0 {
                    source.push(us);
                }
                let mut d = Stats::default();
                let start = Instant::now();
                assert!(scan_disk(&root, &generation, 1, &mut d, start));
                let us = start.elapsed().as_micros();
                if i > 0 {
                    disk.push(us);
                }
                assert!(!s.partial && !d.partial);
                counts = serde_json::json!({"source_files":s.source_files,"source_bytes":s.source_bytes,"disk_files":d.files,"disk_bytes":d.bytes});
                assert_eq!(counts, old["counts"]);
                let start = Instant::now();
                let complete = collect(&root, &generation, 1);
                let us = start.elapsed().as_micros();
                if i > 0 {
                    full.push(us);
                }
                assert!(!complete.partial && complete.source_ready && complete.disk_ready);
                assert_eq!(
                    serde_json::json!({"source_files":complete.source_files,"source_bytes":complete.source_bytes,"disk_files":complete.files,"disk_bytes":complete.bytes}),
                    counts
                );
            }
            cases.push(serde_json::json!({"git":repository,"legacy":old,"current":{"source_us":source,"disk_us":disk,"full_with_git_us":full,"counts":counts}}));
        }
        let output = serde_json::json!({"fixture":{"source_files":3000,"build_files":12000},"samples":7,"warmup":1,"unit":"microseconds","cases":cases,"scope":"Uncached scans, warm OS filesystem; matching Rust/generated-file fixtures in Git and non-Git directories. full_with_git_us also includes Git summaries; legacy combined source+disk timings omit these summaries. Persistent cache is measured separately."});
        let label = std::env::var("DEVTABS_STATS_BENCH_LABEL").unwrap_or_else(|_| "after".into());
        assert!(
            label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("artifacts/statistics-scan-{label}.json"));
        fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
        println!("{output}");
        drop(fixture);
    }
    #[test]
    fn parses_porcelain_v2_branch_and_conflicts() {
        let parsed = parse_git(b"# branch.head main\0# branch.ab +2 -1\0u UU N... 100644 100644 100644 abc def file.rs\0");
        assert_eq!(parsed.branch, "main");
        assert_eq!((parsed.ahead, parsed.behind, parsed.conflicts), (2, 1, 1));
        assert_eq!(parsed.changed, 1);
    }
}

/// Git -z keeps filenames literal (including whitespace). One file is counted
/// once per commit; directory counts are distinct commits, not sums of files.
pub fn parse_hotspots(bytes: &[u8]) -> (Vec<Hotspot>, Vec<Hotspot>, bool) {
    let mut files = BTreeMap::<String, Hotspot>::new();
    let mut dirs = BTreeMap::<String, Hotspot>::new();
    let mut seen_files = std::collections::BTreeSet::new();
    let mut seen_dirs = std::collections::BTreeSet::new();
    let mut timestamp = 0;
    let mut commits = 0;
    for field in bytes.split(|b| *b == 0) {
        let field = field.strip_prefix(b"\n").unwrap_or(field);
        if let Some(value) = field.strip_prefix(b"DT:") {
            timestamp = String::from_utf8_lossy(value).trim().parse().unwrap_or(0);
            commits += 1;
            seen_files.clear();
            seen_dirs.clear();
            continue;
        }
        if commits > 1000 {
            break;
        }
        let mut parts = field.splitn(3, |b| *b == b'\t');
        let Some(added_raw) = parts.next() else {
            continue;
        };
        let Some(removed_raw) = parts.next() else {
            continue;
        };
        let binary = added_raw == b"-" || removed_raw == b"-";
        let added = if binary {
            0
        } else {
            let Some(v) = std::str::from_utf8(added_raw)
                .ok()
                .and_then(|p| p.parse::<u64>().ok())
            else {
                continue;
            };
            v
        };
        let removed = if binary {
            0
        } else {
            let Some(v) = std::str::from_utf8(removed_raw)
                .ok()
                .and_then(|p| p.parse::<u64>().ok())
            else {
                continue;
            };
            v
        };
        let Some(path) = parts.next() else {
            continue;
        };
        let path = String::from_utf8_lossy(path).into_owned();
        let directory = path.rsplit_once('/').map_or(".", |(d, _)| d).to_owned();
        let file = files.entry(path.clone()).or_insert_with(|| Hotspot {
            path: path.clone(),
            ..Default::default()
        });
        file.binary |= binary;
        file.added += added;
        file.removed += removed;
        file.latest = file.latest.max(timestamp);
        if seen_files.insert(path) {
            file.commits += 1;
        }
        let dir = dirs.entry(directory.clone()).or_insert_with(|| Hotspot {
            path: directory.clone(),
            ..Default::default()
        });
        dir.binary |= binary;
        dir.added += added;
        dir.removed += removed;
        dir.latest = dir.latest.max(timestamp);
        if seen_dirs.insert(directory) {
            dir.commits += 1;
        }
    }
    let sort = |map: BTreeMap<String, Hotspot>| {
        let mut rows = map.into_values().collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            b.commits
                .cmp(&a.commits)
                .then((b.added + b.removed).cmp(&(a.added + a.removed)))
                .then(a.path.cmp(&b.path))
        });
        rows.truncate(6);
        rows
    };
    (sort(files), sort(dirs), commits > 1000)
}
/// NUL-separated paths preserve tabs, newlines and Unicode in Git filenames.
pub fn parse_numstat(bytes: &[u8]) -> Vec<(String, u64, u64)> {
    let mut result = bytes
        .split(|b| *b == 0)
        .filter_map(|line| {
            let mut fields = line.splitn(3, |b| *b == b'\t');
            let add = std::str::from_utf8(fields.next()?)
                .ok()?
                .parse()
                .unwrap_or(0);
            let remove = std::str::from_utf8(fields.next()?)
                .ok()?
                .parse()
                .unwrap_or(0);
            let path = String::from_utf8_lossy(fields.next()?).into_owned();
            Some((path, add, remove))
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|(_, a, d)| std::cmp::Reverse(a + d));
    result.truncate(6);
    result
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Cached {
    path: String,
    source_at: u64,
    disk_at: u64,
    stamp: Vec<u64>,
    stats: Stats,
}
fn read_cache(path: &Path, key: &str, stamp: &[u64]) -> Option<Cached> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.len() <= 128 * 1024)
        .and_then(|_| std::fs::read(path).ok())
        .and_then(|b| serde_json::from_slice::<Cached>(&b).ok())
        .filter(|c| c.path == key && c.stamp == stamp && !c.stats.disk_scanning)
}
fn cache_stamp(root: &Path) -> Vec<u64> {
    let mut stamp: Vec<u64> = [
        root.to_path_buf(),
        root.join(".git/HEAD"),
        root.join(".git/index"),
        root.join(".git/refs/heads"),
    ]
    .iter()
    .map(|p| {
        std::fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_nanos().min(u64::MAX as u128) as u64)
    })
    .collect();
    if let Ok(repo) = crate::tasks::Repository::discover(root) {
        stamp.extend(repo.stamp());
    }
    stamp
}
pub fn cached(root: &Path, generation: &Arc<AtomicU64>, ticket: u64) -> Stats {
    cached_at(root, &crate::config::cache(), generation, ticket, false)
}
pub fn cached_with_progress(
    root: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    progress: &mut dyn FnMut(&Stats),
) -> Stats {
    cached_at_with_progress(
        root,
        &crate::config::cache(),
        generation,
        ticket,
        false,
        progress,
    )
}
pub fn cached_at(
    root: &Path,
    home: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    force: bool,
) -> Stats {
    cached_at_with_progress(root, home, generation, ticket, force, &mut |_| {})
}
pub fn cached_at_with_progress(
    root: &Path,
    home: &Path,
    generation: &Arc<AtomicU64>,
    ticket: u64,
    force: bool,
    progress: &mut dyn FnMut(&Stats),
) -> Stats {
    if generation.load(Ordering::Relaxed) != ticket {
        return Stats::default();
    }
    let key = crate::config::key(root);
    let hash = key.bytes().fold(2166136261u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(16777619)
    });
    let path = home.join("stats").join(format!("{hash:016x}.json"));
    let mut stamp = cache_stamp(root);
    stamp.push(6);
    stamp.extend(cache_stamp(&home.join("stats-refresh")));
    let now = crate::config::now();
    let old = read_cache(&path, &key, &stamp)
        .filter(|c| !force && c.stats.source_ready && c.stats.disk_ready);
    if let Some(c) = old.as_ref().filter(|c| {
        now.saturating_sub(c.source_at) < scan_ttl(&c.stats.source_issues)
            && now.saturating_sub(c.disk_at)
                < if c.stats.disk_issues.is_empty() {
                    43200
                } else {
                    scan_ttl(&c.stats.disk_issues)
                }
    }) {
        return c.stats.clone();
    }
    let disk = old.as_ref().filter(|c| {
        now.saturating_sub(c.disk_at)
            < if c.stats.disk_issues.is_empty() {
                43200
            } else {
                scan_ttl(&c.stats.disk_issues)
            }
    });
    let source = if force {
        None
    } else {
        read_cache(
            &home.join("stats").join(format!("{hash:016x}-source.json")),
            &key,
            &stamp,
        )
        .filter(|c| {
            c.stats.source_ready
                && now.saturating_sub(c.source_at) < scan_ttl(&c.stats.source_issues)
        })
    };
    let source_at = source.as_ref().map_or(now, |c| c.source_at);
    let mut result = if let Some(source) = source {
        source.stats
    } else {
        // Source/Git are published before the full disk scan; no repeated scan
        // is needed when the separate source worker already supplied this data.
        collect_inner(root, generation, ticket, Some(&Stats::default()))
    };
    if generation.load(Ordering::Relaxed) != ticket {
        return Stats::default();
    }
    if let Some(disk) = disk {
        result.files = disk.stats.files;
        result.bytes = disk.stats.bytes;
        result.directories = disk.stats.directories.clone();
        result.artifacts = disk.stats.artifacts.clone();
        result.artifact_bytes = disk.stats.artifact_bytes;
        result.dependency_bytes = disk.stats.dependency_bytes;
        result.scanned_entries = disk.stats.scanned_entries;
        result.disk_issues = disk.stats.disk_issues.clone();
        result.partial = !result.source_issues.is_empty() || !result.disk_issues.is_empty();
        result.disk_ready = true;
    } else if !disk::scan(root, generation, ticket, &mut result, 4, progress) {
        return Stats::default();
    }
    if generation.load(Ordering::Relaxed) == ticket {
        let disk_at = disk.map_or(now, |c| c.disk_at);
        let data = Cached {
            path: key,
            source_at,
            disk_at,
            stamp,
            stats: result.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&data)
            && let Err(e) = crate::config::atomic_write(&path, &bytes)
        {
            eprintln!("Write statistics cache: {e:#}");
        }
    }
    result
}

#[cfg(test)]
mod pulse_tests {
    use super::*;
    #[test]
    fn refreshed_source_does_not_inherit_old_source_failure_from_reused_disk() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-source-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let disk = Stats {
            disk_ready: true,
            partial: true,
            source_issues: vec![ScanIssue::TimeLimit],
            ..Default::default()
        };
        let result = collect_inner(&root, &Arc::new(AtomicU64::new(1)), 1, Some(&disk));
        assert!(result.source_ready && result.disk_ready);
        assert!(
            !result.partial && result.source_issues.is_empty() && result.disk_issues.is_empty()
        );
        std::fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn elapsed_time_never_truncates_scans_but_read_errors_remain_explicit() {
        let root = std::env::temp_dir().join(format!("DevTabs-budget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("test.rs"), "fn main() {}\n").unwrap();
        let flag = Arc::new(AtomicU64::new(1));
        let expired = Instant::now() - Duration::from_secs(10);
        let mut source = Stats::default();
        assert!(scan_source(&root, &flag, 1, &mut source, expired));
        assert!(source.source_issues.is_empty());
        assert_eq!(source.source_files, 1);
        assert!(source.disk_issues.is_empty());
        let mut disk = Stats::default();
        assert!(scan_disk(&root, &flag, 1, &mut disk, expired));
        assert!(disk.disk_issues.is_empty());
        assert_eq!(disk.files, 1);
        assert!(disk.source_issues.is_empty());
        let mut missing = Stats::default();
        assert!(scan_disk(
            &root.join("missing"),
            &flag,
            1,
            &mut missing,
            Instant::now()
        ));
        assert_eq!(missing.disk_issues, vec![ScanIssue::ReadError]);
        assert!(!source.partial && !disk.partial && missing.partial);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hotspot_limit_is_explicit_and_binary_changes_are_not_reported_as_text_lines() {
        let mut bytes = Vec::new();
        for i in 0..1001 {
            bytes.extend_from_slice(format!("\0DT:{}\0\n-\t-\timage.png\0", 1001 - i).as_bytes());
        }
        let (files, _, limited) = parse_hotspots(&bytes);
        assert!(limited);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].commits, 1000);
        assert!(files[0].binary);
        assert_eq!((files[0].added, files[0].removed), (0, 0));
    }
}

#[cfg(test)]
mod verification;
