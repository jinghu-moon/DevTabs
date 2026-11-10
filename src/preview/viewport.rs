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
    pub reused_checkpoints: u64,
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
struct PendingSuffix {
    checkpoints: Vec<Checkpoint>,
    first_unchanged: usize,
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
    pending_suffix: Option<PendingSuffix>,
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
            let safe = match display_text(&source) {
                std::borrow::Cow::Borrowed(_) => source.clone(),
                std::borrow::Cow::Owned(s) => Arc::from(s),
            };
            let mut offsets = vec![0];
            let mut offset = 0;
            for line in safe.split_inclusive('\n') {
                offset += line.len();
                offsets.push(offset);
            }
            if offsets.len() == 1 {
                offsets.push(0);
            }
            self.pending_suffix = None;
            if self.path == path && !self.checkpoints.is_empty() {
                let old_rows = self.offsets.len() - 1;
                let new_rows = offsets.len() - 1;
                let mut prefix = 0;
                while prefix < old_rows.min(new_rows)
                    && self.safe[self.offsets[prefix]..self.offsets[prefix + 1]]
                        == safe[offsets[prefix]..offsets[prefix + 1]]
                {
                    prefix += 1;
                }
                let mut suffix = 0;
                while suffix
                    < old_rows
                        .saturating_sub(prefix)
                        .min(new_rows.saturating_sub(prefix))
                    && self.safe
                        [self.offsets[old_rows - 1 - suffix]..self.offsets[old_rows - suffix]]
                        == safe[offsets[new_rows - 1 - suffix]..offsets[new_rows - suffix]]
                {
                    suffix += 1;
                }
                let invalid = self.checkpoints.partition_point(|c| c.row <= prefix);
                let mut pending = self.checkpoints.split_off(invalid);
                pending.retain(|c| c.row >= old_rows - suffix);
                for checkpoint in &mut pending {
                    checkpoint.row = checkpoint
                        .row
                        .saturating_add_signed(new_rows as isize - old_rows as isize);
                }
                if !pending.is_empty() {
                    self.pending_suffix = Some(PendingSuffix {
                        checkpoints: pending,
                        first_unchanged: new_rows - suffix,
                    });
                }
            } else {
                self.checkpoints.clear();
            }
            self.safe = safe;
            self.offsets = offsets;
            self.source = Some(source);
            self.path = path.to_owned();
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
            let mut row = checkpoint.row;
            while row < range.end {
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
                if let Some(pending) = &self.pending_suffix
                    && next >= pending.first_unchanged
                    && pending
                        .checkpoints
                        .iter()
                        .find(|c| c.row == next)
                        .is_some_and(|c| c.parse == parse && c.scopes == scopes)
                {
                    let pending = self.pending_suffix.take().unwrap();
                    let suffix = pending
                        .checkpoints
                        .into_iter()
                        .filter(|c| c.row >= next)
                        .collect::<Vec<_>>();
                    self.stats.reused_checkpoints += suffix.len() as u64;
                    self.checkpoints.extend(suffix);
                }
                if next.is_multiple_of(CHECKPOINT_LINES)
                    && self.checkpoints.last().is_none_or(|c| c.row < next)
                {
                    self.checkpoints.push(Checkpoint {
                        row: next,
                        parse: parse.clone(),
                        scopes: scopes.clone(),
                    });
                }
                row = next;
                if row < range.start {
                    let index = self.checkpoints.partition_point(|c| c.row <= range.start);
                    if index > 0 && self.checkpoints[index - 1].row > row {
                        let cached = &self.checkpoints[index - 1];
                        row = cached.row;
                        parse = cached.parse.clone();
                        scopes = cached.scopes.clone();
                    }
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
