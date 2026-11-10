//! Preserve Windows console records that Crossterm's Windows event reader loses:
//! repeat counts, scan codes and the original Unicode/control-key fields.
use crossterm::event::Event;

pub struct Input {
    pub event: Event,
    pub win32: Option<Vec<u8>>,
    pub repeat: usize,
}

#[cfg(windows)]
mod windows {
    use super::Input;
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use std::{
        collections::VecDeque,
        io,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0},
        System::{Console::*, Threading::WaitForSingleObject},
    };

    pub struct Events {
        handle: HANDLE,
        queued: VecDeque<Input>,
        surrogate: Option<(u16, Vec<u8>)>,
        buttons: u32,
        opening: Vec<Input>,
        pending_at: Option<Instant>,
        paste: Option<String>,
        paste_ending: String,
        paste_overflow: bool,
    }
    fn modifiers(state: u32) -> KeyModifiers {
        let mut result = KeyModifiers::empty();
        if state & SHIFT_PRESSED != 0 {
            result |= KeyModifiers::SHIFT;
        }
        if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
            result |= KeyModifiers::CONTROL;
        }
        if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
            result |= KeyModifiers::ALT;
        }
        result
    }
    impl Events {
        pub fn new() -> io::Result<Self> {
            let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                handle,
                queued: VecDeque::new(),
                surrogate: None,
                buttons: 0,
                opening: vec![],
                pending_at: None,
                paste: None,
                paste_ending: String::new(),
                paste_overflow: false,
            })
        }
        pub fn read(&mut self, timeout: Duration) -> io::Result<Option<Input>> {
            let started = Instant::now();
            loop {
                if self
                    .pending_at
                    .is_some_and(|at| at.elapsed() >= Duration::from_millis(40))
                {
                    self.queued.extend(self.opening.drain(..));
                    self.pending_at = None;
                }
                if let Some(event) = self.queued.pop_front() {
                    return Ok(Some(event));
                }
                let remaining = timeout
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(20));
                let wait =
                    unsafe { WaitForSingleObject(self.handle, remaining.as_millis() as u32) };
                if wait == WAIT_FAILED {
                    return Err(io::Error::last_os_error());
                }
                if wait == WAIT_OBJECT_0 {
                    let mut records = [INPUT_RECORD::default(); 128];
                    let mut count = 0;
                    if unsafe {
                        ReadConsoleInputW(
                            self.handle,
                            records.as_mut_ptr(),
                            records.len() as u32,
                            &mut count,
                        )
                    } == 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    for record in &records[..count as usize] {
                        self.record(record)?;
                    }
                }
                if started.elapsed() >= timeout && self.queued.is_empty() {
                    return Ok(None);
                }
            }
        }
        fn record(&mut self, record: &INPUT_RECORD) -> io::Result<()> {
            // SAFETY: the tag selects the initialized INPUT_RECORD union field.
            match u32::from(record.EventType) {
                KEY_EVENT => unsafe { self.key(record.Event.KeyEvent) },
                MOUSE_EVENT => unsafe { self.mouse(record.Event.MouseEvent) },
                WINDOW_BUFFER_SIZE_EVENT => {
                    let (cols, rows) = crossterm::terminal::size()?;
                    self.queued.push_back(Input {
                        event: Event::Resize(cols, rows),
                        win32: None,
                        repeat: 1,
                    });
                }
                FOCUS_EVENT => {
                    let focused = unsafe { record.Event.FocusEvent.bSetFocus } != 0;
                    self.queued.push_back(Input {
                        event: if focused {
                            Event::FocusGained
                        } else {
                            Event::FocusLost
                        },
                        win32: None,
                        repeat: 1,
                    });
                }
                _ => {}
            }
            Ok(())
        }
        fn key(&mut self, raw: KEY_EVENT_RECORD) {
            let unit = unsafe { raw.uChar.UnicodeChar };
            let mut bytes = format!(
                "\x1b[{};{};{};{};{};{}_",
                raw.wVirtualKeyCode,
                raw.wVirtualScanCode,
                unit,
                u8::from(raw.bKeyDown != 0),
                raw.dwControlKeyState,
                raw.wRepeatCount.max(1)
            )
            .into_bytes();
            if (0xd800..=0xdbff).contains(&unit) {
                self.surrogate = Some((unit, bytes));
                return;
            }
            let character = if (0xdc00..=0xdfff).contains(&unit) {
                let Some((high, mut prefix)) = self.surrogate.take() else {
                    return;
                };
                prefix.append(&mut bytes);
                bytes = prefix;
                char::from_u32(
                    0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00),
                )
                .unwrap_or('\u{fffd}')
            } else {
                char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')
            };
            let mods = modifiers(raw.dwControlKeyState);
            let code = match raw.wVirtualKeyCode {
                8 => KeyCode::Backspace,
                9 if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
                9 => KeyCode::Tab,
                13 => KeyCode::Enter,
                27 => KeyCode::Esc,
                33 => KeyCode::PageUp,
                34 => KeyCode::PageDown,
                35 => KeyCode::End,
                36 => KeyCode::Home,
                37 => KeyCode::Left,
                38 => KeyCode::Up,
                39 => KeyCode::Right,
                40 => KeyCode::Down,
                45 => KeyCode::Insert,
                46 => KeyCode::Delete,
                n @ 112..=135 => KeyCode::F((n - 111) as u8),
                16..=18 | 20 | 144 | 145 => return,
                n @ 65..=90 if character.is_control() => {
                    KeyCode::Char(char::from(n as u8).to_ascii_lowercase())
                }
                32 if character == '\0' => KeyCode::Char(' '),
                _ if character == '\u{1b}' => KeyCode::Esc,
                _ if character == '\r' => KeyCode::Enter,
                _ if character == '\t' => KeyCode::Tab,
                _ if character == '\0' => return,
                _ => KeyCode::Char(character),
            };
            let key = KeyEvent::new_with_kind(
                code,
                mods,
                if raw.bKeyDown != 0 {
                    KeyEventKind::Press
                } else {
                    KeyEventKind::Release
                },
            );
            self.paste_key(Input {
                event: Event::Key(key),
                win32: Some(bytes),
                repeat: usize::from(raw.wRepeatCount.max(1)),
            });
        }
        fn paste_key(&mut self, input: Input) {
            let Event::Key(key) = input.event else {
                return;
            };
            // Windows Terminal delivers bracketed paste as console key records.
            // Assemble it before routing so embedded applications receive ONE paste.
            let text = match key.code {
                KeyCode::Char(c) => Some(c),
                KeyCode::Esc => Some('\x1b'),
                KeyCode::Enter => Some('\r'),
                KeyCode::Tab => Some('\t'),
                _ => None,
            };
            if self.paste.is_some() {
                if key.kind == KeyEventKind::Release {
                    return;
                }
                if let Some(c) = text {
                    for _ in 0..input.repeat {
                        self.paste_ending.push(c);
                        while !"\x1b[201~".starts_with(&self.paste_ending) {
                            let first = self.paste_ending.remove(0);
                            let paste = self.paste.as_mut().unwrap();
                            if paste.len() + first.len_utf8() <= crate::input::MAX_PASTE {
                                paste.push(first);
                            } else {
                                self.paste_overflow = true;
                            }
                        }
                        if self.paste_ending == "\x1b[201~" {
                            let mut paste = self.paste.take().unwrap();
                            // Route an oversized paste through the normal explicit error path.
                            if self.paste_overflow {
                                paste.push_str("oversized");
                            }
                            self.queued.push_back(Input {
                                event: Event::Paste(paste),
                                win32: None,
                                repeat: 1,
                            });
                            self.paste_ending.clear();
                            self.paste_overflow = false;
                            break;
                        }
                    }
                }
                return;
            }
            if self.opening.is_empty() && text != Some('\x1b') {
                self.queued.push_back(input);
                return;
            }
            if key.kind == KeyEventKind::Release {
                if self.opening.is_empty() {
                    self.queued.push_back(input);
                }
                return;
            }
            self.pending_at.get_or_insert_with(Instant::now);
            self.opening.push(input);
            let candidate: String = self
                .opening
                .iter()
                .flat_map(|e| {
                    std::iter::repeat_n(
                        match &e.event {
                            Event::Key(k) => match k.code {
                                KeyCode::Esc => Some('\x1b'),
                                KeyCode::Char(c) => Some(c),
                                _ => None,
                            },
                            _ => None,
                        },
                        e.repeat,
                    )
                    .flatten()
                })
                .collect();
            if candidate == "\x1b[200~" {
                self.opening.clear();
                self.pending_at = None;
                self.paste = Some(String::new());
            } else if !"\x1b[200~".starts_with(&candidate) {
                self.queued.extend(self.opening.drain(..));
                self.pending_at = None;
            }
        }
        fn mouse(&mut self, raw: MOUSE_EVENT_RECORD) {
            let previous = self.buttons;
            self.buttons = raw.dwButtonState & 0xffff;
            let button = |bits: u32| {
                if bits & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
                    MouseButton::Left
                } else if bits & RIGHTMOST_BUTTON_PRESSED != 0 {
                    MouseButton::Right
                } else {
                    MouseButton::Middle
                }
            };
            let kind = match raw.dwEventFlags {
                MOUSE_WHEELED => {
                    if (raw.dwButtonState >> 16) as i16 > 0 {
                        MouseEventKind::ScrollUp
                    } else {
                        MouseEventKind::ScrollDown
                    }
                }
                MOUSE_HWHEELED => {
                    if (raw.dwButtonState >> 16) as i16 > 0 {
                        MouseEventKind::ScrollRight
                    } else {
                        MouseEventKind::ScrollLeft
                    }
                }
                MOUSE_MOVED if self.buttons != 0 => MouseEventKind::Drag(button(self.buttons)),
                MOUSE_MOVED => MouseEventKind::Moved,
                _ if self.buttons & !previous != 0 => {
                    MouseEventKind::Down(button(self.buttons & !previous))
                }
                _ if previous & !self.buttons != 0 => {
                    MouseEventKind::Up(button(previous & !self.buttons))
                }
                _ => return,
            };
            let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
            let top = if unsafe {
                GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info)
            } != 0
            {
                info.srWindow.Top
            } else {
                0
            };
            self.queued.push_back(Input {
                event: Event::Mouse(MouseEvent {
                    kind,
                    column: raw.dwMousePosition.X.max(0) as u16,
                    row: raw.dwMousePosition.Y.saturating_sub(top).max(0) as u16,
                    modifiers: modifiers(raw.dwControlKeyState),
                }),
                win32: None,
                repeat: 1,
            });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn raw(unit: u16, vk: u16, repeat: u16) -> KEY_EVENT_RECORD {
            KEY_EVENT_RECORD {
                bKeyDown: 1,
                wRepeatCount: repeat,
                wVirtualKeyCode: vk,
                wVirtualScanCode: 31,
                uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                dwControlKeyState: 0,
            }
        }
        #[test]
        fn repeats_scancodes_and_surrogate_pairs_are_preserved() {
            let mut source = Events::new().unwrap();
            source.key(raw(b'a' as u16, 65, 4));
            let event = source.queued.pop_front().unwrap();
            assert_eq!(event.repeat, 4);
            assert_eq!(event.win32.unwrap(), b"\x1b[65;31;97;1;0;4_");
            for unit in "😀".encode_utf16() {
                source.key(raw(unit, 0, 1));
            }
            let event = source.queued.pop_front().unwrap();
            assert!(matches!(
                event.event,
                Event::Key(KeyEvent {
                    code: KeyCode::Char('😀'),
                    ..
                })
            ));
            assert_eq!(
                event.win32.unwrap(),
                b"\x1b[0;31;55357;1;0;1_\x1b[0;31;56832;1;0;1_"
            );
        }
        #[test]
        fn windows_bracketed_paste_becomes_one_unicode_event() {
            let mut source = Events::new().unwrap();
            for unit in "\x1b[200~中文\r\naaa\t😀\x1b[201~".encode_utf16() {
                source.key(raw(unit, 0, 1));
            }
            assert_eq!(source.queued.len(), 1);
            assert_eq!(
                source.queued.pop_front().unwrap().event,
                Event::Paste("中文\r\naaa\t😀".into())
            );
            source.key(raw(27, 27, 1));
            source.key(raw(b'x' as u16, 88, 1));
            assert_eq!(
                source.queued.len(),
                2,
                "ordinary Esc must remain an application key"
            );
        }
        #[test]
        fn coalesced_console_repeats_in_paste_delimiter_are_expanded() {
            let mut source = Events::new().unwrap();
            for unit in "\x1b[2".encode_utf16() {
                source.key(raw(unit, 0, 1));
            }
            source.key(raw(b'0' as u16, 48, 2));
            for unit in "~👩‍💻e\u{301}\x1b[201~".encode_utf16() {
                source.key(raw(unit, 0, 1));
            }
            assert_eq!(
                source.queued.pop_front().unwrap().event,
                Event::Paste("👩‍💻e\u{301}".into())
            );
            assert!(source.queued.is_empty());
        }
    }
}
#[cfg(windows)]
pub use windows::Events;

#[cfg(not(windows))]
pub struct Events;
#[cfg(not(windows))]
impl Events {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
    pub fn read(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<Input>> {
        if crossterm::event::poll(timeout)? {
            Ok(Some(Input {
                event: crossterm::event::read()?,
                win32: None,
                repeat: 1,
            }))
        } else {
            Ok(None)
        }
    }
}
