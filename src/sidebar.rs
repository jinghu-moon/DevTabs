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
mod controls;
mod explorer;
mod features;
mod image;
mod images;
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
    panels_with_width(area, split, sidebar, preview, 28)
}
pub fn panels_with_width(
    area: Rect,
    split: bool,
    sidebar: bool,
    preview: bool,
    width: u16,
) -> Panels {
    panels_with_ratio(area, split, sidebar, preview, width, 5000)
}
fn panels_with_ratio(
    area: Rect,
    split: bool,
    sidebar: bool,
    preview: bool,
    width: u16,
    agent_ratio: u16,
) -> Panels {
    let base = if split { 94 } else { 62 };
    if area.width < base || area.height < 12 {
        return Panels::default();
    }
    let s = if sidebar && area.width >= base + 20 {
        width.clamp(20, 80).min(area.width - base)
    } else {
        0
    };
    let body = Rect::new(area.x, area.y, area.width, area.height - 1);
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
        ((u32::from(l) * u32::from(agent_ratio.clamp(1, 9999))).div_ceil(10000) as u16)
            .clamp(62, l - 42)
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
    pub items: Vec<(std::borrow::Cow<'static, str>, Command)>,
    pub selected: usize,
    pub target: PathBuf,
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
    QuickOpen,
    Follow,
    Compact,
    Modified,
    NewFile,
    NewDirectory,
    Rename,
    Trash,
    PreviewSelected,
    ImageFit,
    ImageActual,
    ImageZoomIn,
    ImageZoomOut,
    ImageProfile,
    ImageProtocol,
    ImageRefresh,
    Maximize,
    Wrap,
    Compare,
    Diff,
    Goto,
    Search,
    NextTab,
    PreviousTab,
    Insert,
    CopyAbsolute,
    CopyRelative,
    Reveal,
    External,
    Collapse,
    Release,
    Icons,
    Theme,
    Language,
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
    Forward,
    More,
    CloseOthers,
    CloseRight,
    Bookmark,
    Bookmarks,
    Info,
}
impl Command {
    pub fn id(self) -> &'static str {
        match self {
            Self::QuickOpen => "panel.quick_open",
            Self::Follow => "panel.follow",
            Self::Compact => "panel.compact",
            Self::Modified => "panel.modified",
            Self::NewFile => "panel.new_file",
            Self::NewDirectory => "panel.new_directory",
            Self::Rename => "panel.rename",
            Self::Trash => "panel.trash",
            Self::PreviewSelected => "panel.preview_selected",
            Self::ImageFit => "panel.image_fit",
            Self::ImageActual => "panel.image_actual",
            Self::ImageZoomIn => "panel.image_zoom_in",
            Self::ImageZoomOut => "panel.image_zoom_out",
            Self::ImageProfile => "panel.image_profile",
            Self::ImageProtocol => "panel.image_protocol",
            Self::ImageRefresh => "panel.image_refresh",
            Self::Maximize => "panel.maximize",
            Self::Wrap => "panel.wrap",
            Self::Compare => "panel.compare",
            Self::Diff => "panel.diff",
            Self::Goto => "panel.goto",
            Self::Search => "panel.search",
            Self::NextTab => "panel.next_tab",
            Self::PreviousTab => "panel.previous_tab",
            Self::Insert => "panel.insert",
            Self::CopyAbsolute => "panel.copy_absolute",
            Self::CopyRelative => "panel.copy_relative",
            Self::Reveal => "panel.reveal",
            Self::External => "panel.external",
            Self::Collapse => "panel.collapse",
            Self::Release => "panel.release",
            Self::Icons => "panel.icons",
            Self::Theme => "panel.theme",
            Self::Language => "panel.language",
            Self::Locate => "panel.locate",
            Self::CopyContent => "panel.copy_content",
            Self::CopySource => "panel.copy_source",
            Self::Close => "panel.close",
            Self::Edit => "panel.edit",
            Self::Save => "panel.save",
            Self::Undo => "panel.undo",
            Self::Redo => "panel.redo",
            Self::Cut => "panel.cut",
            Self::Paste => "panel.paste",
            Self::Find => "panel.find",
            Self::Project => "panel.project",
            Self::EndEdit => "panel.end_edit",
            Self::Mode => "panel.mode",
            Self::Outline => "panel.outline",
            Self::Back => "panel.back",
            Self::Forward => "panel.forward",
            Self::More => "panel.more",
            Self::CloseOthers => "panel.close_others",
            Self::CloseRight => "panel.close_right",
            Self::Bookmark => "panel.bookmark",
            Self::Bookmarks => "panel.bookmarks",
            Self::Info => "panel.info",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "panel.quick_open" => Self::QuickOpen,
            "panel.follow" => Self::Follow,
            "panel.compact" => Self::Compact,
            "panel.modified" => Self::Modified,
            "panel.new_file" => Self::NewFile,
            "panel.new_directory" => Self::NewDirectory,
            "panel.rename" => Self::Rename,
            "panel.trash" => Self::Trash,
            "panel.preview_selected" => Self::PreviewSelected,
            "panel.image_fit" => Self::ImageFit,
            "panel.image_actual" => Self::ImageActual,
            "panel.image_zoom_in" => Self::ImageZoomIn,
            "panel.image_zoom_out" => Self::ImageZoomOut,
            "panel.image_profile" => Self::ImageProfile,
            "panel.image_protocol" => Self::ImageProtocol,
            "panel.image_refresh" => Self::ImageRefresh,
            "panel.maximize" => Self::Maximize,
            "panel.wrap" => Self::Wrap,
            "panel.compare" => Self::Compare,
            "panel.diff" => Self::Diff,
            "panel.goto" => Self::Goto,
            "panel.search" => Self::Search,
            "panel.next_tab" => Self::NextTab,
            "panel.previous_tab" => Self::PreviousTab,
            "panel.insert" => Self::Insert,
            "panel.copy_absolute" => Self::CopyAbsolute,
            "panel.copy_relative" => Self::CopyRelative,
            "panel.reveal" => Self::Reveal,
            "panel.external" => Self::External,
            "panel.collapse" => Self::Collapse,
            "panel.release" => Self::Release,
            "panel.icons" => Self::Icons,
            "panel.theme" => Self::Theme,
            "panel.language" => Self::Language,
            "panel.locate" => Self::Locate,
            "panel.copy_content" => Self::CopyContent,
            "panel.copy_source" => Self::CopySource,
            "panel.close" => Self::Close,
            "panel.edit" => Self::Edit,
            "panel.save" => Self::Save,
            "panel.undo" => Self::Undo,
            "panel.redo" => Self::Redo,
            "panel.cut" => Self::Cut,
            "panel.paste" => Self::Paste,
            "panel.find" => Self::Find,
            "panel.project" => Self::Project,
            "panel.end_edit" => Self::EndEdit,
            "panel.mode" => Self::Mode,
            "panel.outline" => Self::Outline,
            "panel.back" => Self::Back,
            "panel.forward" => Self::Forward,
            "panel.more" => Self::More,
            "panel.close_others" => Self::CloseOthers,
            "panel.close_right" => Self::CloseRight,
            "panel.bookmark" => Self::Bookmark,
            "panel.bookmarks" => Self::Bookmarks,
            "panel.info" => Self::Info,
            _ => return None,
        })
    }
}
#[derive(Debug)]
pub enum Action {
    None,
    Agent,
    Insert(Vec<PathBuf>),
    ValidatedInput(Vec<PathBuf>),
    Manage(char),
}
#[derive(Clone)]
enum Pending {
    FileOperation(crate::explorer::Operation),
    OpenPaths(Vec<PathBuf>),
    ExternalPaths(Vec<PathBuf>),
    Reading(preview::ReadingAction),
    Bookmark(crate::bookmarks::Entry),
    CloseGroup(PathBuf, bool),
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

enum Content {
    Text(Box<TextDocument>),
    Image(Box<image::ImageDocument>),
}
#[derive(Default)]
struct TextDocument {
    loaded: Option<std::sync::Arc<Loaded>>,
    rendered: Option<Rendered>,
    editor: Option<crate::editor::Editor>,
}
pub struct Document {
    content: Content,
    view: features::DocumentView,
    reader: reading::DocumentReading,
    pub editing: bool,
    edit_requested: Option<u64>,
    render_revision: u64,
    pub path: PathBuf,
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
    defaults_pending: bool,
}
impl Document {
    fn new(path: PathBuf, id: u64) -> Self {
        Self {
            content: if preview::image::candidate(&path) {
                Content::Image(Default::default())
            } else {
                Content::Text(Default::default())
            },
            view: Default::default(),
            reader: Default::default(),
            editing: false,
            edit_requested: None,
            render_revision: 0,
            path,
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
            defaults_pending: false,
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
        self.view.search_targets.clear();
        self.match_index = 0;
        self.view.search_error.clear();
        if self.search.is_empty() {
            return;
        }
        if let Some(r) = self.rendered() {
            match r.find(&self.search, self.view.search_options) {
                Ok((matches, targets)) => {
                    self.matches = matches;
                    self.view.search_targets = targets;
                }
                Err(e) => self.view.search_error = format!("Invalid pattern: {e}"),
            }
        }
    }
    fn jump_match(&mut self, back: bool) {
        if self.view.search_targets.is_empty() {
            return;
        }
        let n = self.view.search_targets.len();
        self.match_index = if back {
            (self.match_index + n - 1) % n
        } else {
            (self.match_index + 1) % n
        };
        let (row, col) = self.view.search_targets[self.match_index];
        *self.offset_mut() = (row, if self.view.wrap { 0 } else { col });
    }
    pub fn selected_text(&self) -> Option<String> {
        let (mut start, mut end) = self.selection?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let r = self.rendered()?;
        let mut out = String::new();
        for row in start.0..=end.0.min(r.plain.len().saturating_sub(1)) {
            if row > start.0 {
                let continuation = r.projection.as_ref().is_some_and(|p| {
                    p.get(row)
                        .zip(p.get(row - 1))
                        .is_some_and(|(a, b)| a.original == b.original)
                });
                if !continuation {
                    out.push('\n');
                }
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
    pub shortcuts: crate::shortcuts::Index,
    /// Shared preview can be embedded in a context that must never edit files.
    pub read_only: bool,
    explorer: explorer::Explorer,
    preview_defaults: crate::settings::model::Preview,
    image_worker: preview::image::Worker,
    image_display: preview::image::Display,
    pub images_occluded: bool,
    pub image_occlusion: Option<Rect>,
    image_composition: crate::image_output::Composition,
    image_suspended: bool,
    tools: features::Workspace,
    controls: controls::Controls,
    reader: reading::ReadingUi,
    tree_view: Option<TreeView>,
    prompt: Option<Prompt>,
    after_save: Option<Pending>,
    pub ready_manage: Option<char>,
    pub files: Files,
    pub io: Workers,
    pub focus: Focus,
    pub(crate) hover: Option<(u16, u16)>,
    pub(crate) enlarged: Option<Focus>,
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
    pub ready_input: Option<Vec<PathBuf>>,
    pub checking_input: bool,
    locate_pending: Option<PathBuf>,
    locate_explicit: bool,
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
    // File labels are one line. Keep the exact painted rows separately from
    // the component's nearest-item lookup, including while cache is invalid.
    hit_area: Rect,
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
    pub(crate) fn tree_hit(&self, point: (u16, u16)) -> Option<&[Key]> {
        self.tree_view
            .as_ref()
            .filter(|view| view.hit_area.contains(point.into()))?;
        self.files.state.rendered_at(point.into())
    }
    pub fn reveal_line(&mut self, line: usize) -> Result<()> {
        self.navigate_input(&line.to_string())
    }
    pub fn dirty(&self) -> bool {
        self.document
            .as_ref()
            .and_then(|d| d.editor())
            .is_some_and(|e| e.dirty() || e.saving)
    }
    pub fn modal(&self) -> bool {
        self.document
            .as_ref()
            .and_then(|d| d.editor())
            .is_some_and(|e| e.search_input.is_some())
            || self.controls.panel.is_some()
            || self.tools.navigation.is_some()
            || self.tools.diff.is_some()
            || self.tools.diff_pending.is_some()
            || self.prompt.is_some()
            || self.menu.is_some()
            || self.query.is_some()
            || self.reader.outline.is_some()
            || self.explorer.modal()
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
            d.set_editor(None);
            d.editing = false;
            d.set_rendered(None);
            d.render_pending = false;
            d.render_id = self.serial;
            d.edit_requested = None;
        }
    }
    fn perform(&mut self, pending: Pending) -> Result<()> {
        match pending {
            Pending::FileOperation(op) => self.request_operation(op)?,
            Pending::OpenPaths(paths) => self.open_paths(paths)?,
            Pending::ExternalPaths(paths) => self.external_paths(paths)?,
            Pending::Reading(action) => self.activate_reading(action)?,
            Pending::Bookmark(entry) => self.jump_bookmark(entry)?,
            Pending::CloseGroup(target, right) => self.close_group(&target, right),
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
        anyhow::ensure!(!self.read_only, "{}", crate::i18n::t("overview.read_only"));
        let d = self
            .document
            .as_mut()
            .context(crate::i18n::t("ui.no_document"))?;
        if d.editor().is_some() {
            d.editing = true;
            self.focus = Focus::Editor;
            return Ok(());
        }
        ensure!(
            d.edit_requested.is_none(),
            crate::i18n::t("ui.editor_is_loading")
        );
        self.serial += 1;
        let id = self.serial;
        self.io.send(Job::Edit {
            loaded: d
                .loaded()
                .cloned()
                .context(crate::i18n::t("ui.wait_for_the_file_to_load"))?,
            id,
        })?;
        d.edit_requested = Some(id);
        Ok(())
    }
    pub fn save(&mut self) -> Result<()> {
        anyhow::ensure!(!self.read_only, "{}", crate::i18n::t("overview.read_only"));
        let d = self
            .document
            .as_mut()
            .context(crate::i18n::t("ui.no_document"))?;
        let e = d
            .editor_mut()
            .context(crate::i18n::t("ui.file_is_not_in_edit_mode"))?;
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
        e.status = crate::i18n::t("ui.saving_buffer_remains_editable").into();
        Ok(())
    }
    pub fn new(root: PathBuf) -> Self {
        let mut files = Files::new(root.clone());
        files.light = crate::appearance::theme() == crate::appearance::Theme::Light;
        Self {
            shortcuts: crate::shortcuts::Index::current(),
            explorer: explorer::Explorer::new(root.clone()),
            preview_defaults: Default::default(),
            read_only: false,
            image_worker: Default::default(),
            image_display: Default::default(),
            images_occluded: false,
            image_occlusion: None,
            image_composition: Default::default(),
            image_suspended: false,
            tools: Default::default(),
            controls: Default::default(),
            reader: Default::default(),
            tree_view: None,
            prompt: None,
            after_save: None,
            ready_manage: None,
            files,
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
            hover: None,
            enlarged: None,
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
            locate_explicit: false,
        }
    }
    pub fn say(&mut self, text: impl Into<String>, transient: bool) {
        let text = text.into();
        self.notice = text;
        self.notice_at = transient.then(Instant::now);
    }
    pub fn showing(&self) -> bool {
        self.preview_visible && self.document.is_some()
    }
    fn document_area(&self) -> Option<Rect> {
        self.geometry.preview.map(|rect| {
            if self.document.as_ref().is_some_and(|d| d.editing) {
                crate::ui::content(rect)
            } else {
                self.preview_panes()
                    .map_or_else(|| crate::ui::preview_content(rect), |(r, _)| r)
            }
        })
    }
    pub fn update_layout(&mut self, area: Rect, split: bool) -> bool {
        self.tools.area = area;
        self.tools.split = split;
        let mut next = panels_with_ratio(
            area,
            split,
            self.visible,
            self.showing(),
            self.explorer.preferences.width,
            self.explorer.preferences.agent_ratio,
        );
        if self.tools.maximized && self.showing() && area.width >= 40 && area.height >= 12 {
            next.tree = None;
            next.debug = None;
            next.overlay = true;
            next.preview = Some(Rect::new(area.x, area.y, area.width, area.height - 1));
        }
        if let Some(target) = self.enlarged {
            let body = crate::app::chrome::body(area);
            next.tree = (target == Focus::Tree).then_some(body);
            next.preview = (target == Focus::Preview && self.showing()).then_some(body);
            next.agent = (target == Focus::Terminal).then_some(body);
            next.debug = None;
            next.overlay = target != Focus::Terminal;
        }
        let changed = next != self.geometry;
        if changed {
            if next.preview.map(|r| r.width) != self.geometry.preview.map(|r| r.width)
                && let Some(d) = &mut self.document
                && d.rendered().is_some()
                && d.reader.seek.is_none()
            {
                d.reader.seek = Some(d.reading_byte());
            }
            if next.agent.is_none() && self.enlarged.is_none() {
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
            if next.preview.is_none() {
                self.reader.outline = None;
            }
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
        if self.explorer.busy {
            anyhow::bail!(crate::i18n::t("ui.wait_for_the_file_operation_to_finish"));
        }
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
        if self.explorer.preferences.follow && path.starts_with(&self.files.root) {
            self.locate_pending = Some(path.clone());
            self.locate_explicit = false;
            if !self.files.nodes.contains_key(&path)
                && let Some(parent) = path.parent()
                && self.files.nodes.contains_key(parent)
                && self.files.dirs.get(parent).is_some_and(|d| d.complete)
            {
                let _ = self.files.request(parent, true, &self.io);
            }
        }
        self.serial += 1;
        if !preview::image::candidate(&path) && !self.tools.parked.iter().any(|d| d.path == path) {
            self.io.send(Job::Read {
                path: path.clone(),
                id: self.serial,
            })?;
        }
        if !self.reader.traversing {
            self.push_back();
        }
        self.controls.hover = None;
        self.remember_reading();
        self.image_worker.release();
        self.park_document();
        self.register_tab(&path);
        if let Some(index) = self.tools.parked.iter().position(|d| d.path == path) {
            self.document = Some(self.tools.parked.remove(index));
            self.preview_visible = true;
            self.focus = Focus::Preview;
            self.refresh_document()?;
            return Ok(());
        }
        self.source_focus = self.focus;
        let mut document = Document::new(path, self.serial);
        document.view.wrap = self.preview_defaults.wrap.unwrap_or(false);
        document.defaults_pending = !self.explorer.defaults_ready;
        self.document = Some(document);
        self.restore_reading();
        self.preview_visible = true;
        if focus {
            self.focus = Focus::Preview;
        }
        Ok(())
    }
    pub fn close(&mut self) {
        self.controls.hover = None;
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::Close));
            return;
        }
        self.image_worker.release();
        self.remember_reading();
        if let Some(d) = &self.document {
            self.tools.paths.retain(|p| p != &d.path);
        }
        self.document = None;
        if let Some(path) = self.tools.paths.last().cloned() {
            if let Err(e) = self.open(path, true) {
                self.say(e.to_string(), false);
            }
            return;
        }
        self.tools.maximized = false;
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
        if self.document.as_ref().is_some_and(Document::is_image) {
            self.invalidate_image(true);
            return Ok(());
        }
        if let Some(d) = &mut self.document {
            d.reader.images.clear();
            self.image_worker.release();
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
        let viewport_area = self.document_area();
        let Some(d) = &mut self.document else {
            return Ok(());
        };
        let Some(loaded) = d.loaded() else {
            return Ok(());
        };
        if d.render_pending {
            return Ok(());
        }
        let width = viewport_area.map_or(80, |r| r.width);
        self.serial += 1;
        let id = self.serial;
        let source = d.editor().map_or_else(
            || loaded.source.clone(),
            |e| {
                d.rendered()
                    .filter(|_| d.render_revision == e.revision)
                    .map_or_else(|| std::sync::Arc::from(e.source()), |r| r.source.clone())
            },
        );
        d.render_revision = d.editor().map_or(0, |e| e.revision);
        d.code_generation
            .store(id, std::sync::atomic::Ordering::Relaxed);
        let viewport = preview::CodeViewport {
            top: if d.editing {
                d.editor().map_or(0, |e| e.area.scroll_offset().0 as usize)
            } else {
                d.offset().0
            },
            rows: viewport_area.map_or(40, |r| r.height.max(1)) as usize,
        };
        self.io.send(Job::Render {
            wrap: d.view.wrap,
            editor: d.editing,
            path: d.path.clone(),
            id,
            source,
            markdown: d.markdown && !d.editing,
            width,
            light: self.files.light,
            viewport: d
                .rendered()
                .map_or(viewport, |r| r.worker_viewport(viewport)),
            generation: d.code_generation.clone(),
            complete: !d.search.is_empty() || d.markdown_end || d.markdown_copy,
            reading: preview::reading::ReadingOptions {
                image_heights: d.reader.images.heights(),
                expanded: d.reader.expanded.clone(),
                seek: d.reader.seek,
            },
        })?;
        d.render_id = id;
        d.render_pending = true;
        Ok(())
    }
    fn highlight_document(&mut self) {
        let Some(rect) = self.document_area() else {
            return;
        };
        let Some(d) = &mut self.document else {
            return;
        };
        let Some(r) = d.rendered() else {
            return;
        };
        if r.markdown {
            return;
        }
        let revision = d.editor().map_or(0, |e| e.revision);
        let top = if d.editing {
            d.editor().map_or(0, |e| e.area.scroll_offset().0 as usize)
        } else {
            d.offset().0
        };
        let viewport = preview::CodeViewport {
            top,
            rows: rect.height.max(1) as usize,
        };
        let covered = r
            .window
            .as_ref()
            .is_some_and(|w| w.covers(viewport, self.files.light));
        let source = r.source.clone();
        let worker_viewport = r.worker_viewport(viewport);
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
            if let Some(e) = d.editor_mut()
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
            || covered
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
            source,
            viewport: worker_viewport,
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
        let mut changed = self.image_tick(active);
        changed |= self.explorer_tick(active);
        if active {
            self.files.pump_refresh(&self.io);
        }
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
            let Some(output) = self.control_completion(output) else {
                continue;
            };
            let Some(output) = self.feature_completion(output) else {
                continue;
            };
            match output {
                Completed::Bookmarks { .. }
                | Completed::Companion { .. }
                | Completed::Diff { .. } => unreachable!(),
                Completed::Image { path, id } => {
                    if let Some(d) = &mut self.document
                        && d.path == path
                        && d.id == id
                    {
                        if d.editor().is_some_and(|e| e.dirty()) {
                            d.stale =
                                crate::i18n::t("ui.disk_changed_to_an_image_local_edits_retained")
                                    .into();
                            d.loading = false;
                        } else {
                            d.content = Content::Image(Default::default());
                            d.markdown = false;
                            d.editing = false;
                            d.loading = true;
                            self.focus = Focus::Preview;
                        }
                    }
                }
                Completed::Edit { id, result } => {
                    if let Some(d) = &mut self.document
                        && d.edit_requested == Some(id)
                    {
                        d.edit_requested = None;
                        match result {
                            Ok(editor) => {
                                d.set_editor(Some(*editor));
                                d.editing = true;
                                d.markdown = false;
                                d.set_rendered(None);
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
                        && let Some(editor) = d.editor_mut()
                    {
                        editor.saving = false;
                        match result {
                            Ok(loaded) => {
                                let loaded = std::sync::Arc::new(loaded);
                                editor.mark_saved(loaded.clone(), source.replace("\r\n", "\n"));
                                if let Some(old) = d.loaded()
                                    && old.bytes != loaded.bytes
                                {
                                    d.view.previous = Some(old.clone());
                                }
                                d.view.disk = None;
                                d.set_loaded(Some(loaded));
                                d.stale.clear();
                                self.say(crate::i18n::t("ui.saved"), true);
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
                                if let Some(old) = d.loaded()
                                    && old.bytes != loaded.bytes
                                {
                                    d.view.previous = Some(old.clone());
                                    d.view.disk = Some(loaded.clone());
                                }
                                if let Some(editor) = d.editor_mut() {
                                    if loaded.bytes != editor.loaded.bytes
                                        || loaded.identity != editor.loaded.identity
                                    {
                                        if editor.dirty() || editor.saving {
                                            editor.conflict = true;
                                            d.stale=crate::i18n::t("ui.disk_changed_local_edits_retained_f5_reloads_after_confirmation").into();
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
                                                d.set_editor(Some(next));
                                            }
                                            Err(e) => {
                                                d.stale =
                                                    format!("File became preview-only: {e:#}");
                                                d.set_editor(None);
                                                d.editing = false;
                                                self.focus = Focus::Preview;
                                            }
                                        }
                                    } else {
                                        d.loading = false;
                                        continue;
                                    }
                                }
                                let first = d.loaded().is_none();
                                let different =
                                    d.loaded().is_none_or(|old| old.bytes != loaded.bytes);
                                if first && d.reader.seek.is_none() {
                                    d.markdown = loaded.markdown()
                                        && self.preview_defaults.markdown_mode.unwrap_or_default()
                                            != crate::settings::model::MarkdownMode::Source;
                                }
                                d.set_loaded(Some(loaded));
                                d.stale.clear();
                                if different {
                                    d.selection = None;
                                    d.set_rendered(None);
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
                            && d.rendered().is_some()
                        {
                            let revision = d.code_revision;
                            if let Some(e) = d.editor_mut()
                                && e.revision == revision
                            {
                                e.highlight = Some((e.revision, window.clone()));
                            }
                            if let Some(r) = d.rendered_mut() {
                                r.attach_window(Some(window));
                            }
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
                    let expected_width = self.document_area().map_or(80, |r| r.width);
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
                        if d.editor().is_some_and(|e| e.revision != d.render_revision) {
                            continue;
                        }
                        d.code_stats = stats;
                        d.markdown_stats = markdown_stats;
                        // A new snapshot invalidates colors; viewport highlighting is separate.
                        if let Some(e) = d.editor_mut() {
                            e.highlight = result
                                .window
                                .as_ref()
                                .filter(|_| !result.markdown)
                                .map(|w| (e.revision, w.clone()));
                        }
                        d.code_desired = None;
                        d.code_generation
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if d.rendered().is_some_and(|previous| {
                            previous.width != result.width
                                || previous.markdown != result.markdown
                                || previous.source != result.source
                        }) {
                            d.selection = None;
                        }
                        d.set_rendered(Some(result));
                        d.restore_reading_position();
                        let len = d.rendered().map_or(0, |r| r.plain.len());
                        if d.rendered().is_some_and(|r| r.complete) {
                            if d.markdown_end {
                                d.offset_mut().0 = len.saturating_sub(1);
                                d.markdown_end = false;
                            } else {
                                d.offset_mut().0 = d.offset().0.min(len.saturating_sub(1));
                            }
                        }
                        d.search();
                        if d.markdown_search_jump
                            && d.rendered().as_ref().is_some_and(|r| r.complete)
                        {
                            if let Some(&(row, col, _)) = d.matches.first() {
                                if d.offset() != (row, if d.view.wrap { 0 } else { col })
                                    && let Some(origin) = d.reader.search_origin.take()
                                {
                                    self.reader.record(origin);
                                }
                                *d.offset_mut() = (row, if d.view.wrap { 0 } else { col });
                            }
                            d.markdown_search_jump = false;
                        }
                    }
                }
            }
        }
        if active && let Some(d) = &self.document {
            let viewport_area = self.document_area();
            let width = viewport_area.map_or(80, |r| r.width);
            if !d.render_pending
                && d.loaded().is_some()
                && (d.rendered().is_none_or(|r| {
                    r.width != width
                        || r.light != self.files.light
                        || r.markdown != (d.markdown && !d.editing)
                }) || d.reader.seek.is_some()
                    || d.reader.fragment.is_some()
                    || d.editor().is_some_and(|e| e.revision != d.render_revision)
                    || d.rendered().is_some_and(|r| {
                        r.markdown
                            && ((!r.complete
                                && (!d.search.is_empty()
                                    || d.markdown_end
                                    || d.markdown_copy
                                    || d.offset().0.saturating_add(
                                        viewport_area.map_or(40, |r| r.height) as usize,
                                    ) > r.plain.len()))
                                || r.window.as_ref().is_none_or(|w| {
                                    !w.covers(
                                        preview::CodeViewport {
                                            top: d.offset().0,
                                            rows: viewport_area.map_or(40, |r| r.height) as usize,
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
            changed |= self.feature_tick();
        }
        if let Some(d) = &mut self.document
            && d.markdown_copy
            && let Some(r) = d.rendered()
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
                if let Some(i) = d.image_mut() {
                    i.release();
                    self.image_worker.release();
                } else {
                    let _ = self.io.send(Job::Read {
                        path: d.path.clone(),
                        id: d.id,
                    });
                }
            }
        }
        self.was_watching = watching;
        let image_paths = self
            .document
            .as_ref()
            .map_or_else(Vec::new, |d| d.reader.images.paths());
        self.files.watch_images(
            watching,
            self.document.as_ref().map(|d| d.path.as_path()),
            &image_paths,
        );
        if let Some(paths) = self.files.changes() {
            if !paths.is_empty() {
                self.explorer.invalidate();
            }
            if let Some(d) = &mut self.document
                && (d.reader.images.changed(&paths) || paths.contains(&self.files.root))
            {
                d.reader.images.invalidate_pixels();
                self.image_worker.release();
                changed = true;
            }

            if self
                .document
                .as_ref()
                .is_some_and(|d| paths.iter().any(|p| p == &d.path || p == &self.files.root))
                && let Some(d) = &mut self.document
            {
                self.serial += 1;
                d.id = self.serial;
                if let Some(i) = d.image_mut() {
                    i.release();
                    self.image_worker.release();
                } else {
                    let _ = self.io.send(Job::Read {
                        path: d.path.clone(),
                        id: d.id,
                    });
                }
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
                            if self.locate_explicit {
                                self.say(
                                    crate::i18n::t(
                                        "ui.file_is_missing_or_filtered_adjust_filters_or_refresh",
                                    ),
                                    false,
                                );
                            }
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
            Some(Key::Notice(p))
                if self.files.dirs.get(&p).is_some_and(|d| !d.error.is_empty()) =>
            {
                self.files.request(&p, true, &self.io)?
            }
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
            (crate::i18n::t("language"), Command::Language),
            (crate::i18n::t("legacy_7"), Command::Insert),
            (crate::i18n::t("legacy_8"), Command::CopyAbsolute),
            (crate::i18n::t("legacy_9"), Command::CopyRelative),
            (crate::i18n::t("legacy_10"), Command::Reveal),
            (crate::i18n::t("legacy_11"), Command::External),
        ];
        if self.focus == Focus::Tree {
            items.extend([
                (crate::i18n::t("legacy_12"), Command::QuickOpen),
                (crate::i18n::t("legacy_13"), Command::PreviewSelected),
                (crate::i18n::t("legacy_14"), Command::NewFile),
                (crate::i18n::t("legacy_15"), Command::NewDirectory),
                (crate::i18n::t("legacy_16"), Command::Rename),
                (crate::i18n::t("legacy_17"), Command::Trash),
                (crate::i18n::t("legacy_18"), Command::Follow),
                (crate::i18n::t("legacy_19"), Command::Compact),
                (crate::i18n::t("legacy_20"), Command::Modified),
                (crate::i18n::t("legacy_21"), Command::Collapse),
                (crate::i18n::t("legacy_22"), Command::Release),
                (crate::i18n::t("legacy_23"), Command::Icons),
                (crate::i18n::t("legacy_24"), Command::Theme),
            ]);
        } else if self.document.as_ref().is_some_and(Document::is_image) {
            items.extend([
                (crate::i18n::t("legacy_25"), Command::ImageFit),
                (crate::i18n::t("legacy_26"), Command::ImageActual),
                (crate::i18n::t("legacy_27"), Command::ImageZoomIn),
                (crate::i18n::t("legacy_28"), Command::ImageZoomOut),
                (crate::i18n::t("legacy_29"), Command::ImageProfile),
                (crate::i18n::t("legacy_30"), Command::ImageProtocol),
                (crate::i18n::t("legacy_31"), Command::ImageRefresh),
                (crate::i18n::t("legacy_32"), Command::Maximize),
                (crate::i18n::t("legacy_33"), Command::Info),
                (crate::i18n::t("legacy_34"), Command::NextTab),
                (crate::i18n::t("legacy_35"), Command::PreviousTab),
                (crate::i18n::t("legacy_36"), Command::Back),
                (crate::i18n::t("legacy_37"), Command::Forward),
                (crate::i18n::t("legacy_38"), Command::Bookmarks),
                (crate::i18n::t("legacy_39"), Command::CloseOthers),
                (crate::i18n::t("legacy_40"), Command::CloseRight),
                (crate::i18n::t("legacy_41"), Command::Close),
            ]);
        } else {
            items.push((crate::i18n::t("legacy_42"), Command::Edit));
            if self.document.as_ref().is_some_and(|d| d.editor().is_some()) {
                items.extend([
                    (crate::i18n::t("legacy_43"), Command::Save),
                    (crate::i18n::t("legacy_44"), Command::Undo),
                    (crate::i18n::t("legacy_45"), Command::Redo),
                    (crate::i18n::t("legacy_46"), Command::CopyContent),
                    (crate::i18n::t("legacy_47"), Command::Cut),
                    (crate::i18n::t("legacy_48"), Command::Paste),
                    (crate::i18n::t("legacy_49"), Command::Find),
                    (crate::i18n::t("legacy_50"), Command::Project),
                    (crate::i18n::t("legacy_51"), Command::EndEdit),
                ]);
            }
            items.extend([
                (crate::i18n::t("legacy_32"), Command::Maximize),
                (crate::i18n::t("legacy_52"), Command::Wrap),
                (crate::i18n::t("legacy_53"), Command::Compare),
                (crate::i18n::t("legacy_54"), Command::Mode),
                (crate::i18n::t("legacy_55"), Command::Diff),
                (crate::i18n::t("legacy_56"), Command::Goto),
                (crate::i18n::t("legacy_57"), Command::Search),
                (crate::i18n::t("legacy_34"), Command::NextTab),
                (crate::i18n::t("legacy_35"), Command::PreviousTab),
                (crate::i18n::t("legacy_58"), Command::Outline),
                (crate::i18n::t("legacy_36"), Command::Back),
                (crate::i18n::t("legacy_37"), Command::Forward),
                (crate::i18n::t("legacy_39"), Command::CloseOthers),
                (crate::i18n::t("legacy_40"), Command::CloseRight),
                (crate::i18n::t("legacy_59"), Command::Bookmark),
                (crate::i18n::t("legacy_38"), Command::Bookmarks),
                (crate::i18n::t("legacy_33"), Command::Info),
                (crate::i18n::t("legacy_60"), Command::Locate),
                (crate::i18n::t("legacy_61"), Command::CopyContent),
                (crate::i18n::t("legacy_62"), Command::CopySource),
                (crate::i18n::t("legacy_41"), Command::Close),
            ]);
        }
        self.menu = Some(Menu {
            items,
            selected: 0,
            target,
        });
    }
    fn command(&mut self, command: Command, target: PathBuf) -> Result<Action> {
        self.execute_at(command.id(), target)
    }
    fn execute_at(&mut self, id: &str, target: PathBuf) -> Result<Action> {
        let Some(command) = Command::from_id(id) else {
            return Ok(Action::None);
        };
        self.perform_command(command, target)
    }
    fn perform_command(&mut self, command: Command, target: PathBuf) -> Result<Action> {
        if let Some(action) = self.explorer_command(command, &target)? {
            return Ok(action);
        }
        if !self.command_enabled(command) {
            return Ok(Action::None);
        }
        if self.image_command(command) {
            return Ok(Action::None);
        }
        match command {
            Command::QuickOpen
            | Command::Follow
            | Command::Compact
            | Command::Modified
            | Command::NewFile
            | Command::NewDirectory
            | Command::Rename
            | Command::Trash
            | Command::PreviewSelected => unreachable!(),
            Command::ImageFit
            | Command::ImageActual
            | Command::ImageZoomIn
            | Command::ImageZoomOut
            | Command::ImageProfile
            | Command::ImageProtocol
            | Command::ImageRefresh => {}
            Command::More => {
                self.menu();
                return Ok(Action::None);
            }
            Command::Forward => self.reading_history(true)?,
            Command::CloseOthers | Command::CloseRight => {
                self.close_group(&target, matches!(command, Command::CloseRight))
            }
            Command::Bookmark | Command::Bookmarks | Command::Info => {
                self.control_command(command)?;
                return Ok(Action::None);
            }
            Command::Maximize
            | Command::Wrap
            | Command::Compare
            | Command::Diff
            | Command::Goto
            | Command::Search
            | Command::NextTab
            | Command::PreviousTab => {
                self.preview_command(command)?;
                return Ok(Action::None);
            }
            Command::Mode => {
                if let Some(d) = &mut self.document
                    && d.loaded().as_ref().is_some_and(|l| l.markdown())
                {
                    d.reader.seek = Some(d.reading_byte());
                    d.markdown = !d.markdown;
                    d.render_id = 0;
                    d.render_pending = false;
                }
            }
            Command::Outline => self.start_outline(),
            Command::Back => self.reading_back()?,
            Command::Insert => return Ok(Action::Insert(vec![target])),
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
                let theme = if self.files.light {
                    crate::appearance::Theme::Light
                } else {
                    crate::appearance::Theme::Tokyo
                };
                self.explorer
                    .worker
                    .send(crate::explorer::Job::Theme(theme))?;
                crate::appearance::set(theme);
                if let Some(d) = &mut self.document {
                    d.reader.seek = Some(d.reading_byte());
                    d.set_rendered(None);
                    d.selection = None;
                    d.render_pending = false;
                    d.render_id = 0;
                }
            }
            Command::Language => self.cycle_language()?,
            Command::Locate => {
                ensure!(
                    target.starts_with(&self.files.root),
                    crate::i18n::t("ui.file_is_outside_this_project")
                );
                self.visible = true;
                self.locate_pending = Some(target);
                self.locate_explicit = true;
                self.focus = Focus::Tree;
            }
            Command::CopyContent | Command::CopySource => {
                if matches!(command, Command::CopyContent)
                    && let Some(d) = &mut self.document
                    && d.markdown
                    && !d.editing
                    && d.rendered().as_ref().is_some_and(|r| !r.complete)
                {
                    d.markdown_copy = true;
                    return Ok(Action::None);
                }
                let d = self
                    .document
                    .as_ref()
                    .context(crate::i18n::t("ui.no_document"))?;
                let source_selection = if matches!(command, Command::CopySource) && !d.editing {
                    d.selection
                        .and_then(|(a, b)| d.rendered().map(|r| r.selected_source(a, b)))
                } else {
                    None
                };
                let text = if let Some(selected) = source_selection {
                    selected
                } else if let Some(e) = d.editor() {
                    if matches!(command, Command::CopyContent) && d.editing {
                        e.selected().unwrap_or_else(|| e.source())
                    } else if matches!(command, Command::CopySource) || !d.markdown {
                        e.source()
                    } else {
                        d.rendered().context("Rendering")?.plain.join("\n")
                    }
                } else if matches!(command, Command::CopySource) || !d.markdown {
                    preview::copy_text(
                        &d.loaded()
                            .context(crate::i18n::t("ui.file_not_loaded"))?
                            .source,
                    )
                } else {
                    d.rendered().context("Rendering")?.plain.join("\n")
                };
                files::copy(text)?;
            }
            Command::Close => self.close_tab(&target),
            Command::Edit => self.begin_edit()?,
            Command::Save => self.save()?,
            Command::Undo | Command::Redo | Command::Cut | Command::Paste | Command::Find => {
                let e = self
                    .document
                    .as_mut()
                    .and_then(|d| d.editor_mut())
                    .context(crate::i18n::t("ui.no_edit_buffer"))?;
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
                    d.markdown = d.loaded().is_some_and(|l| l.markdown());
                    d.render_pending = false;
                    d.set_rendered(None);
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
        self.say(crate::i18n::t("ui.done"), true);
        Ok(Action::None)
    }
    fn start_query(&mut self, tree: bool) {
        if !tree && let Some(d) = &mut self.document {
            d.reader.search_origin = Some(d.bookmark());
            d.markdown_search_jump = false;
        }
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
            d.markdown_search_jump =
                d.markdown && d.rendered().as_ref().is_some_and(|r| !r.complete);
            d.search();
            if let Some(&(row, col, _)) = d.matches.first() {
                if d.offset() != (row, if d.view.wrap { 0 } else { col })
                    && let Some(origin) = d.reader.search_origin.take()
                {
                    self.reader.record(origin);
                }
                *d.offset_mut() = (row, if d.view.wrap { 0 } else { col });
            }
        }
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        if self.explorer_paste(text) {
            return Ok(());
        }
        if self.control_paste(text) {
            return Ok(());
        }
        if self.feature_paste(text) {
            return Ok(());
        }
        if let Some(o) = &mut self.reader.outline {
            o.paste(text);
            return Ok(());
        }
        if self.prompt.is_some() {
            return Ok(());
        }
        if self.focus == Focus::Editor
            && let Some(e) = self.document.as_mut().and_then(|d| d.editor_mut())
        {
            return e.paste(text);
        }
        if let Some(q) = &mut self.query {
            ensure!(
                q.value.chars().count() + text.chars().count() <= 256,
                crate::i18n::t("ui.query_exceeds_256_characters")
            );
            q.value.push_str(&text.replace(['\r', '\n'], ""));
            self.update_query();
        }
        Ok(())
    }
    pub fn current_wrap(&self) -> Option<bool> {
        self.document.as_ref().map(|d| d.view.wrap)
    }
    pub fn shortcut_context(&self) -> &'static str {
        match self.focus {
            Focus::Tree => "tree",
            Focus::Editor => "editor",
            Focus::Preview if self.document.as_ref().is_some_and(Document::is_image) => {
                "preview.image"
            }
            Focus::Preview => "preview",
            _ => "terminal.input",
        }
    }
    pub fn execute_command(&mut self, id: &str) -> Result<Action> {
        match id {
            "tree.search" => {
                self.start_query(true);
                return Ok(Action::None);
            }
            "tree.hidden" => {
                self.files.hidden = !self.files.hidden;
                self.files.refresh(&self.io)?;
                return Ok(Action::None);
            }
            "tree.ignored" => {
                self.files.ignored = !self.files.ignored;
                self.files.refresh(&self.io)?;
                return Ok(Action::None);
            }
            "preview.match.next" | "preview.match.previous" => {
                self.push_back();
                if let Some(d) = &mut self.document {
                    d.jump_match(id.ends_with("previous"));
                }
                return Ok(Action::None);
            }
            "preview.copy_selection" => {
                let text = self
                    .document
                    .as_ref()
                    .and_then(|d| d.selected_text())
                    .context("Select text first")?;
                files::copy(text)?;
                return Ok(Action::None);
            }
            "preview.copy_path" => {
                if let Some(d) = &self.document {
                    files::copy(d.path.to_string_lossy().into_owned())?;
                }
                return Ok(Action::None);
            }
            _ => {}
        }
        if id.starts_with("panel.") {
            let target = if self.focus == Focus::Tree {
                self.files.selected_path()
            } else {
                self.document.as_ref().map(|d| d.path.clone())
            }
            .unwrap_or_else(|| self.files.root.clone());
            return self.execute_at(id, target);
        }
        if id.ends_with(".menu") {
            self.menu();
        } else if id.ends_with(".refresh") {
            self.explorer.invalidate();
            self.files.retry_watch();
            self.files.refresh(&self.io)?;
            self.refresh_document()?;
        } else if id == "editor.save" {
            self.save()?;
        } else if id.starts_with("editor.")
            && let Some(e) = self.document.as_mut().and_then(|d| d.editor_mut())
        {
            e.execute_command(id)?;
        }
        Ok(Action::None)
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<Action> {
        if let Some(d) = &mut self.document {
            d.defaults_pending = false;
        }
        if !self.modal()
            && !self
                .document
                .as_ref()
                .and_then(|d| d.editor())
                .is_some_and(|e| e.search_input.is_some())
        {
            let context = self.shortcut_context();
            if let crate::shortcuts::Resolution::Execute(id) = self
                .shortcuts
                .resolve(context, &[crate::shortcuts::Chord::event(key)])
            {
                return self.execute_command(id);
            }
        }
        if let Some(action) = self.explorer_key(key)? {
            return Ok(action);
        }
        if self.control_key(key)? {
            return Ok(Action::None);
        }
        if self.image_key(key)? {
            return Ok(Action::None);
        }
        if self.feature_key(key)? {
            return Ok(Action::None);
        }
        if self.outline_key(key) {
            return Ok(Action::None);
        }
        if !self.modal()
            && self.focus == Focus::Preview
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
                            .and_then(|d| d.editor())
                            .is_some_and(|e| e.saving)
                        {
                            self.prompt = Some(Prompt::Unsaved(pending));
                            self.say(
                                crate::i18n::t(
                                    "ui.save_in_progress_wait_before_discarding_or_switching",
                                ),
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
                            crate::i18n::t(
                                "ui.no_loaded_match_unexpanded_directories_were_not_searched",
                            ),
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
        if self.focus == Focus::Editor {
            if let Some(e) = self.document.as_mut().and_then(|d| d.editor_mut()) {
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
                KeyCode::Esc => {
                    self.focus_agent();
                    return Ok(Action::Agent);
                }
                _ => {}
            }
        } else if self.focus == Focus::Preview {
            let page = self
                .document_area()
                .map_or(10, |r| r.height.saturating_sub(1).max(1) as usize);
            let Some(d) = &mut self.document else {
                return Ok(Action::None);
            };
            match key.code {
                KeyCode::Esc => {
                    if d.selection.take().is_none() {
                        if !d.search.is_empty() {
                            d.search.clear();
                            d.search();
                        } else {
                            self.close();
                        }
                    }
                }
                KeyCode::Up => d.offset_mut().0 = d.offset().0.saturating_sub(1),
                KeyCode::Down => d.offset_mut().0 = d.offset().0.saturating_add(1),
                KeyCode::PageUp => d.offset_mut().0 = d.offset().0.saturating_sub(page),
                KeyCode::PageDown => d.offset_mut().0 = d.offset().0.saturating_add(page),
                KeyCode::Left => d.offset_mut().1 = d.offset().1.saturating_sub(4),
                KeyCode::Right => d.offset_mut().1 += 4,
                KeyCode::Home => *d.offset_mut() = (0, 0),
                KeyCode::End => {
                    if d.markdown && d.rendered().as_ref().is_some_and(|r| !r.complete) {
                        d.markdown_end = true;
                    }
                    d.offset_mut().0 = d.rendered().map_or(0, |r| r.plain.len().saturating_sub(1))
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
                self.say(crate::i18n::t("ui.cancelled_no_path_sent"), true);
            }
        }
    }
    pub fn mouse(&mut self, event: MouseEvent) -> Result<(bool, Action)> {
        self.mouse_with_agent(event, self.geometry.agent)
    }
    /// The caller supplies the visible Agent pane, including an in-app panel.
    pub fn mouse_with_agent(
        &mut self,
        event: MouseEvent,
        agent: Option<Rect>,
    ) -> Result<(bool, Action)> {
        if event.kind == MouseEventKind::Moved {
            self.hover = Some((event.column, event.row));
        }
        if let Some(action) = self.explorer_mouse(event)? {
            return Ok((true, action));
        }
        if let Some(menu) = &mut self.menu {
            let popup = menu_popup(self.tools.area, menu.items.len());
            match event.kind {
                MouseEventKind::ScrollDown => {
                    menu.selected = (menu.selected + 1).min(menu.items.len().saturating_sub(1))
                }
                MouseEventKind::ScrollUp => menu.selected = menu.selected.saturating_sub(1),
                MouseEventKind::Down(MouseButton::Left) => {
                    if popup.contains((event.column, event.row).into()) {
                        if event.row > popup.y && event.row < popup.bottom().saturating_sub(1) {
                            let first = menu
                                .selected
                                .saturating_sub(popup.height.saturating_sub(3) as usize);
                            let index = first + (event.row - popup.y - 1) as usize;
                            if index < menu.items.len() {
                                let menu = self.menu.take().unwrap();
                                let action = self.command(menu.items[index].1, menu.target)?;
                                return Ok((true, action));
                            }
                        }
                    } else {
                        self.menu = None;
                    }
                }
                _ => {}
            }
            return Ok((true, Action::None));
        }

        if self.control_mouse(event)? {
            return Ok((true, Action::None));
        }
        if self.outline_mouse(event) {
            return Ok((true, Action::None));
        }
        if self.feature_mouse(event)? {
            return Ok((true, Action::None));
        }
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
                && !d.is_image()
            {
                let buttons = visible_header_buttons(
                    self.preview_controls(),
                    rect,
                    d.editing,
                    d.loaded().is_some_and(|l| l.markdown()),
                    d.markdown,
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
        if self.image_mouse(event)? {
            return Ok((true, Action::None));
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
                        self.say(
                            crate::i18n::t("ui.drag_to_agent_content_release_inserts_a_path"),
                            false,
                        );
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        self.gesture = Gesture::Idle;
                        let hit = self.tree_hit(point).and_then(|id| id.last());
                        if hit == Some(&Key::Entry(path.clone())) && self.click_preview_allowed() {
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
                let valid = agent.is_some_and(|r| crate::ui::content(r).contains(point.into()));
                if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                    if valid {
                        self.gesture = Gesture::Idle;
                        self.drag_restore = false;
                        let action = self.insert_selection(path);
                        self.focus = Focus::Terminal;
                        return Ok((true, action));
                    }
                    self.cancel_drag(true);
                } else if matches!(event.kind, MouseEventKind::Down(_)) {
                    self.cancel_drag(true);
                } else {
                    self.say(
                        if valid {
                            crate::i18n::t("ui.release_to_insert_path")
                        } else {
                            crate::i18n::t("ui.cannot_insert_here")
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
                    } else if let Some(id) = self.tree_hit(point).map(<[Key]>::to_vec) {
                        let previous = self.files.selected_path();
                        self.files.state.select(id);
                        self.select_tree(event.modifiers, previous);
                        match self.files.selected() {
                            Some(Key::Entry(path))
                                if self.files.nodes.get(&path).is_some_and(|n| !n.directory) =>
                            {
                                self.gesture = Gesture::Pressed {
                                    path,
                                    origin: point,
                                };
                            }
                            Some(Key::Entry(path)) => {
                                let depth =
                                    self.files.state.selected().len().saturating_sub(1) as u16;
                                let arrow = rect.x + 3 + depth * 2;
                                if event.column <= arrow {
                                    self.files.toggle(&path, &self.io)?;
                                }
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
                MouseEventKind::Down(MouseButton::Right) => {
                    if let Some(id) = self.tree_hit(point).map(<[Key]>::to_vec) {
                        self.files.state.select(id);
                        if self
                            .files
                            .selected_path()
                            .is_some_and(|p| !self.files.selected_paths.contains(&p))
                        {
                            self.files.selected_paths.clear();
                        }
                    }
                    self.menu();
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
                } else if let Some(e) = self.document.as_mut().and_then(|d| d.editor_mut()) {
                    e.mouse(event, rect);
                }
                return Ok((true, Action::None));
            }
            self.focus = Focus::Preview;
            let inner = self.document_area().expect("preview area");
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
                            .rendered()
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
            && let Some(r) = d.rendered()
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
                    hit_area: Rect::default(),
                });
            }
            let view = self.tree_view.as_mut().expect("tree projection");
            let title = format!(
                " {} ",
                preview::truncate_name(
                    &preview::safe_text(&self.files.nodes[&self.files.root].name),
                    rect.width.saturating_sub(12) as usize
                )
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
                        .highlight_style(if focused {
                            Style::new().bg(crate::appearance::palette().selected)
                        } else {
                            Style::default()
                        })
                        .highlight_symbol("> ")
                        .render(rect, &mut buffer, &mut self.files.state);
                }
                let row_count =
                    scroll::tree_rows(&view.items, self.files.state.opened(), &mut Vec::new());
                view.hit_area = Rect {
                    height: row_count
                        .saturating_sub(self.files.state.get_offset())
                        .min(inner.height as usize) as u16,
                    ..inner
                };
                // Directory labels stay cached across expansion changes. Only
                // the visible folder glyph follows the component's live state.
                if self.files.icons {
                    for y in view.hit_area.y..view.hit_area.bottom() {
                        let Some(id) = self.files.state.rendered_at((inner.x, y).into()) else {
                            continue;
                        };
                        if !matches!(id.last(), Some(Key::Entry(path)) if self.files.nodes.get(path).is_some_and(|n| n.directory))
                        {
                            continue;
                        }
                        let icon = if self.files.state.opened().contains(id) {
                            "\u{f115}"
                        } else {
                            "\u{f114}"
                        };
                        if let Some(x) = (inner.x..inner.right())
                            .find(|x| buffer[(*x, y)].symbol() == "\u{f115}")
                        {
                            buffer[(x, y)].set_symbol(icon);
                        }
                    }
                }
                view.viewport = Some(TreeViewport {
                    row_count,
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
            // Multi-selection decorates only visible rows. Moving the focus or
            // toggling a mark must not rebuild thousands of immutable labels.
            if !self.files.state.selected().is_empty() && inner.width > 1 {
                for y in view.hit_area.y..view.hit_area.bottom() {
                    let marked=self.files.state.rendered_at((inner.x,y).into()).and_then(|id|id.last()).is_some_and(|key|matches!(key,Key::Entry(path) if self.files.selected_paths.contains(path)));
                    if marked {
                        frame.buffer_mut()[(inner.x + 1, y)]
                            .set_symbol("*")
                            .set_fg(Color::Cyan);
                    }
                }
            }
        }
        if let Some(rect) = self.geometry.preview {
            let _perf = crate::perf::span(crate::perf::Metric::PreviewDraw);
            frame.render_widget(Clear, rect);
            let d = self.document.as_ref().expect("preview document");
            if d.editing
                && let Some(e) = d.editor()
            {
                e.draw(frame, rect, self.focus == Focus::Editor);
                draw_header_buttons(
                    frame,
                    self.preview_controls(),
                    rect,
                    true,
                    d.loaded().is_some_and(|l| l.markdown()),
                    false,
                );
                self.draw_scrollbars(frame);
                self.draw_overlay(frame);
                return;
            }
            if d.is_image() {
                self.draw_image(frame, rect);
                self.draw_overlay(frame);
                return;
            }
            let note = d.loaded().map_or("", |l| l.note.as_str());
            let title = format!(
                " {} · {}{} ",
                d.path.file_name().unwrap_or_default().to_string_lossy(),
                crate::i18n::t("preview.read_only"),
                if d.editor().is_some_and(|e| e.dirty()) {
                    crate::i18n::t("ui.unsaved")
                } else {
                    "".into()
                }
            );
            frame.render_widget(
                Block::bordered()
                    .title(features::shortened(
                        &title,
                        visible_header_buttons(
                            self.preview_controls(),
                            rect,
                            false,
                            d.loaded().is_some_and(|l| l.markdown()),
                            d.markdown,
                        )
                        .iter()
                        .map(|(r, _, _)| r.x.saturating_sub(rect.x + 2) as usize)
                        .min()
                        .unwrap_or(rect.width.saturating_sub(2) as usize),
                    ))
                    .border_style(border(self.focus == Focus::Preview)),
                rect,
            );
            draw_header_buttons(
                frame,
                self.preview_controls(),
                rect,
                false,
                d.loaded().is_some_and(|l| l.markdown()),
                d.markdown,
            );
            let inner = self.document_area().expect("preview area");
            let (_, left) = d.offset();
            if let Some(r) = d.rendered() {
                for (y, row) in d.screen_rows(inner.height as usize).into_iter().enumerate() {
                    let y = y as u16;
                    let Some(raw_line) = r.text.lines.get(row) else {
                        if r.markdown && !r.complete {
                            frame.render_widget(
                                Paragraph::new(crate::i18n::t("ui.loading_preview"))
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
                            Paragraph::new(r.projection.as_ref().map_or_else(
                                || format!("{:>4} │", row + 1),
                                |p| {
                                    if row > 0 && p[row].original == p[row - 1].original {
                                        "     │".into()
                                    } else {
                                        format!("{:>4} │", p[row].original + 1)
                                    }
                                },
                            ))
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
                    Paragraph::new(if d.loading {
                        crate::i18n::t("ui.loading")
                    } else {
                        std::borrow::Cow::Borrowed(d.stale.as_str())
                    }),
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
        self.draw_inline_images(frame);
        self.draw_scrollbars(frame);
        self.draw_overlay(frame);
    }
    pub fn draw_overlay(&self, frame: &mut Frame) {
        self.draw_explorer_header(frame);
        self.draw_features(frame);
        if let (Some(rect), Some(d)) = (self.geometry.preview, self.document.as_ref())
            && !d.editing
            && !d.is_image()
            && rect.height > 1
        {
            let mode = if d.markdown {
                "Markdown".into()
            } else {
                crate::i18n::t("chrome.source")
            };
            let label = features::shortened(
                &format!(" {mode} · {} ", d.progress()),
                rect.width.saturating_sub(4) as usize,
            );
            frame.render_widget(
                Paragraph::new(label).style(Style::new().fg(crate::appearance::palette().muted)),
                Rect::new(
                    rect.x + 2,
                    rect.bottom() - 1,
                    rect.width.saturating_sub(4),
                    1,
                ),
            );
        }
        self.draw_control_header(frame);
        if let Some(r) = self.geometry.toggle {
            frame.render_widget(
                Paragraph::new(crate::i18n::t("files_toggle")).style(
                    if self.focus == Focus::Tree {
                        Style::new()
                            .fg(Color::Black)
                            .bg(crate::appearance::palette().accent)
                    } else {
                        Style::new().fg(crate::appearance::palette().muted)
                    },
                ),
                r,
            );
        }
        self.draw_modal_overlays(frame);
    }
    pub(crate) fn draw_modal_overlays(&self, frame: &mut Frame) {
        let area = frame.area();
        if let Some(prompt) = &self.prompt {
            let text = match prompt {
                Prompt::Unsaved(_) => crate::i18n::t(
                    "ui.unsaved_changes_s_save_and_continue_d_discard_esc_cancel_save_failures_keep_this_document_open",
                ),
                Prompt::External(_) => crate::i18n::t(
                    "ui.external_applications_read_the_disk_version_s_save_then_open_o_open_disk_version_esc_cancel",
                ),
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
                        .title(crate::i18n::t("document"))
                        .border_style(Style::new().fg(Color::Yellow)),
                ),
                r,
            );
            return;
        }
        if let Some(menu) = &self.menu {
            let popup = menu_popup(area, menu.items.len());
            frame.render_widget(Clear, popup);
            let lines = menu
                .items
                .iter()
                .enumerate()
                .map(|(i, (label, command))| {
                    let state = self
                        .command_toggle(*command)
                        .map_or("", |on| if on { "[x] " } else { "[ ] " });
                    Line::styled(
                        format!(
                            "{} {state}{label}",
                            if i == menu.selected { ">" } else { " " }
                        ),
                        if !self.command_enabled(*command) {
                            Style::new().fg(Color::DarkGray)
                        } else if i == menu.selected {
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
                    crate::i18n::t("ui.find_loaded_nodes")
                } else {
                    crate::i18n::t("ui.find_preview_text")
                },
                q.value,
                if q.tree {
                    format!("{} matches", q.matches.len())
                } else {
                    self.document.as_ref().map_or(String::new(), |d| {
                        format!(
                            "{} matches{}",
                            d.view.search_targets.len(),
                            if d.loaded().is_some_and(|l| l.truncated) {
                                crate::i18n::t("ui.loaded_text_only")
                            } else {
                                "".into()
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
        self.draw_feature_overlay(frame);
        self.draw_search_options(frame);
        self.draw_outline(frame);
        self.draw_controls(frame);
        self.draw_explorer(frame);
    }
    pub(crate) fn preview_controls(&self) -> bool {
        matches!(self.focus, Focus::Preview | Focus::Editor)
            || self
                .geometry
                .preview
                .is_some_and(|r| self.hover.is_some_and(|p| r.contains(p.into())))
    }
    pub(crate) fn chrome_notice(&self) -> Option<&str> {
        if !self.notice.is_empty() {
            Some(&self.notice)
        } else if !self.files.watch_error.is_empty() {
            Some(&self.files.watch_error)
        } else {
            None
        }
    }
    pub fn status(&self) -> String {
        if !self.notice.is_empty() {
            return self.notice.clone();
        }
        if !self.files.watch_error.is_empty() {
            return self.files.watch_error.clone();
        }
        match self.focus {
            Focus::Tree => crate::i18n::format("tree_status", &[("path", self.files
                .selected_path()
                .unwrap_or_else(|| self.files.root.clone())
                .display().to_string())]),
            Focus::Preview => self.document.as_ref().map_or(String::new(), |d| {
                if let Some(i)=d.image(){return format!("{} · 0 Fit · 1 100% · +/- Zoom · Arrows Pan · P Quality · I Info · F2 More{}",d.path.display(),i.fallback.as_ref().map_or(String::new(),|reason|format!(" · {reason}")));}
                crate::i18n::format("preview_status", &[("progress", d.progress()), ("path", d.path.display().to_string())])
            }),
            Focus::Editor => self
                .document
                .as_ref()
                .and_then(|d| d.editor())
                .map_or(String::new(), |e| {
                    crate::i18n::format("editor_status", &[("path", e.loaded.path.display().to_string()), ("status", e.status.clone())])
                }),
            _ => String::new(),
        }
    }
}
fn menu_popup(area: Rect, items: usize) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(64) / 2,
        area.y + area.height.saturating_sub(items as u16 + 3) / 2,
        area.width.min(64),
        area.height.min(items as u16 + 3),
    )
}
fn header_buttons(
    rect: Rect,
    editing: bool,
    markdown: bool,
    reading: bool,
) -> Vec<(Rect, std::borrow::Cow<'static, str>, Command)> {
    if editing {
        return [
            (crate::i18n::t("button.symbol_3"), Command::Close),
            (crate::i18n::t("button.symbol_2"), Command::Save),
            (crate::i18n::t("button.symbol_0"), Command::Project),
        ]
        .into_iter()
        .enumerate()
        .filter(|(i, _)| rect.width > 4 * (*i as u16 + 1))
        .map(|(i, (label, c))| {
            (
                Rect::new(rect.right() - 4 * (i as u16 + 1), rect.y, 3, 1),
                label,
                c,
            )
        })
        .collect();
    }
    let mut buttons = vec![
        (crate::i18n::t("legacy_0"), Command::Close),
        (crate::i18n::t("legacy_1"), Command::More),
    ];
    if markdown && reading {
        buttons.push((crate::i18n::t("legacy_2"), Command::Outline));
    }
    buttons.extend([
        (crate::i18n::t("legacy_3"), Command::Edit),
        (crate::i18n::t("legacy_4"), Command::Forward),
        (crate::i18n::t("legacy_5"), Command::Back),
        (crate::i18n::t("legacy_6"), Command::Maximize),
    ]);
    let mut right = rect.right().saturating_sub(1);
    buttons
        .into_iter()
        .filter_map(|(label, c)| {
            let width = label.width() as u16;
            if right.saturating_sub(rect.x) < width + 16 {
                return None;
            }
            right -= width;
            let r = Rect::new(right, rect.y, width, 1);
            right = right.saturating_sub(1);
            Some((r, label, c))
        })
        .collect()
}
fn visible_header_buttons(
    expanded: bool,
    rect: Rect,
    editing: bool,
    markdown: bool,
    reading: bool,
) -> Vec<(Rect, std::borrow::Cow<'static, str>, Command)> {
    header_buttons(rect, editing, markdown, reading)
        .into_iter()
        .filter(|(_, _, c)| expanded || matches!(c, Command::More | Command::Project))
        .collect()
}
fn draw_header_buttons(
    frame: &mut Frame,
    expanded: bool,
    rect: Rect,
    editing: bool,
    markdown: bool,
    reading: bool,
) {
    for (r, label, _) in visible_header_buttons(expanded, rect, editing, markdown, reading) {
        frame.render_widget(Paragraph::new(label).style(Style::new().fg(Color::Cyan)), r);
    }
}
fn border(focused: bool) -> Style {
    Style::new().fg(if focused {
        crate::appearance::palette().accent
    } else {
        crate::appearance::palette().muted
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
    fn translated_header_hit_regions_use_terminal_cell_width() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                for width in [48, 80, 120, 180] {
                    let buttons = header_buttons(Rect::new(0, 0, width, 20), false, true, true);
                    for (rect, label, _) in &buttons {
                        assert_eq!(rect.width as usize, label.width());
                        assert!(rect.right() <= width);
                    }
                    for (i, (a, _, _)) in buttons.iter().enumerate() {
                        for (b, _, _) in buttons.iter().skip(i + 1) {
                            assert_eq!(a.intersection(*b).width, 0);
                        }
                    }
                }
            });
        }
    }
    #[test]
    fn marked_last_tree_entry_does_not_fill_empty_rows() {
        let root = std::env::temp_dir().join("DevTabs-tree-empty-hit-fixture");
        let mut s = Sidebar::new(root.clone());
        let path = root.join("last.txt");
        s.files.nodes.insert(
            path.clone(),
            files::Node {
                path: path.clone(),
                name: "last.txt".into(),
                directory: false,
                link: false,
                icon: 'T',
                color: Color::White,
            },
        );
        let mut dir = files::Directory::default();
        dir.children = vec![path.clone()];
        dir.complete = true;
        s.files.dirs.insert(root.clone(), dir);
        s.files.selected_paths.insert(path.clone());
        s.files.state.select(s.files.identifier(&path));
        let first = draw(&mut s, 160);
        let inner = crate::ui::content(s.geometry.tree.unwrap());
        let marks = |b: &Buffer| {
            (inner.y..inner.bottom())
                .filter(|y| b[(inner.x + 1, *y)].symbol() == "*")
                .collect::<Vec<_>>()
        };
        assert_eq!(
            marks(&first),
            vec![inner.y + 1],
            "only the selected file's actual row is marked"
        );
        for y in inner.y + 2..inner.bottom() {
            assert!(
                s.tree_hit((inner.x + 5, y)).is_none(),
                "blank row {y} must have no file target"
            );
        }
        assert_eq!(
            marks(&draw(&mut s, 160)),
            vec![inner.y + 1],
            "cached render preserves the blank tail"
        );
        s.files.selected_paths.clear();
        assert!(marks(&draw(&mut s, 160)).is_empty());
        s.files.state.select(s.files.identifier(&root));
        s.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner.x + 5,
            row: inner.bottom() - 1,
            modifiers: KeyModifiers::NONE,
        })
        .unwrap();
        assert_eq!(
            s.files.selected_path(),
            Some(root),
            "blank rows must not select the last file"
        );
        assert!(
            matches!(s.gesture, Gesture::Idle),
            "blank rows must not begin a file drag"
        );
        s.files.selected_paths.insert(s.files.root.clone());
        s.files.state.close_all();
        assert_eq!(
            marks(&draw(&mut s, 160)),
            vec![inner.y],
            "a marked collapsed directory occupies one row"
        );
    }
    #[test]
    fn folder_icons_follow_expansion_without_rebuilding_labels() {
        let root = std::env::temp_dir().join("DevTabs-folder-icons-fixture");
        let mut s = Sidebar::new(root.clone());
        s.files.compact = true;
        let parent = root.join("parent");
        let nested = parent.join("nested");
        let file = nested.join("file.rs");
        for (path, directory) in [(&parent, true), (&nested, true), (&file, false)] {
            s.files.nodes.insert(
                path.clone(),
                files::Node {
                    path: path.clone(),
                    name: path.file_name().unwrap().to_string_lossy().into(),
                    directory,
                    link: false,
                    icon: if directory { '\u{f115}' } else { 'R' },
                    color: Color::Blue,
                },
            );
        }
        for (path, child) in [(&root, &parent), (&parent, &nested), (&nested, &file)] {
            let mut directory = files::Directory::default();
            directory.children = vec![child.clone()];
            directory.complete = true;
            s.files.dirs.insert(path.clone(), directory);
        }
        let icon = |s: &Sidebar, b: &Buffer, path: &PathBuf| {
            let view = s.tree_view.as_ref().unwrap();
            let y = (view.hit_area.y..view.hit_area.bottom())
                .find(|y| {
                    s.files
                        .state
                        .rendered_at((view.hit_area.x, *y).into())
                        .and_then(|id| id.last())
                        == Some(&Key::Entry(path.clone()))
                })
                .unwrap_or_else(|| panic!("missing folder row {path:?}"));
            (view.hit_area.x..view.hit_area.right())
                .find_map(|x| {
                    let symbol = b[(x, y)].symbol();
                    matches!(symbol, "\u{f114}" | "\u{f115}").then(|| symbol.to_string())
                })
                .unwrap()
        };
        let first = draw(&mut s, 180);
        assert_eq!(icon(&s, &first, &root), "\u{f115}");
        assert_eq!(
            icon(&s, &first, &nested),
            "\u{f114}",
            "collapsed compact folder must look closed"
        );
        let labels = s.tree_view.as_ref().unwrap().items.as_ptr();
        let id = s.files.identifier(&nested);
        s.files.state.open(id.clone());
        let expanded = draw(&mut s, 180);
        assert_eq!(icon(&s, &expanded, &nested), "\u{f115}");
        assert_eq!(s.tree_view.as_ref().unwrap().items.as_ptr(), labels);
        assert_eq!(
            expanded,
            draw(&mut s, 180),
            "cached redraw must preserve open icon"
        );
        s.files.state.close(&id);
        let collapsed = draw(&mut s, 180);
        assert_eq!(icon(&s, &collapsed, &nested), "\u{f114}");
        assert_eq!(s.tree_view.as_ref().unwrap().items.as_ptr(), labels);
        s.files.state.close_all();
        let collapsed = draw(&mut s, 180);
        assert_eq!(icon(&s, &collapsed, &root), "\u{f114}");
        s.files.icons = false;
        let plain = draw(&mut s, 180);
        assert!(
            plain
                .content
                .iter()
                .all(|c| !matches!(c.symbol(), "\u{f114}" | "\u{f115}"))
        );
        s.files.icons = true;
        s.files.light = true;
        compare(&mut s, 180);
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
        d.set_rendered(Some(preview::render(
            "aaa e\u{301} 中文 😀",
            &d.path,
            false,
            100,
            false,
        )));
        d.search = "aa".into();
        d.search();
        d.selection = Some(((0, 5), (0, 6)));
        s.document = Some(d);
        s.preview_visible = true;
        let buffer = draw(&mut s, 100);
        let rect = s.geometry.preview.unwrap();
        let inner = crate::ui::preview_content(rect);
        let x = inner.x + 6;
        let y = inner.y;
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
    #[test]
    fn enlarging_keeps_manual_widths_and_only_restores_the_original_geometry() {
        let mut s = Sidebar::new(std::env::temp_dir());
        s.visible = true;
        s.explorer.preferences.width = 37;
        s.explorer.preferences.agent_ratio = 6500;
        let mut d = Document::new(PathBuf::from("manual.md"), 1);
        d.set_rendered(Some(preview::render(
            "# Heading\nBody",
            &d.path,
            true,
            80,
            false,
        )));
        s.document = Some(d);
        s.preview_visible = true;
        s.update_layout(Rect::new(0, 0, 240, 40), false);
        let original = s.geometry;
        assert_eq!(original.tree.unwrap().width, 37);
        for target in [Focus::Tree, Focus::Preview, Focus::Terminal] {
            s.enlarged = Some(target);
            s.update_layout(Rect::new(0, 0, 240, 40), false);
            let visible = [
                s.geometry.tree,
                s.geometry.preview,
                s.geometry.agent,
                s.geometry.debug,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            assert_eq!(visible, vec![Rect::new(0, 0, 240, 39)]);
            s.enlarged = None;
            s.update_layout(Rect::new(0, 0, 240, 40), false);
            assert_eq!(s.geometry, original);
            assert_eq!(
                (
                    s.explorer.preferences.width,
                    s.explorer.preferences.agent_ratio
                ),
                (37, 6500)
            );
        }
    }
}
