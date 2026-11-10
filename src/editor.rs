//! One lossless document buffer. Saving uses a Windows same-volume replacement and
//! optimistic content checks; arbitrary external rename races are explicitly unsupported.
use crate::preview::{self, Encoding, Loaded};
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::Block,
};
use ratatui_textarea::{CursorMove, TextArea};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

const HISTORY_BYTES: usize = 4 * 1024 * 1024;
// Encoded bytes/BOM; the independent read-only preview limit stays at 1 MiB.
pub const MAX_EDIT_BYTES: usize = 512 * 1024;

pub struct Editor {
    pub area: TextArea<'static>,
    pub loaded: Arc<Loaded>,
    pub revision: u64,
    pub saving: bool,
    pub conflict: bool,
    pub status: String,
    pub search_input: Option<String>,
    pub highlight: Option<(u64, Arc<preview::HighlightWindow>)>,
    pub highlight_requested: u64,
    baseline: String,
    histories: usize,
}
impl Editor {
    pub fn new(loaded: impl Into<Arc<Loaded>>) -> Result<Self> {
        let loaded = loaded.into();
        eligible(&loaded.path)?;
        Self::from_loaded(loaded)
    }
    pub(crate) fn from_loaded(loaded: impl Into<Arc<Loaded>>) -> Result<Self> {
        let loaded = loaded.into();
        ensure!(
            loaded.editable && !loaded.truncated,
            "This file is preview-only: {}",
            loaded.note
        );
        let baseline = loaded.source.replace("\r\n", "\n");
        let lines: Vec<String> = baseline.split('\n').map(str::to_owned).collect();
        validate(&lines, loaded.encoding, loaded.newline)?;
        let histories = history_limit(&baseline);
        let mut area = TextArea::new(lines);
        area.set_max_histories(histories);
        area.set_tab_length(4);
        area.set_hard_tab_indent(true);
        area.set_line_number_style(Style::new().fg(Color::DarkGray));
        area.set_cursor_line_style(Style::new());
        area.set_cursor_style(Style::new().bg(Color::Cyan).fg(Color::Black));
        Ok(Self {
            area,
            loaded,
            revision: 1,
            saving: false,
            conflict: false,
            status: String::new(),
            search_input: None,
            highlight: None,
            highlight_requested: 0,
            baseline,
            histories,
        })
    }
    pub fn text(&self) -> String {
        self.area.lines().join("\n")
    }
    pub fn source(&self) -> String {
        self.text().replace('\n', self.loaded.newline)
    }
    pub fn dirty(&self) -> bool {
        let _perf = crate::perf::span(crate::perf::Metric::EditorDirty);
        self.area
            .lines()
            .iter()
            .map(String::as_str)
            .ne(self.baseline.split('\n'))
    }
    fn edit(&mut self, f: impl FnOnce(&mut TextArea<'static>)) -> Result<()> {
        let _perf = crate::perf::span(crate::perf::Metric::EditorEdit);
        // Work on a candidate so a paste/indent is rejected as a whole, including history.
        let mut candidate = {
            let _perf = crate::perf::span(crate::perf::Metric::EditorClone);
            self.area.clone()
        };
        f(&mut candidate);
        validate(candidate.lines(), self.loaded.encoding, self.loaded.newline)?;
        let changed = candidate.lines() != self.area.lines();
        if changed {
            let bytes = candidate.lines().iter().map(String::len).sum::<usize>();
            let histories = history_limit_bytes(bytes);
            if histories < self.histories {
                candidate.set_max_histories(histories);
                self.histories = histories;
            }
            self.revision += 1;
            self.highlight = None;
        }
        self.area = candidate;
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        ensure!(
            !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')),
            "Paste contains control characters"
        );
        if let Some(query) = &mut self.search_input {
            let text = text.replace(['\r', '\n'], "");
            ensure!(
                query.chars().count() + text.chars().count() <= 256,
                "Query exceeds 256 characters"
            );
            query.push_str(&text);
            self.area
                .set_search_pattern(format!("(?i){}", regex::escape(query)))?;
            return Ok(());
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.edit(|a| {
            a.insert_str(text);
        })
    }
    pub fn selected(&self) -> Option<String> {
        let ((a, b), (c, d)) = self.area.selection_range()?;
        let mut result = String::new();
        for row in a..=c {
            if row > a {
                result.push('\n');
            }
            let line = &self.area.lines()[row];
            let start = if row == a { b } else { 0 };
            let end = if row == c { d } else { line.chars().count() };
            result.extend(line.chars().skip(start).take(end.saturating_sub(start)));
        }
        Some(result)
    }
    pub fn copy(&self) -> Result<()> {
        crate::files::copy(
            self.selected()
                .context("Select text first, or use Copy content")?,
        )
    }
    pub fn cut(&mut self) -> Result<()> {
        self.copy()?;
        self.edit(|a| {
            a.cut();
        })
    }
    pub fn undo(&mut self) -> Result<()> {
        self.edit(|a| {
            a.undo();
        })
    }
    pub fn redo(&mut self) -> Result<()> {
        self.edit(|a| {
            a.redo();
        })
    }
    pub fn indent(&mut self, back: bool) -> Result<()> {
        if let Some(((first, _), (last, end))) = self.area.selection_range() {
            let last = if end == 0 && last > first {
                last - 1
            } else {
                last
            };
            self.edit(|a| {
                a.cancel_selection();
                for row in first..=last {
                    a.move_cursor(CursorMove::Jump(row as u16, 0));
                    if back {
                        if a.lines()[row].starts_with('\t') {
                            a.delete_next_char();
                        } else {
                            for _ in 0..4 {
                                if a.lines()[row].starts_with(' ') {
                                    a.delete_next_char();
                                } else {
                                    break;
                                }
                            }
                        }
                    } else {
                        a.insert_str("\t");
                    }
                }
                a.move_cursor(CursorMove::Jump(first as u16, 0));
                a.start_selection();
                a.move_cursor(CursorMove::Jump(
                    last as u16,
                    a.lines()[last].chars().count() as u16,
                ));
            })
        } else if back {
            let row = self.area.cursor().0;
            self.edit(|a| {
                a.move_cursor(CursorMove::Jump(row as u16, 0));
                if a.lines()[row].starts_with('\t') {
                    a.delete_next_char();
                } else {
                    for _ in 0..4 {
                        if a.lines()[row].starts_with(' ') {
                            a.delete_next_char();
                        } else {
                            break;
                        }
                    }
                }
            })
        } else {
            self.edit(|a| {
                a.insert_tab();
            })
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(q) = &mut self.search_input {
            match key.code {
                KeyCode::Esc => {
                    self.search_input = None;
                    let _ = self.area.set_search_pattern("");
                }
                KeyCode::Enter => {
                    self.search_input = None;
                    self.area.search_forward(true);
                }
                KeyCode::Backspace => {
                    q.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && q.chars().count() < 256 =>
                {
                    q.push(c)
                }
                _ => {}
            }
            if let Some(q) = &self.search_input {
                self.area
                    .set_search_pattern(format!("(?i){}", regex::escape(q)))?;
            }
            return Ok(());
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return self.copy(),
                KeyCode::Char('x') => return self.cut(),
                KeyCode::Char('v') => {
                    let text = arboard::Clipboard::new()?.get_text()?;
                    return self.paste(&text);
                }
                KeyCode::Char('z') => return self.undo(),
                KeyCode::Char('y') => return self.redo(),
                KeyCode::Char('a') => {
                    self.area.select_all();
                    return Ok(());
                }
                KeyCode::Char('f') => {
                    self.search_input = Some(String::new());
                    return Ok(());
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => {
                // Navigation/selection cannot change text or exceed its budget.
                // Do not clone document + undo histories for cursor movement.
                self.area.input(key);
                Ok(())
            }
            KeyCode::Esc => {
                self.area.cancel_selection();
                self.area.set_search_pattern("")?;
                Ok(())
            }
            KeyCode::Tab => self.indent(false),
            KeyCode::BackTab => self.indent(true),
            KeyCode::F(3) => {
                self.area.search_forward(false);
                Ok(())
            }
            KeyCode::F(4) => {
                self.area.search_back(false);
                Ok(())
            }
            _ => self.edit(|a| {
                a.input_without_shortcuts(key);
            }),
        }
    }
    pub fn mouse(&mut self, event: MouseEvent, rect: Rect) {
        let inner = crate::ui::content(rect);
        if !inner.contains((event.column, event.row).into()) {
            return;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left) => {
                let (row, col) = self.area.scroll_offset();
                let cursor = self.area.screen_to_data(
                    (event.row - inner.y + row) as usize,
                    (event.column - inner.x + col).saturating_sub(self.area.line_number_width())
                        as usize,
                );
                if matches!(event.kind, MouseEventKind::Down(_)) {
                    self.area.cancel_selection();
                    self.area
                        .move_cursor(CursorMove::Jump(cursor.0 as u16, cursor.1 as u16));
                    self.area.start_selection();
                } else {
                    self.area
                        .move_cursor(CursorMove::Jump(cursor.0 as u16, cursor.1 as u16));
                }
            }
            MouseEventKind::ScrollUp => self.area.scroll((-3, 0)),
            MouseEventKind::ScrollDown => self.area.scroll((3, 0)),
            _ => {}
        }
    }
    pub fn draw(&self, frame: &mut Frame, rect: Rect, focused: bool) {
        let _perf = crate::perf::span(crate::perf::Metric::EditorDraw);
        let title = format!(
            " {}{} · EDIT{}{} [F2] ",
            self.loaded
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            if self.dirty() { " *" } else { "" },
            if self.saving { " · saving" } else { "" },
            if self.conflict { " · CONFLICT" } else { "" }
        );
        frame.render_widget(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(if focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })),
            rect,
        );
        let inner = crate::ui::content(rect);
        frame.render_widget(&self.area, inner);
        // The component owns all text geometry/selection; optional foreground colors
        // never replace its buffer or alter selected/cursor cells.
        if let Some((revision, syntax)) = &self.highlight
            && *revision == self.revision
        {
            let (top, left) = self.area.scroll_offset();
            let gutter = self.area.line_number_width() as usize;
            for y in 0..inner.height {
                let Some(line) = syntax.line(top as usize + y as usize) else {
                    continue;
                };
                let mut col = 0;
                for span in &line.spans {
                    for ch in span.content.chars() {
                        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                        for dx in 0..width {
                            let x = col + dx + gutter;
                            if x >= left as usize && x - (left as usize) < inner.width as usize {
                                let cell = &mut frame.buffer_mut()
                                    [(inner.x + (x - left as usize) as u16, inner.y + y)];
                                if cell.bg == Color::Reset
                                    && let Some(fg) = span.style.fg
                                {
                                    cell.set_fg(fg);
                                }
                            }
                        }
                        col += width;
                    }
                }
            }
        }
        if let Some(q) = &self.search_input {
            frame.render_widget(
                ratatui::widgets::Paragraph::new(format!("Find: {q} · Enter next · Esc cancel"))
                    .style(Style::new().fg(Color::Black).bg(Color::Cyan)),
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            );
        }
    }
    pub fn mark_saved(&mut self, loaded: impl Into<Arc<Loaded>>, saved_text: String) {
        self.loaded = loaded.into();
        self.baseline = saved_text;
        self.saving = false;
        self.conflict = false;
        self.status = "Saved".into();
    }
}
fn history_limit(text: &str) -> usize {
    history_limit_bytes(text.len())
}
fn history_limit_bytes(bytes: usize) -> usize {
    (HISTORY_BYTES / (bytes.max(1024) * 2)).clamp(1, 50)
}
fn validate(lines: &[String], encoding: Encoding, newline: &str) -> Result<()> {
    let _perf = crate::perf::span(crate::perf::Metric::EditorValidate);
    ensure!(
        lines.len() <= preview::MAX_LINES,
        "Edit exceeds 10,000 lines; operation rejected"
    );
    ensure!(
        lines.iter().all(|l| l.chars().count() <= preview::MAX_LINE),
        "Edit exceeds line limit; operation rejected"
    );
    let bytes = match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => {
            lines.iter().map(String::len).sum::<usize>()
                + newline.len() * lines.len().saturating_sub(1)
                + if encoding == Encoding::Utf8Bom { 3 } else { 0 }
        }
        _ => {
            2 + 2
                * (lines
                    .iter()
                    .map(|l| l.encode_utf16().count())
                    .sum::<usize>()
                    + newline.len() * lines.len().saturating_sub(1))
        }
    };
    ensure!(
        bytes <= MAX_EDIT_BYTES,
        "Edit exceeds 512 KiB; operation rejected"
    );
    Ok(())
}

pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xef, 0xbb, 0xbf], text.as_bytes()].concat(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut bytes = if encoding == Encoding::Utf16Le {
                vec![255, 254]
            } else {
                vec![254, 255]
            };
            for u in text.encode_utf16() {
                bytes.extend(if encoding == Encoding::Utf16Le {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                });
            }
            bytes
        }
    }
}

pub fn eligible(path: &Path) -> Result<()> {
    ensure!(
        !path.to_string_lossy().starts_with("\\\\"),
        "Network paths are preview-only"
    );
    for p in path.ancestors() {
        let meta = fs::symlink_metadata(p)?;
        ensure!(
            !crate::files::is_link(&meta),
            "Linked paths are preview-only"
        );
    }
    let meta = fs::metadata(path)?;
    ensure!(
        meta.is_file() && !meta.permissions().readonly(),
        "File is not a writable regular file"
    );
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let file = File::open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } != 0,
            "Cannot verify hard links"
        );
        ensure!(
            info.nNumberOfLinks == 1,
            "Multiple hard links are preview-only"
        );
    }
    Ok(())
}

pub struct Save {
    pub snapshot: Arc<Loaded>,
    pub source: String,
    pub revision: u64,
}
pub fn save(request: &Save) -> Result<Loaded> {
    let _perf = crate::perf::span(crate::perf::Metric::Save);
    save_staged(request, |output, bytes| {
        output.write_all(bytes)?;
        output.sync_all()?;
        Ok(())
    })
}
fn save_staged(
    request: &Save,
    write: impl FnOnce(&mut File, &[u8]) -> Result<()>,
) -> Result<Loaded> {
    eligible(&request.snapshot.path)?;
    let _lock = SaveLock::acquire(&request.snapshot.path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1 | 4);
    }
    let mut held = options
        .open(&request.snapshot.path)
        .context("Cannot exclude concurrent direct writers; buffer retained")?;
    let mut current = Vec::new();
    Read::by_ref(&mut held)
        .take((preview::MAX_BYTES + 1) as u64)
        .read_to_end(&mut current)?;
    ensure!(
        current == request.snapshot.bytes,
        "Disk content changed; reload or copy edits for external resolution"
    );
    let identity = file_id(&held)?;
    ensure!(
        request.snapshot.identity == Some(identity),
        "File identity changed since opening; save refused"
    );
    let bytes = encode(&request.source, request.snapshot.encoding);
    let temp = request
        .snapshot
        .path
        .with_file_name(format!(".devtabs-{}.tmp", uuid::Uuid::new_v4()));
    let outcome = (|| -> Result<Loaded> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        write(&mut output, &bytes)?;
        drop(output);
        ensure!(
            file_id(&File::open(&request.snapshot.path)?)? == identity,
            "File identity changed during save; refused"
        );
        #[cfg(windows)]
        replace_checked(request, &temp)?;
        #[cfg(not(windows))]
        {
            fs::rename(&temp, &request.snapshot.path)?;
        }
        let loaded = preview::read(&request.snapshot.path)?;
        ensure!(
            loaded.bytes == bytes,
            "Disk changed immediately after save; delivery state unknown, buffer retained"
        );
        Ok(loaded)
    })();
    let _ = fs::remove_file(&temp);
    outcome
}

#[cfg(windows)]
fn replace_checked(request: &Save, temp: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let backup = temp.with_extension("bak");
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let dest = wide(&request.snapshot.path);
    let source = wide(temp);
    let recovery = wide(&backup);
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            dest.as_ptr(),
            source.as_ptr(),
            recovery.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    ensure!(
        ok != 0,
        "Atomic replacement failed: {}; buffer retained; if present, recovery file: {}",
        std::io::Error::last_os_error(),
        backup.display()
    );
    // Capture the file actually replaced, not only an earlier snapshot. An
    // uncooperative rename can race the preflight check. Never delete its data
    // or report success; preserve both local buffer and displaced disk version.
    let previous = preview::read(&backup).with_context(|| {
        format!(
            "Cannot verify replaced file; recovery retained: {}",
            backup.display()
        )
    })?;
    ensure!(
        previous.bytes == request.snapshot.bytes && previous.identity == request.snapshot.identity,
        "External replacement raced save; disk may contain this edit. Replaced version retained at {}; buffer retained; resolve externally",
        backup.display()
    );
    fs::remove_file(&backup)
        .with_context(|| format!("Saved, but recovery cleanup failed: {}", backup.display()))?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn file_id(file: &File) -> Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut id = FILE_ID_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    ensure!(
        ok != 0,
        "Cannot verify file identity: {}",
        std::io::Error::last_os_error()
    );
    Ok((id.VolumeSerialNumber, id.FileId.Identifier))
}
#[cfg(not(windows))]
pub(crate) fn file_id(file: &File) -> Result<(u64, [u8; 16])> {
    Ok((file.metadata()?.len(), [0; 16]))
}

struct SaveLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
impl SaveLock {
    fn acquire(path: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::hash::{Hash, Hasher};
            use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            crate::config::key(path).hash(&mut hash);
            let name: Vec<u16> = format!("Local\\DevTabs-file-{:016x}", hash.finish())
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            ensure!(!handle.is_null(), "Create file save mutex");
            let wait = unsafe { WaitForSingleObject(handle, 5000) };
            if !matches!(wait, 0 | 0x80) {
                unsafe {
                    CloseHandle(handle);
                }
                anyhow::bail!("File save busy; retry");
            }
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(Self {})
        }
    }
}
impl Drop for SaveLock {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn replacement_race_never_discards_the_displaced_external_version() {
        let root = std::env::temp_dir().join(format!("DevTabs-race-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("original.txt");
        fs::write(&path, "original").unwrap();
        let loaded = preview::read(&path).unwrap();
        let request = Save {
            snapshot: Arc::new(loaded),
            source: "local".into(),
            revision: 1,
        };
        let temp = root.join(".devtabs-test.tmp");
        fs::write(&temp, "local").unwrap();
        // Emulate a noncooperating writer after preflight, immediately before
        // the exact production commit primitive.
        let external = root.join("external.tmp");
        fs::write(&external, "external").unwrap();
        fs::rename(&external, &path).unwrap();
        let error = replace_checked(&request, &temp).unwrap_err().to_string();
        assert!(error.contains("External replacement raced save"));
        assert_eq!(fs::read(path).unwrap(), b"local");
        assert_eq!(fs::read(temp.with_extension("bak")).unwrap(), b"external");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn partial_temp_write_and_space_failure_do_not_truncate_original() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-save-fault-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("original.txt");
        fs::write(&path, "original").unwrap();
        let loaded = preview::read(&path).unwrap();
        let request = Save {
            snapshot: Arc::new(loaded),
            source: "replacement".into(),
            revision: 1,
        };
        let result = save_staged(&request, |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(std::io::Error::from_raw_os_error(112).into())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            1,
            "failed staging must clean temporary file"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
