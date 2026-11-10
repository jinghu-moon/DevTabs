use std::{
    collections::HashSet,
    io,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use ratatui::DefaultTerminal;

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
    pub options: Options,
    pub sessions: Vec<Client>,
    pub workspace_id: String,
    pub focus: usize,
    pub mode: Mode,
    pub selection: Option<SelectionView>,
    pub message: String,
    pub quit_requested: Option<Instant>,
    force_exit: bool,
    pub view: Option<String>,
    pub return_overview: bool,
    pub sidebar: Option<crate::sidebar::Sidebar>,
    path_receipt: Option<std::sync::mpsc::Receiver<Result<()>>>,
    last_target: Instant,
    consumed_keys: HashSet<KeyCode>,
    size: ratatui::layout::Rect,
}

impl App {
    pub fn new(options: Options, _executable: &Path, size: ratatui::layout::Rect) -> Result<Self> {
        let named_main = options.view.as_deref() == Some("main");
        crate::host::ensure()?;
        let info = match crate::protocol::rpc(crate::protocol::Request::Open {
            options: options.clone(),
        })? {
            crate::protocol::Response::Workspace(info) => info,
            _ => anyhow::bail!("Session host returned no workspace"),
        };
        let sessions = info
            .sessions
            .into_iter()
            .map(Client::hidden)
            .collect::<Vec<_>>();
        if sessions.len() < 2 {
            anyhow::bail!("Session host did not create Agent and Debug sessions");
        }
        let view = options.view.clone().filter(|v| v != "main");
        let mut options = info.options;
        options.view = view.clone();
        let mut app = Self {
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
            quit_requested: None,
            force_exit: false,
            consumed_keys: HashSet::new(),
            size,
        };
        if app.view.as_deref() == Some("debug-shared") || app.view.as_deref() == Some("debug") {
            app.focus = 1;
        }
        if app.view.is_none() {
            app.sidebar = Some(crate::sidebar::Sidebar::new(app.options.project.clone()));
        }
        if let Some(view) = &app.view
            && let Some(index) = view.strip_prefix("session:")
        {
            app.focus = index.parse()?;
        }
        if (app.view.as_deref() == Some("debug") && app.options.layout != Layout::Tabs)
            || (app.view.as_deref() == Some("debug-shared") && app.options.layout != Layout::Float)
        {
            app.force_exit = true;
            return Ok(app);
        }
        app.attach_visible()?;
        execute!(
            io::stdout(),
            crossterm::terminal::SetTitle(if app.view.as_deref() == Some("debug-shared") {
                crate::windows::debug_title(&app.project_name())
            } else {
                format!("DevTabs · {} · {}", app.project_name(), app.workspace_id)
            })
        )?;
        if app.view.is_none() {
            match app.options.layout {
                Layout::Float => {
                    if let Err(error) = crate::windows::show_debug(&app.workspace_id) {
                        app.message =
                            format!("Debug window: {error:#}; Ctrl+G L switches to split");
                    }
                }
                Layout::Tabs => {
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
                Layout::Split => {}
            }
        }
        Ok(app)
    }
    pub fn sidebar_active(&self) -> bool {
        self.sidebar.is_some() && (self.focus < 2 || self.options.layout == Layout::Split)
    }
    fn update_sidebar_layout(&mut self) -> bool {
        let active = self.sidebar_active();
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.update_layout(
                if active {
                    self.size
                } else {
                    ratatui::layout::Rect::default()
                },
                self.options.layout == Layout::Split,
            )
        } else {
            false
        }
    }
    pub fn rects(&self) -> Vec<(usize, ratatui::layout::Rect)> {
        if self.sidebar_active()
            && let Some(sidebar) = &self.sidebar
        {
            let p = sidebar.geometry;
            let mut result = Vec::new();
            if !p.overlay
                && let Some(r) = p.agent
            {
                result.push((0, r));
            }
            if let Some(r) = p.debug {
                result.push((self.focus.max(1), r));
            }
            return result;
        }
        if self.size.width < 60 || self.size.height < 10 {
            return vec![];
        }
        if self.options.layout == Layout::Split && self.view.is_none() {
            return ui::panes(self.size)
                .map(|r| vec![(0, r[0]), (self.focus.max(1), r[1])])
                .unwrap_or_default();
        }
        vec![(
            self.focus,
            ratatui::layout::Rect::new(0, 0, self.size.width, self.size.height.saturating_sub(2)),
        )]
    }
    fn attach_visible(&mut self) -> Result<()> {
        self.update_sidebar_layout();
        let mut rects = self.rects();
        let overlay = self.sidebar.as_ref().is_some_and(|s| s.geometry.overlay);
        if overlay && let Some(r) = self.sidebar.as_ref().and_then(|s| s.geometry.agent) {
            rects.push((0, r));
        }
        // A minimized terminal has no drawable panes. Keep existing connections
        // and PTY dimensions until it becomes usable again.
        if rects.is_empty() {
            return Ok(());
        }
        for (index, session) in self.sessions.iter_mut().enumerate() {
            if !rects.iter().any(|(i, _)| *i == index) {
                session.detach();
            }
        }
        for (index, rect) in rects {
            if index == 0 && overlay && self.sessions[index].attached() {
                continue;
            }
            let r = ui::content(rect);
            self.sessions
                .get_mut(index)
                .context("Unknown session")?
                .attach(&self.workspace_id, r.height, r.width, self.view.is_none())?;
        }
        Ok(())
    }
    fn layout(&mut self) -> Result<()> {
        let layout = match self.options.layout {
            Layout::Float => Layout::Split,
            Layout::Split => Layout::Tabs,
            Layout::Tabs => Layout::Float,
        };
        for session in &mut self.sessions {
            session.detach();
        }
        crate::protocol::rpc(crate::protocol::Request::Layout {
            workspace: self.workspace_id.clone(),
            layout,
        })?;
        self.options.layout = layout;
        self.focus = 0;
        self.selection = None;
        self.attach_visible()?;
        match layout {
            Layout::Float => crate::windows::show_debug(&self.workspace_id)?,
            Layout::Tabs => {
                if self.sidebar.as_ref().is_some_and(|s| s.document.is_some()) {
                    crate::windows::spawn_view(&self.workspace_id, "debug", false)?;
                } else {
                    self.detach();
                    crate::windows::spawn_view(&self.workspace_id, "main", false)?;
                }
            }
            Layout::Split => {}
        }
        Ok(())
    }
    fn detach(&mut self) {
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
        if index >= self.sessions.len() {
            return;
        }
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.focus_agent();
        }
        if self.view.is_some() && index == 0 {
            if let Err(error) = crate::windows::focus_workspace(&self.workspace_id) {
                self.message = format!("{error:#}");
            }
            return;
        }
        if index == 1 && self.options.layout == Layout::Float && self.view.is_none() {
            if let Err(error) = crate::windows::show_debug(&self.workspace_id) {
                self.message = format!("{error:#}");
            }
            return;
        }
        if index == 1 && self.options.layout == Layout::Tabs && self.view.is_none() {
            if let Err(error) = crate::windows::spawn_view(&self.workspace_id, "debug", false) {
                self.message = format!("{error:#}");
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
        self.message = "Copied selection; Ctrl+G S returns to live output".into();
        Ok(())
    }
    fn key(&mut self, key: KeyEvent, native: Option<Vec<u8>>, repeat: usize) {
        let pressed = key.kind != KeyEventKind::Release;
        if !pressed && self.consumed_keys.remove(&key.code) {
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
                KeyCode::Char('a' | 'A') => self.focus(0),
                KeyCode::Char('d' | 'D') => self.focus(1),
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
                    if let Err(error) = crate::windows::toggle_debug_topmost() {
                        self.message = format!("{error:#}");
                    }
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
                        self.message =
                            "Interrupt requested; Ctrl+G Shift+F can force after 3 seconds".into();
                    }
                }
                KeyCode::Char('F') => {
                    if let Err(error) = self.sessions[self.focus].action(true, true) {
                        self.message = format!("{error:#}");
                    }
                }
                KeyCode::Char('b' | 'B') => {
                    if let Some(s) = &mut self.sidebar {
                        s.toggle();
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('f') => {
                    if let Some(s) = &mut self.sidebar {
                        s.focus_tree();
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('o' | 'O') => {
                    if let Some(s) = &mut self.sidebar
                        && s.document.is_some()
                    {
                        s.preview_visible = true;
                        s.focus = crate::sidebar::Focus::Preview;
                    }
                    let _ = self.attach_visible();
                }
                KeyCode::Char('i' | 'I') => {
                    let path = self.sidebar.as_ref().and_then(|s| s.files.selected_file());
                    if let Some(path) = path {
                        self.sidebar_action(crate::sidebar::Action::Insert(path));
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
                _ => self.message = "Unknown DevTabs command; press Ctrl+G to see commands".into(),
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
                    self.message="Interrupt requested for visible sessions. Finish programs and exit, or Ctrl+G Q → F closes every session after 3s.".into();
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
            && (sidebar.focus != crate::sidebar::Focus::Terminal || sidebar.modal())
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
            crate::sidebar::Action::Agent => self.focus(0),
            crate::sidebar::Action::Insert(path) => {
                if self.path_receipt.is_some()
                    || self.sidebar.as_ref().is_some_and(|s| s.checking_input)
                {
                    if let Some(s) = &mut self.sidebar {
                        s.say(
                            "A path is still being sent; inspect Agent before retrying",
                            false,
                        );
                    }
                    return;
                }
                if let Some(s) = &mut self.sidebar {
                    match s.io.send(crate::files::Job::ValidateInput(path)) {
                        Ok(()) => {
                            s.checking_input = true;
                            s.say("Checking file before sending path", false);
                        }
                        Err(e) => s.say(e.to_string(), false),
                    }
                }
            }
            crate::sidebar::Action::ValidatedInput(path) => {
                let result = (|| -> Result<()> {
                    anyhow::ensure!(
                        self.options.agent != crate::launch::Agent::Shell,
                        "Shell is not an Agent input target"
                    );
                    let session = &self.sessions[0];
                    anyhow::ensure!(
                        session.attached() && session.running(),
                        "Agent disconnected or exited"
                    );
                    let state = session.state();
                    anyhow::ensure!(
                        state.io_error.is_none(),
                        "Agent input unavailable; inspect connection"
                    );
                    anyhow::ensure!(
                        state.command == crate::session::CommandStatus::Running,
                        "Configured Agent is not running"
                    );
                    let bracketed = state.terminal.screen().bracketed_paste();
                    drop(state);
                    let text = crate::files::path_input(&path)?;
                    self.path_receipt =
                        Some(session.send_checked(input::paste(&text, bracketed)?)?);
                    Ok(())
                })();
                if let Some(s) = &mut self.sidebar {
                    match result {
                        Ok(()) => {
                            s.focus_agent();
                            s.say("Sending path; no Enter sent", false);
                        }
                        Err(e) => s.say(format!("Path not sent: {e:#}"), false),
                    }
                }
            }
        }
    }
    fn forward_key(&mut self, key: KeyEvent, native: Option<Vec<u8>>, repeat: usize) {
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
        if let Some(sidebar) = &mut self.sidebar {
            match sidebar.mouse(event) {
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
        if event.row >= self.size.height.saturating_sub(2)
            && event.column >= self.size.width.saturating_sub(18)
            && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
        {
            if self.view.as_deref() == Some("debug-shared") {
                self.detach();
            } else {
                self.focus(1);
            }
            return;
        }
        let rects = self.rects();
        let Some((index, rect)) = rects
            .into_iter()
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
            Event::Key(key) => self.key(key, input.win32, input.repeat),
            Event::Mouse(mouse) if self.mode == Mode::Normal => self.mouse(mouse),
            Event::Paste(text) if self.mode == Mode::Normal && self.selection.is_none() => {
                if let Some(s) = &mut self.sidebar
                    && (s.focus != crate::sidebar::Focus::Terminal || s.modal())
                {
                    s.paste(&text)?;
                    return Ok(());
                }
                if self.rects().is_empty() {
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
                if let Some(s) = &mut self.sidebar {
                    s.cancel_drag(true);
                }
                self.size = ratatui::layout::Rect::new(0, 0, width, height);
                self.selection = None;
                self.attach_visible()?;
            }
            Event::FocusGained | Event::FocusLost => {
                if event == Event::FocusLost
                    && let Some(s) = &mut self.sidebar
                {
                    s.cancel_drag(true);
                }
                if event == Event::FocusGained
                    && self.view.is_none()
                    && self.options.layout == Layout::Float
                {
                    crate::protocol::rpc(crate::protocol::Request::DebugTarget {
                        workspace: Some(self.workspace_id.clone()),
                    })?;
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
    let mut events = crate::events::Events::new()?;
    let mut previous = vec![];
    let mut changed = true;
    let mut finished = None;
    loop {
        let sidebar_active = app.sidebar_active();
        if let Some(sidebar) = &mut app.sidebar {
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
        if let Some(receipt) = &app.path_receipt {
            match receipt.try_recv() {
                Ok(result) => {
                    app.path_receipt = None;
                    if let Some(s) = &mut app.sidebar {
                        match result{Ok(())=>s.say("Path accepted by input chain; no Enter sent",true),Err(e)=>s.say(format!("Delivery rejected or unknown: {e:#}. Inspect Agent before retrying."),false)}
                    }
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.path_receipt = None;
                    if let Some(s) = &mut app.sidebar {
                        s.say("Delivery unknown; inspect Agent before retrying", false);
                    }
                    changed = true;
                }
                Err(_) => {}
            }
        }
        if app.update_sidebar_layout() {
            app.attach_visible()?;
            changed = true;
        }
        let revisions: Vec<_> = app.sessions.iter().map(|s| s.state().revision).collect();
        if changed || revisions != previous {
            let _perf = crate::perf::span(crate::perf::Metric::Frame);
            terminal.draw(|f| ui::draw(f, app))?;
            previous = revisions;
            changed = false;
        }
        if app.force_exit {
            return Ok(());
        }
        let attached = app.sessions.iter().filter(|s| s.attached()).count();
        if attached > 0
            && !app.sidebar.as_ref().is_some_and(|s| s.dirty())
            && app
                .sessions
                .iter()
                .filter(|s| s.attached())
                .all(|s| !s.running())
            && finished.get_or_insert_with(Instant::now).elapsed() > Duration::from_millis(200)
        {
            return Ok(());
        }
        if app.sessions.iter().any(|s| s.attached() && s.running()) {
            finished = None;
        }
        if app.last_target.elapsed() > Duration::from_millis(500) {
            app.last_target = Instant::now();
            if app.view.as_deref() == Some("debug-shared")
                && let Ok(crate::protocol::Response::Target(Some(target))) =
                    crate::protocol::rpc(crate::protocol::Request::DebugTarget { workspace: None })
                && target != app.workspace_id
            {
                for session in &mut app.sessions {
                    session.detach();
                }
                let mut options = app.options.clone();
                options.workspace = Some(target);
                options.view = Some("debug-shared".into());
                match App::new(options, &std::env::current_exe()?, app.size) {
                    Ok(next) => {
                        *app = next;
                        changed = true;
                    }
                    Err(error) => app.message = format!("{error:#}"),
                }
            }
            if let Ok(crate::protocol::Response::Workspaces(workspaces)) =
                crate::protocol::rpc(crate::protocol::Request::List)
            {
                if let Some(work) = workspaces.iter().find(|w| w.id == app.workspace_id) {
                    if work.options.layout != app.options.layout {
                        if app.view.is_some() {
                            app.detach();
                        } else {
                            for session in &mut app.sessions {
                                session.detach();
                            }
                            app.options.layout = work.options.layout;
                            app.focus = 0;
                            app.selection = None;
                            app.attach_visible()?;
                            match app.options.layout {
                                Layout::Float => crate::windows::show_debug(&app.workspace_id)?,
                                Layout::Tabs => {
                                    if app.sidebar.as_ref().is_some_and(|s| s.document.is_some()) {
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
                                Layout::Split => {}
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
            DisableMouseCapture,
            DisableBracketedPaste,
            DisableFocusChange
        );
        ratatui::restore();
    }
}
