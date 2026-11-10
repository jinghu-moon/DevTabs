//! Markdown events become typed, source-backed rows before viewport drawing.
use super::{
    RowKind,
    layout::{self, Piece, Row},
    theme::Palette,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use std::{ops::Range, path::Path};

mod cache;
mod table;
pub use cache::{MarkdownCache, MarkdownStats};

struct Margin {
    first: Vec<Piece>,
    rest: Vec<Piece>,
    pending: bool,
}
struct Code {
    language: String,
    pieces: Vec<Piece>,
    range: Range<usize>,
    fenced: bool,
}
pub(super) struct Output {
    pub rows: Vec<Row>,
    pub notes: Vec<String>,
}

pub(super) fn render(source: &str, width: usize, light: bool) -> Output {
    let _perf = crate::perf::span(crate::perf::Metric::Markdown);
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let math = super::math::MathMarkdown::new(source, options, Some(width));
    let events = math.events(Parser::new_ext(&math.markdown, options).into_offset_iter());
    render_events(source, width, light, events, &math.display_ranges, true)
}
fn render_events<'a>(
    source: &str,
    width: usize,
    light: bool,
    events: impl IntoIterator<Item = (Event<'a>, Range<usize>)>,
    display_ranges: &[Range<usize>],
    trim: bool,
) -> Output {
    let mut w = Writer {
        source,
        width,
        light,
        palette: Palette::new(light),
        styles: vec![Style::default()],
        margins: Vec::new(),
        lists: Vec::new(),
        pending: Vec::new(),
        rows: Vec::new(),
        code: None,
        table: None,
        notes: Vec::new(),
        spans: 0,
        images: Vec::new(),
    };
    for (event, range) in events {
        if w.rows.len() >= super::MAX_RENDER_ROWS || w.spans > super::MAX_SPANS {
            w.notes.push(
                "Markdown layout budget exceeded; remaining content available in source mode"
                    .into(),
            );
            break;
        }
        if matches!(&event, Event::Text(_)) && display_ranges.contains(&range) {
            if let Event::Text(text) = event {
                w.flush();
                w.blank();
                w.spatial(&text, range, RowKind::Math);
                w.blank();
            }
        } else {
            w.event(event, range);
        }
    }
    w.flush();
    if (w.rows.len() >= super::MAX_RENDER_ROWS || w.spans >= super::MAX_SPANS) && w.notes.is_empty()
    {
        w.notes.push(
            "Markdown layout budget exceeded; remaining content available in source mode".into(),
        );
    }
    while trim && w.rows.last().is_some_and(|r| r.line.to_string().is_empty()) {
        w.rows.pop();
    }
    if trim && w.rows.is_empty() {
        w.push(layout::row(Vec::new(), RowKind::Prose, Style::default()));
    }
    Output {
        rows: w.rows,
        notes: w.notes,
    }
}

struct Writer<'a> {
    source: &'a str,
    width: usize,
    light: bool,
    palette: Palette,
    styles: Vec<Style>,
    margins: Vec<Margin>,
    lists: Vec<Option<u64>>,
    pending: Vec<Piece>,
    rows: Vec<Row>,
    code: Option<Code>,
    table: Option<table::Table>,
    notes: Vec<String>,
    spans: usize,
    images: Vec<(String, Range<usize>)>,
}
impl Writer<'_> {
    fn style(&self) -> Style {
        *self.styles.last().expect("root style")
    }
    fn inline(&mut self, style: Style) {
        self.styles.push(self.style().patch(style));
    }
    fn pop(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }
    fn push(&mut self, row: Row) {
        if self.rows.len() >= super::MAX_RENDER_ROWS || self.spans >= super::MAX_SPANS {
            return;
        }
        self.spans += row.line.spans.len();
        self.rows.push(row);
    }
    fn prefixes(&self) -> (Vec<Piece>, Vec<Piece>) {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for m in &self.margins {
            first.extend(if m.pending {
                m.first.clone()
            } else {
                m.rest.clone()
            });
            rest.extend(m.rest.clone());
        }
        (first, rest)
    }
    fn consume_prefix(&mut self) {
        for m in &mut self.margins {
            m.pending = false;
        }
    }
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pieces = std::mem::take(&mut self.pending);
        let (first, rest) = self.prefixes();
        for row in layout::wrap(
            &pieces,
            &first,
            &rest,
            self.width,
            RowKind::Prose,
            Style::default(),
        ) {
            self.push(row);
        }
        self.consume_prefix();
    }
    fn blank(&mut self) {
        if !self.rows.is_empty()
            && self
                .rows
                .last()
                .is_some_and(|r| !r.line.to_string().trim().is_empty())
        {
            self.push(layout::row(
                Vec::new(),
                RowKind::Decoration,
                Style::default(),
            ));
        }
    }
    fn text(&mut self, text: &str, range: Range<usize>) {
        let pieces = if self.source.get(range.clone()) == Some(text) {
            layout::literal(text, self.style(), range.start)
        } else {
            vec![Piece::new(
                super::safe_text(text),
                self.style(),
                Some(range),
                false,
            )]
        };
        if let Some(code) = &mut self.code {
            code.pieces.extend(pieces);
        } else if let Some(table) = &mut self.table {
            table.cell.extend(pieces);
        } else {
            self.pending.extend(pieces);
        }
    }
    fn spatial(&mut self, text: &str, range: Range<usize>, kind: RowKind) {
        let (first, rest) = self.prefixes();
        let available = self.width.saturating_sub(
            first
                .iter()
                .map(|p| layout::display_width(&p.text))
                .sum::<usize>(),
        );
        if text
            .lines()
            .any(|line| layout::display_width(line) > available)
        {
            let raw = super::safe_text(self.source.get(range.clone()).unwrap_or(text));
            for line in raw.lines() {
                let pieces = [Piece::new(
                    line.into(),
                    self.style(),
                    Some(range.clone()),
                    false,
                )];
                for row in layout::wrap(&pieces, &first, &rest, self.width, kind, Style::default())
                {
                    self.push(row);
                }
            }
            self.consume_prefix();
            return;
        }
        // Spatial diagrams/formulas have already been checked against available width.
        for (i, line) in text.lines().enumerate() {
            let mut out = if i == 0 { first.clone() } else { rest.clone() };
            out.push(Piece::new(
                super::safe_text(line),
                self.style(),
                Some(range.clone()),
                false,
            ));
            self.push(layout::row(out, kind, Style::default()));
        }
        self.consume_prefix();
    }
    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { level, .. } => {
                    self.flush();
                    self.blank();
                    self.inline(self.palette.headings[level as usize - 1]);
                }
                Tag::Emphasis => self.inline(self.palette.emphasis),
                Tag::Strong => self.inline(self.palette.strong),
                Tag::Strikethrough => self.inline(Style::new().add_modifier(Modifier::CROSSED_OUT)),
                Tag::Link { .. } => self.inline(self.palette.link),
                Tag::Image { dest_url, .. } => {
                    self.images.push((dest_url.to_string(), range));
                    self.inline(self.palette.link);
                }
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.blank();
                    let p = vec![Piece::synthetic("│ ", self.palette.quote)];
                    self.margins.push(Margin {
                        first: p.clone(),
                        rest: p,
                        pending: true,
                    });
                    self.inline(self.palette.quote);
                }
                Tag::List(start) => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                    self.lists.push(start);
                }
                Tag::Item => {
                    self.flush();
                    let marker = if let Some(Some(n)) = self.lists.last_mut() {
                        let s = format!("{n}. ");
                        *n = n.saturating_add(1);
                        s
                    } else {
                        "• ".into()
                    };
                    let rest = " ".repeat(layout::display_width(&marker));
                    self.margins.push(Margin {
                        first: vec![Piece::synthetic(
                            marker,
                            self.palette.link.remove_modifier(Modifier::UNDERLINED),
                        )],
                        rest: vec![Piece::synthetic(rest, Style::default())],
                        pending: true,
                    });
                }
                Tag::CodeBlock(kind) => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                    let (language, fenced) = match kind {
                        CodeBlockKind::Fenced(info) => (
                            info.split([',', ' ', '\t']).next().unwrap_or("").to_owned(),
                            true,
                        ),
                        CodeBlockKind::Indented => (String::new(), false),
                    };
                    self.code = Some(Code {
                        language,
                        pieces: Vec::new(),
                        range,
                        fenced,
                    });
                }
                Tag::Table(align) => {
                    self.flush();
                    self.blank();
                    self.table = Some(table::Table::new(align, range));
                }
                Tag::TableHead => {
                    if let Some(t) = &mut self.table {
                        t.head = true;
                    }
                }
                Tag::TableCell => {
                    if let Some(t) = &mut self.table {
                        t.cell.clear();
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                }
                TagEnd::Heading(_) => {
                    self.flush();
                    self.pop();
                    self.blank();
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    self.pop()
                }
                TagEnd::Image => {
                    if let Some((destination, source)) = self.images.pop() {
                        self.text(&format!(" ({destination})"), source);
                    }
                    self.pop();
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.pop();
                    self.margins.pop();
                    self.blank();
                }
                TagEnd::Item => {
                    self.flush();
                    self.margins.pop();
                }
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                    if self.lists.is_empty() {
                        self.blank();
                    }
                }
                TagEnd::CodeBlock => self.end_code(),
                TagEnd::TableCell => {
                    if let Some(t) = &mut self.table {
                        t.finish_cell();
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let Some(t) = &mut self.table {
                        t.finish_row();
                    }
                }
                TagEnd::Table => {
                    if let Some(t) = self.table.take() {
                        let (first, rest) = self.prefixes();
                        for row in t.render(self.width, &first, &rest, self.palette, self.source) {
                            self.push(row);
                        }
                        self.consume_prefix();
                        self.blank();
                    }
                }
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                self.text(&text, range)
            }
            Event::Code(text) => {
                self.inline(self.palette.code);
                self.text(&text, range);
                self.pop();
            }
            Event::SoftBreak => self.text(" ", range),
            Event::HardBreak => {
                if let Some(t) = &mut self.table {
                    t.cell.push(Piece::synthetic(" ", Style::default()));
                } else {
                    self.flush();
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(m) = self.margins.last_mut() {
                    let prefix = m.first.iter().map(|p| p.text.as_str()).collect::<String>();
                    let number = prefix.strip_suffix("• ").unwrap_or(&prefix);
                    let marker = format!("{number}{} ", if checked { "☑" } else { "☐" });
                    m.rest = vec![Piece::synthetic(
                        " ".repeat(layout::display_width(&marker)),
                        Style::default(),
                    )];
                    m.first = vec![Piece::new(
                        marker,
                        self.palette.link.remove_modifier(Modifier::UNDERLINED),
                        Some(range),
                        false,
                    )];
                }
            }
            Event::Rule => {
                self.flush();
                let (first, rest) = self.prefixes();
                let available = self.width.saturating_sub(
                    first
                        .iter()
                        .map(|p| layout::display_width(&p.text))
                        .sum::<usize>(),
                );
                let p = vec![Piece::synthetic(
                    "─".repeat(available.min(60)),
                    self.palette.rule,
                )];
                for row in layout::wrap(
                    &p,
                    &first,
                    &rest,
                    self.width,
                    RowKind::Decoration,
                    Style::default(),
                ) {
                    self.push(row);
                }
                self.blank();
            }
            Event::FootnoteReference(text) => self.text(&format!("[^{text}]"), range),
            _ => {}
        }
    }
    fn end_code(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        let _perf = crate::perf::span(crate::perf::Metric::CodeLayout);
        let body = code
            .pieces
            .iter()
            .map(|p| p.text.as_str())
            .collect::<String>();
        let (first, rest) = self.prefixes();
        let available = self.width.saturating_sub(
            first
                .iter()
                .map(|p| layout::display_width(&p.text))
                .sum::<usize>(),
        );
        if code.language.eq_ignore_ascii_case("mermaid")
            && code.fenced
            && closed_fence(self.source.get(code.range.clone()).unwrap_or(""))
        {
            let _perf = crate::perf::span(crate::perf::Metric::Mermaid);
            match devtabs_mermaid::render_spans(&body, available) {
                Ok(diagram) => {
                    for (i, line) in diagram.into_iter().enumerate() {
                        let mut out = if i == 0 { first.clone() } else { rest.clone() };
                        for span in line {
                            let style = match span.role {
                                devtabs_mermaid::Role::Node => self.palette.strong,
                                devtabs_mermaid::Role::Edge => self.palette.rule,
                                devtabs_mermaid::Role::Text => self.palette.code,
                            };
                            out.push(Piece::new(
                                span.text,
                                style,
                                Some(code.range.clone()),
                                false,
                            ));
                        }
                        self.push(layout::row(out, RowKind::Diagram, Style::default()));
                    }
                    self.consume_prefix();
                    self.blank();
                    return;
                }
                Err(e) => self.notes.push(format!("Mermaid: {e}; showing source")),
            }
        }
        let highlighted = super::highlight(
            &body,
            Path::new(&format!("code.{}", code.language)),
            self.light,
        );
        let mut offset = 0;
        for (i, line) in highlighted.lines.into_iter().enumerate() {
            let mut out = if i == 0 { first.clone() } else { rest.clone() };
            for span in line.spans {
                let end = offset + span.content.len();
                let mut parts = layout::slice(&code.pieces, offset..end);
                for p in &mut parts {
                    p.style = span.style;
                }
                out.extend(parts);
                offset = end;
            }
            // Highlighting omits the newline; retain its source coordinates in the next row.
            if body.as_bytes().get(offset) == Some(&b'\n') {
                offset += 1;
            }
            self.push(layout::row(out, RowKind::Code, self.palette.block));
        }
        if body.is_empty() {
            self.push(layout::row(first, RowKind::Code, self.palette.block));
        }
        self.consume_prefix();
        self.blank();
    }
}

fn closed_fence(source: &str) -> bool {
    let mut lines = source.lines();
    let first = lines.next().unwrap_or("").trim_start();
    let Some(marker) = first.chars().next().filter(|c| matches!(c, '`' | '~')) else {
        return false;
    };
    let count = first.chars().take_while(|c| *c == marker).count();
    let Some(last) = lines.last() else {
        return false;
    };
    let last = last.trim().trim_start_matches('>').trim();
    last.chars().count() >= count && last.chars().all(|c| c == marker)
}
