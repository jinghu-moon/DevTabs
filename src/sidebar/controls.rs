//! Preview controls, tab actions and persistent bookmark UI.
use super::*;
use crate::bookmarks::{Entry, Operation};
use ratatui::widgets::Wrap;

#[derive(Default)]
pub(super) struct Controls {
    pub panel: Option<Panel>,
    pub hover: Option<(Rect, PathBuf)>,
    entries: Vec<Entry>,
    selected: usize,
    pending: Option<u64>,
    jump: Option<Entry>,
    error: String,
}
pub(super) enum Panel {
    Info,
    Bookmarks,
    Note(Entry),
}
impl Sidebar {
    pub(super) fn command_enabled(&self, command: Command) -> bool {
        let d = self.document.as_ref();
        match command {
            Command::Back => self.has_history(false),
            Command::Forward => self.has_history(true),
            Command::Edit => d.is_some_and(|d| d.loaded.as_ref().is_some_and(|l| l.editable)),
            Command::Mode | Command::Compare => {
                d.is_some_and(|d| !d.editing && d.loaded.as_ref().is_some_and(|l| l.markdown()))
            }
            Command::Outline => d.is_some_and(|d| d.markdown && !d.editing),
            Command::Save => d
                .and_then(|d| d.editor.as_ref())
                .is_some_and(|e| e.dirty() && !e.saving),
            _ => true,
        }
    }
    pub(super) fn command_toggle(&self, command: Command) -> Option<bool> {
        match command {
            Command::Maximize => Some(self.tools.maximized),
            Command::Wrap => self.document.as_ref().map(|d| d.view.wrap),
            Command::Compare => self.document.as_ref().map(|d| d.view.compare),
            Command::Mode => self.document.as_ref().map(|d| d.markdown),
            _ => None,
        }
    }
    pub(super) fn close_tab(&mut self, path: &PathBuf) {
        self.controls.hover = None;
        if self.document.as_ref().is_some_and(|d| d.path == *path) {
            self.close();
        } else {
            self.tools.paths.retain(|p| p != path);
            self.tools.parked.retain(|d| d.path != *path);
        }
    }
    pub(super) fn close_group(&mut self, target: &PathBuf, right: bool) {
        let Some(index) = self.tools.paths.iter().position(|p| p == target) else {
            return;
        };
        let remove = self
            .tools
            .paths
            .iter()
            .enumerate()
            .filter(|(i, p)| if right { *i > index } else { *p != target })
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>();
        let closes_active = self
            .document
            .as_ref()
            .is_some_and(|d| remove.contains(&d.path));
        if closes_active && self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::CloseGroup(target.clone(), right)));
            return;
        }
        self.controls.hover = None;
        for path in remove
            .iter()
            .filter(|p| self.document.as_ref().is_none_or(|d| d.path != **p))
        {
            self.tools.paths.retain(|p| p != path);
            self.tools.parked.retain(|d| d.path != *path);
        }
        if closes_active {
            self.close();
        }
    }
    fn bookmark_request(&mut self, operation: Operation) -> Result<()> {
        ensure!(
            self.controls.pending.is_none(),
            "Bookmark operation in progress"
        );
        self.serial += 1;
        self.io.send(Job::Bookmarks {
            project: self.files.root.clone(),
            id: self.serial,
            operation,
        })?;
        self.controls.pending = Some(self.serial);
        self.controls.error.clear();
        Ok(())
    }
    pub(super) fn control_command(&mut self, command: Command) -> Result<()> {
        self.controls.hover = None;
        match command {
            Command::Info => {
                self.controls.panel = Some(Panel::Info);
            }
            Command::Bookmarks => {
                self.bookmark_request(Operation::List)?;
                self.controls.panel = Some(Panel::Bookmarks);
            }
            Command::Bookmark => {
                ensure!(
                    self.controls.pending.is_none(),
                    "Bookmark operation in progress"
                );
                let d = self.document.as_ref().context("No document")?;
                let source = d.source_snapshot().context("Wait for file loading")?;
                let byte = d.reading_byte().min(source.len());
                let line = source[..byte].bytes().filter(|&b| b == b'\n').count();
                let anchor = source
                    .lines()
                    .nth(line)
                    .unwrap_or("")
                    .chars()
                    .take(128)
                    .collect();
                self.controls.panel = Some(Panel::Note(Entry {
                    id: uuid::Uuid::new_v4(),
                    project: self.files.root.clone(),
                    path: d.path.clone(),
                    byte,
                    line,
                    anchor,
                    markdown: d.markdown,
                    note: String::new(),
                }));
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn jump_bookmark(&mut self, entry: Entry) -> Result<()> {
        if self.dirty() && self.document.as_ref().is_some_and(|d| d.path != entry.path) {
            self.prompt = Some(Prompt::Unsaved(Pending::Bookmark(entry)));
            return Ok(());
        }
        if self.document.as_ref().is_some_and(|d| d.path == entry.path) {
            self.push_back();
        }
        self.open(entry.path.clone(), true)?;
        self.controls.jump = Some(entry);
        self.control_tick();
        Ok(())
    }
    pub(super) fn control_tick(&mut self) {
        let Some(entry) = &self.controls.jump else {
            return;
        };
        let Some(d) = &mut self.document else {
            self.controls.jump = None;
            return;
        };
        if d.path != entry.path {
            self.controls.jump = None;
            return;
        }
        if d.loading {
            return;
        }
        let Some(loaded) = &d.loaded else {
            self.controls.jump = None;
            return;
        };
        let byte = crate::bookmarks::position(entry, &loaded.source);
        let markdown = entry.markdown && loaded.markdown();
        if d.markdown != markdown {
            d.markdown = markdown;
            d.rendered = None;
            d.render_pending = false;
            d.render_id = 0;
        }
        self.controls.jump = None;
        self.seek_reading(byte);
    }
    pub(super) fn control_completion(&mut self, output: Completed) -> Option<Completed> {
        if let Completed::Bookmarks { id, result } = output {
            if self.controls.pending == Some(id) {
                self.controls.pending = None;
                match result {
                    Ok(entries) => {
                        self.controls.entries = entries;
                        self.controls.selected = self
                            .controls
                            .selected
                            .min(self.controls.entries.len().saturating_sub(1));
                        if matches!(self.controls.panel, Some(Panel::Note(_))) {
                            self.controls.panel = Some(Panel::Bookmarks);
                        }
                        self.say("Bookmarks loaded / saved", true);
                    }
                    Err(e) => {
                        self.controls.error = format!("{e:#}");
                        self.say(self.controls.error.clone(), false);
                    }
                }
            }
            None
        } else {
            Some(output)
        }
    }
    pub(super) fn control_paste(&mut self, text: &str) -> bool {
        if let Some(Panel::Note(entry)) = &mut self.controls.panel {
            if self.controls.pending.is_none() {
                let left = 256usize.saturating_sub(entry.note.chars().count());
                entry
                    .note
                    .extend(text.chars().filter(|c| !c.is_control()).take(left));
            }
            true
        } else {
            self.controls.panel.is_some()
        }
    }
    pub(super) fn control_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.controls.panel.is_some() {
            if key.code == KeyCode::Esc {
                self.controls.panel = None;
                return Ok(true);
            }
            if self.controls.pending.is_some() {
                return Ok(true);
            }
            match &mut self.controls.panel {
                Some(Panel::Info) => {
                    if matches!(key.code, KeyCode::Char('c' | 'C')) {
                        files::copy(self.file_information())?;
                        self.say("Copied file information", true);
                    }
                }
                Some(Panel::Note(entry)) => match key.code {
                    KeyCode::Enter => {
                        let entry = entry.clone();
                        self.bookmark_request(Operation::Put(entry))?;
                    }
                    KeyCode::Backspace => {
                        entry.note.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && !c.is_control()
                            && entry.note.chars().count() < 256 =>
                    {
                        entry.note.push(c)
                    }
                    _ => {}
                },
                Some(Panel::Bookmarks) => match key.code {
                    KeyCode::Up => {
                        self.controls.selected = self.controls.selected.saturating_sub(1)
                    }
                    KeyCode::Down => {
                        self.controls.selected = (self.controls.selected + 1)
                            .min(self.controls.entries.len().saturating_sub(1))
                    }
                    KeyCode::Home => self.controls.selected = 0,
                    KeyCode::End => {
                        self.controls.selected = self.controls.entries.len().saturating_sub(1)
                    }
                    KeyCode::Enter => {
                        if let Some(entry) =
                            self.controls.entries.get(self.controls.selected).cloned()
                        {
                            self.jump_bookmark(entry)?;
                            self.controls.panel = None;
                        }
                    }
                    KeyCode::Delete => {
                        if let Some(entry) = self.controls.entries.get(self.controls.selected) {
                            self.bookmark_request(Operation::Delete(entry.id))?;
                        }
                    }
                    KeyCode::Char('e' | 'E') => {
                        if let Some(entry) =
                            self.controls.entries.get(self.controls.selected).cloned()
                        {
                            self.controls.panel = Some(Panel::Note(entry));
                        }
                    }
                    KeyCode::Char('r' | 'R') => self.bookmark_request(Operation::List)?,
                    _ => {}
                },
                None => {}
            }
            return Ok(true);
        }
        if self.modal() || self.focus != Focus::Preview {
            return Ok(false);
        }
        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Right {
            self.reading_history(true)?;
            return Ok(true);
        }
        if key.modifiers.contains(KeyModifiers::ALT) && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            if let Some(d) = &self.document {
                files::copy(d.path.to_string_lossy().into_owned())?;
                self.say("Copied absolute path", true);
            }
            return Ok(true);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('b' | 'B'))
        {
            self.control_command(Command::Bookmarks)?;
            return Ok(true);
        }
        if !key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            let command = match key.code {
                KeyCode::Char('b' | 'B') => Some(Command::Bookmark),
                KeyCode::Char('i' | 'I') => Some(Command::Info),
                _ => None,
            };
            if let Some(command) = command {
                self.control_command(command)?;
                return Ok(true);
            }
            if matches!(key.code, KeyCode::Char('n' | 'N'))
                && self
                    .document
                    .as_ref()
                    .is_some_and(|d| !d.view.search_targets.is_empty())
            {
                self.push_back();
            }
        }
        Ok(false)
    }
    pub(super) fn control_mouse(&mut self, event: MouseEvent) -> Result<bool> {
        if self.controls.panel.is_some() {
            if matches!(self.controls.panel, Some(Panel::Bookmarks))
                && self.controls.pending.is_none()
            {
                let popup = self.control_popup(self.tools.area);
                let rows = popup.height.saturating_sub(4).max(1) as usize;
                let first = self.controls.selected.saturating_sub(rows - 1);
                match event.kind {
                    MouseEventKind::ScrollDown => {
                        self.controls.selected = (self.controls.selected + 1)
                            .min(self.controls.entries.len().saturating_sub(1))
                    }
                    MouseEventKind::ScrollUp => {
                        self.controls.selected = self.controls.selected.saturating_sub(1)
                    }
                    MouseEventKind::Down(MouseButton::Left)
                        if popup.contains((event.column, event.row).into())
                            && event.row >= popup.y + 2
                            && event.row < popup.bottom().saturating_sub(2) =>
                    {
                        let index = first + (event.row - popup.y - 2) as usize;
                        if let Some(entry) = self.controls.entries.get(index).cloned() {
                            self.controls.selected = index;
                            self.jump_bookmark(entry)?;
                            self.controls.panel = None;
                        }
                    }
                    _ => {}
                }
            }
            return Ok(true);
        }
        let point = (event.column, event.row).into();
        let tab = self
            .tab_buttons()
            .into_iter()
            .find(|(r, _)| r.contains(point))
            .or_else(|| {
                let rect = self.geometry.preview?;
                let d = self.document.as_ref()?;
                if d.editing || !self.showing() || event.row != rect.y {
                    return None;
                }
                let first = header_buttons(
                    rect,
                    false,
                    d.loaded.as_ref().is_some_and(|l| l.markdown()),
                    d.markdown,
                )
                .iter()
                .map(|(r, _, _)| r.x)
                .min()
                .unwrap_or(rect.right());
                let title = Rect::new(rect.x + 1, rect.y, first.saturating_sub(rect.x + 1), 1);
                title.contains(point).then(|| (title, d.path.clone()))
            });
        if event.kind == MouseEventKind::Moved {
            self.controls.hover = if self.modal() { None } else { tab.clone() };
            return Ok(tab.is_some());
        }
        self.controls.hover = None;
        if self.modal() {
            return Ok(false);
        }
        if let Some((rect, path)) = tab {
            match event.kind {
                MouseEventKind::Down(MouseButton::Middle) => {
                    self.close_tab(&path);
                    return Ok(true);
                }
                MouseEventKind::Down(MouseButton::Left)
                    if rect.width >= 8
                        && event.row == rect.y + 1
                        && event.column == rect.right() - 3 =>
                {
                    self.close_tab(&path);
                    return Ok(true);
                }
                MouseEventKind::Down(MouseButton::Right) => {
                    self.menu = Some(Menu {
                        items: vec![
                            ("Copy absolute path", Command::CopyAbsolute),
                            ("Close tab", Command::Close),
                            ("Close other tabs", Command::CloseOthers),
                            ("Close tabs to the right", Command::CloseRight),
                        ],
                        selected: 0,
                        target: path,
                    });
                    return Ok(true);
                }
                _ => {}
            }
        }
        Ok(false)
    }
    fn file_information(&self) -> String {
        let Some(d) = &self.document else {
            return "No document".into();
        };
        let Some(l) = &d.loaded else {
            return "File not loaded".into();
        };
        let encoding = match l.encoding {
            preview::Encoding::Utf8 => "UTF-8",
            preview::Encoding::Utf8Bom => "UTF-8 BOM",
            preview::Encoding::Utf16Le => "UTF-16 LE",
            preview::Encoding::Utf16Be => "UTF-16 BE",
        };
        let newline = if l.newline == "\r\n" { "CRLF" } else { "LF" };
        let modified = l
            .modified
            .map(format_utc)
            .unwrap_or_else(|| "Unknown".into());
        format!(
            "Path: {}\nSize: {} bytes ({:.1} KiB)\nLoaded: {} bytes\nEncoding: {encoding}\nNewline: {newline}\nModified: {modified}\nTruncated: {}\nEditable: {}\n{}\nSnapshot captured at last read/save.\nC Copy information · Esc Close",
            preview::safe_text(&d.path.to_string_lossy()),
            l.disk_bytes,
            l.disk_bytes as f64 / 1024.0,
            l.bytes.len(),
            if l.truncated { "Yes" } else { "No" },
            if l.editable { "Yes" } else { "No" },
            l.note
        )
    }
    fn control_popup(&self, area: Rect) -> Rect {
        let r = self.geometry.preview.unwrap_or(area).intersection(area);
        let width = r.width.min(92);
        let height = r
            .height
            .min(if matches!(self.controls.panel, Some(Panel::Note(_))) {
                9
            } else {
                18
            });
        Rect::new(
            r.x + (r.width - width) / 2,
            r.y + (r.height - height) / 2,
            width,
            height,
        )
    }
    pub(super) fn draw_control_header(&self, frame: &mut Frame) {
        // Redraw header states without changing hit geometry.
        if let (Some(rect), Some(d)) = (self.geometry.preview, self.document.as_ref())
            && !d.editing
        {
            for (r, label, c) in header_buttons(
                rect,
                false,
                d.loaded.as_ref().is_some_and(|l| l.markdown()),
                d.markdown,
            ) {
                let enabled = self.command_enabled(c);
                let active = matches!(c, Command::Maximize) && self.tools.maximized;
                let style = if !enabled {
                    Style::new().fg(Color::DarkGray)
                } else if active {
                    Style::new().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::new().fg(Color::Cyan)
                };
                frame.render_widget(Paragraph::new(label).style(style), r);
            }
        }
    }
    pub(super) fn draw_controls(&self, frame: &mut Frame) {
        if let Some(panel) = &self.controls.panel {
            let r = self.control_popup(frame.area());
            crate::ui::clear_popup(frame, r);
            let style = Style::new()
                .fg(if self.files.light {
                    Color::Black
                } else {
                    Color::White
                })
                .bg(if self.files.light {
                    Color::Rgb(244, 246, 250)
                } else {
                    Color::Rgb(24, 27, 34)
                });
            let (title, text) = match panel {
                Panel::Info => (" File information ", self.file_information()),
                Panel::Note(e) => (
                    " Bookmark note ",
                    format!(
                        "{} · Ln {}\n{}\n\nEnter Save · Esc Cancel · {}/256{}\n{}",
                        e.path.file_name().unwrap_or_default().to_string_lossy(),
                        e.line + 1,
                        e.note,
                        e.note.chars().count(),
                        if self.controls.pending.is_some() {
                            " · Saving…"
                        } else {
                            ""
                        },
                        self.controls.error
                    ),
                ),
                Panel::Bookmarks => {
                    let rows = r.height.saturating_sub(4).max(1) as usize;
                    let first = self.controls.selected.saturating_sub(rows - 1);
                    let mut text = if self.controls.pending.is_some() {
                        "Loading / saving…\n".into()
                    } else {
                        format!(
                            "{} bookmarks · Enter Jump · E Note · Del Delete\n",
                            self.controls.entries.len()
                        )
                    };
                    for (i, e) in self
                        .controls
                        .entries
                        .iter()
                        .enumerate()
                        .skip(first)
                        .take(rows)
                    {
                        text.push_str(&features::shortened(
                            &format!(
                                "{} {}:{} · {}",
                                if i == self.controls.selected {
                                    ">"
                                } else {
                                    " "
                                },
                                e.path
                                    .strip_prefix(&self.files.root)
                                    .unwrap_or(&e.path)
                                    .display(),
                                e.line + 1,
                                e.note
                            ),
                            r.width.saturating_sub(2) as usize,
                        ));
                        text.push('\n');
                    }
                    if self.controls.entries.is_empty() {
                        text.push_str("No bookmarks. Press B in preview to add one.\n");
                    }
                    text.push_str(&self.controls.error);
                    (" Reading bookmarks · Esc Close ", text)
                }
            };
            let widget = Paragraph::new(preview::safe_text(&text))
                .style(style)
                .block(Block::bordered().title(title));
            let widget = if matches!(panel, Panel::Bookmarks) {
                widget
            } else {
                widget.wrap(Wrap { trim: false })
            };
            frame.render_widget(widget, r);
        } else if self.showing()
            && !self.modal()
            && let Some((tab, path)) = &self.controls.hover
        {
            let area = frame.area();
            let r = Rect::new(
                area.x + 1,
                (tab.bottom() + 1).min(area.bottom().saturating_sub(5)),
                area.width.saturating_sub(2),
                5.min(area.height),
            );
            crate::ui::clear_popup(frame, r);
            frame.render_widget(
                Paragraph::new(format!(
                    "{}\nAlt+C Copy active path · Right click Copy tab path",
                    preview::safe_text(&path.to_string_lossy())
                ))
                .wrap(Wrap { trim: false })
                .style(Style::new().fg(Color::White).bg(Color::Rgb(24, 27, 34)))
                .block(Block::bordered().title(" Full path ")),
                r,
            );
        }
    }
}

fn format_utc(time: std::time::SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(std::time::UNIX_EPOCH) else {
        return "Before 1970".into();
    };
    let seconds = elapsed.as_secs();
    // Gregorian calendar conversion from days since Unix epoch.
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modified_time_formats_epoch_leap_day_and_year_boundary() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(format_utc(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            format_utc(UNIX_EPOCH + Duration::from_secs(1709210096)),
            "2024-02-29 12:34:56 UTC"
        );
        assert_eq!(
            format_utc(UNIX_EPOCH + Duration::from_secs(1767225599)),
            "2025-12-31 23:59:59 UTC"
        );
    }
}
