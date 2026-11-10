//! Preview workspace controls. Inactive tabs retain snapshots, not rendered caches.
use super::*;
use ratatui::text::Span;
use ratatui::{style::Modifier, widgets::Borders};
use std::sync::Arc;

const MAX_TABS: usize = 8;
#[derive(Default)]
pub(super) struct DocumentView {
    pub wrap: bool,
    pub compare: bool,
    pub search_options: preview::tools::SearchOptions,
    pub search_error: String,
    pub search_targets: Vec<(usize, usize)>,
    pub previous: Option<Arc<Loaded>>,
    pub disk: Option<Arc<Loaded>>,
    source: Option<Arc<str>>,
    starts: Vec<usize>,
    pub companion: Option<Rendered>,
    pub companion_pending: Option<u64>,
    companion_left: usize,
}
#[derive(Default)]
pub(super) struct Workspace {
    pub maximized: bool,
    pub area: Rect,
    pub split: bool,
    pub paths: Vec<PathBuf>,
    pub parked: Vec<Document>,
    pub navigation: Option<Navigation>,
    pub diff: Option<DiffView>,
    pub diff_pending: Option<(PathBuf, u64)>,
}
pub(super) struct Navigation {
    value: String,
    error: String,
}
pub(super) struct DiffView {
    data: preview::tools::Diff,
    top: usize,
    left: u16,
}

impl Document {
    fn source_snapshot(&self) -> Option<Arc<str>> {
        self.rendered
            .as_ref()
            .map(|r| r.source.clone())
            .or_else(|| self.loaded.as_ref().map(|l| l.source.clone()))
    }
    fn update_source_index(&mut self) {
        let Some(source) = self.source_snapshot() else {
            return;
        };
        if self
            .view
            .source
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &source))
        {
            return;
        }
        self.view.starts.clear();
        self.view.starts.push(0);
        self.view.starts.extend(
            source
                .match_indices('\n')
                .map(|(i, _)| i + 1)
                .filter(|&i| i < source.len()),
        );
        self.view.source = Some(source);
    }
    fn source_line(&self, byte: usize) -> usize {
        self.view
            .starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }
    pub(super) fn progress(&self) -> String {
        let Some(source) = &self.view.source else {
            return "Loading".into();
        };
        let byte = self.reading_byte().min(source.len());
        let line = self.source_line(byte) + 1;
        let percent = if source.is_empty() {
            100
        } else {
            byte.saturating_mul(100) / source.len()
        };
        let (index, total) = (
            if self.view.search_targets.is_empty() {
                0
            } else {
                self.match_index + 1
            },
            self.view.search_targets.len(),
        );
        format!(
            " Ln {line}/{} · {percent}% · {index}/{total}{}{} ",
            self.view.starts.len(),
            if self.view.wrap { " · Wrap" } else { "" },
            if self.view.compare { " · Compare" } else { "" }
        )
    }
    fn invalidate_view(&mut self) {
        self.reader.seek = Some(self.reading_byte());
        self.rendered = None;
        self.render_pending = false;
        self.render_id = 0;
        self.code_pending = false;
        self.code_desired = None;
        self.selection = None;
        self.code_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.view.companion = None;
        self.view.companion_pending = None;
    }
}
pub(super) fn shortened(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut cells = 0;
    for g in text.graphemes(true) {
        if cells + g.width() + 1 > width {
            break;
        }
        out.push_str(g);
        cells += g.width();
    }
    if width > 0 {
        out.push('…');
    }
    out
}
impl Sidebar {
    pub(super) fn preview_panes(&self) -> Option<(Rect, Option<Rect>)> {
        let rect = self.geometry.preview?;
        let mut inner = crate::ui::preview_content(rect);
        if self.tools.paths.len() > 1 {
            inner.y += 3;
            inner.height = inner.height.saturating_sub(3);
        }
        let compare = self
            .document
            .as_ref()
            .is_some_and(|d| d.view.compare && d.markdown && !d.editing);
        if compare && inner.width >= 36 {
            let left = inner.width / 2;
            Some((
                Rect::new(inner.x, inner.y, left, inner.height),
                Some(Rect::new(
                    inner.x + left + 1,
                    inner.y,
                    inner.width - left - 1,
                    inner.height,
                )),
            ))
        } else {
            Some((inner, None))
        }
    }
    pub(super) fn park_document(&mut self) {
        if let Some(mut d) = self.document.take() {
            d.invalidate_view();
            d.edit_requested = None;
            if let Some(e) = &mut d.editor {
                e.highlight = None;
            }
            d.view.previous = None;
            d.view.disk = None;
            self.tools.parked.retain(|p| p.path != d.path);
            self.tools.parked.push(d);
        }
        self.reader.outline = None;
        self.reader.drag = None;
        self.tools.diff = None;
        self.tools.diff_pending = None;
    }
    pub(super) fn register_tab(&mut self, path: &PathBuf) {
        if !self.tools.paths.contains(path) {
            self.tools.paths.push(path.clone());
        }
        while self.tools.paths.len() > MAX_TABS {
            let removed = self.tools.paths.remove(0);
            self.tools.parked.retain(|d| d.path != removed);
        }
    }
    fn cycle_tab(&mut self, back: bool) -> Result<()> {
        let n = self.tools.paths.len();
        if n < 2 {
            return Ok(());
        }
        let index = self
            .document
            .as_ref()
            .and_then(|d| self.tools.paths.iter().position(|p| p == &d.path))
            .unwrap_or(0);
        let index = if back {
            (index + n - 1) % n
        } else {
            (index + 1) % n
        };
        self.open(self.tools.paths[index].clone(), true)
    }
    pub(super) fn preview_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Maximize => {
                self.tools.maximized = !self.tools.maximized;
                self.update_layout(self.tools.area, self.tools.split);
            }
            Command::Wrap => {
                if let Some(d) = &mut self.document {
                    d.view.wrap = !d.view.wrap;
                    if d.view.wrap {
                        d.offset_mut().1 = 0;
                    }
                    d.invalidate_view();
                }
            }
            Command::Compare => {
                if let Some(d) = &mut self.document {
                    ensure!(
                        d.loaded.as_ref().is_some_and(|l| l.markdown()) && !d.editing,
                        "Source comparison requires Markdown preview"
                    );
                    d.view.compare = !d.view.compare;
                    d.markdown = true;
                    d.invalidate_view();
                }
            }
            Command::Diff => {
                let d = self.document.as_ref().context("No document")?;
                let old = d
                    .view
                    .previous
                    .as_ref()
                    .or(d.loaded.as_ref())
                    .context("File not loaded")?
                    .source
                    .clone();
                let new = d
                    .view
                    .disk
                    .as_ref()
                    .or(d.loaded.as_ref())
                    .context("File not loaded")?
                    .source
                    .clone();
                let old = if let Some(e) = &d.editor
                    && e.dirty()
                {
                    Arc::from(e.source())
                } else {
                    old
                };
                self.serial += 1;
                let id = self.serial;
                let path = d.path.clone();
                self.io.send(Job::Diff {
                    path: path.clone(),
                    id,
                    old,
                    new,
                })?;
                self.tools.diff_pending = Some((path, id));
            }
            Command::Goto => {
                self.tools.navigation = Some(Navigation {
                    value: String::new(),
                    error: String::new(),
                })
            }
            Command::Search => self.start_query(false),
            Command::NextTab => self.cycle_tab(false)?,
            Command::PreviousTab => self.cycle_tab(true)?,
            _ => {}
        }
        Ok(())
    }
    fn navigate_input(&mut self, input: &str) -> Result<()> {
        let d = self.document.as_mut().context("No document")?;
        d.update_source_index();
        let input = input.trim();
        let source = d.view.source.as_ref().context("File not loaded")?;
        let byte = if let Some(percent) = input.strip_suffix('%') {
            let percent = percent
                .parse::<usize>()
                .context("Use a source line or 0–100%")?;
            ensure!(percent <= 100, "Percentage must be 0–100");
            let mut at = source.len().saturating_mul(percent) / 100;
            while !source.is_char_boundary(at) {
                at = at.saturating_sub(1);
            }
            at
        } else {
            let line = input
                .parse::<usize>()
                .context("Use a source line or 0–100%")?;
            *d.view
                .starts
                .get(line.checked_sub(1).context("Line numbers start at 1")?)
                .context("Line is outside this preview")?
        };
        self.seek_reading(byte);
        Ok(())
    }
    pub(super) fn feature_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(n) = &mut self.tools.navigation {
            match key.code {
                KeyCode::Esc => self.tools.navigation = None,
                KeyCode::Enter => {
                    let input = n.value.clone();
                    match self.navigate_input(&input) {
                        Ok(()) => self.tools.navigation = None,
                        Err(e) => self.tools.navigation.as_mut().unwrap().error = e.to_string(),
                    }
                }
                KeyCode::Backspace => {
                    n.value.pop();
                }
                KeyCode::Char(c) if (c.is_ascii_digit() || c == '%') && n.value.len() < 16 => {
                    n.value.push(c);
                }
                _ => {}
            }
            return Ok(true);
        }
        if self.tools.diff_pending.is_some() || self.tools.diff.is_some() {
            if key.code == KeyCode::Esc {
                self.tools.diff = None;
                self.tools.diff_pending = None;
            } else if let Some(v) = &mut self.tools.diff {
                let page = self
                    .geometry
                    .preview
                    .map_or(10, |r| r.height.saturating_sub(4) as usize)
                    .max(1);
                match key.code {
                    KeyCode::Down => v.top = v.top.saturating_add(1),
                    KeyCode::Up => v.top = v.top.saturating_sub(1),
                    KeyCode::PageDown => v.top = v.top.saturating_add(page),
                    KeyCode::PageUp => v.top = v.top.saturating_sub(page),
                    KeyCode::Home => v.top = 0,
                    KeyCode::End => v.top = v.data.lines.len().saturating_sub(1),
                    KeyCode::Right => v.left = v.left.saturating_add(4),
                    KeyCode::Left => v.left = v.left.saturating_sub(4),
                    _ => {}
                }
                v.top = v.top.min(v.data.lines.len().saturating_sub(1));
            }
            return Ok(true);
        }
        if self.query.as_ref().is_some_and(|q| !q.tree)
            && key.modifiers.contains(KeyModifiers::ALT)
            && let Some(d) = &mut self.document
        {
            match key.code {
                KeyCode::Char('c' | 'C') => {
                    d.view.search_options.case_sensitive = !d.view.search_options.case_sensitive
                }
                KeyCode::Char('w' | 'W') => {
                    d.view.search_options.whole_word = !d.view.search_options.whole_word
                }
                KeyCode::Char('r' | 'R') => {
                    d.view.search_options.regex = !d.view.search_options.regex
                }
                _ => return Ok(false),
            }
            self.update_query();
            return Ok(true);
        }
        if !self.modal() && self.focus == Focus::Preview {
            let command = match key.code {
                KeyCode::Char('z' | 'Z') => Some(Command::Maximize),
                KeyCode::Char('w' | 'W') => Some(Command::Wrap),
                KeyCode::Char('v' | 'V') => Some(Command::Compare),
                KeyCode::Char('d' | 'D') => Some(Command::Diff),
                KeyCode::Char('g' | 'G') => Some(Command::Goto),
                KeyCode::PageDown if key.modifiers.contains(KeyModifiers::ALT) => {
                    Some(Command::NextTab)
                }
                KeyCode::PageUp if key.modifiers.contains(KeyModifiers::ALT) => {
                    Some(Command::PreviousTab)
                }
                _ => None,
            };
            if let Some(c) = command {
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !matches!(c, Command::NextTab | Command::PreviousTab)
                {
                    return Ok(false);
                }
                self.preview_command(c)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn feature_paste(&mut self, text: &str) -> bool {
        if let Some(n) = &mut self.tools.navigation {
            n.value.extend(
                text.chars()
                    .filter(|c| c.is_ascii_digit() || *c == '%')
                    .take(16usize.saturating_sub(n.value.len())),
            );
            true
        } else {
            false
        }
    }
    pub(super) fn feature_tick(&mut self) -> bool {
        let panes = self.preview_panes();
        let Some(d) = &mut self.document else {
            return false;
        };
        d.update_source_index();
        let Some((_, Some(rect))) = panes else {
            return false;
        };
        let top = d.source_line(d.reading_byte());
        let Some(source) = d.view.source.clone() else {
            return false;
        };
        let valid = d.view.companion.as_ref().is_some_and(|r| {
            r.width == rect.width
                && r.light == self.files.light
                && Arc::ptr_eq(&r.source, &source)
                && r.window.as_ref().is_some_and(|w| {
                    w.covers(
                        preview::CodeViewport {
                            top,
                            rows: rect.height as usize,
                        },
                        self.files.light,
                    )
                })
        });
        if valid || d.view.companion_pending.is_some() {
            return false;
        }
        self.serial += 1;
        let id = self.serial;
        if self
            .io
            .send(Job::Companion {
                path: d.path.clone(),
                id,
                source,
                width: rect.width,
                light: self.files.light,
                viewport: preview::CodeViewport {
                    top,
                    rows: rect.height as usize,
                },
            })
            .is_ok()
        {
            d.view.companion_pending = Some(id);
        }
        false
    }
    pub(super) fn feature_completion(&mut self, output: Completed) -> Option<Completed> {
        match output {
            Completed::Companion { path, id, result } => {
                if let Some(d) = &mut self.document
                    && d.path == path
                    && d.view.companion_pending == Some(id)
                {
                    d.view.companion_pending = None;
                    if d.source_snapshot()
                        .is_some_and(|source| Arc::ptr_eq(&source, &result.source))
                        && result.light == self.files.light
                    {
                        d.view.companion = Some(result);
                    }
                }
                None
            }
            Completed::Diff { path, id, result } => {
                if self.tools.diff_pending.as_ref() == Some(&(path, id)) {
                    self.tools.diff_pending = None;
                    self.tools.diff = Some(DiffView {
                        data: result,
                        top: 0,
                        left: 0,
                    });
                }
                None
            }
            other => Some(other),
        }
    }
    fn tab_buttons(&self) -> Vec<(Rect, PathBuf)> {
        if self.tools.paths.len() < 2 || self.document.as_ref().is_some_and(|d| d.editing) {
            return Vec::new();
        }
        let Some(rect) = self.geometry.preview else {
            return Vec::new();
        };
        let available = rect.width.saturating_sub(2);
        let count = (available / 24).max(1).min(self.tools.paths.len() as u16) as usize;
        let selected = self
            .document
            .as_ref()
            .and_then(|d| self.tools.paths.iter().position(|p| p == &d.path))
            .unwrap_or(0);
        let first = selected.saturating_sub(count - 1);
        let width = (available / count as u16).clamp(1, 33);
        self.tools
            .paths
            .iter()
            .skip(first)
            .take(count)
            .enumerate()
            .map(|(i, p)| {
                (
                    Rect::new(
                        rect.x + 1 + i as u16 * width,
                        rect.y + 1,
                        width.saturating_sub(1),
                        3,
                    ),
                    p.clone(),
                )
            })
            .collect()
    }
    pub(super) fn feature_mouse(&mut self, event: MouseEvent) -> Result<bool> {
        if self.tools.diff.is_some() || self.tools.diff_pending.is_some() {
            if let Some(v) = &mut self.tools.diff {
                match event.kind {
                    MouseEventKind::ScrollUp => v.top = v.top.saturating_sub(3),
                    MouseEventKind::ScrollDown => {
                        v.top = (v.top + 3).min(v.data.lines.len().saturating_sub(1))
                    }
                    _ => {}
                }
            }
            return Ok(true);
        }
        if self.modal() {
            return Ok(false);
        }
        if self.geometry.preview.is_some_and(|r| {
            (r.y + 1..=r.y + 3).contains(&event.row)
                && event.column > r.x
                && event.column < r.right() - 1
        }) && self.tools.paths.len() > 1
        {
            match event.kind {
                MouseEventKind::ScrollDown => {
                    self.cycle_tab(false)?;
                    return Ok(true);
                }
                MouseEventKind::ScrollUp => {
                    self.cycle_tab(true)?;
                    return Ok(true);
                }
                _ => {}
            }
        }
        let point = (event.column, event.row).into();
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some((_, path)) = self
                .tab_buttons()
                .into_iter()
                .find(|(r, _)| r.contains(point))
        {
            self.open(path, true)?;
            return Ok(true);
        }
        if let Some((_, Some(rect))) = self.preview_panes()
            && rect.contains(point)
        {
            let d = self.document.as_mut().unwrap();
            let top = d.source_line(d.reading_byte());
            let target = match event.kind {
                MouseEventKind::ScrollDown => Some(top + 3),
                MouseEventKind::ScrollUp => Some(top.saturating_sub(3)),
                MouseEventKind::Down(MouseButton::Left) => {
                    Some(top + (event.row - rect.y) as usize)
                }
                MouseEventKind::ScrollRight => {
                    d.view.companion_left += 4;
                    None
                }
                MouseEventKind::ScrollLeft => {
                    d.view.companion_left = d.view.companion_left.saturating_sub(4);
                    None
                }
                _ => None,
            };
            if let Some(line) = target
                && let Some(byte) = d
                    .view
                    .starts
                    .get(line.min(d.view.starts.len().saturating_sub(1)))
                    .copied()
            {
                self.seek_reading(byte);
            }
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn draw_features(&self, frame: &mut Frame) {
        if self.geometry.preview.is_none() {
            return;
        }
        if let Some(d) = &self.document {
            let tabs = self.tab_buttons();
            if !tabs.is_empty()
                && let Some(rect) = self.geometry.preview
            {
                let background = if self.files.light {
                    Color::Rgb(220, 224, 232)
                } else {
                    Color::Rgb(24, 27, 34)
                };
                frame.render_widget(
                    Block::default().style(Style::new().bg(background)),
                    Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 3),
                );
            }
            for (r, path) in tabs {
                let current = d.path == path;
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let duplicate = self
                    .tools
                    .paths
                    .iter()
                    .filter(|p| p.file_name() == path.file_name())
                    .count()
                    > 1;
                let label = if duplicate {
                    format!(
                        "{} · {}",
                        name,
                        path.parent()
                            .and_then(|p| p.file_name())
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                } else {
                    name.into_owned()
                };
                let (foreground, background, edge) = match (self.files.light, current) {
                    (false, true) => (
                        Color::Rgb(230, 237, 246),
                        Color::Rgb(48, 63, 78),
                        Color::Cyan,
                    ),
                    (false, false) => (
                        Color::Rgb(159, 172, 190),
                        Color::Rgb(32, 36, 45),
                        Color::Rgb(70, 79, 94),
                    ),
                    (true, true) => (
                        Color::Rgb(25, 58, 83),
                        Color::Rgb(204, 229, 245),
                        Color::Rgb(0, 110, 150),
                    ),
                    (true, false) => (
                        Color::Rgb(92, 101, 119),
                        Color::Rgb(232, 235, 241),
                        Color::Rgb(160, 169, 185),
                    ),
                };
                let style = Style::new().fg(foreground).bg(background);
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {}",
                        if current { "●" } else { " " },
                        shortened(&label, r.width.saturating_sub(5) as usize)
                    ))
                    .style(if current {
                        style.add_modifier(Modifier::BOLD)
                    } else {
                        style
                    })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::new().fg(edge).bg(background)),
                    ),
                    r,
                );
            }
            if let Some((_, Some(inner))) = self.preview_panes() {
                let divider = Rect::new(inner.x - 1, inner.y, 1, inner.height);
                frame.render_widget(
                    Paragraph::new(vec![Line::raw("│"); inner.height as usize])
                        .style(Style::new().fg(Color::DarkGray)),
                    divider,
                );
                let top = d.source_line(d.reading_byte());
                if let Some(source) = &d.view.companion
                    && source.light == self.files.light
                    && source.width == inner.width
                    && d.source_snapshot()
                        .is_some_and(|s| Arc::ptr_eq(&s, &source.source))
                {
                    for y in 0..inner.height {
                        let row = top + y as usize;
                        let Some(raw) = source.text.lines.get(row) else {
                            break;
                        };
                        let line = source
                            .window
                            .as_ref()
                            .filter(|w| w.light == self.files.light)
                            .and_then(|w| w.line(row))
                            .unwrap_or(raw);
                        frame.render_widget(
                            Paragraph::new(format!("{:>4} │", row + 1))
                                .style(Style::new().fg(Color::DarkGray)),
                            Rect::new(inner.x, inner.y + y, 6.min(inner.width), 1),
                        );
                        frame.render_widget(
                            Paragraph::new(line.clone())
                                .scroll((0, d.view.companion_left.min(u16::MAX as usize) as u16)),
                            Rect::new(
                                inner.x + 6.min(inner.width),
                                inner.y + y,
                                inner.width.saturating_sub(6),
                                1,
                            ),
                        );
                    }
                } else {
                    frame.render_widget(Paragraph::new("Loading source…"), inner);
                }
            }
        }
    }
    pub(super) fn draw_feature_overlay(&self, frame: &mut Frame) {
        let Some(rect) = self.geometry.preview else {
            return;
        };
        if self.tools.navigation.is_some()
            || self.tools.diff.is_some()
            || self.tools.diff_pending.is_some()
        {
            let popup = if self.tools.navigation.is_some() {
                Rect::new(
                    rect.x + 2,
                    rect.y + 2,
                    rect.width.saturating_sub(4),
                    6.min(rect.height.saturating_sub(4)),
                )
            } else {
                rect
            };
            crate::ui::clear_popup(frame, popup);
            let base = if self.files.light {
                Style::new()
                    .fg(Color::Rgb(76, 79, 105))
                    .bg(Color::Rgb(239, 241, 245))
            } else {
                Style::new()
                    .fg(Color::Rgb(205, 214, 244))
                    .bg(Color::Rgb(35, 39, 48))
            };
            if let Some(n) = &self.tools.navigation {
                frame.render_widget(
                    Paragraph::new(format!(
                        "Line number or percent: {}\n{}\nEnter Go · Esc Cancel",
                        n.value, n.error
                    ))
                    .block(Block::bordered().title(" Go to position "))
                    .style(base),
                    popup,
                );
            } else {
                let title = self.tools.diff.as_ref().map_or(
                    " Changes · Loading · Esc Close ".into(),
                    |v| {
                        format!(
                            " Changes · +{} -{} · ↑↓ PgUp/PgDn · Esc Close ",
                            v.data.added, v.data.removed
                        )
                    },
                );
                frame.render_widget(Block::bordered().title(title).style(base), popup);
                let inner = crate::ui::preview_content(popup);
                if let Some(v) = &self.tools.diff {
                    for (y, line) in v
                        .data
                        .lines
                        .iter()
                        .skip(v.top)
                        .take(inner.height as usize)
                        .enumerate()
                    {
                        frame.render_widget(
                            Paragraph::new(line.clone()).scroll((0, v.left)).style(base),
                            Rect::new(inner.x, inner.y + y as u16, inner.width, 1),
                        );
                    }
                }
            }
        }
    }
    pub(super) fn draw_search_options(&self, frame: &mut Frame) {
        if self.query.as_ref().is_some_and(|q| !q.tree)
            && let Some(d) = &self.document
        {
            let o = d.view.search_options;
            let text = format!(
                "Alt+C Case [{}]  Alt+W Word [{}]  Alt+R Regex [{}]  {}{}",
                if o.case_sensitive { "x" } else { " " },
                if o.whole_word { "x" } else { " " },
                if o.regex { "x" } else { " " },
                d.view.search_error,
                if d.view.search_targets.len() == preview::tools::MAX_MATCHES {
                    " · first 20000 matches"
                } else {
                    ""
                }
            );
            let area = frame.area();
            frame.render_widget(
                Paragraph::new(Line::from(Span::raw(text)))
                    .style(Style::new().fg(Color::Black).bg(Color::Cyan)),
                Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            );
        }
    }
}
