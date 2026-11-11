pub(crate) mod chrome;
pub(crate) mod parallel;
pub(crate) mod popup;
pub(crate) mod terminals;
use std::{
    collections::HashSet,
    io,
    path::Path,
    time::{Duration, Instant},
};

use crate::image_output::Terminal as DefaultTerminal;
use anyhow::{Context, Result};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};

use crate::{
    input,
    launch::{Layout, Options},
    selection::{Point, Selection},
    session::Client,
    ui,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Prefix,
    Confirm,
}

pub struct SelectionView {
    pub pane: usize,
    pub screen: vt100::Screen,
    pub range: Option<Selection>,
}

pub struct App {
    pub(crate) parallel: parallel::State,
    pub options: Options,
    pub sessions: Vec<Client>,
    pub workspace_id: String,
    pub focus: usize,
    pub mode: Mode,
    pub selection: Option<SelectionView>,
    pub message: String,
    message_seen: Option<(String, Instant)>,
    chrome_capture: bool,
    pub quit_requested: Option<Instant>,
    force_exit: bool,
    pub view: Option<String>,
    pub return_overview: bool,
    pub sidebar: Option<crate::sidebar::Sidebar>,
    path_receipt: Option<(String, std::sync::mpsc::Receiver<Result<()>>)>,
    last_target: Instant,
    consumed_keys: HashSet<KeyCode>,
    size: ratatui::layout::Rect,
    pub(crate) popup: Option<usize>,
    pub(crate) popup_window: popup::Window,
    floating_first_frame: Option<u64>,
    terminal_menu: Option<usize>,
    pub(crate) hover: Option<(u16, u16)>,
    pub(crate) zoom: Option<chrome::Zoom>,
    terminal_hits: Vec<(ratatui::layout::Rect, terminals::Action)>,
}

impl App {
    pub fn new(options: Options, _executable: &Path, size: ratatui::layout::Rect) -> Result<Self> {
        let navigation = options.navigation.clone();
        let image_protocol = options.image_protocol;
        let image_cell_size = options.image_cell_size;
        let named_main = options.view.as_deref() == Some("main");
        crate::host::ensure()?;
        anyhow::ensure!(
            options.view.as_deref() != Some("floating"),
            "Floating terminals are now in-app panels; open the main workspace instead"
        );
        let info = match crate::protocol::rpc(crate::protocol::Request::Open {
            options: Box::new(options.clone()),
        })? {
            crate::protocol::Response::Workspace(info) => info,
            _ => anyhow::bail!("Session host returned no workspace"),
        };
        let auto_debug = info.options.layout == Layout::Float
            && info.sessions.iter().any(|s| {
                s.task.role == crate::tasks::Role::Debug
                    && s.placement == crate::presentation::Placement::Docked
            });
        let sessions = info
            .sessions
            .into_iter()
            .map(Client::hidden)
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            anyhow::bail!("Session host did not create an Agent session");
        }
        let view = options.view.clone().filter(|v| v != "main");
        let mut options = info.options;
        options.view = view.clone();
        options.image_protocol = image_protocol;
        options.image_cell_size = image_cell_size;
        let mut app = Self {
            parallel: parallel::State {
                tasks: info.tasks,
                ..Default::default()
            },
            options,
            sessions,
            workspace_id: info.id,
            view,
            return_overview: false,
            sidebar: None,
            path_receipt: None,
            last_target: Instant::now(),
            focus: 0,
            mode: Mode::Normal,
            selection: None,
            message: String::new(),
            message_seen: None,
            chrome_capture: false,
            quit_requested: None,
            force_exit: false,
            consumed_keys: HashSet::new(),
            size,
            popup: None,
            popup_window: popup::Window::load(),
            floating_first_frame: None,
            terminal_menu: None,
            terminal_hits: vec![],
            hover: None,
            zoom: None,
        };
        if let Some(target) = &navigation {
            anyhow::ensure!(
                target.workspace == app.workspace_id,
                "Navigation workspace mismatch"
            );
            app.focus = app
                .sessions
                .iter()
                .position(|s| s.info.id == target.session && s.info.task.task_id == target.task)
                .context("Requested task/session is no longer available")?;
            app.options.navigation = Some(target.clone());
        }
        if app.view.as_deref() == Some("debug") {
            app.focus = app
                .role_index(crate::tasks::Role::Debug)
                .context("No Debug terminal")?;
        }
        if app.view.is_none() {
            use std::io::Read;
            let path = crate::config::home()
                .join("layouts")
                .join(format!("{}.json", app.workspace_id));
            if let Ok(file) = std::fs::File::open(path) {
                let mut bytes = vec![];
                file.take(65537).read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 65536, "Task layout exceeds 64 KiB");
                app.parallel.layout = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(
                    app.parallel.layout.pair.len() <= 2
                        && app
                            .parallel
                            .layout
                            .pair
                            .iter()
                            .collect::<HashSet<_>>()
                            .len()
                            == app.parallel.layout.pair.len(),
                    "Invalid pinned task identities"
                );
                if navigation.is_none()
                    && let Some(saved) = &app.parallel.layout.focus
                    && let Some(i) = app.sessions.iter().position(|s| s.info.id == *saved)
                {
                    app.focus = i;
                } else if navigation.is_none()
                    && let Some(task) = &app.parallel.layout.focused_task
                    && let Some(i) = app.sessions.iter().position(|s| {
                        s.info.task.task_id == *task
                            && s.info.task.role == crate::tasks::Role::Agent
                    })
                {
                    app.focus = i;
                }
            }
            app.sidebar = Some(crate::sidebar::Sidebar::new(
                app.sessions[app.focus].info.task.initial_cwd.clone(),
            ));
            app.parallel.context = app.sessions[app.focus].info.task.task_id.clone();
        }
        if let Some(view) = &app.view
            && let Some(index) = view.strip_prefix("session:")
        {
            app.focus = index.parse()?;
        }
        if let Some(view) = &app.view
            && let Some(id) = view.strip_prefix("session-id:")
        {
            app.focus = app
                .sessions
                .iter()
                .position(|s| s.info.id == id)
                .context("Requested terminal no longer exists")?;
        }
        if app.view.as_deref() == Some("debug")
            && (app.options.layout != Layout::Tabs || !app.docked(app.focus))
        {
            app.force_exit = true;
            return Ok(app);
        }
        if app.view.is_none()
            && !app.docked(app.focus)
            && let Some(i) = app
                .sessions
                .iter()
                .position(|s| s.info.placement == crate::presentation::Placement::Docked)
        {
            app.focus = i;
        }
        app.attach_visible()?;
        if navigation.is_some() {
            let index = app.focus;
            app.focus(index);
        }
        execute!(
            io::stdout(),
            crossterm::terminal::SetTitle(format!(
                "DevTabs · {} · {}",
                app.project_name(),
                app.workspace_id
            ))
        )?;
        if app.view.is_none() {
            let first = matches!(
                crate::protocol::rpc(crate::protocol::Request::ViewReady {
                    workspace: app.workspace_id.clone()
                })?,
                crate::protocol::Response::FirstView(true)
            );
            match app.options.layout {
                Layout::Float if auto_debug && first => {
                    if let Err(error) = app.show_default_debug() {
                        app.message =
                            format!("Debug window: {error:#}; Ctrl+G L switches to split");
                    }
                }
                Layout::Tabs if app.sessions.len() > 1 && app.agents().len() < 2 => {
                    if !named_main {
                        app.detach();
                        crate::windows::spawn_view(&app.workspace_id, "main", false)?;
                        return Ok(app);
                    }
                    if let Err(error) =
                        crate::windows::spawn_view(&app.workspace_id, "debug", false)
                    {
                        app.message = format!("Debug tab: {error:#}");
                    }
                }
                Layout::Split | Layout::Float | Layout::Tabs => {}
            }
        }
        Ok(app)
    }
    pub fn sidebar_active(&self) -> bool {
        self.sidebar.is_some()
    }
    pub fn overview_options(&self) -> Options {
        let mut options = self.options.clone();
        let session = &self.sessions[self.focus].info;
        options.navigation = Some(Box::new(crate::tasks::Navigation {
            project: options.project.clone(),
            workspace: self.workspace_id.clone(),
            task: session.task.task_id.clone(),
            session: session.id.clone(),
        }));
        options.workspace = Some(self.workspace_id.clone());
        options
    }
    pub(crate) fn update_image_occlusion(&mut self) {
        let cover = self
            .popup
            .filter(|_| self.zoom.is_none())
            .map(|_| chrome::cover(self.popup_window.rect(self.size), self.size));
        let active = self.sidebar_active();
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.images_occluded = self.mode != Mode::Normal
                || self.terminal_menu.is_some()
                || self.parallel.dialog.is_some()
                || !active;
            sidebar.image_occlusion = cover;
        }
    }
    fn update_sidebar_layout(&mut self) -> bool {
        // Leaving or closing the enlarged target restores the saved normal layout.
        let keep_zoom = match self.zoom {
            Some(chrome::Zoom::Tree) => self
                .sidebar
                .as_ref()
                .is_some_and(|s| s.visible && s.focus == crate::sidebar::Focus::Tree),
            Some(chrome::Zoom::Preview) => self.sidebar.as_ref().is_some_and(|s| {
                s.showing()
                    && matches!(
                        s.focus,
                        crate::sidebar::Focus::Preview | crate::sidebar::Focus::Editor
                    )
            }),
            Some(chrome::Zoom::Terminal(i)) => {
                self.focus == i
                    && self
                        .sidebar
                        .as_ref()
                        .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal)
            }
            None => true,
        };
        if !keep_zoom {
            self.zoom = None;
        }
        let active = self.sidebar_active();
        let split = self.options.layout == Layout::Split
            && self.agents().len() < 2
            && self
                .role_index(crate::tasks::Role::Agent)
                .is_some_and(|i| self.docked(i))
            && self.secondary_index().is_some();
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.enlarged = match self.zoom {
                Some(chrome::Zoom::Tree) => Some(crate::sidebar::Focus::Tree),
                Some(chrome::Zoom::Preview) => Some(crate::sidebar::Focus::Preview),
                Some(chrome::Zoom::Terminal(_)) => Some(crate::sidebar::Focus::Terminal),
                None => None,
            };
            sidebar.update_layout(
                if active {
                    ratatui::layout::Rect {
                        height: self.size.height,
                        ..self.size
                    }
                } else {
                    ratatui::layout::Rect::default()
                },
                split,
            )
        } else {
            false
        }
    }
    fn docked_rects(&self) -> Vec<(usize, ratatui::layout::Rect)> {
        if self.size.width < 60 || self.size.height < 10 {
            return vec![];
        }
        if let Some(chrome::Zoom::Terminal(i)) = self.zoom {
            return vec![(i, chrome::body(self.size))];
        }
        if let Some(rects) = self.parallel_rects() {
            return rects;
        }
        let agent = self
            .role_index(crate::tasks::Role::Agent)
            .unwrap_or(self.focus);
        let secondary = self.secondary_index().unwrap_or(agent);
        let primary = if self.options.layout == Layout::Split
            && self.view.is_none()
            && self.docked(agent)
            && secondary != agent
        {
            agent
        } else {
            if self.docked(self.focus) {
                self.focus
            } else {
                self.sessions
                    .iter()
                    .position(|s| s.info.placement == crate::presentation::Placement::Docked)
                    .unwrap_or(0)
            }
        };
        let mut rects = if self.sidebar_active()
            && let Some(sidebar) = &self.sidebar
        {
            let p = sidebar.geometry;
            let mut out = vec![];
            if !p.overlay
                && let Some(r) = p.agent
            {
                out.push((primary, r));
            }
            if let Some(r) = p.debug {
                out.push((secondary, r));
            }
            out
        } else if self.options.layout == Layout::Split
            && self.view.is_none()
            && self.docked(agent)
            && secondary != agent
            && self.docked(secondary)
        {
            ui::panes(ratatui::layout::Rect {
                height: self.size.height,
                ..self.size
            })
            .map(|r| vec![(agent, r[0]), (secondary, r[1])])
            .unwrap_or_default()
        } else {
            vec![(
                primary,
                ratatui::layout::Rect::new(
                    0,
                    0,
                    self.size.width,
                    self.size.height.saturating_sub(1),
                ),
            )]
        };
        rects.retain(|(i, _)| self.docked(*i));
        rects
    }
    pub fn rects(&self) -> Vec<(usize, ratatui::layout::Rect)> {
        let mut rects = self.docked_rects();
        if let Some(i) = self.popup
            && self.zoom.is_none()
            && self.size.width >= 60
            && self.size.height >= 10
        {
            rects.push((i, self.popup_window.rect(self.size)));
        }
        rects
    }
    fn attach_visible(&mut self) -> Result<()> {
        if self.parallel.restore_popup
            && let Some(i) = self.popup
        {
            self.parallel.restore_popup = false;
            self.popup = None;
            let focus = self.focus;
            let result = self.show_popup(i);
            self.focus = focus;
            result?;
        }
        self.update_sidebar_layout();
        let mut rects = self.rects();
        let overlay = self.sidebar.as_ref().is_some_and(|s| s.geometry.overlay);
        let overlay_agent = self
            .role_index(crate::tasks::Role::Agent)
            .unwrap_or(self.focus);
        if overlay
            && self.zoom.is_none()
            && self.docked(overlay_agent)
            && let Some(r) = self.sidebar.as_ref().and_then(|s| s.geometry.agent)
        {
            rects.push((overlay_agent, r));
        }
        // A minimized terminal has no drawable panes. Keep existing connections
        // and PTY dimensions until it becomes usable again.
        if self.size.width < 60
            || self.size.height < 10
            || (rects.is_empty()
                && self
                    .sessions
                    .iter()
                    .any(|s| s.info.placement == crate::presentation::Placement::Docked))
        {
            return Ok(());
        }
        for (index, session) in self.sessions.iter_mut().enumerate() {
            if !rects.iter().any(|(i, _)| *i == index)
                && !(self.zoom.is_some() && self.popup == Some(index))
            {
                session.detach();
            }
        }
        for (index, rect) in rects {
            if index == overlay_agent && overlay && self.sessions[index].attached() {
                continue;
            }
            let r = ui::content(rect);
            if self.popup == Some(index) && !self.sessions[index].attached() {
                continue;
            }
            if let Err(error) = self
                .sessions
                .get_mut(index)
                .context("Unknown session")?
                .attach_view(
                    &self.workspace_id,
                    r.height,
                    r.width,
                    self.view.is_none(),
                    None,
                )
            {
                if error
                    .to_string()
                    .contains("Session is collapsed or displayed")
                {
                    self.sessions[index].detach();
                } else {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    fn layout(&mut self) -> Result<()> {
        let layout = match self.options.layout {
            Layout::Float => Layout::Split,
            Layout::Split => Layout::Tabs,
            Layout::Tabs => Layout::Float,
        };
        crate::protocol::rpc(crate::protocol::Request::Layout {
            workspace: self.workspace_id.clone(),
            layout,
        })?;
        self.options.layout = layout;
        if self.agents().len() > 1 {
            self.attach_visible()?;
            return Ok(());
        }
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if self.popup != Some(i) {
                session.detach();
            }
        }
        self.focus = self
            .sessions
            .iter()
            .position(|s| s.info.placement == crate::presentation::Placement::Docked)
            .unwrap_or(0);
        self.selection = None;
        self.attach_visible()?;
        match layout {
            Layout::Float => self.show_default_debug()?,
            Layout::Tabs if self.popup.is_none() && self.sessions.len() > 1 => {
                if self.sidebar.as_ref().is_some_and(|s| s.document.is_some()) {
                    crate::windows::spawn_view(&self.workspace_id, "debug", false)?;
                } else {
                    self.detach();
                    crate::windows::spawn_view(&self.workspace_id, "main", false)?;
                }
            }
            Layout::Split | Layout::Tabs => {}
        }
        Ok(())
    }
    fn show_default_debug(&mut self) -> Result<()> {
        // Explicit placement wins over layout defaults, including a target in
        // another project and an intentional collapse of this project's Debug.
        let Some(debug) = self.role_index(crate::tasks::Role::Debug) else {
            return Ok(());
        };
        if self.docked(debug)
            && matches!(
                crate::protocol::rpc(crate::protocol::Request::FloatingTarget)?,
                crate::protocol::Response::Floating(None)
            )
        {
            self.show_popup(debug)?;
        }
        Ok(())
    }
    fn detach(&mut self) {
        if let Some(task) = self
            .parallel
            .sidebars
            .iter()
            .find(|(_, s)| s.dirty())
            .map(|(id, _)| id.clone())
            && let Some(index) = self
                .sessions
                .iter()
                .position(|s| s.info.task.task_id == task)
        {
            self.switch_context(index);
            self.focus = index;
        }
        if let Some(s) = &mut self.sidebar
            && s.dirty()
        {
            s.leave('x');
            s.say(
                "Sessions disconnected; save, discard or cancel before leaving this buffer",
                false,
            );
            return;
        }
        for session in &mut self.sessions {
            session.detach();
        }
        self.force_exit = true;
    }
    pub fn project_name(&self) -> String {
        if let Some(title) = &self.options.title {
            return title.clone();
        }
        self.options
            .project
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    fn send(&mut self, index: usize, bytes: Vec<u8>) {
        if let Err(error) = self.sessions[index].send(bytes) {
            self.message = error.to_string();
        }
    }
    fn focus(&mut self, index: usize) {
        if self.zoom.is_some() && self.zoom != Some(chrome::Zoom::Terminal(index)) {
            self.zoom = None;
        }
        if index >= self.sessions.len() {
            self.message = crate::i18n::t("overview.no_debug").into_owned();
            return;
        }
        self.switch_context(index);
        self.pin_selected_task(index);
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.focus_agent();
        }
        if !self.docked(index) {
            let result = self.show_popup(index);
            if let Err(error) = result {
                self.message = format!("{error:#}");
            }
            return;
        }
        if self.view.is_some() && self.sessions[index].info.task.role == crate::tasks::Role::Agent {
            if let Err(error) = crate::windows::focus_workspace(&self.workspace_id) {
                self.message = format!("{error:#}");
            }
            return;
        }
        if self.sessions[index].info.task.role == crate::tasks::Role::Debug
            && self.options.layout == Layout::Tabs
            && self.view.is_none()
        {
            let view = format!("session-id:{}", self.sessions[index].info.id);
            if let Err(error) = crate::windows::spawn_view(&self.workspace_id, &view, false) {
                self.message = format!("{error:#}");
            }
            return;
        }
        if self.agents().len() > 1
            && self.sessions[index].info.task.role != crate::tasks::Role::Agent
            && self.options.layout == Layout::Float
        {
            if let Err(e) = self.show_popup(index) {
                self.message = format!("{e:#}");
            }
            return;
        }
        if self.focus == index {
            let _ = self.attach_visible();
            return;
        }
        let report = self.sessions[self.focus]
            .state()
            .terminal
            .parser
            .callbacks()
            .focus_events;
        if report {
            self.send(self.focus, b"\x1b[O".to_vec());
        }
        self.focus = index;
        self.parallel.layout.focus = Some(self.sessions[index].info.id.clone());
        self.parallel.layout.focused_task = Some(self.sessions[index].info.task.task_id.clone());
        self.save_task_layout();
        self.selection = None;
        if let Err(error) = self.attach_visible() {
            self.message = format!("{error:#}");
        }
        let report = self.sessions[index]
            .state()
            .terminal
            .parser
            .callbacks()
            .focus_events;
        if report {
            self.send(index, b"\x1b[I".to_vec());
        }
    }
    fn toggle_selection(&mut self) {
        if let Err(error) = self.sessions[self.focus].history() {
            self.message = format!("{error:#}");
            return;
        }
        self.selection = if self.selection.is_some() {
            None
        } else {
            Some(SelectionView {
                pane: self.focus,
                screen: self.sessions[self.focus].state().terminal.screen().clone(),
                range: None,
            })
        };
    }
    fn copy(&mut self) -> Result<()> {
        let selection = self
            .selection
            .as_ref()
            .context("Ctrl+G S enters selection mode; drag text before copying")?;
        let range = selection.range.context("Drag text to select it first")?;
        let text = range.text(&selection.screen);
        let mut clipboard = arboard::Clipboard::new().context("Open Windows clipboard")?;
        clipboard.set_text(text).context("Copy selected text")?;
        self.message = crate::i18n::t("ui.copied_selection_ctrl_g_s_returns_to_live_output").into();
        Ok(())
    }
    fn key(&mut self, key: KeyEvent, native: Option<Vec<u8>>, repeat: usize) {
        let pressed = key.kind != KeyEventKind::Release;
        if !pressed && self.consumed_keys.remove(&key.code) {
            return;
        }
        if self.parallel.dialog.is_some() {
            self.consumed_keys.insert(key.code);
            self.task_key(key);
            return;
        }
        if self.parallel.drag && key.code == KeyCode::Esc {
            self.cancel_task_drag();
            let _ = self.attach_visible();
            return;
        }
        if self.terminal_menu.is_some() {
            if pressed {
                self.consumed_keys.insert(key.code);
                self.terminal_key(key);
            }
            return;
        }
        let prefix =
            key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL);
        if self.mode == Mode::Prefix {
            if !pressed {
                return;
            }
            self.mode = Mode::Normal;
            if let KeyCode::Char(c) = key.code
                && matches!(c, 'x' | 'X' | 'p' | 'P' | 'q' | 'Q')
                && self.sidebar.as_mut().is_some_and(|s| !s.leave(c))
            {
                return;
            }
            if !prefix {
                self.consumed_keys.insert(key.code);
            }
            match key.code {
                KeyCode::Char('z' | 'Z') => self.toggle_zoom(),
                KeyCode::Char('w' | 'W') => self.terminal_menu = Some(self.focus),
                KeyCode::Char('+') => self.new_task(),
                KeyCode::Char('c' | 'C') => self.toggle_task_pins(),
                KeyCode::Char('e' | 'E') => self.replace_pinned_task(),
                KeyCode::Char('[') => self.select_task(-1),
                KeyCode::Char(']') => self.select_task(1),
                KeyCode::Char('v' | 'V') => {
                    self.parallel.layout.axis = match self.parallel.layout.axis {
                        parallel::Axis::Auto | parallel::Axis::Stack => parallel::Axis::Sides,
                        parallel::Axis::Sides => parallel::Axis::Stack,
                    };
                    self.save_task_layout();
                    let _ = self.attach_visible();
                }
                KeyCode::Char(c @ '1'..='9') => {
                    if let Some(&i) = self.agents().get(c as usize - '1' as usize) {
                        self.focus(i);
                    }
                }
                KeyCode::Char('a' | 'A') => {
                    if let Some(i) = self.role_index(crate::tasks::Role::Agent) {
                        self.focus(i);
                    }
                }
                KeyCode::Char('u' | 'U') => {
                    if let Some(sidebar) = &mut self.sidebar
                        && let Err(error) = sidebar.cycle_language()
                    {
                        self.message = format!("{error:#}")
                    }
                }
                KeyCode::Char('d' | 'D') => {
                    if let Some(i) = self.role_index(crate::tasks::Role::Debug) {
                        self.focus(i);
                    } else {
                        self.start_task_role(crate::tasks::Role::Debug, true);
                    }
                }
                KeyCode::Char('j' | 'J') => self.start_task_role(crate::tasks::Role::Agent, true),
                KeyCode::Char('h' | 'H') => self.start_task_role(crate::tasks::Role::Agent, false),
                KeyCode::Char('l' | 'L') => {
                    if let Err(error) = self.layout() {
                        self.message = format!("{error:#}");
                    }
                }
                KeyCode::Char('x' | 'X') => self.detach(),
                KeyCode::Char('p' | 'P') => {
                    self.return_overview = true;
                    self.detach();
                }
                KeyCode::Char('t' | 'T') => {
                    self.popup_window.edit = Some(popup::Edit::Move);
                }
                KeyCode::Char('n' | 'N') => {
                    let mut next = (self.focus + 1) % self.sessions.len();
                    if next == 1
                        && self.view.is_none()
                        && self.options.layout != Layout::Split
                        && self.sessions.len() > 2
                    {
                        next = 2;
                    }
                    self.focus(next);
                }
                KeyCode::Char('k' | 'K') => {
                    if let Err(error) = self.sessions[self.focus].action(true, false) {
                        self.message = format!("{error:#}");
                    } else {
                        self.message = crate::i18n::t(
                            "ui.interrupt_requested_ctrl_g_shift_f_can_force_after_3_seconds",
                        )
                        .into();
                    }
                }
                KeyCode::Char('F') => {
                    if let Err(error) = self.sessions[self.focus].action(true, true) {
                        self.message = format!("{error:#}");
                    }
                }
                KeyCode::Char('b' | 'B') => {
                    self.zoom = None;
                    if let Some(s) = &mut self.sidebar {
                        s.toggle();
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('f') => {
                    self.zoom = None;
                    if let Some(s) = &mut self.sidebar {
                        s.focus_tree();
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('o' | 'O') => {
                    self.zoom = None;
                    if let Some(s) = &mut self.sidebar
                        && s.document.is_some()
                    {
                        s.preview_visible = true;
                        s.focus = crate::sidebar::Focus::Preview;
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('i' | 'I') => {
                    let action = self
                        .sidebar
                        .as_ref()
                        .and_then(|s| s.files.selected_file().map(|path| s.insert_selection(path)));
                    if let Some(action) = action {
                        self.sidebar_action(action);
                    }
                }
                KeyCode::Char('r' | 'R') => {
                    if let Err(error) = self.sessions[self.focus].action(false, false) {
                        self.message = format!("{error:#}");
                    }
                }
                KeyCode::Char('s' | 'S') => self.toggle_selection(),
                KeyCode::Char('y' | 'Y') => {
                    if let Err(e) = self.copy() {
                        self.message = e.to_string();
                    }
                }
                KeyCode::Char('q' | 'Q') => {
                    self.mode = Mode::Confirm;
                    self.quit_requested.get_or_insert_with(Instant::now);
                }
                _ if prefix => self.forward_key(key, native, repeat),
                KeyCode::Esc => {}
                _ => {
                    self.message =
                        crate::i18n::t("ui.unknown_devtabs_command_press_ctrl_g_to_see_commands")
                            .into()
                }
            }
            return;
        }
        if self.mode == Mode::Confirm {
            if !pressed {
                return;
            }
            self.consumed_keys.insert(key.code);
            match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    for index in 0..self.sessions.len() {
                        if self.sessions[index].running() {
                            self.send(index, vec![3]);
                        }
                    }
                    self.message=crate::i18n::t("ui.interrupt_requested_for_visible_sessions_finish_programs_and_exit_or_ctrl_g_q_f_closes_every_session_after_3s").into();
                }
                KeyCode::Char('f' | 'F')
                    if self
                        .quit_requested
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(3)) =>
                {
                    if let Err(error) = crate::protocol::rpc(crate::protocol::Request::Close {
                        workspace: self.workspace_id.clone(),
                    }) {
                        self.message = format!("{error:#}");
                        return;
                    }
                    self.force_exit = true;
                }
                _ => {}
            }
            return;
        }
        if prefix {
            if key.kind == KeyEventKind::Press {
                self.mode = Mode::Prefix;
                self.consumed_keys.insert(key.code);
            }
            return;
        }
        if self.popup.is_some() && self.popup_window.edit.is_some() {
            if pressed {
                self.consumed_keys.insert(key.code);
                self.popup_key(key);
            }
            return;
        }
        if let Some(selection) = &mut self.selection {
            if pressed {
                let offset = selection.screen.scrollback();
                match key.code {
                    KeyCode::PageUp => selection.screen.set_scrollback(offset + 10),
                    KeyCode::PageDown => selection.screen.set_scrollback(offset.saturating_sub(10)),
                    KeyCode::Esc => {
                        self.selection = None;
                        return;
                    }
                    _ => return,
                }
                selection.range = None;
            }
            return;
        }
        if self.rects().is_empty() && !self.sidebar.as_ref().is_some_and(|s| s.showing()) {
            return;
        }
        if let Some(sidebar) = &mut self.sidebar
            && (sidebar.focus != crate::sidebar::Focus::Terminal
                || sidebar.modal()
                || (key.code == KeyCode::Char('p') && key.modifiers == KeyModifiers::CONTROL))
        {
            if pressed {
                self.consumed_keys.insert(key.code);
                match sidebar.key(key) {
                    Ok(action) => self.sidebar_action(action),
                    Err(e) => {
                        if let Some(s) = &mut self.sidebar {
                            s.say(format!("{e:#}"), false);
                        }
                    }
                }
                let _ = self.attach_visible();
            }
            return;
        }
        self.forward_key(key, native, repeat);
    }
    fn sidebar_action(&mut self, action: crate::sidebar::Action) {
        match action {
            crate::sidebar::Action::None => {}
            crate::sidebar::Action::Manage(c) => {
                self.mode = Mode::Prefix;
                self.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), None, 1);
            }
            crate::sidebar::Action::Agent => {
                if let Some(i) = self.role_index(crate::tasks::Role::Agent) {
                    self.focus(i);
                }
            }
            crate::sidebar::Action::Insert(paths) => {
                if self.path_receipt.is_some()
                    || self.parallel.input.is_some()
                    || self.sidebar.as_ref().is_some_and(|s| s.checking_input)
                {
                    if let Some(s) = &mut self.sidebar {
                        s.say(
                            crate::i18n::t(
                                "ui.a_path_is_still_being_sent_inspect_agent_before_retrying",
                            ),
                            false,
                        );
                    }
                    return;
                }
                self.parallel.input = self.input_target();
                if let Some(s) = &mut self.sidebar {
                    match s.validate_selected_paths(paths) {
                        Ok(()) => {
                            s.checking_input = true;
                            s.say(
                                crate::i18n::t("ui.checking_file_before_sending_path"),
                                false,
                            );
                        }
                        Err(e) => s.say(e.to_string(), false),
                    }
                }
            }
            crate::sidebar::Action::ValidatedInput(paths) => {
                let result = (|| -> Result<()> {
                    let target = self
                        .parallel
                        .input
                        .take()
                        .context("No pending input target")?;
                    let index = self.resolve_input_target(&target)?;
                    anyhow::ensure!(
                        self.parallel
                            .tasks
                            .iter()
                            .find(|t| t.id == target.task)
                            .map_or(self.options.agent, |t| t.options.agent)
                            != crate::launch::Agent::Shell,
                        crate::i18n::t("ui.shell_is_not_an_agent_input_target")
                    );
                    let session = &self.sessions[index];
                    anyhow::ensure!(
                        session.attached() && session.running(),
                        crate::i18n::t("ui.agent_disconnected_or_exited")
                    );
                    let state = session.state();
                    anyhow::ensure!(
                        state.io_error.is_none(),
                        crate::i18n::t("ui.agent_input_unavailable_inspect_connection")
                    );
                    anyhow::ensure!(
                        state.command == crate::session::CommandStatus::Running,
                        crate::i18n::t("ui.configured_agent_is_not_running")
                    );
                    let bracketed = state.terminal.screen().bracketed_paste();
                    drop(state);
                    let text = crate::explorer::paths_input(&paths)?;
                    self.path_receipt = Some((
                        target.task,
                        session.send_checked(input::paste(&text, bracketed)?)?,
                    ));
                    Ok(())
                })();
                if result.is_ok() {
                    self.selection = None;
                }
                if let Some(s) = &mut self.sidebar {
                    match result {
                        Ok(()) => {
                            s.focus_agent();
                            s.say(crate::i18n::t("ui.sending_path_no_enter_sent"), false);
                        }
                        Err(e) => s.say(format!("Path not sent: {e:#}"), false),
                    }
                }
            }
        }
    }
    fn forward_key(&mut self, key: KeyEvent, native: Option<Vec<u8>>, repeat: usize) {
        if !self.allowed(self.focus) || !self.sessions[self.focus].attached() {
            return;
        }
        let bytes = {
            let state = self.sessions[self.focus].state();
            let win32 = state.terminal.parser.callbacks().win32_input;
            let encoded = input::key(key, state.terminal.screen().application_cursor(), win32);
            if win32 {
                native.unwrap_or(encoded)
            } else {
                encoded.repeat(repeat)
            }
        };
        if !bytes.is_empty() {
            self.sessions[self.focus].live();
            self.send(self.focus, bytes);
        }
    }
    fn mouse(&mut self, event: MouseEvent) {
        if self.task_mouse(event) {
            return;
        }
        if event.kind == MouseEventKind::Moved {
            self.hover = Some((event.column, event.row));
        }
        if let Some(s) = &mut self.sidebar {
            s.hover = self.hover;
        }
        if self.prefix_mouse(event) {
            return;
        }
        let on_popup = self.zoom.is_none()
            && self.popup.is_some()
            && self
                .popup_window
                .rect(self.size)
                .contains((event.column, event.row).into());
        let agent_index = self
            .role_index(crate::tasks::Role::Agent)
            .unwrap_or(self.focus);
        let agent = if on_popup && self.popup != Some(agent_index) {
            None
        } else {
            self.rects()
                .into_iter()
                .find(|(i, _)| *i == agent_index)
                .map(|(_, r)| r)
        };
        // A file drag owns its gesture through release, even over the panel.
        if let Some(sidebar) = &mut self.sidebar
            && !matches!(sidebar.gesture, crate::sidebar::Gesture::Idle)
        {
            match sidebar.mouse_with_agent(event, agent) {
                Ok((true, action)) => {
                    self.sidebar_action(action);
                    let _ = self.attach_visible();
                    return;
                }
                Err(e) => {
                    sidebar.say(format!("{e:#}"), false);
                    return;
                }
                _ => {}
            }
        }
        // Divider capture continues across terminal panes and floating panels.
        if let Some(sidebar) = &mut self.sidebar
            && sidebar.resizing()
        {
            if let Err(error) = sidebar.mouse_with_agent(event, agent) {
                sidebar.say(format!("{error:#}"), false);
            }
            let _ = self.attach_visible();
            return;
        }
        if self.terminal_mouse(event) {
            return;
        }
        let on_popup = self.zoom.is_none()
            && self.popup.is_some()
            && self
                .popup_window
                .rect(self.size)
                .contains((event.column, event.row).into());
        if !on_popup && let Some(sidebar) = &mut self.sidebar {
            match sidebar.mouse_with_agent(event, agent) {
                Ok((true, action)) => {
                    self.sidebar_action(action);
                    let _ = self.attach_visible();
                    return;
                }
                Err(e) => {
                    sidebar.say(format!("{e:#}"), false);
                    return;
                }
                _ => {}
            }
        }
        let rects = self.rects();
        let Some((index, rect)) = rects
            .into_iter()
            .rev()
            .map(|(i, r)| (i, ui::content(r)))
            .find(|(_, r)| r.contains((event.column, event.row).into()))
        else {
            return;
        };
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.focus(index);
        }
        if index != self.focus {
            return;
        }
        let point = Point {
            row: event.row - rect.y,
            col: event.column - rect.x,
        };
        if let Some(selection) = &mut self.selection {
            if selection.pane != index {
                return;
            }
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    selection.range = Some(Selection {
                        start: point,
                        end: point,
                    })
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    if let Some(range) = &mut selection.range {
                        range.end = point;
                    }
                }
                MouseEventKind::ScrollUp => {
                    selection
                        .screen
                        .set_scrollback(selection.screen.scrollback() + 3);
                    selection.range = None;
                }
                MouseEventKind::ScrollDown => {
                    selection
                        .screen
                        .set_scrollback(selection.screen.scrollback().saturating_sub(3));
                    selection.range = None;
                }
                _ => {}
            }
            return;
        }
        let (bytes, mouse_enabled) = {
            let state = self.sessions[index].state();
            (
                input::mouse(event, point.col, point.row, state.terminal.screen()),
                state.terminal.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None,
            )
        };
        if let Some(bytes) = bytes {
            self.send(index, bytes);
        } else if !mouse_enabled {
            match event.kind {
                MouseEventKind::ScrollUp => self.sessions[index].scroll(3),
                MouseEventKind::ScrollDown => self.sessions[index].scroll(-3),
                _ => {}
            }
        }
    }
    fn event(&mut self, input: crate::events::Input) -> Result<()> {
        let _perf = crate::perf::span(crate::perf::Metric::Input);
        let was_terminal = self
            .sidebar
            .as_ref()
            .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal);
        let was_index = self.focus;
        let event = input.event;
        match event {
            Event::Paste(text) if self.parallel.dialog.is_some() => self.task_paste(&text),
            Event::Key(key) => self.key(key, input.win32, input.repeat),
            Event::Mouse(mouse) if self.mode != Mode::Confirm => self.mouse(mouse),
            Event::Paste(text)
                if self.mode == Mode::Normal
                    && self.selection.is_none()
                    && self.terminal_menu.is_none()
                    && self.parallel.dialog.is_none()
                    && !self.popup_window.editing() =>
            {
                if let Some(s) = &mut self.sidebar
                    && (s.focus != crate::sidebar::Focus::Terminal || s.modal())
                {
                    s.paste(&text)?;
                    return Ok(());
                }
                if !self.allowed(self.focus)
                    || !self.sessions[self.focus].attached()
                    || !self.rects().iter().any(|(i, _)| *i == self.focus)
                {
                    return Ok(());
                }
                let bracketed = self.sessions[self.focus]
                    .state()
                    .terminal
                    .screen()
                    .bracketed_paste();
                let bytes = input::paste(&text, bracketed)?;
                self.sessions[self.focus].live();
                self.send(self.focus, bytes);
            }
            Event::Resize(width, height) => {
                self.cancel_task_drag();
                if let Some(s) = &mut self.sidebar {
                    s.cancel_drag(true);
                }
                self.popup_window.cancel_drag();
                self.chrome_capture = false;
                self.size = ratatui::layout::Rect::new(0, 0, width, height);
                self.selection = None;
                self.attach_visible()?;
            }
            Event::FocusGained | Event::FocusLost => {
                if event == Event::FocusLost {
                    self.cancel_task_drag();
                    self.chrome_capture = false;
                    self.hover = None;
                    if let Some(s) = &mut self.sidebar {
                        s.hover = None;
                    }
                }
                if event == Event::FocusLost {
                    self.popup_window.cancel_drag();
                }
                if event == Event::FocusLost
                    && let Some(s) = &mut self.sidebar
                {
                    s.cancel_drag(true);
                }
                let report = self.sessions[self.focus]
                    .state()
                    .terminal
                    .parser
                    .callbacks()
                    .focus_events;
                if report
                    && self
                        .sidebar
                        .as_ref()
                        .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal)
                {
                    self.send(
                        self.focus,
                        if event == Event::FocusGained {
                            b"\x1b[I".to_vec()
                        } else {
                            b"\x1b[O".to_vec()
                        },
                    );
                }
            }
            _ => {}
        }
        let is_terminal = self
            .sidebar
            .as_ref()
            .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal);
        if was_index == self.focus
            && was_terminal != is_terminal
            && self.sessions[self.focus].attached()
            && self.sessions[self.focus]
                .state()
                .terminal
                .parser
                .callbacks()
                .focus_events
        {
            self.send(
                self.focus,
                if is_terminal {
                    b"\x1b[I".to_vec()
                } else {
                    b"\x1b[O".to_vec()
                },
            );
        }
        Ok(())
    }
}

pub fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let mut events = crate::preview::image::probe::InputReader::new(
        app.options.image_protocol,
        app.options.image_cell_size,
    )?;
    if let Some(s) = &mut app.sidebar {
        s.configure_images(events.display);
    }
    let mut terminal_display = events.display;
    app.parallel.display = events.display;
    let mut pixels = crate::image_output::PixelDamage::default();
    let mut previous = vec![];
    let mut changed = true;
    let mut finished = None;
    loop {
        changed |= app.task_tick();
        let sidebar_active = app.sidebar_active();
        app.update_image_occlusion();
        if let Some(sidebar) = &mut app.sidebar {
            if terminal_display != events.display {
                terminal_display = events.display;
                app.parallel.display = events.display;
                sidebar.configure_images(events.display);
                changed = true;
            }
            changed |= sidebar.tick(sidebar_active);
        }
        if let Some(path) = app.sidebar.as_mut().and_then(|s| s.ready_input.take()) {
            app.sidebar_action(crate::sidebar::Action::ValidatedInput(path));
            changed = true;
        }
        if let Some(command) = app.sidebar.as_mut().and_then(|s| s.ready_manage.take()) {
            app.sidebar_action(crate::sidebar::Action::Manage(command));
            changed = true;
        }
        if let Some((task, receipt)) = &app.path_receipt {
            let task = task.clone();
            match receipt.try_recv() {
                Ok(result) => {
                    app.path_receipt = None;
                    let sidebar = if app.parallel.context == task {
                        app.sidebar.as_mut()
                    } else {
                        app.parallel.sidebars.get_mut(&task)
                    };
                    if let Some(s) = sidebar {
                        match result{Ok(())=>s.say(crate::i18n::t("ui.path_accepted_by_input_chain_no_enter_sent"),true),Err(e)=>s.say(format!("Delivery rejected or unknown: {e:#}. Inspect Agent before retrying."),false)}
                    }
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.path_receipt = None;
                    let sidebar = if app.parallel.context == task {
                        app.sidebar.as_mut()
                    } else {
                        app.parallel.sidebars.get_mut(&task)
                    };
                    if let Some(s) = sidebar {
                        s.say(
                            crate::i18n::t("ui.delivery_unknown_inspect_agent_before_retrying"),
                            false,
                        );
                    }
                    changed = true;
                }
                Err(_) => {}
            }
        }
        changed |= app.refresh_message(Instant::now());
        if app.update_sidebar_layout() {
            app.attach_visible()?;
            changed = true;
        }
        let revisions: Vec<_> = app.sessions.iter().map(|s| s.state().revision).collect();
        if changed || revisions != previous {
            let _perf = crate::perf::span(crate::perf::Metric::Frame);
            app.update_image_occlusion();
            let plan = app
                .sidebar
                .as_ref()
                .map_or_else(Vec::new, |s| s.image_plans());
            pixels.draw(terminal, &plan, |f| ui::draw(f, app))?;
            if app
                .popup
                .is_some_and(|i| app.sessions[i].state().revision > 0)
                && let Some(start) = app.floating_first_frame.take()
            {
                crate::perf::record(
                    crate::perf::Metric::FloatingFirstFrame,
                    Duration::from_millis(
                        crate::presentation::timestamp_ms().saturating_sub(start),
                    ),
                );
            }
            previous = revisions;
            changed = false;
        }
        if app.force_exit {
            return Ok(());
        }
        let attached = app.sessions.iter().filter(|s| s.attached()).count();
        if attached > 0
            && !app.sidebar.as_ref().is_some_and(|s| s.dirty())
            && !app.parallel.sidebars.values().any(|s| s.dirty())
            && app.sessions.iter().all(|s| !s.running())
            && finished.get_or_insert_with(Instant::now).elapsed() > Duration::from_millis(200)
        {
            return Ok(());
        }
        if app.sessions.iter().any(|s| s.attached() && s.running()) {
            finished = None;
        }
        if app.last_target.elapsed() > Duration::from_millis(500) {
            app.last_target = Instant::now();
            if let Ok(crate::protocol::Response::Workspaces(workspaces)) =
                crate::protocol::rpc(crate::protocol::Request::List)
            {
                if let Some(work) = workspaces.iter().find(|w| w.id == app.workspace_id) {
                    changed |= app.sync_presentations(work)?;
                    if work.options.layout != app.options.layout {
                        if app.view.is_some() {
                            app.detach();
                        } else {
                            for (i, session) in app.sessions.iter_mut().enumerate() {
                                if app.popup != Some(i) {
                                    session.detach();
                                }
                            }
                            app.options.layout = work.options.layout;
                            if app.agents().len() < 2 {
                                app.focus = app
                                    .sessions
                                    .iter()
                                    .position(|s| {
                                        s.info.placement == crate::presentation::Placement::Docked
                                    })
                                    .unwrap_or(0);
                            }
                            app.selection = None;
                            app.attach_visible()?;
                            if app.agents().len() < 2 {
                                match app.options.layout {
                                    Layout::Float => app.show_default_debug()?,
                                    Layout::Tabs if app.popup.is_none() => {
                                        if app
                                            .sidebar
                                            .as_ref()
                                            .is_some_and(|s| s.document.is_some())
                                        {
                                            crate::windows::spawn_view(
                                                &app.workspace_id,
                                                "debug",
                                                false,
                                            )?;
                                        } else {
                                            app.detach();
                                            crate::windows::spawn_view(
                                                &app.workspace_id,
                                                "main",
                                                false,
                                            )?;
                                        }
                                    }
                                    Layout::Split | Layout::Tabs => {}
                                }
                            }
                            changed = true;
                        }
                    }
                } else {
                    app.detach();
                }
            }
            if app.sessions.iter().any(|s| {
                s.attached()
                    && s.state()
                        .io_error
                        .as_ref()
                        .is_some_and(|e| e.contains("no longer owns session input"))
            }) {
                app.detach();
                if !app.sidebar.as_ref().is_some_and(|s| s.dirty()) {
                    return Ok(());
                }
            }
        }
        // Keep the force-close countdown visible even when neither PTY outputs.
        changed |= app.mode == Mode::Confirm;
        if let Some(event) = events.read(Duration::from_millis(20))? {
            if let Err(error) = app.event(event) {
                app.message = format!("{error:#}");
            }
            changed = true;
        }
    }
}

pub struct TerminalGuard;
impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        let guard = Self;
        execute!(
            io::stdout(),
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            crossterm::terminal::EndSynchronizedUpdate,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            DisableMouseCapture,
            DisableBracketedPaste,
            DisableFocusChange
        );
        ratatui::restore();
    }
}
