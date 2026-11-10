//! File panels share the application's event loop and layout; they never own a PTY.
use crate::{
    files::{self, Completed, Files, Job, Key, Workers},
    preview::{self, Loaded, Rendered},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Clear, Paragraph, StatefulWidget},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tui_tree_widget::{Tree, TreeItem};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
mod reading;
mod scroll;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Terminal,
    Tree,
    Preview,
    Editor,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Panels {
    pub toggle: Option<Rect>,
    pub tree: Option<Rect>,
    pub agent: Option<Rect>,
    pub preview: Option<Rect>,
    pub debug: Option<Rect>,
    pub overlay: bool,
}
pub fn panels(area: Rect, split: bool, sidebar: bool, preview: bool) -> Panels {
    let base = if split { 94 } else { 62 };
    if area.width < base || area.height < 12 {
        return Panels::default();
    }
    let s = if sidebar && area.width >= base + 20 {
        28.min(area.width - base)
    } else {
        0
    };
    let body = Rect::new(area.x, area.y, area.width, area.height - 2);
    let mut result = Panels {
        toggle: Some(Rect::new(area.x, area.bottom() - 1, 8, 1)),
        tree: (s > 0).then_some(Rect::new(body.x, body.y, s, body.height)),
        ..Panels::default()
    };
    let t = body.width - s;
    let d = if split {
        32.max(t * 30 / 100).min(t - 62)
    } else {
        0
    };
    let l = t - d;
    let a = if preview && l >= 104 {
        62.max(l.div_ceil(2))
    } else {
        l
    };
    result.agent = Some(Rect::new(body.x + s, body.y, a, body.height));
    if d > 0 {
        result.debug = Some(Rect::new(body.right() - d, body.y, d, body.height));
    }
    if preview {
        result.overlay = l < 104;
        result.preview = Some(if result.overlay {
            result.agent.unwrap()
        } else {
            Rect::new(body.x + s + a, body.y, l - a, body.height)
        });
    }
    result
}

#[derive(Clone, Debug)]
pub enum Gesture {
    Idle,
    Pressed { path: PathBuf, origin: (u16, u16) },
    Dragging { path: PathBuf },
}
struct Query {
    value: String,
    tree: bool,
    original: (Vec<Key>, Vec<Vec<Key>>, usize),
    matches: Vec<PathBuf>,
    index: usize,
}
#[derive(Clone)]
pub struct Menu {
    pub items: Vec<(&'static str, Command)>,
    pub selected: usize,
    pub target: PathBuf,
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Insert,
    CopyAbsolute,
    CopyRelative,
    Reveal,
    External,
    Collapse,
    Release,
    Icons,
    Theme,
    Locate,
    CopyContent,
    CopySource,
    Close,
    Edit,
    Save,
    Undo,
    Redo,
    Cut,
    Paste,
    Find,
    Project,
    EndEdit,
    Mode,
    Outline,
    Back,
}
#[derive(Debug)]
pub enum Action {
    None,
    Agent,
    Insert(PathBuf),
    ValidatedInput(PathBuf),
    Manage(char),
}
#[derive(Clone)]
enum Pending {
    Reading(preview::ReadingAction),
    Open(PathBuf, bool),
    Close,
    Manage(char),
    Reload,
    EndEdit,
    External(PathBuf),
}
enum Prompt {
    Unsaved(Pending),
    External(PathBuf),
}

pub struct Document {
    reader: reading::DocumentReading,
    pub editor: Option<crate::editor::Editor>,
    pub editing: bool,
    edit_requested: Option<u64>,
    render_revision: u64,
    pub path: PathBuf,
    pub loaded: Option<std::sync::Arc<Loaded>>,
    pub rendered: Option<Rendered>,
    pub loading: bool,
    pub stale: String,
    pub markdown: bool,
    pub scroll: [(usize, usize); 2],
    pub selection: Option<((usize, usize), (usize, usize))>,
    pub search: String,
    pub matches: Vec<(usize, usize, usize)>,
    pub match_index: usize,
    id: u64,
    render_id: u64,
    render_pending: bool,
    code_stats: Option<preview::HighlightStats>,
    code_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    code_desired: Option<(preview::CodeViewport, u64, bool)>,
    code_changed: Instant,
    code_pending: bool,
    code_id: u64,
    code_revision: u64,
    markdown_stats: Option<preview::MarkdownStats>,
    markdown_end: bool,
    markdown_copy: bool,
    markdown_search_jump: bool,
}
impl Document {
    fn new(path: PathBuf, id: u64) -> Self {
        Self {
            reader: Default::default(),
            editor: None,
            editing: false,
            edit_requested: None,
            render_revision: 0,
            path,
            loaded: None,
            rendered: None,
            loading: true,
            stale: String::new(),
            markdown: false,
            scroll: [(0, 0); 2],
            selection: None,
            search: String::new(),
            matches: vec![],
            match_index: 0,
            id,
            render_id: 0,
            render_pending: false,
            code_stats: None,
            code_generation: Default::default(),
            code_desired: None,
            code_changed: Instant::now(),
            code_pending: false,
            code_id: 0,
            code_revision: 0,
            markdown_stats: None,
            markdown_end: false,
            markdown_copy: false,
            markdown_search_jump: false,
        }
    }
    pub fn offset(&self) -> (usize, usize) {
        self.scroll[usize::from(self.markdown)]
    }
    pub fn highlight_stats(&self) -> Option<preview::HighlightStats> {
        self.code_stats
    }
    pub fn markdown_stats(&self) -> Option<preview::MarkdownStats> {
        self.markdown_stats
    }
    pub fn offset_mut(&mut self) -> &mut (usize, usize) {
        &mut self.scroll[usize::from(self.markdown)]
    }
    fn search(&mut self) {
        let _perf = crate::perf::span(crate::perf::Metric::PreviewSearch);
        self.matches.clear();
        self.match_index = 0;
        if self.search.is_empty() {
            return;
        }
        if let Some(r) = &self.rendered {
            for (row, line) in r.plain.iter().enumerate() {
                // Preserve original character positions even when Unicode case folding changes byte lengths.
                let chars: Vec<char> = line.chars().collect();
                let needle = self.search.to_lowercase();
                let count = self.search.chars().count();
                for col in 0..chars.len() {
                    let part: String = chars[col..chars.len().min(col + count)].iter().collect();
                    if part.to_lowercase() == needle {
                        self.matches.push((row, col, col + count));
                    }
                }
            }
        }
    }
    fn jump_match(&mut self, back: bool) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.match_index = if back {
            (self.match_index + n - 1) % n
        } else {
            (self.match_index + 1) % n
        };
        let (row, col, _) = self.matches[self.match_index];
        *self.offset_mut() = (row, col);
    }
    pub fn selected_text(&self) -> Option<String> {
        let (mut start, mut end) = self.selection?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let r = self.rendered.as_ref()?;
        let mut out = String::new();
        for row in start.0..=end.0.min(r.plain.len().saturating_sub(1)) {
            if row > start.0 {
                out.push('\n');
            }
            let line = &r.plain[row];
            let a = if row == start.0 { start.1 } else { 0 };
            let b = if row == end.0 {
                end.1
            } else {
                line.chars().count()
            };
            out.extend(line.chars().skip(a).take(b.saturating_sub(a)));
        }
        Some(out)
    }
}

pub struct Sidebar {
    reader: reading::ReadingUi,
    tree_view: Option<TreeView>,
    prompt: Option<Prompt>,
    after_save: Option<Pending>,
    pub ready_manage: Option<char>,
    pub files: Files,
    pub io: Workers,
    pub focus: Focus,
    pub visible: bool,
    pub document: Option<Document>,
    pub preview_visible: bool,
    pub gesture: Gesture,
    pub menu: Option<Menu>,
    query: Option<Query>,
    pub notice: String,
    notice_at: Option<Instant>,
    serial: u64,
    pub geometry: Panels,
    source_focus: Focus,
    suspended: Option<Focus>,
    drag_restore: bool,
    was_active: bool,
    was_watching: bool,
    pub ready_input: Option<PathBuf>,
    pub checking_input: bool,
    locate_pending: Option<PathBuf>,
}
// One bounded projection, rebuilt only when its labels/structure/width change.
// Selection, scroll and focus stay in TreeState and never invalidate the labels.
struct TreeView {
    revision: u64,
    width: u16,
    icons: bool,
    light: bool,
    preview: Option<PathBuf>,
    items: Vec<TreeItem<'static, Key>>,
    viewport: Option<TreeViewport>,
}
struct TreeViewport {
    row_count: usize,
    rect: Rect,
    focused: bool,
    offset: usize,
    selected: Vec<Key>,
    opened: std::collections::HashSet<Vec<Key>>,
    buffer: ratatui::buffer::Buffer,
}
impl Sidebar {
    pub fn dirty(&self) -> bool {
        self.document
            .as_ref()
            .and_then(|d| d.editor.as_ref())
            .is_some_and(|e| e.dirty() || e.saving)
    }
    pub fn modal(&self) -> bool {
        self.prompt.is_some()
            || self.menu.is_some()
            || self.query.is_some()
            || self.reader.outline.is_some()
    }
    pub fn leave(&mut self, command: char) -> bool {
        if self.modal() {
            return false;
        }
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::Manage(command)));
            false
        } else {
            true
        }
    }
    fn discard(&mut self) {
        self.serial += 1;
        if let Some(d) = &mut self.document {
            d.reader.seek = Some(d.reading_byte());
            d.editor = None;
            d.editing = false;
            d.rendered = None;
            d.render_pending = false;
            d.render_id = self.serial;
            d.edit_requested = None;
        }
    }
    fn perform(&mut self, pending: Pending) -> Result<()> {
        match pending {
            Pending::Reading(action) => self.activate_reading(action)?,
            Pending::Open(path, focus) => self.open(path, focus)?,
            Pending::Close => self.close(),
            Pending::Manage(c) => self.ready_manage = Some(c),
            Pending::Reload => {
                self.discard();
                self.refresh_document()?;
            }
            Pending::EndEdit => {
                self.discard();
                self.focus = Focus::Preview;
            }
            Pending::External(path) => files::external(&path, false)?,
        }
        Ok(())
    }
    fn open_external(&mut self, path: PathBuf) -> Result<()> {
        if self.dirty() && self.document.as_ref().is_some_and(|d| d.path == path) {
            self.prompt = Some(Prompt::External(path));
        } else {
            files::external(&path, false)?;
        }
        Ok(())
    }
    fn begin_edit(&mut self) -> Result<()> {
        let d = self.document.as_mut().context("No document")?;
        if d.editor.is_some() {
            d.editing = true;
            self.focus = Focus::Editor;
            return Ok(());
        }
        ensure!(d.edit_requested.is_none(), "Editor is loading");
        self.serial += 1;
        let id = self.serial;
        self.io.send(Job::Edit {
            loaded: d.loaded.clone().context("Wait for the file to load")?,
            id,
        })?;
        d.edit_requested = Some(id);
        Ok(())
    }
    pub fn save(&mut self) -> Result<()> {
        let d = self.document.as_mut().context("No document")?;
        let e = d.editor.as_mut().context("File is not in edit mode")?;
        if e.saving {
            return Ok(());
        }
        if !e.dirty() {
            if let Some(p) = self.after_save.take() {
                self.perform(p)?;
            }
            return Ok(());
        }
        let request = crate::editor::Save {
            snapshot: e.loaded.clone(),
            source: e.source(),
            revision: e.revision,
        };
        self.io.send(Job::Save { request })?;
        e.saving = true;
        e.status = "Saving; buffer remains editable".into();
        Ok(())
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            reader: Default::default(),
            tree_view: None,
            prompt: None,
            after_save: None,
            ready_manage: None,
            files: Files::new(root),
            io: Workers::new(),
            focus: Focus::Terminal,
            visible: true,
            document: None,
            preview_visible: false,
            gesture: Gesture::Idle,
            menu: None,
            query: None,
            notice: String::new(),
            notice_at: None,
            serial: 0,
            geometry: Panels::default(),
            source_focus: Focus::Tree,
            suspended: None,
            drag_restore: false,
            was_active: false,
            was_watching: false,
            ready_input: None,
            checking_input: false,
            locate_pending: None,
        }
    }
    pub fn say(&mut self, text: impl Into<String>, transient: bool) {
        self.notice = text.into();
        self.notice_at = transient.then(Instant::now);
    }
    pub fn showing(&self) -> bool {
        self.preview_visible && self.document.is_some()
    }
    pub fn update_layout(&mut self, area: Rect, split: bool) -> bool {
        let next = panels(area, split, self.visible, self.showing());
        let changed = next != self.geometry;
        if changed {
            if next.preview.map(|r| r.width) != self.geometry.preview.map(|r| r.width)
                && let Some(d) = &mut self.document
                && d.rendered.is_some()
                && d.reader.seek.is_none()
            {
                d.reader.seek = Some(d.reading_byte());
            }
            if next.agent.is_none() {
                self.suspended.get_or_insert(self.focus);
            } else if let Some(f) = self.suspended.take() {
                self.focus = f;
            }
            if next.agent.is_some() && self.focus == Focus::Tree && next.tree.is_none() {
                self.focus = if next.overlay {
                    Focus::Preview
                } else {
                    Focus::Terminal
                };
            }
            if matches!(self.focus, Focus::Preview | Focus::Editor) && next.preview.is_none() {
                self.focus = Focus::Terminal;
            }
            self.geometry = next;
        }
        changed
    }
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.focus = Focus::Tree;
        } else if self.focus == Focus::Tree {
            self.focus = if self.geometry.overlay && self.showing() {
                Focus::Preview
            } else {
                Focus::Terminal
            };
        }
    }
    pub fn focus_tree(&mut self) {
        self.visible = true;
        self.focus = Focus::Tree;
    }
    pub fn focus_agent(&mut self) {
        self.focus = Focus::Terminal;
        if self.geometry.overlay {
            self.preview_visible = false;
        }
    }
    pub fn open(&mut self, path: PathBuf, focus: bool) -> Result<()> {
        if self.document.as_ref().is_some_and(|d| d.path == path) {
            self.preview_visible = true;
            if focus {
                self.source_focus = self.focus;
                self.focus = if self.document.as_ref().is_some_and(|d| d.editing) {
                    Focus::Editor
                } else {
                    Focus::Preview
                };
            }
            return Ok(());
        }
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::Open(path, focus)));
            return Ok(());
        }
        self.serial += 1;
        self.io.send(Job::Read {
            path: path.clone(),
            id: self.serial,
        })?;
        self.source_focus = self.focus;
        self.remember_reading();
        self.document = Some(Document::new(path, self.serial));
        self.restore_reading();
        self.preview_visible = true;
        if focus {
            self.focus = Focus::Preview;
        }
        Ok(())
    }
    pub fn close(&mut self) {
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::Close));
            return;
        }
        self.remember_reading();
        self.document = None;
        self.preview_visible = false;
        if matches!(self.focus, Focus::Preview | Focus::Editor) {
            self.focus = if self.source_focus == Focus::Tree && self.geometry.tree.is_some() {
                Focus::Tree
            } else {
                Focus::Terminal
            };
        }
    }
    pub fn refresh_document(&mut self) -> Result<()> {
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::Reload));
            return Ok(());
        }
        if let Some(d) = &mut self.document {
            self.serial += 1;
            d.id = self.serial;
            d.loading = true;
            self.io.send(Job::Read {
                path: d.path.clone(),
                id: d.id,
            })?;
        }
        Ok(())
    }
    fn render_document(&mut self) -> Result<()> {
        let Some(d) = &mut self.document else {
            return Ok(());
        };
        let Some(loaded) = &d.loaded else {
            return Ok(());
        };
        if d.render_pending {
            return Ok(());
        }
        let width = self
            .geometry
            .preview
            .map(crate::ui::content)
            .map_or(80, |r| r.width);
        self.serial += 1;
        let id = self.serial;
        let source = d.editor.as_ref().map_or_else(
            || loaded.source.clone(),
            |e| {
                d.rendered
                    .as_ref()
                    .filter(|_| d.render_revision == e.revision)
                    .map_or_else(|| std::sync::Arc::from(e.source()), |r| r.source.clone())
            },
        );
        d.render_revision = d.editor.as_ref().map_or(0, |e| e.revision);
        d.code_generation
            .store(id, std::sync::atomic::Ordering::Relaxed);
        let viewport = preview::CodeViewport {
            top: if d.editing {
                d.editor
                    .as_ref()
                    .map_or(0, |e| e.area.scroll_offset().0 as usize)
            } else {
                d.offset().0
            },
            rows: self
                .geometry
                .preview
                .map(crate::ui::content)
                .map_or(40, |r| r.height.max(1)) as usize,
        };
        self.io.send(Job::Render {
            editor: d.editing,
            path: d.path.clone(),
            id,
            source,
            markdown: d.markdown && !d.editing,
            width,
            light: self.files.light,
            viewport,
            generation: d.code_generation.clone(),
            complete: !d.search.is_empty() || d.markdown_end || d.markdown_copy,
            reading: preview::reading::ReadingOptions {
                expanded: d.reader.expanded.clone(),
                seek: d.reader.seek,
            },
        })?;
        d.render_id = id;
        d.render_pending = true;
        Ok(())
    }
    fn highlight_document(&mut self) {
        let Some(rect) = self.geometry.preview.map(crate::ui::content) else {
            return;
        };
        let Some(d) = &mut self.document else {
            return;
        };
        let Some(r) = &d.rendered else {
            return;
        };
        if r.markdown {
            return;
        }
        let revision = d.editor.as_ref().map_or(0, |e| e.revision);
        let top = if d.editing {
            d.editor
                .as_ref()
                .map_or(0, |e| e.area.scroll_offset().0 as usize)
        } else {
            d.offset().0
        };
        let viewport = preview::CodeViewport {
            top,
            rows: rect.height.max(1) as usize,
        };
        let desired = (viewport, revision, self.files.light);
        if d.code_desired != Some(desired) {
            let first = d.code_desired.is_none();
            d.code_desired = Some(desired);
            d.code_changed = Instant::now()
                - if first {
                    Duration::from_millis(80)
                } else {
                    Duration::ZERO
                };
            d.code_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Some(e) = &mut d.editor
                && e.highlight
                    .as_ref()
                    .is_some_and(|(_, w)| w.light != self.files.light)
            {
                e.highlight = None;
            }
        }
        if d.render_pending
            || revision != d.render_revision
            || d.code_pending
            || d.code_changed.elapsed() < Duration::from_millis(80)
            || r.window
                .as_ref()
                .is_some_and(|w| w.covers(viewport, self.files.light))
        {
            return;
        }
        self.serial += 1;
        let id = self.serial;
        d.code_generation
            .store(id, std::sync::atomic::Ordering::Relaxed);
        let result = self.io.send(Job::Highlight {
            path: d.path.clone(),
            id,
            source: r.source.clone(),
            viewport,
            light: self.files.light,
            generation: d.code_generation.clone(),
            expected: id,
        });
        match result {
            Ok(()) => {
                d.code_id = id;
                d.code_revision = revision;
                d.code_pending = true;
            }
            Err(e) => self.say(e.to_string(), false),
        }
    }
    pub fn tick(&mut self, active: bool) -> bool {
        let _perf = crate::perf::span(crate::perf::Metric::Tick);
        let mut changed = false;
        if self
            .notice_at
            .is_some_and(|t| t.elapsed() > Duration::from_secs(3))
        {
            self.notice.clear();
            self.notice_at = None;
            changed = true;
        }
        if !active {
            self.files.watch(false, None);
            self.was_watching = false;
        }
        if active && !self.was_active {
            let root = self.files.root.clone();
            if let Err(e) = self.files.request(&root, true, &self.io) {
                self.say(e.to_string(), false);
            }
            self.was_active = true;
        }
        let mut outputs = Vec::new();
        while let Ok(c) = self.io.rx.try_recv() {
            outputs.push(c);
        }
        for output in outputs {
            changed = true;
            match output {
                Completed::Edit { id, result } => {
                    if let Some(d) = &mut self.document
                        && d.edit_requested == Some(id)
                    {
                        d.edit_requested = None;
                        match result {
                            Ok(editor) => {
                                d.editor = Some(*editor);
                                d.editing = true;
                                d.markdown = false;
                                d.rendered = None;
                                d.render_pending = false;
                                d.render_id = 0;
                                if self.preview_visible
                                    && matches!(self.focus, Focus::Preview | Focus::Editor)
                                {
                                    self.focus = Focus::Editor;
                                }
                            }
                            Err(e) => self.say(format!("Cannot edit: {e:#}"), false),
                        }
                    }
                }
                Completed::Save {
                    path,
                    revision,
                    source,
                    result,
                } => {
                    if let Some(d) = &mut self.document
                        && d.path == path
                        && let Some(editor) = &mut d.editor
                    {
                        editor.saving = false;
                        match result {
                            Ok(loaded) => {
                                let loaded = std::sync::Arc::new(loaded);
                                editor.mark_saved(loaded.clone(), source.replace("\r\n", "\n"));
                                d.loaded = Some(loaded);
                                d.stale.clear();
                                self.say("Saved", true);
                                if let Some(pending) = self.after_save.take() {
                                    if self.dirty() {
                                        self.prompt = Some(Prompt::Unsaved(pending));
                                    } else if let Err(e) = self.perform(pending) {
                                        self.say(e.to_string(), false);
                                    }
                                }
                            }
                            Err(e) => {
                                editor.conflict = true;
                                editor.status = format!("Save failed: {e:#}");
                                let message = editor.status.clone();
                                self.after_save = None;
                                self.say(message, false);
                            }
                        }
                        let _ = revision;
                    }
                }
                Completed::Input(result) => {
                    self.checking_input = false;
                    match result {
                        Ok(path) => self.ready_input = Some(path),
                        Err(e) => self.say(format!("Path not sent: {e:#}"), false),
                    }
                }
                Completed::Directory {
                    path,
                    generation,
                    batch,
                } => {
                    self.files.apply(path, generation, batch);
                    let opened: Vec<_> = self
                        .files
                        .state
                        .opened()
                        .iter()
                        .filter_map(|id| match id.last() {
                            Some(Key::Entry(p)) => Some(p.clone()),
                            _ => None,
                        })
                        .collect();
                    for p in opened {
                        if self.files.nodes.contains_key(&p) && !self.files.dirs.contains_key(&p) {
                            let _ = self.files.request(&p, false, &self.io);
                        }
                    }
                }
                Completed::Read { path, id, result } => {
                    if let Some(d) = &mut self.document
                        && d.path == path
                        && d.id == id
                    {
                        d.loading = false;
                        match result {
                            Ok(loaded) => {
                                let loaded = std::sync::Arc::new(loaded);
                                if let Some(editor) = &mut d.editor {
                                    if loaded.bytes != editor.loaded.bytes
                                        || loaded.identity != editor.loaded.identity
                                    {
                                        if editor.dirty() || editor.saving {
                                            editor.conflict = true;
                                            d.stale="Disk changed; local edits retained. F5 reloads after confirmation.".into();
                                            continue;
                                        }
                                        let cursor = editor.area.cursor();
                                        match crate::editor::Editor::from_loaded(loaded.clone()) {
                                            Ok(mut next) => {
                                                next.area.move_cursor(
                                                    ratatui_textarea::CursorMove::Jump(
                                                        cursor.0 as u16,
                                                        cursor.1 as u16,
                                                    ),
                                                );
                                                d.editor = Some(next);
                                            }
                                            Err(e) => {
                                                d.stale =
                                                    format!("File became preview-only: {e:#}");
                                                d.editor = None;
                                                d.editing = false;
                                                self.focus = Focus::Preview;
                                            }
                                        }
                                    } else {
                                        d.loading = false;
                                        continue;
                                    }
                                }
                                let first = d.loaded.is_none();
                                let different = d
                                    .loaded
                                    .as_ref()
                                    .is_none_or(|old| old.bytes != loaded.bytes);
                                if first && d.reader.seek.is_none() {
                                    d.markdown = loaded.markdown();
                                }
                                d.loaded = Some(loaded);
                                d.stale.clear();
                                if different {
                                    d.selection = None;
                                    d.rendered = None;
                                    d.render_pending = false;
                                    d.render_id = 0;
                                }
                            }
                            Err(e) => {
                                d.stale = format!("Content may be stale: {e:#}");
                            }
                        }
                    }
                }
                Completed::Highlight {
                    path,
                    id,
                    window,
                    stats,
                } => {
                    if let Some(d) = &mut self.document
                        && d.path == path
                        && d.code_id == id
                    {
                        d.code_pending = false;
                        d.code_stats = Some(stats);
                        if d.code_generation.load(std::sync::atomic::Ordering::Relaxed) != id {
                            continue;
                        }
                        if let Some(window) = window
                            && let Some(r) = &mut d.rendered
                        {
                            if let Some(e) = &mut d.editor
                                && e.revision == d.code_revision
                            {
                                e.highlight = Some((e.revision, window.clone()));
                            }
                            r.window = Some(window);
                        }
                    }
                }
                Completed::Render {
                    path,
                    id,
                    result,
                    stats,
                    markdown_stats,
                } => {
                    let expected_width = self
                        .geometry
                        .preview
                        .map(crate::ui::content)
                        .map_or(80, |r| r.width);
                    if let Some(d) = &mut self.document
                        && d.path == path
                        && d.render_id == id
                    {
                        d.render_pending = false;
                        if result.width != expected_width
                            || result.light != self.files.light
                            || result.markdown != (d.markdown && !d.editing)
                        {
                            continue;
                        }
                        if d.editor
                            .as_ref()
                            .is_some_and(|e| e.revision != d.render_revision)
                        {
                            continue;
                        }
                        d.code_stats = stats;
                        d.markdown_stats = markdown_stats;
                        // A new snapshot invalidates colors; viewport highlighting is separate.
                        if let Some(e) = &mut d.editor {
                            e.highlight = result
                                .window
                                .as_ref()
                                .filter(|_| !result.markdown)
                                .map(|w| (e.revision, w.clone()));
                        }
                        d.code_desired = None;
                        d.code_generation
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if d.rendered.as_ref().is_some_and(|previous| {
                            previous.width != result.width
                                || previous.markdown != result.markdown
                                || previous.source != result.source
                        }) {
                            d.selection = None;
                        }
                        d.rendered = Some(result);
                        d.restore_reading_position();
                        let len = d.rendered.as_ref().map_or(0, |r| r.plain.len());
                        if d.rendered.as_ref().is_some_and(|r| r.complete) {
                            if d.markdown_end {
                                d.offset_mut().0 = len.saturating_sub(1);
                                d.markdown_end = false;
                            } else {
                                d.offset_mut().0 = d.offset().0.min(len.saturating_sub(1));
                            }
                        }
                        d.search();
                        if d.markdown_search_jump && d.rendered.as_ref().is_some_and(|r| r.complete)
                        {
                            if let Some(&(row, col, _)) = d.matches.first() {
                                *d.offset_mut() = (row, col);
                            }
                            d.markdown_search_jump = false;
                        }
                    }
                }
            }
        }
        if active && let Some(d) = &self.document {
            let width = self
                .geometry
                .preview
                .map(crate::ui::content)
                .map_or(80, |r| r.width);
            if !d.render_pending
                && d.loaded.is_some()
                && (d.rendered.as_ref().is_none_or(|r| {
                    r.width != width
                        || r.light != self.files.light
                        || r.markdown != (d.markdown && !d.editing)
                }) || d.reader.seek.is_some()
                    || d.reader.fragment.is_some()
                    || d.editor
                        .as_ref()
                        .is_some_and(|e| e.revision != d.render_revision)
                    || d.rendered.as_ref().is_some_and(|r| {
                        r.markdown
                            && ((!r.complete
                                && (!d.search.is_empty()
                                    || d.markdown_end
                                    || d.markdown_copy
                                    || d.offset().0.saturating_add(
                                        self.geometry
                                            .preview
                                            .map(crate::ui::content)
                                            .map_or(40, |r| r.height)
                                            as usize,
                                    ) > r.plain.len()))
                                || r.window.as_ref().is_none_or(|w| {
                                    !w.covers(
                                        preview::CodeViewport {
                                            top: d.offset().0,
                                            rows: self
                                                .geometry
                                                .preview
                                                .map(crate::ui::content)
                                                .map_or(40, |r| r.height)
                                                as usize,
                                        },
                                        self.files.light,
                                    )
                                }))
                    }))
                && let Err(e) = self.render_document()
            {
                self.say(e.to_string(), false);
            }
        }
        if active && self.showing() {
            self.highlight_document();
        }
        if let Some(d) = &mut self.document
            && d.markdown_copy
            && let Some(r) = &d.rendered
            && r.complete
        {
            let text = r.plain.join("\n");
            d.markdown_copy = false;
            if let Err(e) = files::copy(text) {
                self.say(e.to_string(), false);
            }
        }
        self.files.pinned = self.document.as_ref().map(|d| d.path.clone());
        let watching = active && (self.geometry.tree.is_some() || self.showing());
        if watching && !self.was_watching && self.was_active {
            if let Err(e) = self.files.refresh(&self.io) {
                self.say(e.to_string(), false);
            }
            // A disabled watcher may have missed a disk update. Recheck even a dirty
            // buffer; the Read result compares the snapshot without replacing edits.
            if let Some(d) = &mut self.document {
                self.serial += 1;
                d.id = self.serial;
                let _ = self.io.send(Job::Read {
                    path: d.path.clone(),
                    id: d.id,
                });
            }
        }
        self.was_watching = watching;
        self.files
            .watch(watching, self.document.as_ref().map(|d| d.path.as_path()));
        if let Some(paths) = self.files.changes() {
            if self
                .document
                .as_ref()
                .is_some_and(|d| paths.iter().any(|p| p == &d.path || p == &self.files.root))
                && let Some(d) = &mut self.document
            {
                self.serial += 1;
                d.id = self.serial;
                let _ = self.io.send(Job::Read {
                    path: d.path.clone(),
                    id: d.id,
                });
            }
            // Refresh only previously loaded scopes; expanded descendants are restored lazily.
            if !paths.is_empty() {
                let _ = self.files.refresh_paths(&paths, &self.io);
                changed = true;
            }
        }
        if let Some(target) = self.locate_pending.clone() {
            let id = self.files.identifier(&target);
            let mut waiting = false;
            for (i, key) in id.iter().enumerate() {
                if let Key::Entry(path) = key {
                    if !self.files.nodes.contains_key(path) {
                        let parent = path.parent().unwrap_or(&self.files.root);
                        if let Some(dir) = self.files.dirs.get(parent)
                            && dir.complete
                        {
                            self.say(
                                "File is missing or filtered; adjust filters or refresh",
                                false,
                            );
                            self.locate_pending = None;
                        } else {
                            waiting = true;
                        }
                        break;
                    }
                    if i + 1 < id.len() {
                        self.files.state.open(id[..=i].to_vec());
                        if !self
                            .files
                            .dirs
                            .get(path)
                            .is_some_and(|d| d.complete || d.loading)
                        {
                            let _ = self.files.request(path, false, &self.io);
                            waiting = true;
                        }
                    }
                }
            }
            if !waiting && self.locate_pending.is_some() {
                self.files.state.select(id);
                self.files.state.scroll_selected_into_view();
                self.locate_pending = None;
            }
        }
        changed
    }
    fn activate_tree(&mut self, focus: bool) -> Result<()> {
        match self.files.selected() {
            Some(Key::Entry(p)) => {
                if self.files.nodes.get(&p).is_some_and(|n| n.directory) {
                    self.files.toggle(&p, &self.io)?;
                } else {
                    self.open(p, focus)?;
                }
            }
            Some(Key::More(p)) => self.files.request(&p, false, &self.io)?,
            _ => {}
        }
        Ok(())
    }
    pub fn menu(&mut self) {
        let target = if self.focus == Focus::Tree {
            self.files.selected_path()
        } else {
            self.document.as_ref().map(|d| d.path.clone())
        };
        let Some(target) = target else {
            return;
        };
        let mut items = vec![
            ("Insert path into Agent", Command::Insert),
            ("Copy absolute path", Command::CopyAbsolute),
            ("Copy project-relative path", Command::CopyRelative),
            ("Reveal in Explorer", Command::Reveal),
            ("Open disk version", Command::External),
        ];
        if self.focus == Focus::Tree {
            items.extend([
                ("Collapse all", Command::Collapse),
                ("Release collapsed cache", Command::Release),
                ("Toggle file icons", Command::Icons),
                ("Toggle light/dark colors", Command::Theme),
            ]);
        } else {
            items.push(("Edit source", Command::Edit));
            if self.document.as_ref().is_some_and(|d| d.editor.is_some()) {
                items.extend([
                    ("Save", Command::Save),
                    ("Undo", Command::Undo),
                    ("Redo", Command::Redo),
                    ("Copy selection", Command::CopyContent),
                    ("Cut selection", Command::Cut),
                    ("Paste", Command::Paste),
                    ("Find", Command::Find),
                    ("Preview current content", Command::Project),
                    ("End editing", Command::EndEdit),
                ]);
            }
            items.extend([
                ("Heading outline [T]", Command::Outline),
                ("Reading back [Alt+Left]", Command::Back),
                ("Locate in file tree", Command::Locate),
                ("Copy content", Command::CopyContent),
                ("Copy source", Command::CopySource),
                ("Close preview", Command::Close),
            ]);
        }
        self.menu = Some(Menu {
            items,
            selected: 0,
            target,
        });
    }
    fn command(&mut self, command: Command, target: PathBuf) -> Result<Action> {
        match command {
            Command::Mode => {
                if let Some(d) = &mut self.document
                    && d.loaded.as_ref().is_some_and(|l| l.markdown())
                {
                    d.reader.seek = Some(d.reading_byte());
                    d.markdown = !d.markdown;
                    d.render_id = 0;
                    d.render_pending = false;
                }
            }
            Command::Outline => self.start_outline(),
            Command::Back => self.reading_back()?,
            Command::Insert => return Ok(Action::Insert(target)),
            Command::CopyAbsolute => files::copy(target.to_string_lossy().into_owned())?,
            Command::CopyRelative => {
                let p = target.strip_prefix(&self.files.root)?;
                files::copy(if p.as_os_str().is_empty() {
                    ".".into()
                } else {
                    p.to_string_lossy().into_owned()
                })?;
            }
            Command::Reveal => files::external(&target, true)?,
            Command::External => self.open_external(target)?,
            Command::Collapse => self.files.collapse(&self.io),
            Command::Release => self.files.release_collapsed(&self.io),
            Command::Icons => {
                self.cancel_drag(true);
                self.files.icons = !self.files.icons;
            }
            Command::Theme => {
                self.files.light = !self.files.light;
                if let Some(d) = &mut self.document {
                    d.reader.seek = Some(d.reading_byte());
                    d.rendered = None;
                    d.selection = None;
                    d.render_pending = false;
                    d.render_id = 0;
                }
            }
            Command::Locate => {
                self.visible = true;
                self.locate_pending = Some(target);
                self.focus = Focus::Tree;
            }
            Command::CopyContent | Command::CopySource => {
                if matches!(command, Command::CopyContent)
                    && let Some(d) = &mut self.document
                    && d.markdown
                    && !d.editing
                    && d.rendered.as_ref().is_some_and(|r| !r.complete)
                {
                    d.markdown_copy = true;
                    return Ok(Action::None);
                }
                let d = self.document.as_ref().context("No document")?;
                let source_selection = if matches!(command, Command::CopySource) && !d.editing {
                    d.selection
                        .and_then(|(a, b)| d.rendered.as_ref().map(|r| r.selected_source(a, b)))
                } else {
                    None
                };
                let text = if let Some(selected) = source_selection {
                    selected
                } else if let Some(e) = &d.editor {
                    if matches!(command, Command::CopyContent) && d.editing {
                        e.selected().unwrap_or_else(|| e.source())
                    } else if matches!(command, Command::CopySource) || !d.markdown {
                        e.source()
                    } else {
                        d.rendered.as_ref().context("Rendering")?.plain.join("\n")
                    }
                } else if matches!(command, Command::CopySource) || !d.markdown {
                    preview::copy_text(&d.loaded.as_ref().context("File not loaded")?.source)
                } else {
                    d.rendered.as_ref().context("Rendering")?.plain.join("\n")
                };
                files::copy(text)?;
            }
            Command::Close => self.close(),
            Command::Edit => self.begin_edit()?,
            Command::Save => self.save()?,
            Command::Undo | Command::Redo | Command::Cut | Command::Paste | Command::Find => {
                let e = self
                    .document
                    .as_mut()
                    .and_then(|d| d.editor.as_mut())
                    .context("No edit buffer")?;
                match command {
                    Command::Undo => e.undo()?,
                    Command::Redo => e.redo()?,
                    Command::Cut => e.cut()?,
                    Command::Paste => {
                        let text = arboard::Clipboard::new()?.get_text()?;
                        e.paste(&text)?;
                    }
                    Command::Find => e.search_input = Some(String::new()),
                    _ => {}
                }
                self.focus = Focus::Editor;
            }
            Command::Project => {
                if let Some(d) = &mut self.document {
                    d.editing = false;
                    d.markdown = d.loaded.as_ref().is_some_and(|l| l.markdown());
                    d.render_pending = false;
                    d.rendered = None;
                    d.render_id = 0;
                    self.focus = Focus::Preview;
                }
            }
            Command::EndEdit => {
                if self.dirty() {
                    self.prompt = Some(Prompt::Unsaved(Pending::EndEdit));
                } else {
                    self.perform(Pending::EndEdit)?;
                }
            }
        }
        self.say("Done", true);
        Ok(Action::None)
    }
    fn start_query(&mut self, tree: bool) {
        self.query = Some(Query {
            value: String::new(),
            tree,
            original: (
                self.files.state.selected().to_vec(),
                self.files.state.opened().iter().cloned().collect(),
                self.files.state.get_offset(),
            ),
            matches: vec![],
            index: 0,
        });
    }
    fn update_query(&mut self) {
        let Some(q) = &mut self.query else {
            return;
        };
        if q.tree {
            q.matches = self
                .files
                .nodes
                .values()
                .filter(|n| n.name.to_lowercase().contains(&q.value.to_lowercase()))
                .map(|n| n.path.clone())
                .collect();
            q.matches.sort();
            q.index = q.index.min(q.matches.len().saturating_sub(1));
            if let Some(p) = q.matches.get(q.index) {
                let id = self.files.identifier(p);
                for i in 1..id.len() {
                    self.files.state.open(id[..i].to_vec());
                }
                self.files.state.select(id);
                self.files.state.scroll_selected_into_view();
            }
        } else if let Some(d) = &mut self.document {
            d.search = q.value.clone();
            d.markdown_search_jump = d.markdown && d.rendered.as_ref().is_some_and(|r| !r.complete);
            d.search();
            if let Some(&(row, col, _)) = d.matches.first() {
                *d.offset_mut() = (row, col);
            }
        }
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        if let Some(o) = &mut self.reader.outline {
            o.paste(text);
            return Ok(());
        }
        if self.prompt.is_some() {
            return Ok(());
        }
        if self.focus == Focus::Editor
            && let Some(e) = self.document.as_mut().and_then(|d| d.editor.as_mut())
        {
            return e.paste(text);
        }
        if let Some(q) = &mut self.query {
            ensure!(
                q.value.chars().count() + text.chars().count() <= 256,
                "Query exceeds 256 characters"
            );
            q.value.push_str(&text.replace(['\r', '\n'], ""));
            self.update_query();
        }
        Ok(())
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<Action> {
        if self.outline_key(key) {
            return Ok(Action::None);
        }
        if self.focus == Focus::Preview
            && key.code == KeyCode::Left
            && key.modifiers.contains(KeyModifiers::ALT)
        {
            self.reading_back()?;
            return Ok(Action::None);
        }
        // Selecting an already selected node can request ensure-visible even
        // when the public TreeState values do not change.
        if self.focus == Focus::Tree
            && let Some(view) = &mut self.tree_view
        {
            view.viewport = None;
        }
        if let Some(prompt) = self.prompt.take() {
            match prompt {
                Prompt::Unsaved(pending) => match key.code {
                    KeyCode::Esc | KeyCode::Char('c' | 'C') => {}
                    KeyCode::Char('s' | 'S') => {
                        self.after_save = Some(pending);
                        if let Err(e) = self.save() {
                            self.after_save = None;
                            return Err(e);
                        }
                    }
                    KeyCode::Char('d' | 'D') => {
                        if self
                            .document
                            .as_ref()
                            .and_then(|d| d.editor.as_ref())
                            .is_some_and(|e| e.saving)
                        {
                            self.prompt = Some(Prompt::Unsaved(pending));
                            self.say(
                                "Save in progress; wait before discarding or switching",
                                false,
                            );
                        } else {
                            self.discard();
                            self.perform(pending)?;
                        }
                    }
                    _ => self.prompt = Some(Prompt::Unsaved(pending)),
                },
                Prompt::External(path) => match key.code {
                    KeyCode::Esc | KeyCode::Char('c' | 'C') => {}
                    KeyCode::Char('o' | 'O') => files::external(&path, false)?,
                    KeyCode::Char('s' | 'S') => {
                        self.after_save = Some(Pending::External(path));
                        if let Err(e) = self.save() {
                            self.after_save = None;
                            return Err(e);
                        }
                    }
                    _ => self.prompt = Some(Prompt::External(path)),
                },
            }
            return Ok(Action::None);
        }
        if let Some(q) = &mut self.query {
            match key.code {
                KeyCode::Esc => {
                    let q = self.query.take().unwrap();
                    if q.tree {
                        self.files.state.close_all();
                        for id in q.original.1 {
                            self.files.state.open(id);
                        }
                        self.files.state.select(q.original.0);
                        self.files.state.scroll_up(usize::MAX);
                        self.files.state.scroll_down(q.original.2);
                        self.files.repair_selection();
                    }
                }
                KeyCode::Enter => {
                    let q = self.query.take().unwrap();
                    if q.tree && q.matches.is_empty() {
                        self.say(
                            "No loaded match; unexpanded directories were not searched",
                            false,
                        );
                    }
                }
                KeyCode::Backspace => {
                    q.value.pop();
                    self.update_query();
                }
                KeyCode::Up | KeyCode::Down if q.tree => {
                    if !q.matches.is_empty() {
                        q.index = if key.code == KeyCode::Up {
                            (q.index + q.matches.len() - 1) % q.matches.len()
                        } else {
                            (q.index + 1) % q.matches.len()
                        };
                        self.update_query();
                    }
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && q.value.chars().count() < 256 =>
                {
                    q.value.push(c);
                    self.update_query();
                }
                _ => {}
            }
            return Ok(Action::None);
        }
        if let Some(menu) = &mut self.menu {
            match key.code {
                KeyCode::Esc => self.menu = None,
                KeyCode::Up => menu.selected = menu.selected.saturating_sub(1),
                KeyCode::Down => menu.selected = (menu.selected + 1).min(menu.items.len() - 1),
                KeyCode::Enter => {
                    let menu = self.menu.take().unwrap();
                    return self.command(menu.items[menu.selected].1, menu.target);
                }
                _ => {}
            }
            return Ok(Action::None);
        }
        if key.code == KeyCode::Esc && !matches!(self.gesture, Gesture::Idle) {
            self.cancel_drag(true);
            return Ok(Action::None);
        }
        if key.code == KeyCode::F(2) {
            self.menu();
            return Ok(Action::None);
        }
        if key.code == KeyCode::F(5) {
            self.files.retry_watch();
            self.files.refresh(&self.io)?;
            self.refresh_document()?;
            return Ok(Action::None);
        }
        if self.focus == Focus::Editor {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                self.save()?;
            } else if let Some(e) = self.document.as_mut().and_then(|d| d.editor.as_mut()) {
                e.key(key)?;
            }
            return Ok(Action::None);
        }
        if self.focus == Focus::Tree {
            match key.code {
                KeyCode::Up => {
                    self.files.state.key_up();
                }
                KeyCode::Down => {
                    self.files.state.key_down();
                }
                KeyCode::Home => {
                    self.files.state.select_first();
                }
                KeyCode::End => {
                    self.files.state.select_last();
                }
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        self.files.state.key_up();
                    }
                }
                KeyCode::PageDown => {
                    for _ in 0..10 {
                        self.files.state.key_down();
                    }
                }
                KeyCode::Left => {
                    if let Some(p) = self.files.selected_path() {
                        let id = self.files.identifier(&p);
                        if self.files.state.opened().contains(&id) {
                            self.files.toggle(&p, &self.io)?;
                        } else {
                            self.files.state.key_left();
                        }
                    }
                }
                KeyCode::Right => {
                    if let Some(p) = self.files.selected_path()
                        && self.files.nodes.get(&p).is_some_and(|n| n.directory)
                        && !self
                            .files
                            .state
                            .opened()
                            .contains(&self.files.identifier(&p))
                    {
                        self.files.toggle(&p, &self.io)?;
                    }
                }
                KeyCode::Enter => self.activate_tree(true)?,
                KeyCode::Char('/') => self.start_query(true),
                KeyCode::Char('.') => {
                    self.files.hidden = !self.files.hidden;
                    self.files.refresh(&self.io)?;
                }
                KeyCode::Char('i' | 'I') => {
                    self.files.ignored = !self.files.ignored;
                    self.files.refresh(&self.io)?;
                }
                KeyCode::Esc => {
                    self.focus_agent();
                    return Ok(Action::Agent);
                }
                _ => {}
            }
        } else if self.focus == Focus::Preview {
            let page = self
                .geometry
                .preview
                .map(crate::ui::content)
                .map_or(10, |r| r.height.saturating_sub(1).max(1) as usize);
            if matches!(key.code, KeyCode::Char('t' | 'T')) {
                self.start_outline();
                return Ok(Action::None);
            }
            if matches!(key.code, KeyCode::Char('e' | 'E')) {
                self.begin_edit()?;
                return Ok(Action::None);
            }
            if key.code == KeyCode::Char('/') {
                self.start_query(false);
                return Ok(Action::None);
            }
            if matches!(key.code, KeyCode::Char('o' | 'O')) {
                if let Some(d) = &self.document {
                    let path = d.path.clone();
                    self.open_external(path)?;
                }
                return Ok(Action::None);
            }
            let Some(d) = &mut self.document else {
                return Ok(Action::None);
            };
            match key.code {
                KeyCode::Esc => {
                    if d.selection.take().is_none() {
                        if !d.search.is_empty() {
                            d.search.clear();
                            d.matches.clear();
                        } else {
                            self.close();
                        }
                    }
                }
                KeyCode::Char('m' | 'M') => {
                    if d.loaded.as_ref().is_some_and(|l| l.markdown()) {
                        d.reader.seek = Some(d.reading_byte());
                        d.markdown = !d.markdown;
                        d.selection = None;
                        d.render_pending = false;
                        d.render_id = 0;
                    }
                }
                KeyCode::Char('n') => d.jump_match(false),
                KeyCode::Char('N') => d.jump_match(true),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let text = d
                        .selected_text()
                        .context("Select text, or use F2 → Copy content")?;
                    files::copy(text)?;
                    self.say("Copied selection", true);
                }
                KeyCode::Up => d.offset_mut().0 = d.offset().0.saturating_sub(1),
                KeyCode::Down => d.offset_mut().0 = d.offset().0.saturating_add(1),
                KeyCode::PageUp => d.offset_mut().0 = d.offset().0.saturating_sub(page),
                KeyCode::PageDown => d.offset_mut().0 = d.offset().0.saturating_add(page),
                KeyCode::Left => d.offset_mut().1 = d.offset().1.saturating_sub(4),
                KeyCode::Right => d.offset_mut().1 += 4,
                KeyCode::Home => *d.offset_mut() = (0, 0),
                KeyCode::End => {
                    if d.markdown && d.rendered.as_ref().is_some_and(|r| !r.complete) {
                        d.markdown_end = true;
                    }
                    d.offset_mut().0 = d
                        .rendered
                        .as_ref()
                        .map_or(0, |r| r.plain.len().saturating_sub(1))
                }
                _ => {}
            }
        }
        Ok(Action::None)
    }
    pub fn cancel_drag(&mut self, feedback: bool) {
        if !matches!(self.gesture, Gesture::Idle) {
            if self.drag_restore {
                self.preview_visible = true;
            }
            self.gesture = Gesture::Idle;
            self.drag_restore = false;
            if feedback {
                self.say("Cancelled; no path sent", true);
            }
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) -> Result<(bool, Action)> {
        if self.modal() {
            return Ok((true, Action::None));
        }
        if self.scrollbar_mouse(event) {
            return Ok((true, Action::None));
        }
        let point = (event.column, event.row);
        if matches!(self.gesture, Gesture::Idle)
            && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
        {
            if self
                .geometry
                .toggle
                .is_some_and(|r| r.contains(point.into()))
            {
                self.toggle();
                return Ok((true, Action::None));
            }
            if let Some(rect) = self.geometry.preview
                && event.row == rect.y
                && let Some(d) = &self.document
            {
                let buttons = header_buttons(
                    rect,
                    d.editing,
                    d.loaded.as_ref().is_some_and(|l| l.markdown()),
                );
                if let Some((_, _, command)) = buttons
                    .into_iter()
                    .find(|(r, _, _)| r.contains(point.into()))
                {
                    let path = d.path.clone();
                    self.focus = if d.editing {
                        Focus::Editor
                    } else {
                        Focus::Preview
                    };
                    return Ok((true, self.command(command, path)?));
                }
            }
        }
        match &self.gesture {
            Gesture::Pressed { path, origin } => {
                let path = path.clone();
                let origin = *origin;
                match event.kind {
                    MouseEventKind::Drag(MouseButton::Left)
                        if point.0.abs_diff(origin.0) + point.1.abs_diff(origin.1) >= 2 =>
                    {
                        self.gesture = Gesture::Dragging { path };
                        self.drag_restore = self.geometry.overlay && self.preview_visible;
                        if self.drag_restore {
                            self.preview_visible = false;
                        }
                        self.say("Drag to Agent content; release inserts a path", false);
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        self.gesture = Gesture::Idle;
                        let hit = self
                            .files
                            .state
                            .rendered_at(point.into())
                            .and_then(|id| id.last());
                        if hit == Some(&Key::Entry(path.clone())) {
                            self.open(path, false)?;
                        }
                    }
                    MouseEventKind::Down(_) => self.cancel_drag(true),
                    _ => {}
                }
                return Ok((true, Action::None));
            }
            Gesture::Dragging { path } => {
                let path = path.clone();
                let valid = self
                    .geometry
                    .agent
                    .is_some_and(|r| crate::ui::content(r).contains(point.into()));
                if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                    if valid {
                        self.gesture = Gesture::Idle;
                        self.drag_restore = false;
                        self.focus = Focus::Terminal;
                        return Ok((true, Action::Insert(path)));
                    }
                    self.cancel_drag(true);
                } else if matches!(event.kind, MouseEventKind::Down(_)) {
                    self.cancel_drag(true);
                } else {
                    self.say(
                        if valid {
                            "Release to insert path"
                        } else {
                            "Cannot insert here"
                        },
                        false,
                    );
                }
                return Ok((true, Action::None));
            }
            Gesture::Idle => {}
        }
        if let Some(rect) = self.geometry.tree
            && rect.contains(point.into())
        {
            if let Some(view) = &mut self.tree_view {
                view.viewport = None;
            }
            self.focus = Focus::Tree;
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if event.row == rect.y {
                        self.menu();
                    } else if let Some(id) = self
                        .files
                        .state
                        .rendered_at(point.into())
                        .map(<[Key]>::to_vec)
                    {
                        self.files.state.select(id);
                        match self.files.selected() {
                            Some(Key::Entry(path))
                                if self.files.nodes.get(&path).is_some_and(|n| !n.directory) =>
                            {
                                self.gesture = Gesture::Pressed {
                                    path,
                                    origin: point,
                                };
                            }
                            _ => self.activate_tree(false)?,
                        }
                    }
                }
                MouseEventKind::ScrollUp => {
                    self.files.state.scroll_up(3);
                }
                MouseEventKind::ScrollDown => {
                    self.files.state.scroll_down(3);
                }
                _ => {}
            }
            return Ok((true, Action::None));
        }
        if let Some(rect) = self.geometry.preview
            && rect.contains(point.into())
        {
            if self.document.as_ref().is_some_and(|d| d.editing) {
                self.focus = Focus::Editor;
                if event.row == rect.y
                    && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                {
                    self.menu();
                } else if let Some(e) = self.document.as_mut().and_then(|d| d.editor.as_mut()) {
                    e.mouse(event, rect);
                }
                return Ok((true, Action::None));
            }
            self.focus = Focus::Preview;
            let inner = crate::ui::content(rect);
            if event.row == rect.y && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            {
                self.menu();
                return Ok((true, Action::None));
            }
            if let Some(d) = &mut self.document {
                match event.kind {
                    MouseEventKind::ScrollUp => d.offset_mut().0 = d.offset().0.saturating_sub(3),
                    MouseEventKind::ScrollDown => d.offset_mut().0 = d.offset().0.saturating_add(3),
                    MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::Drag(MouseButton::Left)
                    | MouseEventKind::Up(MouseButton::Left)
                        if inner.contains(point.into()) =>
                    {
                        let (top, left) = d.offset();
                        let gutter = if d.markdown { 0 } else { 6 };
                        let row = d
                            .screen_rows(inner.height as usize)
                            .get(usize::from(event.row - inner.y))
                            .copied()
                            .unwrap_or(top);
                        let screen_col =
                            usize::from(event.column - inner.x).saturating_sub(gutter) + left;
                        let col = d
                            .rendered
                            .as_ref()
                            .and_then(|r| r.plain.get(row))
                            .map_or(0, |line| char_at_cell(line, screen_col));
                        let p = (row, col);
                        if matches!(event.kind, MouseEventKind::Down(_)) {
                            d.reader.pressed = Some((p, point));
                            d.selection = Some((p, p));
                        } else if let Some(s) = &mut d.selection {
                            s.1 = p;
                        }
                        if matches!(event.kind, MouseEventKind::Drag(_))
                            && d.reader.pressed != Some((p, point))
                        {
                            d.reader.pressed = None;
                        }
                        if matches!(event.kind, MouseEventKind::Up(_)) {
                            let clicked = d.reader.pressed.take() == Some((p, point))
                                && d.selection.is_some_and(|(a, b)| a == b);
                            if clicked && self.reading_click(row, col, point, inner)? {
                                return Ok((true, Action::None));
                            }
                        }
                    }
                    _ => {}
                }
            }
            return Ok((true, Action::None));
        }
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.focus = Focus::Terminal;
        }
        Ok((false, Action::None))
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let _perf = crate::perf::span(crate::perf::Metric::SidebarDraw);
        if let Some(d) = &mut self.document
            && !d.editing
            && let Some(r) = &d.rendered
            && r.complete
        {
            let max = r.plain.len().saturating_sub(1);
            d.offset_mut().0 = d.offset().0.min(max);
        }
        if let Some(rect) = self.geometry.tree {
            let preview = self.document.as_ref().map(|d| d.path.as_path());
            let inner = crate::ui::content(rect);
            let valid = self.tree_view.as_ref().is_some_and(|view| {
                view.revision == self.files.revision()
                    && view.width == inner.width
                    && view.icons == self.files.icons
                    && view.light == self.files.light
                    && view.preview.as_deref() == preview
            });
            if !valid {
                // Drop the old projection before allocating its replacement.
                self.tree_view = None;
                self.tree_view = Some(TreeView {
                    revision: self.files.revision(),
                    width: inner.width,
                    icons: self.files.icons,
                    light: self.files.light,
                    preview: preview.map(PathBuf::from),
                    items: self.files.items(preview, inner.width as usize),
                    viewport: None,
                });
            }
            let view = self.tree_view.as_mut().expect("tree projection");
            let title = format!(
                " Files [F2] {}{} ",
                if self.files.hidden { "H" } else { "" },
                if self.files.ignored { "I" } else { "" }
            );
            let focused = self.focus == Focus::Tree;
            let valid = view.viewport.as_ref().is_some_and(|cached| {
                cached.rect == rect
                    && cached.focused == focused
                    && cached.offset == self.files.state.get_offset()
                    && cached.selected == self.files.state.selected()
                    && cached.opened == *self.files.state.opened()
            });
            if !valid {
                let _perf = crate::perf::span(crate::perf::Metric::TreeRender);
                let mut buffer = ratatui::buffer::Buffer::empty(rect);
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        buffer[(x, y)] = frame.buffer_mut()[(x, y)].clone();
                    }
                }
                if let Ok(tree) = Tree::new(&view.items) {
                    tree.block(Block::bordered().title(title).border_style(border(focused)))
                        .highlight_style(Style::new().bg(Color::DarkGray))
                        .highlight_symbol("> ")
                        .render(rect, &mut buffer, &mut self.files.state);
                }
                view.viewport = Some(TreeViewport {
                    row_count: scroll::tree_rows(
                        &view.items,
                        self.files.state.opened(),
                        &mut Vec::new(),
                    ),
                    rect,
                    focused,
                    offset: self.files.state.get_offset(),
                    selected: self.files.state.selected().to_vec(),
                    opened: self.files.state.opened().clone(),
                    buffer,
                });
            }
            // The widget's own last-render hit map stays valid until the same
            // inputs change. Agent output alone only copies the visible cells.
            let _perf = crate::perf::span(crate::perf::Metric::TreeBlit);
            let buffer = &view.viewport.as_ref().expect("tree viewport").buffer;
            for y in rect.y..rect.bottom() {
                let start = (y - frame.area().y) as usize * frame.area().width as usize
                    + (rect.x - frame.area().x) as usize;
                let source = (y - rect.y) as usize * rect.width as usize;
                frame.buffer_mut().content[start..start + rect.width as usize]
                    .clone_from_slice(&buffer.content[source..source + rect.width as usize]);
            }
        }
        if let Some(rect) = self.geometry.preview {
            let _perf = crate::perf::span(crate::perf::Metric::PreviewDraw);
            frame.render_widget(Clear, rect);
            let d = self.document.as_ref().expect("preview document");
            if d.editing
                && let Some(e) = &d.editor
            {
                e.draw(frame, rect, self.focus == Focus::Editor);
                draw_header_buttons(
                    frame,
                    rect,
                    true,
                    d.loaded.as_ref().is_some_and(|l| l.markdown()),
                );
                self.draw_scrollbars(frame);
                self.draw_overlay(frame);
                return;
            }
            let note = d.loaded.as_ref().map_or("", |l| l.note.as_str());
            let title = format!(
                " {} · read-only{}{} [F2] ",
                d.path.file_name().unwrap_or_default().to_string_lossy(),
                if d.markdown {
                    " · Markdown"
                } else {
                    " · source"
                },
                if d.editor.as_ref().is_some_and(|e| e.dirty()) {
                    " · UNSAVED *"
                } else {
                    ""
                }
            );
            frame.render_widget(
                Block::bordered()
                    .title(title)
                    .border_style(border(self.focus == Focus::Preview)),
                rect,
            );
            draw_header_buttons(
                frame,
                rect,
                false,
                d.loaded.as_ref().is_some_and(|l| l.markdown()),
            );
            let inner = crate::ui::content(rect);
            let (_, left) = d.offset();
            if let Some(r) = &d.rendered {
                for (y, row) in d.screen_rows(inner.height as usize).into_iter().enumerate() {
                    let y = y as u16;
                    let Some(raw_line) = r.text.lines.get(row) else {
                        if r.markdown && !r.complete {
                            frame.render_widget(
                                Paragraph::new("Loading preview…")
                                    .style(Style::new().fg(Color::DarkGray)),
                                Rect::new(inner.x, inner.y + y, inner.width, 1),
                            );
                        }
                        break;
                    };
                    let line = r
                        .window
                        .as_ref()
                        .filter(|w| w.light == self.files.light)
                        .and_then(|w| w.line(row))
                        .unwrap_or(raw_line);
                    let gutter = if d.markdown { 0 } else { 6.min(inner.width) };
                    if gutter > 0 {
                        frame.render_widget(
                            Paragraph::new(format!("{:>4} │", row + 1))
                                .style(Style::new().fg(Color::DarkGray)),
                            Rect::new(inner.x, inner.y + y, gutter, 1),
                        );
                    }
                    let content = Rect::new(inner.x + gutter, inner.y + y, inner.width - gutter, 1);
                    frame.render_widget(
                        Paragraph::new(line.clone())
                            .style(line.style)
                            .scroll((0, left.min(u16::MAX as usize) as u16)),
                        content,
                    );
                    if r.rows.get(row).is_some_and(|r| r.code_language.is_some()) {
                        self.draw_reading_header(frame, row, content);
                        continue;
                    }
                    let plain = &r.plain[row];
                    // Search results are sorted by (row, character). Locate this
                    // row once, then advance through overlapping matches with
                    // the visible graphemes. Never scan all results per cell.
                    let start = d.matches.partition_point(|m| m.0 < row);
                    let end = d.matches.partition_point(|m| m.0 <= row);
                    let row_matches = &d.matches[start..end];
                    if d.selection.is_none() && row_matches.is_empty() {
                        continue;
                    }
                    let mut match_index = 0;
                    let mut col = 0;
                    let mut chars = 0;
                    for g in plain.graphemes(true) {
                        if col >= left.saturating_add(content.width as usize) {
                            break;
                        }
                        let w = g.width();
                        let end_chars = chars + g.chars().count();
                        let selected = d.selection.is_some_and(|(mut a, mut b)| {
                            if a > b {
                                std::mem::swap(&mut a, &mut b);
                            }
                            (row, end_chars) > a && (row, chars) < b
                        });
                        while match_index < row_matches.len() && row_matches[match_index].2 <= chars
                        {
                            match_index += 1;
                        }
                        let found = row_matches
                            .get(match_index)
                            .is_some_and(|&(_, a, b)| chars < b && end_chars > a);
                        if selected || found {
                            for x in col..col + w {
                                if x >= left && x - left < content.width as usize {
                                    frame.buffer_mut()[(content.x + (x - left) as u16, content.y)]
                                        .set_style(
                                            Style::new()
                                                .bg(if selected {
                                                    Color::Cyan
                                                } else {
                                                    Color::Yellow
                                                })
                                                .fg(Color::Black),
                                        );
                                }
                            }
                        }
                        col += w;
                        chars = end_chars;
                    }
                }
            } else {
                frame.render_widget(
                    Paragraph::new(if d.loading { "Loading…" } else { &d.stale }),
                    inner,
                );
            }
            if !d.stale.is_empty() || !note.is_empty() {
                frame.render_widget(
                    Paragraph::new(format!("{} {note}", d.stale))
                        .style(Style::new().fg(Color::Yellow)),
                    Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                );
            }
        }
        self.draw_scrollbars(frame);
        self.draw_overlay(frame);
    }
    pub fn draw_overlay(&self, frame: &mut Frame) {
        let area = frame.area();
        if let Some(r) = self.geometry.toggle {
            frame.render_widget(
                Paragraph::new("[Files]").style(Style::new().fg(Color::Black).bg(Color::Cyan)),
                r,
            );
        }
        if let Some(prompt) = &self.prompt {
            let text = match prompt {
                Prompt::Unsaved(_) => {
                    "Unsaved changes\nS Save and continue · D Discard · Esc Cancel\nSave failures keep this document open."
                }
                Prompt::External(_) => {
                    "External applications read the disk version.\nS Save then open · O Open disk version · Esc Cancel"
                }
            };
            let r = Rect::new(
                area.x + area.width.saturating_sub(74) / 2,
                area.y + area.height.saturating_sub(6) / 2,
                area.width.min(74),
                area.height.min(6),
            );
            frame.render_widget(Clear, r);
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::bordered()
                        .title(" Document ")
                        .border_style(Style::new().fg(Color::Yellow)),
                ),
                r,
            );
            return;
        }
        if let Some(menu) = &self.menu {
            let popup = Rect::new(
                area.x + area.width.saturating_sub(64) / 2,
                area.y + area.height.saturating_sub(menu.items.len() as u16 + 3) / 2,
                area.width.min(64),
                area.height.min(menu.items.len() as u16 + 3),
            );
            frame.render_widget(Clear, popup);
            let lines = menu
                .items
                .iter()
                .enumerate()
                .map(|(i, (label, _))| {
                    Line::styled(
                        format!("{} {label}", if i == menu.selected { ">" } else { " " }),
                        if i == menu.selected {
                            Style::new().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::new()
                        },
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((
                        menu.selected
                            .saturating_sub(popup.height.saturating_sub(3) as usize)
                            .min(u16::MAX as usize) as u16,
                        0,
                    ))
                    .block(Block::bordered().title(format!(" {} ", menu.target.display()))),
                popup,
            );
        }
        if let Some(q) = &self.query {
            let text = format!(
                "{}: {}  {}",
                if q.tree {
                    "Find loaded nodes"
                } else {
                    "Find preview text"
                },
                q.value,
                if q.tree {
                    format!("{} matches", q.matches.len())
                } else {
                    self.document.as_ref().map_or(String::new(), |d| {
                        format!(
                            "{} matches{}",
                            d.matches.len(),
                            if d.loaded.as_ref().is_some_and(|l| l.truncated) {
                                " (loaded text only)"
                            } else {
                                ""
                            }
                        )
                    })
                }
            );
            frame.render_widget(
                Paragraph::new(text).style(Style::new().fg(Color::Black).bg(Color::Cyan)),
                Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
            );
        }
        self.draw_outline(frame);
    }
    pub fn status(&self) -> String {
        if !self.notice.is_empty() {
            return self.notice.clone();
        }
        if !self.files.watch_error.is_empty() {
            return self.files.watch_error.clone();
        }
        match self.focus {
            Focus::Tree => format!(
                "{} · / Find loaded · Enter Preview · F2 Actions · Esc Agent",
                self.files
                    .selected_path()
                    .unwrap_or_else(|| self.files.root.clone())
                    .display()
            ),
            Focus::Preview => self.document.as_ref().map_or(String::new(), |d| {
                format!(
                    "{} · / Find · M Mode · T Outline · Alt+← Back · F2 Actions · {}/{} matches",
                    d.path.display(),
                    if d.matches.is_empty() {
                        0
                    } else {
                        d.match_index + 1
                    },
                    d.matches.len()
                )
            }),
            Focus::Editor => self
                .document
                .as_ref()
                .and_then(|d| d.editor.as_ref())
                .map_or(String::new(), |e| {
                    format!(
                        "{} · Ctrl+S Save · Ctrl+F Find · F2 Actions · {}",
                        e.loaded.path.display(),
                        e.status
                    )
                }),
            _ => String::new(),
        }
    }
}
fn header_buttons(rect: Rect, editing: bool, markdown: bool) -> Vec<(Rect, &'static str, Command)> {
    let mut buttons = vec![
        ("[X]", Command::Close),
        (
            if editing { "[S]" } else { "[E]" },
            if editing {
                Command::Save
            } else {
                Command::Edit
            },
        ),
    ];
    if editing {
        buttons.push(("[P]", Command::Project));
    } else if markdown {
        buttons.push(("[M]", Command::Mode));
    }
    buttons
        .into_iter()
        .enumerate()
        .map(|(i, (label, command))| {
            (
                Rect::new(rect.right() - 4 * (i as u16 + 1), rect.y, 3, 1),
                label,
                command,
            )
        })
        .collect()
}
fn draw_header_buttons(frame: &mut Frame, rect: Rect, editing: bool, markdown: bool) {
    for (r, label, _) in header_buttons(rect, editing, markdown) {
        frame.render_widget(Paragraph::new(label).style(Style::new().fg(Color::Cyan)), r);
    }
}
fn border(focused: bool) -> Style {
    Style::new().fg(if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    })
}
pub fn char_at_cell(line: &str, cell: usize) -> usize {
    let mut col = 0;
    let mut chars = 0;
    for g in line.graphemes(true) {
        if col + g.width() > cell {
            break;
        }
        col += g.width();
        chars += g.chars().count();
    }
    chars
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
    fn draw(s: &mut Sidebar, width: u16) -> Buffer {
        s.update_layout(Rect::new(0, 0, width, 30), false);
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| s.draw(f)).unwrap();
        terminal.backend().buffer().clone()
    }
    fn compare(s: &mut Sidebar, width: u16) {
        let first = draw(s, width);
        let cached = draw(s, width);
        assert_eq!(first, cached);
        s.tree_view = None;
        assert_eq!(
            cached,
            draw(s, width),
            "cached tree must match a full component render"
        );
    }
    #[test]
    fn tree_view_cache_keeps_selection_scroll_focus_badges_and_hit_map() {
        let root = std::env::temp_dir().join("DevTabs-tree-projection-fixture");
        let mut s = Sidebar::new(root.clone());
        let nodes = (0..100)
            .map(|i| {
                let name = format!("item{i:03}.rs");
                let path = root.join(&name);
                s.files.nodes.insert(
                    path.clone(),
                    files::Node {
                        path: path.clone(),
                        name,
                        directory: false,
                        link: false,
                        icon: 'R',
                        color: Color::Red,
                    },
                );
                path
            })
            .collect();
        let mut directory = files::Directory::default();
        directory.children = nodes;
        directory.complete = true;
        s.files.dirs.insert(root.clone(), directory);
        compare(&mut s, 160);
        s.focus = Focus::Tree;
        s.files
            .state
            .select(s.files.identifier(&root.join("item099.rs")));
        compare(&mut s, 160);
        assert!(s.files.state.get_offset() > 0);
        s.files.state.scroll_up(10);
        compare(&mut s, 160);
        s.files.state.close_all();
        compare(&mut s, 160);
        s.files.state.open(vec![Key::Entry(root.clone())]);
        compare(&mut s, 160);
        s.files.icons = false;
        s.files.light = true;
        compare(&mut s, 160);
        s.document = Some(Document::new(root.join("item005.rs"), 1));
        compare(&mut s, 160);
        compare(&mut s, 84); // new tree width
        let rect = s.geometry.tree.unwrap();
        let hit = s
            .files
            .state
            .rendered_at((rect.x + 5, rect.y + 2).into())
            .unwrap()
            .to_vec();
        let path = hit.last().cloned().unwrap();
        s.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 5,
            row: rect.y + 2,
            modifiers: KeyModifiers::NONE,
        })
        .unwrap();
        assert_eq!(s.files.selected(), Some(path));
        assert!(s.tree_view.as_ref().unwrap().viewport.is_none());
        compare(&mut s, 84);
    }
    #[test]
    fn visible_search_colors_match_overlaps_combining_and_wide_cells() {
        let mut s = Sidebar::new(std::env::temp_dir());
        s.visible = false;
        let mut d = Document::new(PathBuf::from("text.txt"), 1);
        d.rendered = Some(preview::render(
            "aaa e\u{301} 中文 😀",
            &d.path,
            false,
            100,
            false,
        ));
        d.search = "aa".into();
        d.search();
        d.selection = Some(((0, 5), (0, 6)));
        s.document = Some(d);
        s.preview_visible = true;
        let buffer = draw(&mut s, 100);
        let rect = s.geometry.preview.unwrap();
        let x = rect.x + 1 + 6;
        let y = rect.y + 1;
        for col in 0..3 {
            assert_eq!(buffer[(x + col, y)].bg, Color::Yellow);
        }
        assert_eq!(
            buffer[(x + 4, y)].bg,
            Color::Cyan,
            "selection intersects the complete combining grapheme"
        );
        // A far-away match must not color unrelated rows or cells.
        s.document.as_mut().unwrap().matches.push((999, 0, 1));
        assert_eq!(draw(&mut s, 100), buffer);
    }
}
