//! Styled wrapping with exact byte ranges where text remains literal.
use super::{RowKind, RowSource, SourceSegment};
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(crate) fn display_width(text: &str) -> usize {
    text.width()
        + text
            .chars()
            .filter(|ch| matches!(ch, '\u{FF9E}' | '\u{FF9F}'))
            .count()
}

#[derive(Clone)]
pub(super) struct Piece {
    pub text: String,
    pub style: Style,
    pub source: Option<Range<usize>>,
    pub exact: bool,
}
impl Piece {
    pub fn new(text: String, style: Style, source: Option<Range<usize>>, exact: bool) -> Self {
        Self {
            text,
            style,
            source,
            exact,
        }
    }
    pub fn synthetic(text: impl Into<String>, style: Style) -> Self {
        Self::new(text.into(), style, None, false)
    }
}

/// Sanitization can expand tabs or remove CR. Keep neighboring literal ranges
/// exact so selecting one token does not copy the whole transformed code block.
pub(super) fn literal(text: &str, style: Style, start: usize) -> Vec<Piece> {
    if !text.chars().any(|c| c.is_control() && c != '\n') {
        return vec![Piece::new(
            text.into(),
            style,
            Some(start..start + text.len()),
            true,
        )];
    }
    let mut out = Vec::new();
    let mut pending = 0;
    let mut col = 0;
    for (i, c) in text.char_indices() {
        if c.is_control() && c != '\n' {
            if pending < i {
                out.push(Piece::new(
                    text[pending..i].into(),
                    style,
                    Some(start + pending..start + i),
                    true,
                ));
            }
            let value = match c {
                '\r' => String::new(),
                '\t' => {
                    let n = 4 - col % 4;
                    col += n;
                    " ".repeat(n)
                }
                _ => {
                    col += 1;
                    "�".into()
                }
            };
            if !value.is_empty() {
                out.push(Piece::new(
                    value,
                    style,
                    Some(start + i..start + i + c.len_utf8()),
                    false,
                ));
            }
            pending = i + c.len_utf8();
        } else if c == '\n' {
            col = 0;
        } else {
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        }
    }
    if pending < text.len() {
        out.push(Piece::new(
            text[pending..].into(),
            style,
            Some(start + pending..start + text.len()),
            true,
        ));
    }
    out
}

pub(super) struct Row {
    pub line: Line<'static>,
    pub source: RowSource,
}

pub(super) fn slice(pieces: &[Piece], range: Range<usize>) -> Vec<Piece> {
    let mut offset = 0;
    pieces
        .iter()
        .filter_map(|p| {
            let start = range.start.max(offset);
            let end = range.end.min(offset + p.text.len());
            let out = (start < end).then(|| {
                let source = p.source.as_ref().map(|source| {
                    if p.exact {
                        source.start + start - offset..source.start + end - offset
                    } else {
                        source.clone()
                    }
                });
                Piece::new(
                    p.text[start - offset..end - offset].into(),
                    p.style,
                    source,
                    p.exact,
                )
            });
            offset += p.text.len();
            out
        })
        .collect()
}

pub(super) fn row(pieces: Vec<Piece>, kind: RowKind, base: Style) -> Row {
    let mut chars = 0;
    let mut segments = Vec::new();
    let spans = pieces
        .into_iter()
        .filter(|p| !p.text.is_empty())
        .map(|p| {
            let end = chars + p.text.chars().count();
            if let Some(source) = p.source {
                segments.push(SourceSegment {
                    display: chars..end,
                    source,
                    exact: p.exact,
                });
            }
            chars = end;
            Span::styled(p.text, base.patch(p.style))
        })
        .collect::<Vec<_>>();
    Row {
        line: Line::from(spans).style(base),
        source: RowSource {
            kind,
            segments,
            code_language: None,
        },
    }
}

pub(super) fn wrap(
    pieces: &[Piece],
    first: &[Piece],
    rest: &[Piece],
    width: usize,
    kind: RowKind,
    base: Style,
) -> Vec<Row> {
    let flat = pieces.iter().map(|p| p.text.as_str()).collect::<String>();
    let prefix_width = first
        .iter()
        .map(|p| display_width(&p.text))
        .sum::<usize>()
        .max(rest.iter().map(|p| display_width(&p.text)).sum::<usize>());
    let available = width.saturating_sub(prefix_width).max(1);
    if prefix_width >= width {
        return vec![fit(row(first.to_vec(), kind, base), width)];
    }
    let options = textwrap::Options::new(available)
        .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit)
        .word_separator(textwrap::WordSeparator::Custom(words))
        .break_words(false)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);
    let wrapped = textwrap::wrap(&flat, options);
    let mut offset = 0;
    let mut rows = Vec::new();
    for part in &wrapped {
        if rows.len() >= super::MAX_RENDER_ROWS {
            break;
        }
        if part.is_empty() && !flat.is_empty() {
            continue;
        }
        let start = offset + flat[offset..].find(part.as_ref()).unwrap_or(0);
        let end = start + part.len();
        offset = end;
        let mut chunk_start = start;
        let mut used = 0;
        for (index, g) in flat[start..end].grapheme_indices(true) {
            if rows.len() >= super::MAX_RENDER_ROWS {
                break;
            }
            let w = display_width(g);
            if used + w > available && used > 0 {
                let mut out = if rows.is_empty() {
                    first.to_vec()
                } else {
                    rest.to_vec()
                };
                out.extend(slice(pieces, chunk_start..start + index));
                rows.push(fit(row(out, kind, base), width));
                chunk_start = start + index;
                used = 0;
            }
            used += w;
        }
        let mut out = if rows.is_empty() {
            first.to_vec()
        } else {
            rest.to_vec()
        };
        out.extend(slice(pieces, chunk_start..end));
        if rows.len() < super::MAX_RENDER_ROWS {
            rows.push(fit(row(out, kind, base), width));
        }
    }
    rows
}

fn words(line: &str) -> Box<dyn Iterator<Item = textwrap::core::Word<'_>> + '_> {
    let mut start = 0;
    Box::new(unicode_linebreak::linebreaks(line).map(move |(end, _)| {
        let word = textwrap::core::Word::from(&line[start..end]);
        start = end;
        word
    }))
}

// A one-column viewport cannot fit a two-column grapheme. Replace it explicitly,
// retaining its source range; never emit half of a grapheme or overflow the row.
fn fit(input: Row, width: usize) -> Row {
    if display_width(&input.line.to_string()) <= width {
        return input;
    }
    let mut col = 0;
    let mut chars = 0;
    let mut pieces = Vec::new();
    for span in &input.line.spans {
        let mapping = input.source.segments.iter().find(|s| {
            s.display.start == chars && s.display.end == chars + span.content.chars().count()
        });
        for (byte, g) in span.content.grapheme_indices(true) {
            if col >= width {
                break;
            }
            let w = display_width(g);
            let fits = col + w <= width;
            let source = mapping.map(|m| {
                if m.exact {
                    m.source.start + byte..m.source.start + byte + g.len()
                } else {
                    m.source.clone()
                }
            });
            pieces.push(Piece::new(
                if fits { g.to_owned() } else { "�".into() },
                span.style,
                source,
                mapping.is_some_and(|m| m.exact) && fits,
            ));
            col += if fits { w } else { 1 };
        }
        chars += span.content.chars().count();
    }
    row(pieces, input.source.kind, input.line.style)
}
