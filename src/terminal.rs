use std::num::NonZeroU16;

use ratatui::{
    buffer::Cell,
    style::{Color, Modifier, Style},
};
use tui_term::widget::{Cell as TermCell, Screen as TermScreen};

pub fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value.clamp(1, i16::MAX as u16)).unwrap()
}

#[derive(Default)]
pub struct Responses {
    bytes: Vec<u8>,
    pub focus_events: bool,
    pub win32_input: bool,
}

impl Responses {
    fn reply(&mut self, value: &str) {
        // Bound responses even when an untrusted process floods queries.
        if self.bytes.len() + value.len() <= 65536 {
            self.bytes.extend_from_slice(value.as_bytes());
        }
    }
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

impl vt100::Callbacks for Responses {
    fn unhandled_escape(&mut self, _: &mut vt100::Screen, i1: Option<u8>, _: Option<u8>, b: u8) {
        if i1.is_none() && b == b'Z' {
            self.reply("\x1b[?1;2c");
        }
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let p = params.first().and_then(|v| v.first()).copied().unwrap_or(0);
        match (i1, i2, c, p) {
            (None, None, 'n', 5) => self.reply("\x1b[0n"),
            (prefix @ (None | Some(b'?')), None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                let question = if prefix.is_some() { "?" } else { "" };
                self.reply(&format!("\x1b[{question}{};{}R", row + 1, col + 1));
            }
            (None, None, 'c', 0) => self.reply("\x1b[?1;2c"),
            (Some(b'>'), None, 'c', 0) => self.reply("\x1b[>0;1;0c"),
            (Some(b'?'), None, 'u', _) => self.reply("\x1b[?0u"),
            (Some(b'?'), Some(b'$'), 'p', mode) => {
                let state = match mode {
                    1 => Some(screen.application_cursor()),
                    25 => Some(!screen.hide_cursor()),
                    1049 => Some(screen.alternate_screen()),
                    2004 => Some(screen.bracketed_paste()),
                    1004 => Some(self.focus_events),
                    9001 => Some(self.win32_input),
                    _ => None,
                };
                let value = state.map_or(0, |enabled| if enabled { 1 } else { 2 });
                self.reply(&format!("\x1b[?{mode};{value}$y"));
            }
            (Some(b'?'), None, code @ ('h' | 'l'), _) => {
                for mode in params.iter().flat_map(|v| v.iter()) {
                    if *mode == 1004 {
                        self.focus_events = code == 'h';
                    }
                    if *mode == 9001 {
                        self.win32_input = code == 'h';
                    }
                }
            }
            (None, None, 't', 18) => {
                let (rows, cols) = screen.size();
                self.reply(&format!("\x1b[8;{rows};{cols}t"));
            }
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.get(1).copied() == Some(b"?") {
            match params.first().copied() {
                Some(b"10") => self.reply("\x1b]10;rgb:d4d4/d4d4/d4d4\x1b\\"),
                Some(b"11") => self.reply("\x1b]11;rgb:1010/1212/1717\x1b\\"),
                _ => {}
            }
        }
    }
}

pub struct Terminal {
    pub parser: vt100::Parser<Responses>,
}

impl Terminal {
    pub fn new(rows: u16, cols: u16, history: usize) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                nonzero(rows),
                nonzero(cols),
                history,
                Responses::default(),
            ),
        }
    }
    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        let _perf = crate::perf::span(crate::perf::Metric::TerminalParse);
        self.parser.process(bytes);
        self.parser.callbacks_mut().take()
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(nonzero(rows), nonzero(cols));
    }
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub fn scroll(&mut self, delta: isize) {
        let current = self.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_add_signed(delta));
    }
    pub fn live(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::from_screen(self.screen())
    }
}

impl Snapshot {
    pub fn from_screen(screen: &vt100::Screen) -> Self {
        let (rows, cols) = screen.size();
        let mut cells = Vec::with_capacity(usize::from(rows.get()) * usize::from(cols.get()));
        for row in 0..rows.get() {
            for col in 0..cols.get() {
                cells.push(RenderCell(
                    screen
                        .cell(row, col)
                        .expect("cell inside screen dimensions")
                        .clone(),
                ));
            }
        }
        let (row, col) = screen.cursor_position();
        Self {
            cols: cols.get(),
            cells,
            cursor: (
                row.saturating_add(u16::try_from(screen.scrollback()).unwrap_or(u16::MAX)),
                col,
            ),
            hidden: screen.hide_cursor() || screen.scrollback() > 0,
        }
    }
}

pub struct RenderCell(vt100::Cell);
impl TermCell for RenderCell {
    fn has_contents(&self) -> bool {
        self.0.has_contents()
    }
    fn apply(&self, cell: &mut Cell) {
        let c = &self.0;
        cell.set_symbol(if c.has_contents() { c.contents() } else { " " });
        let mut mods = Modifier::empty();
        for (enabled, flag) in [
            (c.bold(), Modifier::BOLD),
            (c.dim(), Modifier::DIM),
            (c.italic(), Modifier::ITALIC),
            (c.underline(), Modifier::UNDERLINED),
            (c.inverse(), Modifier::REVERSED),
        ] {
            if enabled {
                mods |= flag;
            }
        }
        cell.set_style(
            Style::reset()
                .fg(color(c.fgcolor(), Color::Rgb(212, 212, 212)))
                .bg(color(c.bgcolor(), Color::Rgb(16, 18, 23)))
                .add_modifier(mods),
        );
    }
}
fn color(c: vt100::Color, default: Color) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub struct Snapshot {
    cols: u16,
    cells: Vec<RenderCell>,
    cursor: (u16, u16),
    hidden: bool,
}
impl TermScreen for Snapshot {
    type C = RenderCell;
    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        if col >= self.cols {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.cols) + usize::from(col))
    }
    fn hide_cursor(&self) -> bool {
        self.hidden
    }
    fn cursor_position(&self) -> (u16, u16) {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_utf8_queries_and_modes() {
        let mut t = Terminal::new(10, 40, 50);
        for b in "你好\x1b[3;7H\x1b[6n".as_bytes() {
            let reply = t.process(&[*b]);
            if *b == b'n' {
                assert_eq!(reply, b"\x1b[3;7R");
            }
        }
        assert_eq!(t.screen().cell(0, 0).unwrap().contents(), "你");
        assert_eq!(
            t.process(b"\x1b[5n\x1b[c\x1b[>c\x1b[?u"),
            b"\x1b[0n\x1b[?1;2c\x1b[>0;1;0c\x1b[?0u"
        );
        t.process(b"\x1b[?2004h");
        assert!(t.screen().bracketed_paste());
        assert_eq!(
            t.process(b"\x1b[?2004$p\x1b[18t"),
            b"\x1b[?2004;1$y\x1b[8;10;40t"
        );
    }

    #[test]
    fn history_is_bounded_and_live_cursor_hidden_while_scrolled() {
        let mut t = Terminal::new(4, 20, 10);
        for _ in 0..100 {
            t.process(b"output\r\n");
        }
        assert_eq!(t.screen().scrollback_len(), 10);
        t.scroll(5);
        assert!(t.snapshot().hide_cursor());
        t.resize(0, 0);
        assert_eq!(t.screen().size(), (nonzero(1), nonzero(1)));
        t.live();
        assert_eq!(t.screen().scrollback(), 0);
    }
}
