//! Only visible/prefetched rows own styled spans. Prefix parsing preserves lexical state.
use super::{display_text, highlight_allowed, syntax_for, theme};
use ratatui::text::{Line, Span, Text};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use syntect::{
    highlighting::{HighlightIterator, HighlightState, Highlighter},
    parsing::{ParseState, ScopeStack},
};

const CHECKPOINT_LINES: usize = 128;
const PREFETCH_LINES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeViewport {
    pub top: usize,
    pub rows: usize,
}
impl CodeViewport {
    fn range(self, total: usize) -> Range<usize> {
        self.top.min(total)..self.top.saturating_add(self.rows).min(total)
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct HighlightStats {
    pub requests: u64,
    pub cache_hits: u64,
    pub cancelled: u64,
    pub parsed_lines: u64,
    pub colored_lines: u64,
    pub retained_lines: usize,
    pub checkpoints: usize,
}

#[derive(Debug)]
pub struct HighlightWindow {
    pub first: usize,
    pub text: Text<'static>,
    pub light: bool,
    pub total_rows: usize,
    /// Includes plain fallback windows, so unsupported syntax isn't retried every tick.
    pub range: Range<usize>,
}
impl HighlightWindow {
    pub fn covers(&self, viewport: CodeViewport, light: bool) -> bool {
        let range = viewport.range(self.total_rows);
        self.light == light && self.range.start <= range.start && self.range.end >= range.end
    }
    pub fn line(&self, row: usize) -> Option<&Line<'static>> {
        self.text.lines.get(row.checked_sub(self.first)?)
    }
}

struct Checkpoint {
    row: usize,
    parse: ParseState,
    scopes: ScopeStack,
}
#[derive(Default)]
pub struct CodeHighlighter {
    source: Option<Arc<str>>,
    safe: Arc<str>,
    path: PathBuf,
    offsets: Vec<usize>,
    checkpoints: Vec<Checkpoint>,
    window: Option<Arc<HighlightWindow>>,
    stats: HighlightStats,
}
impl CodeHighlighter {
    pub fn stats(&self) -> HighlightStats {
        self.stats
    }

    /// `generation` changes on viewport/content/theme changes; obsolete work stops per line.
    pub fn render(
        &mut self,
        source: Arc<str>,
        path: &Path,
        viewport: CodeViewport,
        light: bool,
        generation: &AtomicU64,
        expected: u64,
    ) -> Option<Arc<HighlightWindow>> {
        let _perf = crate::perf::span(crate::perf::Metric::ViewportHighlight);
        self.stats.requests += 1;
        if generation.load(Ordering::Relaxed) != expected {
            self.stats.cancelled += 1;
            return None;
        }
        if self
            .source
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &source))
            || self.path != path
        {
            self.safe = match display_text(&source) {
                std::borrow::Cow::Borrowed(_) => source.clone(),
                std::borrow::Cow::Owned(s) => Arc::from(s),
            };
            self.offsets.clear();
            self.offsets.push(0);
            let mut offset = 0;
            for line in self.safe.split_inclusive('\n') {
                offset += line.len();
                self.offsets.push(offset);
            }
            if self.offsets.len() == 1 {
                self.offsets.push(0);
            }
            self.source = Some(source);
            self.path = path.to_owned();
            self.checkpoints.clear();
            self.window = None;
        }
        if let Some(window) = &self.window
            && window.covers(viewport, light)
        {
            self.stats.cache_hits += 1;
            return Some(window.clone());
        }
        let total = self.offsets.len() - 1;
        let visible = viewport.range(total);
        let range = visible.start.saturating_sub(PREFETCH_LINES)
            ..visible.end.saturating_add(PREFETCH_LINES).min(total);
        let syntax = highlight_allowed(&self.safe)
            .then(|| syntax_for(path))
            .flatten();
        let mut lines = Vec::new();
        let mut span_count = 0;
        if let Some((set, syntax)) = syntax {
            if self.checkpoints.is_empty() {
                self.checkpoints.push(Checkpoint {
                    row: 0,
                    parse: ParseState::new(syntax),
                    scopes: ScopeStack::new(),
                });
            }
            let checkpoint =
                &self.checkpoints[self.checkpoints.partition_point(|c| c.row <= range.start) - 1];
            let mut parse = checkpoint.parse.clone();
            let mut scopes = checkpoint.scopes.clone();
            let highlighter = Highlighter::new(theme::syntax(light));
            let mut highlight = None;
            for row in checkpoint.row..range.end {
                if generation.load(Ordering::Relaxed) != expected {
                    self.stats.cancelled += 1;
                    self.stats.checkpoints = self.checkpoints.len();
                    return None;
                }
                let raw = &self.safe[self.offsets[row]..self.offsets[row + 1]];
                self.stats.parsed_lines += 1;
                let Ok(ops) = parse.parse_line(raw, set) else {
                    lines.clear();
                    break;
                };
                if row < range.start {
                    let mut failed = false;
                    for (_, op) in &ops {
                        if scopes.apply(op).is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if failed {
                        lines.clear();
                        break;
                    }
                } else {
                    let state = highlight
                        .get_or_insert_with(|| HighlightState::new(&highlighter, scopes.clone()));
                    let spans = HighlightIterator::new(state, &ops, raw, &highlighter)
                        .filter_map(|(style, text)| {
                            let text = text.trim_end_matches(['\r', '\n']);
                            (!text.is_empty())
                                .then(|| Span::styled(text.to_owned(), theme::convert(style)))
                        })
                        .collect::<Vec<_>>();
                    span_count += spans.len();
                    if span_count > super::MAX_SPANS {
                        lines.clear();
                        break;
                    }
                    scopes = state.path.clone();
                    lines.push(Line::from(spans));
                    self.stats.colored_lines += 1;
                }
                let next = row + 1;
                if next.is_multiple_of(CHECKPOINT_LINES)
                    && self.checkpoints.last().is_none_or(|c| c.row < next)
                {
                    self.checkpoints.push(Checkpoint {
                        row: next,
                        parse: parse.clone(),
                        scopes: scopes.clone(),
                    });
                }
            }
        }
        if lines.len() != range.len() {
            lines = range
                .clone()
                .map(|row| {
                    Line::raw(
                        self.safe[self.offsets[row]..self.offsets[row + 1]]
                            .trim_end_matches(['\r', '\n'])
                            .to_owned(),
                    )
                })
                .collect();
        }
        let window = Arc::new(HighlightWindow {
            first: range.start,
            text: Text::from(lines),
            light,
            total_rows: total,
            range,
        });
        self.stats.retained_lines = window.text.lines.len();
        self.stats.checkpoints = self.checkpoints.len();
        self.window = Some(window.clone());
        Some(window)
    }
}
