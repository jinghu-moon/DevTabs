//! Parse references globally once; lay out complete top-level blocks only as requested.
use super::{Event, Options, Parser, TagEnd, render_events};
use crate::preview::{CodeViewport, HighlightWindow, Rendered, RowSource, layout::Row};
use ratatui::text::{Line, Text};
use std::{collections::VecDeque, ops::Range, sync::Arc};

#[derive(Clone, Copy, Default, Debug, serde::Serialize)]
pub struct MarkdownStats {
    pub indexed_blocks: usize,
    pub laid_out_blocks: u64,
    pub cache_hits: u64,
    pub cached_blocks: usize,
    pub cached_rows: usize,
    pub known_rows: usize,
    pub complete: bool,
}
struct Block {
    events: Vec<(Event<'static>, Range<usize>)>,
    rows: Range<usize>,
}
#[derive(Default)]
pub struct MarkdownCache {
    source: Option<Arc<str>>,
    width: u16,
    light: bool,
    blocks: Vec<Block>,
    display_ranges: Vec<Range<usize>>,
    next: usize,
    plain: Arc<Vec<String>>,
    rows: Arc<Vec<RowSource>>,
    neutral: Arc<Text<'static>>,
    cached: VecDeque<(usize, Arc<Vec<Row>>)>,
    notes: Vec<String>,
    spans: usize,
    done: bool,
    stats: MarkdownStats,
}
impl MarkdownCache {
    pub fn stats(&self) -> MarkdownStats {
        self.stats
    }
    fn reset(&mut self, source: Arc<str>, width: u16, light: bool) {
        *self = Self::default();
        self.width = width;
        self.light = light;
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        let math = crate::preview::math::MathMarkdown::deferred(
            &source,
            options,
            Some(width.max(1) as usize),
        );
        let events = math.events(Parser::new_ext(&math.markdown, options).into_offset_iter());
        let mut depth = 0usize;
        let mut block = Vec::new();
        for (event, range) in events {
            match &event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth = depth.saturating_sub(1),
                _ => {}
            }
            // Only split where Writer has flushed all pending inline content and containers.
            let boundary = depth == 0
                && matches!(
                    event,
                    Event::End(
                        TagEnd::Paragraph
                            | TagEnd::Heading(_)
                            | TagEnd::List(_)
                            | TagEnd::BlockQuote(_)
                            | TagEnd::CodeBlock
                            | TagEnd::Table
                    ) | Event::Rule
                );
            block.push((event.into_static(), range));
            if boundary {
                self.blocks.push(Block {
                    events: std::mem::take(&mut block),
                    rows: 0..0,
                });
            }
        }
        if !block.is_empty() {
            self.blocks.push(Block {
                events: block,
                rows: 0..0,
            });
        }
        self.display_ranges = math.display_ranges.clone();
        self.stats.indexed_blocks = self.blocks.len();
        self.source = Some(source);
    }
    fn layout(&mut self, index: usize) -> Arc<Vec<Row>> {
        if let Some(at) = self.cached.iter().position(|(i, _)| *i == index) {
            self.stats.cache_hits += 1;
            let pair = self.cached.remove(at).unwrap();
            let rows = pair.1.clone();
            self.cached.push_back(pair);
            return rows;
        }
        let source = self.source.as_ref().unwrap();
        let events = &self.blocks[index].events;
        let start = events.iter().map(|(_, r)| r.start).min().unwrap_or(0);
        let end = events.iter().map(|(_, r)| r.end).max().unwrap_or(start);
        let math = crate::preview::math::MathMarkdown::new(
            &source[start..end],
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
            Some(self.width.max(1) as usize),
        );
        let events = events.iter().cloned().map(|(event, range)| {
            if matches!(event, Event::Text(_))
                && let Some((_, text)) = math
                    .replacements
                    .iter()
                    .find(|(r, _)| r.start + start == range.start && r.end + start == range.end)
            {
                (Event::Text(text.clone().into()), range)
            } else {
                (event, range)
            }
        });
        let output = render_events(
            self.source.as_ref().unwrap(),
            self.width.max(1) as usize,
            self.light,
            events,
            &self.display_ranges,
            false,
        );
        self.stats.laid_out_blocks += 1;
        for note in output.notes {
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        let rows = Arc::new(output.rows);
        self.cached.push_back((index, rows.clone()));
        while self.cached.len() > 1
            && (self.cached.len() > 64
                || self.cached.iter().map(|(_, r)| r.len()).sum::<usize>() > 512)
        {
            self.cached.pop_front();
        }
        rows
    }
    pub fn render(
        &mut self,
        source: Arc<str>,
        width: u16,
        light: bool,
        viewport: CodeViewport,
        all: bool,
    ) -> Rendered {
        let _perf = crate::perf::span(crate::perf::Metric::MarkdownViewport);
        if self
            .source
            .as_ref()
            .is_none_or(|s| !Arc::ptr_eq(s, &source))
            || self.width != width
            || self.light != light
        {
            self.reset(source.clone(), width, light);
        }
        let wanted = if all {
            usize::MAX
        } else {
            viewport
                .top
                .saturating_add(viewport.rows)
                .saturating_add(16)
        };
        while !self.done && self.plain.len() < wanted {
            if self.next == self.blocks.len() {
                self.done = true;
                break;
            }
            let block = self.layout(self.next);
            let start = self.plain.len();
            for row in block.iter() {
                if self.plain.len() >= crate::preview::MAX_RENDER_ROWS
                    || self.spans >= crate::preview::MAX_SPANS
                {
                    self.done = true;
                    self.notes.push("Markdown layout budget exceeded; remaining content available in source mode".into());
                    break;
                }
                self.spans += row.line.spans.len();
                let plain = row.line.to_string();
                Arc::make_mut(&mut self.neutral)
                    .lines
                    .push(Line::raw(plain.clone()));
                Arc::make_mut(&mut self.plain).push(plain);
                Arc::make_mut(&mut self.rows).push(row.source.clone());
            }
            self.blocks[self.next].rows = start..self.plain.len();
            self.next += 1;
            self.done |= self.next == self.blocks.len();
        }
        if self.done {
            while self.plain.last().is_some_and(|s| s.is_empty()) {
                Arc::make_mut(&mut self.plain).pop();
                Arc::make_mut(&mut self.rows).pop();
                Arc::make_mut(&mut self.neutral).lines.pop();
            }
            if self.plain.is_empty() {
                Arc::make_mut(&mut self.plain).push(String::new());
                Arc::make_mut(&mut self.rows).push(RowSource {
                    kind: crate::preview::RowKind::Prose,
                    segments: Vec::new(),
                });
                Arc::make_mut(&mut self.neutral).lines.push(Line::default());
            }
        }
        let top = viewport.top.min(self.plain.len().saturating_sub(1));
        let wanted = top.saturating_sub(16)
            ..top
                .saturating_add(viewport.rows)
                .saturating_add(16)
                .min(self.plain.len());
        let mut styled = Vec::new();
        let mut first = None;
        for index in 0..self.next {
            let range = self.blocks[index].rows.clone();
            if range.end <= wanted.start || range.start >= wanted.end {
                continue;
            }
            let block = self.layout(index);
            let from = wanted.start.max(range.start);
            let to = wanted.end.min(range.end);
            first.get_or_insert(from);
            styled.extend(
                block
                    .iter()
                    .skip(from - range.start)
                    .take(to - from)
                    .map(|r| r.line.clone()),
            );
        }
        let first = first.unwrap_or(0);
        if styled.is_empty() && self.plain.len() == 1 && self.plain[0].is_empty() {
            styled.push(Line::default());
        }
        let window = Arc::new(HighlightWindow {
            first,
            range: first..first + styled.len(),
            text: Text::from(styled),
            light,
            total_rows: self.plain.len(),
        });
        self.stats.cached_blocks = self.cached.len();
        self.stats.cached_rows = self.cached.iter().map(|(_, r)| r.len()).sum();
        self.stats.known_rows = self.plain.len();
        self.stats.complete = self.done;
        Rendered {
            text: self.neutral.clone(),
            window: Some(window),
            plain: self.plain.clone(),
            source,
            rows: self.rows.clone(),
            complete: self.done,
            width,
            markdown: true,
            light,
            note: self.notes.join("; "),
        }
    }
}
