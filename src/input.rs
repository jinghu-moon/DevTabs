use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers as Mods, MouseButton, MouseEvent, MouseEventKind,
};

pub const MAX_PASTE: usize = 1024 * 1024;

pub fn paste(text: &str, bracketed: bool) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        text.len() <= MAX_PASTE,
        "Paste exceeds 1 MiB; split it into smaller chunks"
    );
    if bracketed {
        Ok([b"\x1b[200~".as_slice(), text.as_bytes(), b"\x1b[201~"].concat())
    } else {
        Ok(text.replace("\r\n", "\n").replace('\n', "\r").into_bytes())
    }
}

pub fn key(event: KeyEvent, application_cursor: bool, win32_input: bool) -> Vec<u8> {
    if win32_input {
        return win32_key(event);
    }
    if event.kind == KeyEventKind::Release {
        return vec![];
    }
    let m = event.modifiers;
    let modifier = 1
        + u8::from(m.contains(Mods::SHIFT))
        + 2 * u8::from(m.contains(Mods::ALT))
        + 4 * u8::from(m.contains(Mods::CONTROL));
    let mut bytes = match event.code {
        KeyCode::Char(c) if m.contains(Mods::CONTROL) => {
            let c = c.to_ascii_uppercase();
            match c {
                '@'..='_' => vec![(c as u8) & 0x1f],
                ' ' | '2' => vec![0],
                '3' => vec![27],
                '4' => vec![28],
                '5' => vec![29],
                '6' => vec![30],
                '7' | '/' => vec![31],
                '8' | '?' => vec![127],
                _ => c.to_string().into_bytes(),
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => {
            if modifier > 1 {
                format!("\x1b[13;{modifier}u").into_bytes()
            } else {
                vec![13]
            }
        }
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![9],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![27],
        KeyCode::Null => vec![0],
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => {
            let final_char = match event.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}").into_bytes()
            } else {
                format!(
                    "\x1b{}{final_char}",
                    if application_cursor { 'O' } else { '[' }
                )
                .into_bytes()
            }
        }
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match event.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                _ => 6,
            };
            tilde(n, modifier)
        }
        KeyCode::F(n @ 1..=4) => {
            let c = char::from(b'P' + n - 1);
            if modifier == 1 {
                format!("\x1bO{c}").into_bytes()
            } else {
                format!("\x1b[1;{modifier}{c}").into_bytes()
            }
        }
        KeyCode::F(n @ 5..=12) => tilde(
            [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)],
            modifier,
        ),
        _ => vec![],
    };
    if m.contains(Mods::ALT)
        && matches!(
            event.code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Tab
        )
    {
        bytes.insert(0, 27);
    }
    bytes
}

fn tilde(code: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

// Windows Terminal's win32-input-mode preserves modifier combinations that
// legacy control bytes cannot express, including native Agent clipboard keys.
fn win32_key(event: KeyEvent) -> Vec<u8> {
    let (vk, character, enhanced) = match event.code {
        KeyCode::Char(c) => (
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase() as u16
            } else {
                0
            },
            Some(c),
            false,
        ),
        KeyCode::Enter => (13, Some('\r'), false),
        KeyCode::Tab | KeyCode::BackTab => (9, Some('\t'), false),
        KeyCode::Backspace => (8, Some('\u{8}'), false),
        KeyCode::Esc => (27, Some('\u{1b}'), false),
        KeyCode::Left => (37, None, true),
        KeyCode::Up => (38, None, true),
        KeyCode::Right => (39, None, true),
        KeyCode::Down => (40, None, true),
        KeyCode::Home => (36, None, true),
        KeyCode::End => (35, None, true),
        KeyCode::PageUp => (33, None, true),
        KeyCode::PageDown => (34, None, true),
        KeyCode::Insert => (45, None, true),
        KeyCode::Delete => (46, None, true),
        KeyCode::F(n @ 1..=24) => (111 + u16::from(n), None, false),
        _ => return vec![],
    };
    let mut control = if enhanced { 0x100 } else { 0 };
    if event.modifiers.contains(Mods::SHIFT) || event.code == KeyCode::BackTab {
        control |= 0x10;
    }
    if event.modifiers.contains(Mods::CONTROL) {
        control |= 8;
    }
    if event.modifiers.contains(Mods::ALT) {
        control |= 2;
    }
    let down = u8::from(event.kind != KeyEventKind::Release);
    let mut units = [0u16; 2];
    let text = character.map_or(&[0][..], |c| c.encode_utf16(&mut units));
    let mut result = Vec::new();
    for &u in text {
        result.extend_from_slice(format!("\x1b[{vk};0;{u};{down};{control};1_").as_bytes());
    }
    result
}

pub fn mouse(event: MouseEvent, col: u16, row: u16, screen: &vt100::Screen) -> Option<Vec<u8>> {
    use vt100::{MouseProtocolEncoding as Enc, MouseProtocolMode as Mode};
    let mode = screen.mouse_protocol_mode();
    if mode == Mode::None {
        return None;
    }
    let button = |b| -> u16 {
        match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        }
    };
    let (mut code, released) = match event.kind {
        MouseEventKind::Down(b) => (button(b), false),
        MouseEventKind::Up(b) if mode != Mode::Press => (button(b), true),
        MouseEventKind::Drag(b) if matches!(mode, Mode::ButtonMotion | Mode::AnyMotion) => {
            (32 + button(b), false)
        }
        MouseEventKind::Moved if mode == Mode::AnyMotion => (35, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
        _ => return None,
    };
    if event.modifiers.contains(Mods::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(Mods::ALT) {
        code += 8;
    }
    if event.modifiers.contains(Mods::CONTROL) {
        code += 16;
    }
    let (x, y) = (col + 1, row + 1);
    match screen.mouse_protocol_encoding() {
        Enc::Sgr => {
            Some(format!("\x1b[<{code};{x};{y}{}", if released { 'm' } else { 'M' }).into_bytes())
        }
        encoding => {
            if released {
                code = 3;
            }
            let mut bytes = b"\x1b[M".to_vec();
            for n in [code + 32, x + 32, y + 32] {
                if encoding == Enc::Default {
                    if n > 255 {
                        return None;
                    }
                    bytes.push(n as u8);
                } else {
                    bytes.extend_from_slice(char::from_u32(u32::from(n))?.to_string().as_bytes());
                }
            }
            Some(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_arrows_and_paste_preserve_terminal_semantics() {
        assert_eq!(
            key(
                KeyEvent::new(KeyCode::Char('c'), Mods::CONTROL),
                false,
                false
            ),
            [3]
        );
        assert_eq!(
            key(KeyEvent::new(KeyCode::Up, Mods::NONE), true, false),
            b"\x1bOA"
        );
        assert_eq!(
            key(KeyEvent::new(KeyCode::Right, Mods::CONTROL), false, false),
            b"\x1b[1;5C"
        );
        assert_eq!(
            paste("中文\nline", true).unwrap(),
            "\x1b[200~中文\nline\x1b[201~".as_bytes()
        );
        assert_eq!(paste("a\r\nb\nc", false).unwrap(), b"a\rb\rc");
        assert!(paste(&"x".repeat(MAX_PASTE + 1), true).is_err());
    }
    #[test]
    fn win32_mode_preserves_clipboard_modifiers() {
        assert_eq!(
            key(
                KeyEvent::new(KeyCode::Char('v'), Mods::CONTROL | Mods::SHIFT),
                false,
                true
            ),
            b"\x1b[86;0;118;1;24;1_"
        );
        let mut release = KeyEvent::new(KeyCode::Char('a'), Mods::NONE);
        release.kind = KeyEventKind::Release;
        assert!(key(release, false, false).is_empty());
        assert_eq!(key(release, false, true), b"\x1b[65;0;97;0;0;1_");
    }
    #[test]
    fn sgr_mouse_uses_local_coordinates_and_release_suffix() {
        let mut t = crate::terminal::Terminal::new(20, 40, 0);
        t.process(b"\x1b[?1000h\x1b[?1006h");
        let e = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 200,
            row: 100,
            modifiers: Mods::NONE,
        };
        assert_eq!(mouse(e, 3, 5, t.screen()).unwrap(), b"\x1b[<0;4;6m");
    }
}
