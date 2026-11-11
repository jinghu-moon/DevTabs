//! Common controls for every managed terminal; no role-specific window state.
use super::*;
use crate::{
    presentation::{Placement, Target},
    protocol::{Request, Response, WorkspaceInfo},
};
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Copy)]
pub(super) enum Action {
    NewTask,
    Overview,
    Activate(usize),
    Menu(usize),
    Place(usize, Placement),
    Float(usize),
    Move(usize),
    Resize(usize),
}
impl App {
    pub(crate) fn terminal_menu_open(&self) -> bool {
        self.terminal_menu.is_some()
    }
    pub(crate) fn clear_terminal_hits(&mut self) {
        self.terminal_hits.clear();
    }
    pub(super) fn docked(&self, i: usize) -> bool {
        self.sessions
            .get(i)
            .is_some_and(|s| s.info.placement == Placement::Docked)
    }
    pub(super) fn allowed(&self, i: usize) -> bool {
        self.docked(i) || self.popup == Some(i)
    }

    pub(super) fn target(&self, i: usize) -> Target {
        Target {
            workspace: self.workspace_id.clone(),
            session: self.sessions[i].info.id.clone(),
        }
    }
    pub(super) fn sync_presentations(&mut self, work: &WorkspaceInfo) -> Result<bool> {
        let mut moved = false;
        let mut changed = false;
        self.parallel.tasks = work.tasks.clone();
        for info in &work.sessions {
            let Some(i) = self.sessions.iter().position(|s| s.info.id == info.id) else {
                self.sessions.push(Client::hidden(info.clone()));
                changed = true;
                moved = true;
                continue;
            };
            let local = &mut self.sessions[i];
            anyhow::ensure!(
                local.info.task.run_id == info.task.run_id,
                "Session run identity changed"
            );
            let placement_changed = local.info.placement != info.placement;
            changed |= placement_changed
                || local.info.command != info.command
                || local.info.status != info.status;
            if placement_changed {
                moved = true;
                if info.placement != Placement::Docked && self.popup != Some(i) {
                    local.detach();
                }
            }
            local.info = info.clone();
            if !local.attached() {
                let mut state = local.state();
                state.status = info.status.clone();
                state.command = info.command.clone();
                state.shell_pid = info.shell_pid;
            }
        }
        for local in &mut self.sessions {
            if local.info.task.role != crate::tasks::Role::Extra
                && !work.sessions.iter().any(|s| s.id == local.info.id)
            {
                local.detach();
                local.info.task.role = crate::tasks::Role::Extra;
                local.info.placement = Placement::Collapsed;
                local.info.name = format!(
                    "{} · {}",
                    local.info.name,
                    crate::i18n::t("parallel.previous_run")
                );
                local.info.status = crate::session::Status::Failed(
                    crate::i18n::t("parallel.replaced_run").into_owned(),
                );
                local.state().status = local.info.status.clone();
                changed = true;
                moved = true;
            }
        }
        if let Some((task, role)) = self.parallel.role_focus.clone()
            && let Some(info) = work
                .sessions
                .iter()
                .find(|s| s.task.task_id == task && s.task.role == role)
            && let Some(i) = self.sessions.iter().position(|s| s.info.id == info.id)
        {
            self.parallel.role_focus = None;
            if self.parallel.context == task {
                self.focus(i);
            }
        }
        if let Some(i) = self.popup
            && self.sessions[i].info.placement != Placement::Floating
        {
            self.sessions[i].detach();
            self.popup = None;
            self.popup_window.cancel_drag();
            moved = true;
        }
        if moved {
            self.zoom = None;
            self.selection = None;
            if !self.allowed(self.focus)
                && let Some(i) = self
                    .sessions
                    .iter()
                    .position(|s| s.info.placement == Placement::Docked)
            {
                self.focus = i;
            }
            self.attach_visible()?;
        }
        Ok(changed)
    }
    fn terminal_action(&mut self, action: Action) -> Result<()> {
        self.terminal_menu = None;
        match action {
            Action::NewTask => self.new_task(),
            Action::Overview => self.back_to_overview(),
            Action::Menu(i) => self.terminal_menu = Some(i),
            Action::Activate(i) => self.focus(i),
            Action::Float(i) => self.show_popup(i)?,
            Action::Move(i) => {
                self.show_popup(i)?;
                self.popup_window.edit = Some(popup::Edit::Move);
            }
            Action::Resize(i) => {
                self.show_popup(i)?;
                self.popup_window.edit = Some(popup::Edit::Resize);
            }
            Action::Place(i, placement) => {
                self.zoom = None;
                crate::protocol::rpc(Request::Present {
                    target: self.target(i),
                    placement,
                })?;
                if self.popup == Some(i) {
                    self.popup = None;
                    self.popup_window.cancel_drag();
                    self.sessions[i].detach();
                }
                if let Response::Workspaces(work) = crate::protocol::rpc(Request::List)?
                    && let Some(work) = work.iter().find(|w| w.id == self.workspace_id)
                {
                    self.sync_presentations(work)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn terminal_key(&mut self, key: KeyEvent) {
        let Some(mut selected) = self.terminal_menu else {
            return;
        };
        let action = match key.code {
            KeyCode::Esc => {
                self.terminal_menu = None;
                return;
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                selected = (selected + 1).min(self.sessions.len() - 1);
                None
            }
            KeyCode::Home => {
                selected = 0;
                None
            }
            KeyCode::End => {
                selected = self.sessions.len() - 1;
                None
            }
            KeyCode::Enter => Some(Action::Activate(selected)),
            KeyCode::Char('d' | 'D') => Some(Action::Place(selected, Placement::Docked)),
            KeyCode::Char('c' | 'C') => Some(Action::Place(selected, Placement::Collapsed)),
            KeyCode::Char('f' | 'F') => Some(Action::Float(selected)),
            KeyCode::Char('m' | 'M') => Some(Action::Move(selected)),
            KeyCode::Char('r' | 'R') => Some(Action::Resize(selected)),
            _ => None,
        };
        self.terminal_menu = Some(selected);
        if let Some(action) = action
            && let Err(error) = self.terminal_action(action)
        {
            self.message = format!("{error:#}");
        }
    }
    pub(super) fn terminal_mouse(&mut self, event: MouseEvent) -> bool {
        if self.mode == Mode::Confirm || self.sidebar.as_ref().is_some_and(|s| s.modal()) {
            return false;
        }
        if let Some(selected) = self.terminal_menu {
            if matches!(
                event.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) {
                self.terminal_key(KeyEvent::new(
                    if event.kind == MouseEventKind::ScrollUp {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    },
                    KeyModifiers::NONE,
                ));
                return true;
            }
            if matches!(event.kind, MouseEventKind::Down(_)) {
                if let Some((_, action)) = self
                    .terminal_hits
                    .iter()
                    .find(|(r, _)| r.contains((event.column, event.row).into()))
                    .copied()
                {
                    if let Err(error) = self.terminal_action(action) {
                        self.message = format!("{error:#}");
                    }
                } else {
                    self.terminal_menu = None;
                }
                return true;
            }
            let _ = selected;
            return true;
        }
        if self.popup_drag(event) {
            return true;
        }
        if matches!(event.kind, MouseEventKind::Down(_))
            && let Some((_, mut action)) = self
                .terminal_hits
                .iter()
                .find(|(r, _)| r.contains((event.column, event.row).into()))
                .copied()
        {
            if event.kind == MouseEventKind::Down(MouseButton::Right)
                && let Action::Activate(i) = action
            {
                action = Action::Menu(i);
            }
            if let Err(error) = self.terminal_action(action) {
                self.message = format!("{error:#}");
            }
            return true;
        }
        false
    }
}
fn placement(p: Placement) -> std::borrow::Cow<'static, str> {
    crate::i18n::t(match p {
        Placement::Docked => "terminal.docked",
        Placement::Collapsed => "terminal.collapsed",
        Placement::Floating => "terminal.floating",
    })
}
pub(super) fn footer_hit(
    frame: &mut Frame,
    app: &mut App,
    r: Rect,
    label: String,
    action: Action,
    selected: bool,
) {
    frame.render_widget(
        Paragraph::new(label).style(if selected {
            Style::new()
                .fg(Color::Black)
                .bg(crate::appearance::palette().accent)
        } else {
            Style::new().fg(crate::appearance::palette().accent)
        }),
        r,
    );
    app.terminal_hits.push((r, action));
}
pub(crate) fn draw(frame: &mut Frame, app: &mut App) {
    app.terminal_hits.clear();
    let area = frame.area();
    if app.size.width < 60 || app.size.height < 10 {
        return;
    }
    if app.rects().is_empty()
        && !app
            .sidebar
            .as_ref()
            .is_some_and(|s| s.document.is_some() && s.preview_visible)
    {
        let r = Rect::new(area.x + 2, area.y + 2, area.width.saturating_sub(4), 2);
        let label = if app
            .sessions
            .iter()
            .any(|s| s.info.placement == Placement::Docked)
        {
            crate::i18n::format(
                "min_size",
                &[(
                    "minimum",
                    if app.options.layout == crate::launch::Layout::Split {
                        "94 × 13"
                    } else {
                        "62 × 13"
                    }
                    .into(),
                )],
            )
        } else {
            crate::i18n::t("terminal.empty").into_owned()
        };
        frame.render_widget(Paragraph::new(label), r);
    }
    let right = super::chrome::footer_controls(frame, app);
    super::chrome::footer(frame, app, right);
    let popup_rect = app
        .popup
        .filter(|_| app.zoom.is_none())
        .map(|_| super::chrome::cover(app.popup_window.rect(area), area));
    for (i, r) in app.rects() {
        if app.popup == Some(i) {
            app.terminal_hits
                .retain(|(hit, _)| hit.intersection(r).is_empty());
        }
        if app.popup != Some(i)
            && popup_rect.is_some_and(|p| p.intersects(Rect::new(r.x, r.y, r.width, 1)))
        {
            continue;
        }
        let show = app.terminal_focused(i)
            || (app.hovered(r)
                && !popup_rect.is_some_and(|p| app.popup != Some(i) && app.hovered(p)));
        if !show {
            if r.width >= 12 {
                footer_hit(
                    frame,
                    app,
                    Rect::new(r.right() - 6, r.y, 5, 1),
                    "[...]".into(),
                    Action::Menu(i),
                    false,
                );
            }
            continue;
        }
        let mut actions = if app.popup == Some(i) {
            vec![
                (
                    crate::i18n::t("terminal.collapse"),
                    Action::Place(i, Placement::Collapsed),
                ),
                (
                    crate::i18n::t("terminal.dock"),
                    Action::Place(i, Placement::Docked),
                ),
                (crate::i18n::t("terminal.move"), Action::Move(i)),
                (crate::i18n::t("terminal.resize"), Action::Resize(i)),
            ]
        } else {
            vec![
                (
                    crate::i18n::t("terminal.collapse"),
                    Action::Place(i, Placement::Collapsed),
                ),
                (crate::i18n::t("terminal.float"), Action::Float(i)),
            ]
        };
        actions.push(("...".into(), Action::Menu(i)));
        let width = actions.iter().map(|(s, _)| s.width() + 3).sum::<usize>() as u16;
        let label_width =
            app.sessions[i].info.name.width() as u16 + if app.focus == i { 12 } else { 4 };
        if width + label_width + 4 > r.width {
            if r.width >= 12 {
                footer_hit(
                    frame,
                    app,
                    Rect::new(r.right() - 6, r.y, 5, 1),
                    "[...]".into(),
                    Action::Menu(i),
                    false,
                );
            }
            continue;
        }
        let mut x = r.right() - width - 1;
        for (label, action) in actions {
            let label = format!("[{label}] ");
            let w = label.width() as u16;
            footer_hit(frame, app, Rect::new(x, r.y, w, 1), label, action, false);
            x += w;
        }
    }
}
pub(crate) fn overlay(frame: &mut Frame, app: &mut App) {
    let Some(selected) = app.terminal_menu else {
        return;
    };
    app.terminal_hits.clear();
    let area = frame.area();
    let w = area.width.min(86);
    let h = area.height.min(22);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    crate::ui::clear_popup(frame, r);
    frame.render_widget(Block::bordered().title(crate::i18n::t("terminal.menu")), r);
    let inner = r.inner(Margin::new(2, 1));
    let page = inner.height.saturating_sub(4).max(1) as usize;
    let first = selected.saturating_sub(page - 1);
    for i in first..(first + page).min(app.sessions.len()) {
        let info = &app.sessions[i].info;
        let label = format!(
            "{} {} · {}",
            if selected == i { "▶" } else { " " },
            app.parallel
                .tasks
                .iter()
                .find(|t| t.id == info.task.task_id)
                .map_or_else(
                    || info.name.clone(),
                    |t| format!("{} · {} · {}", t.name, info.name, t.branch)
                ),
            placement(info.placement)
        );
        footer_hit(
            frame,
            app,
            Rect::new(inner.x, inner.y + (i - first) as u16, inner.width, 1),
            label,
            Action::Menu(i),
            selected == i,
        );
    }
    let actions = [
        ("terminal.dock", Action::Place(selected, Placement::Docked)),
        ("terminal.float", Action::Float(selected)),
        (
            "terminal.collapse",
            Action::Place(selected, Placement::Collapsed),
        ),
    ];
    let mut x = inner.x;
    for (key, action) in actions {
        let label = format!("[{}] ", crate::i18n::t(key));
        let w = (label.width() as u16).min(inner.right().saturating_sub(x));
        footer_hit(
            frame,
            app,
            Rect::new(x, inner.bottom().saturating_sub(3), w, 1),
            label,
            action,
            false,
        );
        x += w;
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(crate::i18n::t("terminal.keys")),
            Line::from(crate::i18n::t("terminal.identity")),
        ]),
        Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2),
    );
}

pub(super) fn short_name(s: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        let n = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + n > limit {
            out.push('…');
            break;
        }
        out.push(c);
        width += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_task_layout_uses_roles_when_debug_precedes_agent() {
        let mut app = app();
        app.sessions.truncate(2);
        app.options.layout = Layout::Split;
        app.focus = 1;
        for s in &mut app.sessions {
            s.info.task.task_id = "A".into();
            s.info.placement = Placement::Docked;
        }
        app.sessions[0].info.task.role = crate::tasks::Role::Debug;
        app.sessions[1].info.task.role = crate::tasks::Role::Agent;
        let rects = app.rects();
        assert_eq!(rects[0].0, 1);
        assert_eq!(rects[1].0, 0);
    }
    #[test]
    fn parallel_agents_share_one_frontend_and_keep_focus_on_background_addition() {
        let mut app = app();
        app.sessions.truncate(2);
        app.size = Rect::new(0, 0, 200, 50);
        for (i, session) in app.sessions.iter_mut().enumerate() {
            session.info.placement = Placement::Docked;
            session.info.task.task_id = format!("t{i}");
            session.info.task.run_id = format!("r{i}");
            session.info.task.role = crate::tasks::Role::Agent;
        }
        app.options.layout = Layout::Float;
        let rects = app.rects();
        assert_eq!(
            rects.len(),
            2,
            "Both Agents must share the existing frontend"
        );
        assert!(rects[0].1.intersection(rects[1].1).is_empty());
        assert_eq!(app.focus, 0);
    }
    #[test]
    fn workspace_snapshot_reordering_and_new_sessions_preserve_existing_client_identity() {
        let mut app = app();
        app.sessions.truncate(2);
        app.size = Rect::new(0, 0, 20, 5); // no attach while minimized
        app.focus = 1;
        app.sessions[1].state().revision = 991;
        let mut infos: Vec<_> = app.sessions.iter().map(|s| s.info.clone()).collect();
        infos.reverse();
        let mut third = infos[0].clone();
        third.id = "third".into();
        infos.push(third);
        let work = WorkspaceInfo {
            id: app.workspace_id.clone(),
            project: app.options.project.clone(),
            options: app.options.clone(),
            sessions: infos,
            tasks: vec![],
        };
        assert!(app.sync_presentations(&work).is_ok());
        assert_eq!(app.sessions.len(), 3);
        assert_eq!(app.sessions[1].info.id, "s1");
        assert_eq!(app.sessions[1].state().revision, 991);
        assert_eq!(app.focus, 1);
    }
    #[test]
    fn selecting_hidden_task_temporarily_zooms_without_replacing_pinned_agents() {
        let mut app = app();
        app.sessions.truncate(3);
        for (i, s) in app.sessions.iter_mut().enumerate() {
            s.info.task.task_id = format!("t{i}");
            s.info.task.role = crate::tasks::Role::Agent;
            s.info.placement = Placement::Docked;
        }
        app.parallel.layout.pair = vec!["t0".into(), "t1".into()];
        app.pin_selected_task(2);
        assert_eq!(app.parallel.layout.pair, vec!["t0", "t1"]);
        assert_eq!(app.zoom, Some(super::super::chrome::Zoom::Terminal(2)));
    }
    #[test]
    fn asynchronous_path_validation_cannot_use_a_replaced_or_lost_input_lease() {
        let app = app();
        let ticket = super::super::parallel::InputTarget {
            session: "s0".into(),
            run: String::new(),
            task: String::new(),
            lease: Some("old-attachment".into()),
        };
        assert!(app.resolve_input_target(&ticket).is_err());
    }
    #[test]
    fn switching_task_hides_the_previous_tasks_debug_popup_and_restores_its_context() {
        let mut app = app();
        app.sessions.truncate(3);
        for (i, s) in app.sessions.iter_mut().enumerate() {
            s.info.task.task_id = if i == 2 { "B" } else { "A" }.into();
            s.info.task.initial_cwd = std::env::temp_dir();
            s.info.task.role = if i == 1 {
                crate::tasks::Role::Debug
            } else {
                crate::tasks::Role::Agent
            };
        }
        app.parallel.context = "A".into();
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        app.sidebar.as_mut().unwrap().visible = false;
        app.sessions[1].info.placement = Placement::Floating;
        app.popup = Some(1);
        app.switch_context(2);
        assert!(
            app.popup.is_none(),
            "Debug A must not cover task B's preview"
        );
        app.switch_context(0);
        assert_eq!(app.popup, Some(1));
        assert!(!app.sidebar.as_ref().unwrap().visible);
    }
    #[test]
    fn task_switch_preserves_unsaved_draft_and_never_saves_it_to_either_checkout() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-task-draft-{}", uuid::Uuid::new_v4()));
        let a = root.join("A");
        let b = root.join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("same.md"), "# A\n").unwrap();
        std::fs::write(b.join("same.md"), "# B\n").unwrap();
        let mut app = app();
        app.sessions.truncate(2);
        for (i, s) in app.sessions.iter_mut().enumerate() {
            s.info.task.task_id = format!("t{i}");
            s.info.task.initial_cwd = if i == 0 { a.clone() } else { b.clone() };
        }
        app.parallel.context = "t0".into();
        app.sidebar = Some(crate::sidebar::Sidebar::new(a.clone()));
        let s = app.sidebar.as_mut().unwrap();
        s.open(a.join("same.md"), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s
            .document
            .as_ref()
            .is_some_and(|d| !d.loading && d.loaded().is_some())
        {
            s.tick(true);
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        s.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .unwrap();
        while !s.document.as_ref().is_some_and(|d| d.editor().is_some()) {
            s.tick(true);
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        s.paste("unsaved A").unwrap();
        let draft = s.document.as_ref().unwrap().editor().unwrap().text();
        app.switch_context(1);
        assert_eq!(app.sidebar.as_ref().unwrap().files.root, b);
        assert!(app.sidebar.as_ref().unwrap().document.is_none());
        app.switch_context(0);
        assert_eq!(
            app.sidebar
                .as_ref()
                .unwrap()
                .document
                .as_ref()
                .unwrap()
                .editor()
                .unwrap()
                .text(),
            draft
        );
        assert!(app.sidebar.as_ref().unwrap().dirty());
        app.switch_context(1);
        app.detach();
        assert!(!app.force_exit, "parked draft must guard frontend exit");
        assert_eq!(app.parallel.context, "t0");
        assert_eq!(std::fs::read_to_string(a.join("same.md")).unwrap(), "# A\n");
        assert_eq!(std::fs::read_to_string(b.join("same.md")).unwrap(), "# B\n");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parallel_geometry_keeps_minima_and_saved_ratio_across_narrow_windows() {
        use super::super::parallel::{Axis, split};
        for axis in [Axis::Auto, Axis::Sides, Axis::Stack] {
            for width in [40, 79, 80, 119, 120, 240] {
                for height in [10, 19, 20, 60] {
                    let area = Rect::new(3, 2, width, height);
                    let rects = split(area, axis, 7700);
                    for r in &rects {
                        assert!(r.width >= 40 && r.height >= 10);
                        assert_eq!(r.intersection(area), *r);
                    }
                    if rects.len() == 2 {
                        assert!(rects[0].intersection(rects[1]).is_empty());
                        assert_eq!(rects.iter().map(|r| r.area()).sum::<u32>(), area.area());
                    }
                }
            }
        }
    }
    fn app() -> App {
        let sessions = (0..8)
            .map(|i| {
                Client::hidden(crate::protocol::SessionInfo {
                    task: Box::default(),
                    id: format!("s{i}"),
                    name: format!("终端 {i} with a long name"),
                    shell_pid: Some(i),
                    status: crate::session::Status::Running,
                    command: crate::session::CommandStatus::Idle,
                    placement: Placement::Collapsed,
                })
            })
            .collect();
        App {
            parallel: Default::default(),
            options: Options::parse_for_view(),
            sessions,
            workspace_id: "workspace".into(),
            focus: 0,
            mode: Mode::Normal,
            selection: None,
            message: String::new(),
            message_seen: None,
            chrome_capture: false,
            quit_requested: None,
            force_exit: false,
            view: None,
            return_overview: false,
            sidebar: None,
            path_receipt: None,
            last_target: Instant::now(),
            consumed_keys: HashSet::new(),
            size: Rect::new(0, 0, 100, 30),
            popup: None,
            popup_window: popup::Window::default(),
            floating_first_frame: None,
            terminal_menu: None,
            terminal_hits: vec![],
            hover: None,
            zoom: None,
        }
    }
    #[test]
    fn agent_only_renders_every_layout_with_sidebar_and_never_creates_debug() {
        let mut app = app();
        app.sessions.truncate(1);
        app.sessions[0].info.placement = Placement::Docked;
        app.options.composition = crate::launch::Composition::AgentOnly;
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        for layout in [Layout::Split, Layout::Float, Layout::Tabs] {
            app.options.layout = layout;
            app.update_sidebar_layout();
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            assert!(app.sidebar.as_ref().unwrap().geometry.agent.is_some());
            assert_eq!(app.sessions.len(), 1);
            app.focus(1);
            assert!(!app.message.is_empty());
            assert_eq!(app.focus, 0);
            assert!(app.popup.is_none());
        }
    }
    #[test]
    fn debug_panel_keeps_native_image_visible_and_does_not_leave_skip_cells_over_terminal() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-panel-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("picture.png");
        image::RgbaImage::from_pixel(640, 680, image::Rgba([80, 120, 220, 255]))
            .save(&path)
            .unwrap();
        let mut app = app();
        app.size = Rect::new(0, 0, 180, 40);
        app.sessions[0].info.placement = Placement::Docked;
        let mut sidebar = crate::sidebar::Sidebar::new(root.clone());
        sidebar.visible = false;
        sidebar.configure_images(crate::preview::image::Display::explicit(
            crate::preview::image::ProtocolChoice::Sixel,
            Some(crate::preview::image::CellSize {
                width: 8,
                height: 17,
            }),
        ));
        sidebar.open(path, true).unwrap();
        app.sidebar = Some(sidebar);
        app.update_sidebar_layout();
        let at = Instant::now();
        while app.sidebar.as_ref().unwrap().image_plan().is_none() {
            app.sidebar.as_mut().unwrap().tick(true);
            assert!(at.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
        }
        let original = app.sidebar.as_ref().unwrap().image_plan().unwrap();
        app.popup = Some(1);
        app.sessions[1].info.placement = Placement::Floating;
        app.update_image_occlusion();
        let cover = super::super::chrome::cover(app.popup_window.rect(app.size), app.size);
        let sidebar = app.sidebar.as_ref().unwrap();
        assert!(
            !sidebar.images_occluded,
            "opening Debug must not suppress the whole preview"
        );
        assert_eq!(sidebar.image_plan(), Some(original));
        let plans = sidebar.image_plans();
        assert!(!plans.is_empty());
        assert!(plans.iter().all(|(r, _, _)| !r.intersects(cover)));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(180, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        for y in cover.y..cover.bottom() {
            for x in cover.x..cover.right() {
                assert!(!matches!(
                    terminal.backend().buffer()[(x, y)].diff_option,
                    ratatui::buffer::CellDiffOption::Skip
                        | ratatui::buffer::CellDiffOption::ForcedWidth(_)
                ));
            }
        }
        app.popup = None;
        app.update_image_occlusion();
        assert_eq!(
            app.sidebar.as_ref().unwrap().image_plans(),
            vec![(original.0, original.1, 0)]
        );
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn all_collapsed_ui_keeps_controls_and_menu_blocks_terminal_input() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                let mut app = app();
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
                terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
                assert!(app.rects().is_empty());
                assert!(!app.force_exit);
                assert!(
                    app.terminal_hits
                        .iter()
                        .any(|(_, a)| matches!(a, Action::Menu(_)))
                );
                app.mode = Mode::Prefix;
                app.key(
                    KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                    None,
                    1,
                );
                assert_eq!(app.terminal_menu, Some(0));
                app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), None, 1);
                assert_eq!(app.terminal_menu, Some(7));
                terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
                assert!(
                    app.terminal_hits
                        .iter()
                        .any(|(_, a)| matches!(a, Action::Place(7, Placement::Docked)))
                );
                app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), None, 1);
                assert!(app.terminal_menu.is_none());
                app.forward_key(
                    KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                    None,
                    1,
                );
                assert!(
                    app.message.is_empty(),
                    "hidden session must not receive input"
                );
            });
        }
    }
    #[test]
    fn translated_controls_and_modal_clicks_stay_within_visible_cells() {
        crate::i18n::with_language(crate::i18n::Language::ZhCn, || {
            let mut app = app();
            app.sessions[0].info.placement = Placement::Docked;
            for width in [160, 59, 60, 90] {
                app.size = Rect::new(0, 0, width, 30);
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 30)).unwrap();
                terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
                assert!(
                    app.terminal_hits
                        .iter()
                        .all(|(r, _)| r.right() <= width && r.bottom() <= 30)
                );
                if width < 60 {
                    assert!(app.terminal_hits.is_empty(), "resizing clears old controls");
                }
            }
            app.terminal_menu = Some(0);
            app.mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 20,
                row: 10,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(app.terminal_menu, Some(1));
            assert!(app.selection.is_none());
        });
    }
    #[test]
    fn terminal_panel_clears_underlying_content_and_menu_stays_above_it() {
        crate::i18n::with_language(crate::i18n::Language::EnUs, || {
            let mut app = app();
            app.sessions[0].info.placement = Placement::Docked;
            app.sessions[1].info.placement = Placement::Floating;
            app.sessions[1].info.name = "Debug".into();
            app.popup = Some(1);
            app.focus = 1;
            app.sessions[0]
                .state()
                .terminal
                .process(b"\x1b[2J\x1b[HBACKGROUND-CONTENT");
            app.sessions[1]
                .state()
                .terminal
                .process(b"\x1b[2J\x1b[HPANEL-CONTENT");
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let r = app.popup_window.rect(app.size);
            let buffer = terminal.backend().buffer();
            let text = (r.x + 1..r.right() - 1)
                .map(|x| buffer[(x, r.y + 1)].symbol())
                .collect::<String>();
            assert!(text.starts_with("PANEL-CONTENT"), "{text}");
            assert_eq!(buffer[(r.x, r.y + 1)].symbol(), "│");
            assert!(
                app.terminal_hits
                    .iter()
                    .any(|(_, a)| matches!(a, Action::Place(1, Placement::Docked)))
            );
            app.terminal_menu = Some(1);
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("Terminal sessions"));
            assert!(
                app.terminal_hits
                    .iter()
                    .all(|(_, a)| !matches!(a, Action::Activate(_)))
            );
        });
    }
    #[test]
    fn panel_mouse_and_keyboard_geometry_never_click_through_and_cancel_restores() {
        let mut app = app();
        app.popup = Some(1);
        app.sessions[1].info.placement = Placement::Floating;
        app.focus = 1;
        let original = app.popup_window.rect(app.size);
        // Empty controls: the title starts a captured move, including outside bounds.
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert!(app.terminal_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            original.x + 1,
            original.y
        )));
        assert!(app.terminal_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 99, 29)));
        let moved = app.popup_window.rect(app.size);
        assert_eq!(
            (original.width, original.height),
            (moved.width, moved.height)
        );
        assert!(moved.right() <= 100 && moved.bottom() < app.size.height);
        app.popup_window.cancel_drag();
        app.popup_window.edit = Some(popup::Edit::Resize);
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), None, 1);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), None, 1);
        assert_eq!(app.popup_window.rect(app.size), moved);
        assert!(!app.popup_window.editing());
        assert!(app.selection.is_none());
        assert!(app.message.is_empty());
    }
    #[test]
    fn file_drop_reaches_inline_agent_and_is_blocked_by_debug_panel() {
        let mut app = app();
        let path = std::env::temp_dir().join("panel-drop-fixture.md");
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        for i in [0, 1] {
            app.popup = Some(i);
            app.sessions[i].info.placement = Placement::Floating;
            let r = app.popup_window.rect(app.size);
            let sidebar = app.sidebar.as_mut().unwrap();
            sidebar.gesture = crate::sidebar::Gesture::Dragging { path: path.clone() };
            sidebar.checking_input = false;
            app.mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: r.x + 2,
                row: r.y + 2,
                modifiers: KeyModifiers::NONE,
            });
            let sidebar = app.sidebar.as_ref().unwrap();
            assert!(matches!(sidebar.gesture, crate::sidebar::Gesture::Idle));
            assert_eq!(
                sidebar.checking_input,
                i == 0,
                "only the visible Agent may accept a drop"
            );
        }
    }
    #[test]
    fn collapsing_the_agent_preserves_unsaved_file_editor_and_preview() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-collapse-editor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("draft.md");
        std::fs::write(&file, "# original\n").unwrap();
        let mut sidebar = crate::sidebar::Sidebar::new(root.clone());
        sidebar.open(file.clone(), true).unwrap();
        let until = |s: &mut crate::sidebar::Sidebar, editor: bool| {
            let at = Instant::now();
            loop {
                s.tick(true);
                let ready = s.document.as_ref().is_some_and(|d| {
                    if editor {
                        d.editor().is_some()
                    } else {
                        !d.loading && d.rendered().is_some()
                    }
                });
                if ready {
                    break;
                }
                assert!(at.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        until(&mut sidebar, false);
        sidebar
            .key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .unwrap();
        until(&mut sidebar, true);
        sidebar.paste("unsaved").unwrap();
        assert!(sidebar.dirty());
        let before = sidebar.document.as_ref().unwrap().editor().unwrap().text();
        let mut app = app();
        app.sessions[0].info.placement = Placement::Docked;
        app.sidebar = Some(sidebar);
        let work = WorkspaceInfo {
            tasks: vec![],
            id: app.workspace_id.clone(),
            project: root.clone(),
            options: app.options.clone(),
            sessions: app
                .sessions
                .iter()
                .map(|s| {
                    let mut i = s.info.clone();
                    i.placement = Placement::Collapsed;
                    i
                })
                .collect(),
        };
        assert!(app.sync_presentations(&work).unwrap());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let sidebar = app.sidebar.as_ref().unwrap();
        assert!(sidebar.dirty());
        assert_eq!(
            sidebar.document.as_ref().unwrap().editor().unwrap().text(),
            before
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), "# original\n");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn prefix_overlay_routes_real_events_and_never_leaks_mouse_or_release() {
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                let mut app = app();
                for (w, h) in [(60, 10), (100, 30), (180, 40)] {
                    app.size = Rect::new(0, 0, w, h);
                    let mut terminal =
                        ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
                    app.key(
                        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
                        None,
                        1,
                    );
                    assert!(app.mode == Mode::Prefix);
                    let rows = super::super::chrome::command_rows(app.size);
                    assert_eq!(rows.len(), super::super::chrome::COMMANDS.len());
                    for (r, _) in &rows {
                        assert!(
                            r.x >= app.size.x
                                && r.y >= app.size.y
                                && r.right() <= app.size.right()
                                && r.bottom() <= app.size.bottom()
                        );
                    }
                    terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
                    let (r, _) = rows.iter().find(|(_, key)| *key == 'w').unwrap();
                    app.event(crate::events::Input {
                        event: Event::Mouse(MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: r.x,
                            row: r.y,
                            modifiers: KeyModifiers::NONE,
                        }),
                        win32: None,
                        repeat: 1,
                    })
                    .unwrap();
                    assert!(app.mode == Mode::Normal);
                    assert_eq!(app.terminal_menu, Some(0));
                    assert!(app.consumed_keys.contains(&KeyCode::Char('w')));
                    let mut release = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
                    release.kind = KeyEventKind::Release;
                    app.key(release, None, 1);
                    assert!(!app.consumed_keys.contains(&KeyCode::Char('w')));
                    app.mouse(MouseEvent {
                        kind: MouseEventKind::Up(MouseButton::Left),
                        column: r.x,
                        row: r.y,
                        modifiers: KeyModifiers::NONE,
                    });
                    assert!(!app.chrome_capture);
                    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), None, 1);
                    app.mode = Mode::Prefix;
                    app.mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 0,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    });
                    assert!(app.mode == Mode::Normal);
                    app.mouse(MouseEvent {
                        kind: MouseEventKind::Up(MouseButton::Left),
                        column: 0,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    });
                    assert!(!app.chrome_capture);
                    assert!(app.selection.is_none() && app.message.is_empty());
                }
            });
        }
    }

    #[test]
    fn one_footer_and_focus_keep_blurred_terminal_controls_unhittable() {
        use crate::sidebar::Focus;
        let mut app = app();
        app.sessions[0].info.placement = Placement::Docked;
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        app.sidebar.as_mut().unwrap().focus = Focus::Tree;
        app.update_sidebar_layout();
        let r = app.rects()[0].1;
        assert_eq!(r.bottom(), 29);
        assert!(!app.terminal_focused(0));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        assert!(
            app.terminal_hits
                .iter()
                .any(|(r, a)| r.y == 29 && matches!(a, Action::Menu(_)))
        );
        assert!(
            !app.terminal_hits
                .iter()
                .any(|(r, a)| r.y == 0 && matches!(a, Action::Place(..) | Action::Float(_)))
        );
        app.hover = Some((r.x + 1, r.y + 1));
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        assert!(
            app.terminal_hits
                .iter()
                .any(|(r, a)| r.y == 0 && matches!(a, Action::Place(..)))
        );
        assert!(
            !app.terminal_focused(0),
            "hover must not steal keyboard focus"
        );
        app.hover = None;
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        assert!(
            !app.terminal_hits
                .iter()
                .any(|(r, a)| r.y == 0 && matches!(a, Action::Place(..)))
        );
    }

    #[test]
    fn explicit_zoom_restores_geometry_and_never_rewrites_manual_sizes() {
        use super::super::chrome::Zoom;
        use crate::sidebar::Focus;
        let mut app = app();
        app.size = Rect::new(0, 0, 200, 40);
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        app.sidebar.as_mut().unwrap().focus = Focus::Tree;
        app.update_sidebar_layout();
        let original = app.sidebar.as_ref().unwrap().geometry;
        // Keyboard toggles can be exercised with all terminals collapsed, without a host.
        for _ in 0..2 {
            app.mode = Mode::Prefix;
            app.key(
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
                None,
                1,
            );
        }
        assert!(app.zoom.is_none());
        assert_eq!(app.sidebar.as_ref().unwrap().geometry, original);
        for target in [Zoom::Tree, Zoom::Terminal(0), Zoom::Preview] {
            app.sidebar.as_mut().unwrap().focus = match target {
                Zoom::Tree => Focus::Tree,
                Zoom::Preview => Focus::Preview,
                Zoom::Terminal(_) => Focus::Terminal,
            };
            app.zoom = Some(target);
            app.update_sidebar_layout();
            if target == Zoom::Tree {
                assert_eq!(
                    app.sidebar.as_ref().unwrap().geometry.tree,
                    Some(Rect::new(0, 0, 200, 39))
                );
            }
            if target == Zoom::Terminal(0) {
                assert_eq!(app.rects(), vec![(0, Rect::new(0, 0, 200, 39))]);
            }
            app.zoom = None;
            app.update_sidebar_layout();
            assert_eq!(app.sidebar.as_ref().unwrap().geometry, original);
        }
        let unchanged = app.sidebar.as_ref().unwrap().geometry;
        app.sidebar.as_mut().unwrap().focus = Focus::Terminal;
        app.update_sidebar_layout();
        assert_eq!(
            app.sidebar.as_ref().unwrap().geometry,
            unchanged,
            "ordinary focus never resizes PTYs"
        );
        assert!(app.message.is_empty());
    }

    #[test]
    fn hidden_float_cannot_capture_zoomed_panel_mouse_and_messages_expire() {
        let mut app = app();
        app.popup = Some(1);
        app.zoom = Some(super::super::chrome::Zoom::Terminal(0));
        let r = app.popup_window.rect(app.size);
        assert!(!app.popup_drag(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x + 1,
            row: r.y,
            modifiers: KeyModifiers::NONE
        }));
        let now = Instant::now();
        app.message = "Done".into();
        assert!(!app.refresh_message(now));
        assert!(!app.refresh_message(now + Duration::from_secs(5)));
        assert!(app.refresh_message(now + Duration::from_secs(6)));
        assert!(app.message.is_empty());
    }
    #[test]
    fn enlarged_floating_terminal_is_rendered_and_restored_without_changing_bounds() {
        let mut app = app();
        app.popup = Some(1);
        app.sessions[1].info.placement = Placement::Floating;
        app.focus = 1;
        let original = app.popup_window.rect(app.size);
        app.zoom = Some(super::super::chrome::Zoom::Terminal(1));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let b = terminal.backend().buffer();
        assert_eq!(b[(0, 0)].symbol(), "┌");
        assert_eq!(b[(99, 28)].symbol(), "┘");
        assert_eq!(app.popup_window.rect(app.size), original);
        app.zoom = None;
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(original.x, original.y)].symbol(),
            "┌"
        );
    }
    #[test]
    fn closing_or_leaving_enlarged_target_restores_other_panels() {
        use super::super::chrome::Zoom;
        use crate::sidebar::Focus;
        let mut app = app();
        app.sidebar = Some(crate::sidebar::Sidebar::new(std::env::temp_dir()));
        app.zoom = Some(Zoom::Preview);
        app.sidebar.as_mut().unwrap().focus = Focus::Preview;
        app.update_sidebar_layout();
        assert!(app.zoom.is_none());
        assert!(app.sidebar.as_ref().unwrap().geometry.agent.is_some());
        app.zoom = Some(Zoom::Tree);
        app.sidebar.as_mut().unwrap().focus = Focus::Terminal;
        app.update_sidebar_layout();
        assert!(app.zoom.is_none());
        assert!(app.sidebar.as_ref().unwrap().geometry.agent.is_some());
    }
}
