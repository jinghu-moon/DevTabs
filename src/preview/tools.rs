//! Bounded text tools shared by the read-only preview and its worker.
use super::*;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};
use regex::RegexBuilder;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}
pub const MAX_MATCHES: usize = 20_000;
pub type Matches = Vec<(usize, usize, usize)>;
pub fn search(lines: &[String], query: &str, options: SearchOptions) -> Result<Matches> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let pattern = if options.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()?;
    let mut matches = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let boundaries = line
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(line.len()))
            .collect::<Vec<_>>();
        let mut at = 0;
        while at <= line.len() {
            let Some(m) = regex.find_at(line, at) else {
                break;
            };
            if !m.is_empty() {
                matches.push((
                    row,
                    boundaries.partition_point(|&i| i < m.start()),
                    boundaries.partition_point(|&i| i < m.end()),
                ));
                if matches.len() == MAX_MATCHES {
                    return Ok(matches);
                }
            }
            let Some(c) = line[m.start()..].chars().next() else {
                break;
            };
            at = m.start() + c.len_utf8();
        }
    }
    Ok(matches)
}

#[derive(Clone, Debug)]
pub struct VisualRow {
    pub original: usize,
    pub chars: Range<usize>,
}
fn slice_line(line: &Line<'static>, range: Range<usize>) -> Line<'static> {
    let mut offset = 0;
    let mut spans = Vec::new();
    for span in &line.spans {
        let len = span.content.chars().count();
        let a = range.start.saturating_sub(offset).min(len);
        let b = range.end.saturating_sub(offset).min(len);
        if a < b {
            spans.push(Span::styled(
                span.content.chars().skip(a).take(b - a).collect::<String>(),
                span.style,
            ));
        }
        offset += len;
        if offset >= range.end {
            break;
        }
    }
    Line::from(spans).style(line.style)
}
impl Rendered {
    pub fn find(
        &self,
        query: &str,
        options: SearchOptions,
    ) -> Result<(Matches, Vec<(usize, usize)>)> {
        let Some(p) = &self.projection else {
            let matches = search(&self.plain, query, options)?;
            let targets = matches.iter().map(|&(row, col, _)| (row, col)).collect();
            return Ok((matches, targets));
        };
        let mut logical: Vec<String> = Vec::new();
        let mut groups: Vec<Range<usize>> = Vec::new();
        for (i, r) in p.iter().enumerate() {
            if i == 0 || r.original != p[i - 1].original {
                logical.push(String::new());
                groups.push(i..i);
            }
            logical.last_mut().unwrap().push_str(&self.plain[i]);
            groups.last_mut().unwrap().end = i + 1;
        }
        let mut fragments = Vec::new();
        let mut targets = Vec::new();
        for (group, a, b) in search(&logical, query, options)? {
            let rows = &groups[group];
            let mut target = None;
            for i in rows.clone() {
                let r = &p[i];
                let start = a.max(r.chars.start);
                let end = b.min(r.chars.end);
                if start < end {
                    target.get_or_insert((i, start - r.chars.start));
                    fragments.push((i, start - r.chars.start, end - r.chars.start));
                }
            }
            if let Some(point) = target {
                targets.push(point);
            }
        }
        fragments.sort_unstable();
        Ok((fragments, targets))
    }
    /// Wrap only code rows; prose and spatial Markdown layouts retain their layout.
    pub fn wrap_code(&mut self, width: usize) {
        let width = width.max(2);
        let mut projection = Vec::new();
        let mut plain = Vec::new();
        let mut rows = Vec::new();
        let mut neutral = Vec::new();
        for (row, text) in self.plain.iter().enumerate() {
            let mut ranges = Vec::new();
            let mut start = 0;
            let mut chars = 0;
            let mut cells = 0;
            if self.rows.get(row).is_some_and(|r| r.kind == RowKind::Code) {
                for g in text.graphemes(true) {
                    if cells > 0 && cells + g.width() > width {
                        ranges.push(start..chars);
                        start = chars;
                        cells = 0;
                    }
                    cells += g.width();
                    chars += g.chars().count();
                }
            } else {
                chars = text.chars().count();
            }
            ranges.push(start..chars);
            if projection.len() + ranges.len() > 50_000 {
                self.note = "Wrapping exceeds 50000 display rows; showing unwrapped code".into();
                return;
            }
            for range in ranges {
                plain.push(text.chars().skip(range.start).take(range.len()).collect());
                neutral.push(slice_line(&self.text.lines[row], range.clone()));
                let mut mapping = self.rows.get(row).cloned().unwrap_or_else(|| RowSource {
                    kind: RowKind::Code,
                    code_language: None,
                    code_copy: None,
                    table: None,
                    segments: Vec::new(),
                });
                mapping.segments = mapping
                    .segments
                    .into_iter()
                    .filter_map(|mut s| {
                        let a = range.start.max(s.display.start);
                        let b = range.end.min(s.display.end);
                        if a >= b {
                            return None;
                        }
                        if s.exact {
                            let raw = &self.source[s.source.clone()];
                            let byte = |c| raw.char_indices().nth(c).map_or(raw.len(), |(i, _)| i);
                            s.source = s.source.start + byte(a - s.display.start)
                                ..s.source.start + byte(b - s.display.start);
                        }
                        s.display = a - range.start..b - range.start;
                        Some(s)
                    })
                    .collect();
                rows.push(mapping);
                projection.push(VisualRow {
                    original: row,
                    chars: range,
                });
            }
        }
        self.projection = Some(Arc::new(projection));
        self.plain = Arc::new(plain);
        self.rows = Arc::new(rows);
        self.text = Arc::new(Text::from(neutral));
        let window = self.window.take();
        self.attach_window(window);
    }
    pub fn worker_viewport(&self, viewport: CodeViewport) -> CodeViewport {
        let Some(p) = &self.projection else {
            return viewport;
        };
        let top = p
            .get(viewport.top)
            .map_or(p.last().map_or(0, |r| r.original), |r| r.original);
        let end = p
            .get(viewport.top.saturating_add(viewport.rows))
            .map_or(top + viewport.rows, |r| r.original + 1);
        CodeViewport {
            top,
            rows: end.saturating_sub(top).max(1),
        }
    }
    pub fn attach_window(&mut self, window: Option<Arc<HighlightWindow>>) {
        let Some(p) = &self.projection else {
            self.window = window;
            return;
        };
        self.window = window.map(|w| {
            let first = p.partition_point(|r| r.original < w.first);
            let end = p.partition_point(|r| r.original < w.range.end);
            let lines = p[first..end]
                .iter()
                .map(|r| {
                    w.line(r.original)
                        .map_or_else(Line::default, |l| slice_line(l, r.chars.clone()))
                })
                .collect::<Vec<_>>();
            Arc::new(HighlightWindow {
                first,
                text: Text::from(lines),
                light: w.light,
                total_rows: p.len(),
                range: first..end,
            })
        });
    }
}

#[derive(Debug)]
pub struct Diff {
    pub patch: String,
    pub hunks: Vec<usize>,
    pub lines: Vec<Line<'static>>,
    pub added: usize,
    pub removed: usize,
}
pub fn diff(old: &str, new: &str) -> Diff {
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(250))
        .diff_lines(old, new);
    let mut added = 0;
    let mut removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            _ => {}
        }
    }
    let patch = diff
        .unified_diff()
        .context_radius(3)
        .header("previous", "disk")
        .to_string();
    let lines = if patch.is_empty() {
        vec![Line::raw("No changes")]
    } else {
        safe_text(&patch)
            .lines()
            .map(|s| {
                Line::styled(
                    s.to_owned(),
                    Style::new().fg(if s.starts_with('+') {
                        Color::Green
                    } else if s.starts_with('-') {
                        Color::Red
                    } else if s.starts_with("@@") {
                        Color::Cyan
                    } else {
                        Color::Gray
                    }),
                )
            })
            .collect()
    };
    let hunks = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.to_string().starts_with("@@").then_some(i))
        .collect();
    Diff {
        patch,
        hunks,
        lines,
        added,
        removed,
    }
}
