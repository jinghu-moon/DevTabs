//! Width-aware tables; fragmented tables become field/value records.
use super::super::{
    RowKind,
    layout::{self, Piece, Row},
    theme::Palette,
};
use pulldown_cmark::Alignment;
use ratatui::style::{Modifier, Style};
use std::ops::Range;

type Cell = Vec<Piece>;
pub(super) struct Table {
    pub cell: Cell,
    pub head: bool,
    align: Vec<Alignment>,
    header: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
    current: Vec<Cell>,
    range: Range<usize>,
}
impl Table {
    pub fn new(align: Vec<Alignment>, range: Range<usize>) -> Self {
        Self {
            cell: Vec::new(),
            head: false,
            align,
            header: Vec::new(),
            rows: Vec::new(),
            current: Vec::new(),
            range,
        }
    }
    pub fn finish_cell(&mut self) {
        self.current.push(std::mem::take(&mut self.cell));
    }
    pub fn finish_row(&mut self) {
        let row = std::mem::take(&mut self.current);
        if self.head {
            self.header = row;
            self.head = false;
        } else {
            self.rows.push(row);
        }
    }
    pub fn render(
        self,
        width: usize,
        first: &[Piece],
        rest: &[Piece],
        palette: Palette,
        source: &str,
    ) -> Vec<Row> {
        let _perf = crate::perf::span(crate::perf::Metric::TableLayout);
        let columns = self.header.len();
        if columns == 0 {
            return Vec::new();
        }
        let available = width.saturating_sub(
            first
                .iter()
                .map(|p| layout::display_width(&p.text))
                .sum::<usize>(),
        );
        if columns > 32 || self.rows.len().saturating_mul(columns) > 10_000 {
            let text = source.get(self.range.clone()).unwrap_or("");
            return layout::wrap(
                &[Piece::new(
                    super::super::safe_text(text),
                    Style::default(),
                    Some(self.range),
                    false,
                )],
                first,
                rest,
                width,
                RowKind::Table,
                Style::default(),
            );
        }
        let mut sizes = vec![3; columns];
        let mut floors = vec![3; columns];
        let mut classes = vec![1; columns];
        for row in std::iter::once(&self.header).chain(&self.rows) {
            for (i, cell) in row.iter().take(columns).enumerate() {
                let text = plain(cell);
                sizes[i] = sizes[i].max(layout::display_width(&text));
                let longest = text
                    .split_whitespace()
                    .map(layout::display_width)
                    .max()
                    .unwrap_or(0);
                floors[i] = floors[i].max(longest.min(12));
                if text.contains('/') || text.contains('\\') || text.contains("://") {
                    classes[i] = 0;
                }
            }
        }
        for i in 0..columns {
            if sizes[i] <= 10 {
                classes[i] = 2;
            }
            floors[i] = floors[i].max(layout::display_width(&plain(&self.header[i])).min(8));
        }
        // Keep the record view on very narrow terminals; ordinary previews may pan tables.
        let available = if available >= 40 {
            available.max(
                sizes
                    .iter()
                    .sum::<usize>()
                    .saturating_add((columns - 1) * 3)
                    .min(512),
            )
        } else {
            available
        };
        let content = available.saturating_sub((columns - 1) * 3);
        if floors.iter().sum::<usize>() > content {
            return self.records(width, first, rest, palette);
        }
        let mut amount = sizes.iter().sum::<usize>().saturating_sub(content);
        for class in [0, 1, 2] {
            if amount == 0 {
                break;
            }
            let slack = (0..columns)
                .filter(|i| classes[*i] == class)
                .map(|i| sizes[i] - floors[i])
                .sum::<usize>();
            let remove = amount.min(slack);
            if remove == 0 {
                continue;
            }
            // Find a shared cap in logarithmic time, then distribute the remainder.
            let mut low = 0;
            let mut high = *sizes.iter().max().unwrap_or(&0);
            while low < high {
                let cap = low + (high - low) / 2;
                let freed = (0..columns)
                    .filter(|i| classes[*i] == class)
                    .map(|i| sizes[i].saturating_sub(cap.max(floors[i])))
                    .sum::<usize>();
                if freed > remove {
                    low = cap + 1;
                } else {
                    high = cap;
                }
            }
            let mut freed = 0;
            for i in 0..columns {
                if classes[i] == class {
                    let new = sizes[i].min(low.max(floors[i]));
                    freed += sizes[i] - new;
                    sizes[i] = new;
                }
            }
            let mut remainder = remove - freed;
            for i in 0..columns {
                if remainder > 0 && classes[i] == class && sizes[i] > floors[i] {
                    sizes[i] -= 1;
                    remainder -= 1;
                }
            }
            amount -= remove;
        }
        let mut out = Vec::new();
        let mut header_rows = 0;
        for (index, cells) in std::iter::once(&self.header).chain(&self.rows).enumerate() {
            if out.len() >= super::super::MAX_RENDER_ROWS {
                break;
            }
            let columns = (0..columns)
                .map(|i| {
                    let empty = Vec::new();
                    let cell = cells.get(i).unwrap_or(&empty);
                    layout::wrap(
                        cell,
                        &[],
                        &[],
                        sizes[i],
                        RowKind::Table,
                        if index == 0 {
                            palette.strong
                        } else {
                            Style::default()
                        },
                    )
                })
                .collect::<Vec<_>>();
            let height = columns.iter().map(Vec::len).max().unwrap_or(1);
            for y in 0..height {
                if out.len() >= super::super::MAX_RENDER_ROWS {
                    break;
                }
                let mut pieces = if out.is_empty() {
                    first.to_vec()
                } else {
                    rest.to_vec()
                };
                for (i, col) in columns.iter().enumerate() {
                    if i > 0 {
                        pieces.push(Piece::synthetic(" │ ", palette.rule));
                    }
                    let row = col.get(y);
                    let used = row.map_or(0, |r| layout::display_width(&r.line.to_string()));
                    let padding = sizes[i].saturating_sub(used);
                    let before = match self.align.get(i) {
                        Some(Alignment::Right) => padding,
                        Some(Alignment::Center) => padding / 2,
                        _ => 0,
                    };
                    pieces.push(Piece::synthetic(" ".repeat(before), Style::default()));
                    if let Some(row) = row {
                        pieces.extend(pieces_from_row(row));
                    }
                    pieces.push(Piece::synthetic(
                        " ".repeat(padding - before),
                        Style::default(),
                    ));
                }
                out.push(layout::row(
                    pieces,
                    RowKind::Table,
                    if index == 0 {
                        palette.strong
                    } else {
                        Style::default()
                    },
                ));
            }
            if index == 0 {
                let mut pieces = rest.to_vec();
                pieces.push(Piece::synthetic(
                    sizes
                        .iter()
                        .map(|n| "━".repeat(*n))
                        .collect::<Vec<_>>()
                        .join("━┿━"),
                    palette.rule,
                ));
                out.push(layout::row(pieces, RowKind::Decoration, palette.rule));
                header_rows = out.len();
            }
        }
        for (offset, row) in out.iter_mut().enumerate() {
            row.source.table = Some(super::super::TableRow {
                offset,
                header_rows,
            });
        }
        out
    }
    fn records(&self, width: usize, first: &[Piece], rest: &[Piece], palette: Palette) -> Vec<Row> {
        let mut out = Vec::new();
        for (n, record) in self.rows.iter().enumerate() {
            if out.len() >= super::super::MAX_RENDER_ROWS {
                break;
            }
            if n > 0 {
                out.push(layout::row(
                    rest.to_vec(),
                    RowKind::Decoration,
                    Style::default(),
                ));
            }
            for (i, cell) in record.iter().enumerate() {
                let mut pieces = self.header.get(i).cloned().unwrap_or_default();
                for p in &mut pieces {
                    p.style = p.style.add_modifier(Modifier::BOLD);
                }
                pieces.push(Piece::synthetic(": ", palette.rule));
                pieces.extend(cell.clone());
                out.extend(layout::wrap(
                    &pieces,
                    if out.is_empty() { first } else { rest },
                    rest,
                    width,
                    RowKind::Table,
                    Style::default(),
                ));
            }
        }
        if self.rows.is_empty() {
            for cell in &self.header {
                out.extend(layout::wrap(
                    cell,
                    first,
                    rest,
                    width,
                    RowKind::Table,
                    palette.strong,
                ));
            }
        }
        out
    }
}
fn plain(cell: &[Piece]) -> String {
    cell.iter().map(|p| p.text.as_str()).collect()
}
fn pieces_from_row(row: &Row) -> Vec<Piece> {
    let mut offset = 0;
    row.line
        .spans
        .iter()
        .flat_map(|s| {
            let len = s.content.chars().count();
            let source = row
                .source
                .segments
                .iter()
                .find(|p| p.display.start == offset && p.display.end == offset + len);
            offset += len;
            [Piece::new(
                s.content.to_string(),
                s.style,
                source.map(|p| p.source.clone()),
                source.is_some_and(|p| p.exact),
            )]
        })
        .collect()
}
