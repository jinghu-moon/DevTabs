//! Preview controls, tab actions and persistent bookmark UI.
use super::*;
use crate::bookmarks::{Entry, Operation};
use ratatui::widgets::Wrap;

#[derive(Default)]
pub(super) struct Controls {
    pub panel: Option<Panel>,
    pub hover: Option<(Rect, PathBuf)>,
    entries: Vec<Entry>,
    note_base: Option<Entry>,
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
        if self.read_only
            && matches!(
                command,
                Command::Edit
                    | Command::Save
                    | Command::Cut
                    | Command::Paste
                    | Command::Undo
                    | Command::Redo
                    | Command::NewFile
                    | Command::NewDirectory
                    | Command::Rename
                    | Command::Trash
            )
        {
            return false;
        }
        let d = self.document.as_ref();
        if d.is_some_and(Document::is_image)
            && matches!(
                command,
                Command::Edit
                    | Command::Save
                    | Command::Undo
                    | Command::Redo
                    | Command::Cut
                    | Command::Paste
                    | Command::Find
                    | Command::Project
                    | Command::EndEdit
                    | Command::Mode
                    | Command::Outline
                    | Command::Wrap
                    | Command::Compare
                    | Command::Diff
                    | Command::Goto
                    | Command::Search
                    | Command::Bookmark
                    | Command::CopyContent
                    | Command::CopySource
            )
        {
            return false;
        }
        match command {
            Command::Back => self.has_history(false),
            Command::Forward => self.has_history(true),
            Command::Edit => d.is_some_and(|d| d.loaded().is_some_and(|l| l.editable)),
            Command::Mode | Command::Compare => {
                d.is_some_and(|d| !d.editing && d.loaded().as_ref().is_some_and(|l| l.markdown()))
            }
            Command::Outline => d.is_some_and(|d| d.markdown && !d.editing),
            Command::Save => d
                .and_then(|d| d.editor())
                .is_some_and(|e| e.dirty() && !e.saving),
            _ => true,
        }
    }
    pub(super) fn command_toggle(&self, command: Command) -> Option<bool> {
        match command {
            Command::Follow => Some(self.explorer.preferences.follow),
            Command::Compact => Some(self.files.compact),
            Command::Modified => Some(self.files.modified_only),
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
            crate::i18n::t("ui.bookmark_operation_in_progress")
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
        if !self.command_enabled(command) {
            return Ok(());
        }
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
                    crate::i18n::t("ui.bookmark_operation_in_progress")
                );
                let d = self
                    .document
                    .as_ref()
                    .context(crate::i18n::t("ui.no_document"))?;
                let source = d
                    .source_snapshot()
                    .context(crate::i18n::t("ui.wait_for_file_loading"))?;
                let byte = d.reading_byte().min(source.len());
                let line = source[..byte].bytes().filter(|&b| b == b'\n').count();
                let anchor = source
                    .lines()
                    .nth(line)
                    .unwrap_or("")
                    .chars()
                    .take(128)
                    .collect();
                self.controls.note_base = None;
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
        let Some(loaded) = d.loaded() else {
            self.controls.jump = None;
            return;
        };
        let byte = crate::bookmarks::position(entry, &loaded.source);
        let markdown = entry.markdown && loaded.markdown();
        if d.markdown != markdown {
            d.markdown = markdown;
            d.set_rendered(None);
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
                        self.say(crate::i18n::t("preview.bookmarks_saved"), true);
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
                        self.say(crate::i18n::t("preview.information_copied"), true);
                    }
                }
                Some(Panel::Note(entry)) => match key.code {
                    KeyCode::Enter => {
                        let entry = entry.clone();
                        let operation = match self.controls.note_base.clone() {
                            Some(base) => Operation::Edit { base, draft: entry },
                            None => Operation::Put(entry),
                        };
                        self.bookmark_request(operation)?;
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
                            self.bookmark_request(Operation::Delete(entry.clone()))?;
                        }
                    }
                    KeyCode::Char('e' | 'E') => {
                        if let Some(entry) =
                            self.controls.entries.get(self.controls.selected).cloned()
                        {
                            self.controls.note_base = Some(entry.clone());
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
                let first = visible_header_buttons(
                    self.preview_controls(),
                    rect,
                    false,
                    d.loaded().is_some_and(|l| l.markdown()),
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
                            (crate::i18n::t("legacy_8"), Command::CopyAbsolute),
                            (crate::i18n::t("ui.close_tab"), Command::Close),
                            (crate::i18n::t("legacy_39"), Command::CloseOthers),
                            (crate::i18n::t("legacy_40"), Command::CloseRight),
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
            return crate::i18n::t("ui.no_document").into();
        };
        if let Some(i) = d.image() {
            return i.information(&d.path, self.image_display);
        }
        let Some(l) = d.loaded() else {
            return crate::i18n::t("ui.file_not_loaded").into();
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
            .unwrap_or_else(|| crate::i18n::t("ui.unknown").into());
        crate::i18n::format(
            "preview.file_information",
            &[
                ("path", preview::safe_text(&d.path.to_string_lossy())),
                ("bytes", l.disk_bytes.to_string()),
                ("kib", format!("{:.1}", l.disk_bytes as f64 / 1024.0)),
                ("loaded", l.bytes.len().to_string()),
                ("encoding", encoding.into()),
                ("newline", newline.into()),
                ("modified", modified),
                (
                    "truncated",
                    crate::i18n::t(if l.truncated { "ui.yes" } else { "ui.no" }).into_owned(),
                ),
                (
                    "editable",
                    crate::i18n::t(if l.editable { "ui.yes" } else { "ui.no" }).into_owned(),
                ),
                ("note", l.note.clone()),
            ],
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
            && !d.is_image()
        {
            for (r, label, c) in visible_header_buttons(
                self.preview_controls(),
                rect,
                false,
                d.loaded().is_some_and(|l| l.markdown()),
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
                Panel::Info => (
                    crate::i18n::t("ui.file_information"),
                    self.file_information(),
                ),
                Panel::Note(e) => (
                    crate::i18n::t("ui.bookmark_note"),
                    format!(
                        "{} · Ln {}\n{}\n\nEnter Save · Esc Cancel · {}/256{}\n{}",
                        e.path.file_name().unwrap_or_default().to_string_lossy(),
                        e.line + 1,
                        e.note,
                        e.note.chars().count(),
                        if self.controls.pending.is_some() {
                            crate::i18n::t("ui.saving")
                        } else {
                            "".into()
                        },
                        self.controls.error
                    ),
                ),
                Panel::Bookmarks => {
                    let rows = r.height.saturating_sub(4).max(1) as usize;
                    let first = self.controls.selected.saturating_sub(rows - 1);
                    let mut text = if self.controls.pending.is_some() {
                        crate::i18n::t("ui.loading_saving").into()
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
                        text.push_str(&crate::i18n::t(
                            "ui.no_bookmarks_press_b_in_preview_to_add_one",
                        ));
                    }
                    text.push_str(&self.controls.error);
                    (crate::i18n::t("ui.reading_bookmarks_esc_close"), text)
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
                .block(Block::bordered().title(crate::i18n::t("ui.full_path"))),
                r,
            );
        }
    }
}

fn format_utc(time: std::time::SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(std::time::UNIX_EPOCH) else {
        return crate::i18n::t("ui.before_1970").into();
    };
    let seconds = elapsed.as_secs();
    let (year, month, day) = crate::utc::date(seconds / 86400);
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
