#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: u16,
    pub col: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub start: Point,
    pub end: Point,
}
impl Selection {
    pub fn bounds(self) -> (Point, Point) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
    pub fn contains(self, point: Point) -> bool {
        let (start, end) = self.bounds();
        point >= start && point <= end
    }
    pub fn text(self, screen: &vt100::Screen) -> String {
        let (start, end) = self.bounds();
        let cols = screen.size().1.get();
        let mut text = String::new();
        for row in start.row..=end.row {
            let left = if row == start.row { start.col } else { 0 };
            let right = if row == end.row {
                end.col.saturating_add(1).min(cols)
            } else {
                cols
            };
            let mut line = String::new();
            for col in left..right {
                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    line.push_str(if cell.has_contents() {
                        cell.contents()
                    } else {
                        " "
                    });
                }
            }
            text.push_str(line.trim_end());
            if row != end.row && !screen.row_wrapped(row) {
                text.push('\n');
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_extracts_only_its_pane_and_keeps_wide_text() {
        let mut terminal = crate::terminal::Terminal::new(5, 20, 20);
        terminal.process("你好 world\r\nnext".as_bytes());
        let select = Selection {
            start: Point { row: 1, col: 3 },
            end: Point { row: 0, col: 0 },
        };
        assert_eq!(select.text(terminal.screen()), "你好 world\nnext");
    }
}
