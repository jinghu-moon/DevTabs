//! Task-local document contexts and same-frontend Agent composition.
use super::*;
use crate::{
    protocol::{Request, Response},
    tasks::{Plan, Role, Task},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Paragraph},
};
use std::{collections::BTreeMap, sync::mpsc};

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    #[default]
    Auto,
    Sides,
    Stack,
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutState {
    pub pair: Vec<String>,
    pub axis: Axis,
    pub ratio: u16,
    pub focus: Option<String>,
    pub focused_task: Option<String>,
}
#[derive(Default)]
pub struct State {
    pub view_leases: Option<BTreeMap<String, std::fs::File>>,
    pub review: super::review::State,
    pub tasks: Vec<Task>,
    pub context: String,
    pub sidebars: BTreeMap<String, crate::sidebar::Sidebar>,
    pub popups: BTreeMap<String, usize>,
    pub restore_popup: bool,
    pub layout: LayoutState,
    pub display: crate::preview::image::Display,
    pub drag: bool,
    pub drag_before: Option<u16>,
    pub dialog: Option<Dialog>,
    pub input: Option<InputTarget>,
    pub operations: std::collections::VecDeque<String>,
    pub operation_pending: Option<mpsc::Receiver<Result<Response>>>,
    pub operation_at: Option<Instant>,
    pub role_focus: Option<(String, Role)>,
}
#[derive(Clone)]
pub struct InputTarget {
    pub session: String,
    pub run: String,
    pub task: String,
    pub lease: Option<String>,
}
pub struct Dialog {
    source: String,
    fields: [String; 3],
    field: usize,
    plan: Option<Box<Plan>>,
    pending: Option<mpsc::Receiver<Result<Response>>>,
    error: String,
}

/// Geometry alone never changes focus, pinning, or the stored ratio.
pub fn split(area: Rect, axis: Axis, ratio: u16) -> Vec<Rect> {
    let sides = match axis {
        Axis::Auto => area.width >= 120,
        Axis::Sides => true,
        Axis::Stack => false,
    };
    let (length, minimum) = if sides {
        (area.width, 40)
    } else {
        (area.height, 10)
    };
    if area.width < 40 || area.height < 10 || length < minimum * 2 {
        return vec![area];
    }
    let ratio = if ratio == 0 { 5000 } else { ratio.min(9000) };
    let first = ((u32::from(length) * u32::from(ratio)) / 10000) as u16;
    let first = first.clamp(minimum, length - minimum);
    if sides {
        vec![
            Rect::new(area.x, area.y, first, area.height),
            Rect::new(area.x + first, area.y, length - first, area.height),
        ]
    } else {
        vec![
            Rect::new(area.x, area.y, area.width, first),
            Rect::new(area.x, area.y + first, area.width, length - first),
        ]
    }
}
fn split_many(area: Rect, axis: Axis, count: usize) -> Vec<Rect> {
    let sides = match axis {
        Axis::Auto => usize::from(area.width) >= 60 * count,
        Axis::Sides => true,
        Axis::Stack => false,
    };
    let length = if sides { area.width } else { area.height };
    let minimum = if sides { 40 } else { 10 };
    if usize::from(length) < minimum * count || area.width < 40 || area.height < 10 {
        return vec![area];
    }
    let size = length / count as u16;
    (0..count)
        .map(|i| {
            let start = i as u16 * size;
            let n = if i + 1 == count { length - start } else { size };
            if sides {
                Rect::new(area.x + start, area.y, n, area.height)
            } else {
                Rect::new(area.x, area.y + start, area.width, n)
            }
        })
        .collect()
}
impl App {
    pub(super) fn agents(&self) -> Vec<usize> {
        let mut seen = HashSet::new();
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.info.task.role == Role::Agent
                    && !s.info.task.task_id.is_empty()
                    && seen.insert(s.info.task.task_id.clone())
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub(super) fn role_index(&self, role: Role) -> Option<usize> {
        let task = &self.sessions[self.focus].info.task.task_id;
        self.sessions
            .iter()
            .position(|s| s.info.task.task_id == *task && s.info.task.role == role)
            .or_else(|| {
                if self.parallel.tasks.is_empty() {
                    match role {
                        Role::Agent => Some(0),
                        Role::Debug => (self.sessions.len() > 1).then_some(1),
                        _ => None,
                    }
                } else {
                    None
                }
            })
    }
    pub(super) fn secondary_index(&self) -> Option<usize> {
        let agent = self.role_index(Role::Agent).unwrap_or(self.focus);
        if self.focus != agent && self.docked(self.focus) {
            return Some(self.focus);
        }
        self.role_index(Role::Debug)
            .filter(|&i| self.docked(i))
            .or_else(|| {
                self.sessions
                    .iter()
                    .enumerate()
                    .find(|(i, s)| {
                        *i != agent
                            && s.info.task.task_id == self.sessions[agent].info.task.task_id
                            && self.docked(*i)
                    })
                    .map(|(i, _)| i)
            })
    }
    pub(super) fn parallel_rects(&self) -> Option<Vec<(usize, Rect)>> {
        let agents = self.agents();
        if agents.len() < 2 || self.view.is_some() {
            return None;
        }
        let mut area = if let Some(sidebar) = &self.sidebar {
            if sidebar.geometry.overlay {
                return Some(vec![]);
            }
            sidebar.geometry.agent?
        } else {
            chrome::body(self.size)
        };
        if area.width < 40 || area.height < 10 {
            return Some(vec![]);
        }
        let mut pair: Vec<_> = self
            .parallel
            .layout
            .pair
            .iter()
            .filter_map(|id| {
                agents
                    .iter()
                    .copied()
                    .find(|&i| self.sessions[i].info.task.task_id == *id && self.docked(i))
            })
            .take(2)
            .collect();
        if self.parallel.layout.pair.is_empty() {
            pair.extend(agents.into_iter().filter(|&i| self.docked(i)));
        }
        if pair.is_empty() {
            return Some(vec![]);
        }
        let debug = self.role_index(Role::Debug).filter(|&i| {
            self.options.layout == Layout::Split && self.docked(i) && area.height >= 30
        });
        let debug_rect = debug.map(|i| {
            let height = (area.height * 3 / 10).max(10);
            area.height -= height;
            (i, Rect::new(area.x, area.bottom(), area.width, height))
        });
        let rects = if pair.len() == 2 {
            split(area, self.parallel.layout.axis, self.parallel.layout.ratio)
        } else if pair.len() > 2 {
            split_many(area, self.parallel.layout.axis, pair.len())
        } else {
            vec![area]
        };
        if rects.len() == 1 && self.sessions[self.focus].info.task.role == Role::Agent {
            pair = vec![self.focus];
        }
        let mut out: Vec<_> = pair.into_iter().zip(rects).collect();
        out.extend(debug_rect);
        Some(out)
    }
    pub(super) fn switch_context(&mut self, index: usize) {
        let id = self.sessions[index].info.task.task_id.clone();
        if id.is_empty() || self.parallel.context == id || self.view.is_some() {
            return;
        }
        if let Some(i) = self.popup.take() {
            self.sessions[i].detach();
            self.parallel
                .popups
                .insert(self.parallel.context.clone(), i);
        } else {
            self.parallel.popups.remove(&self.parallel.context);
        }
        if let Some(mut old) = self.sidebar.take() {
            old.cancel_drag(true);
            old.images_occluded = true;
            old.tick(false);
            self.parallel
                .sidebars
                .insert(self.parallel.context.clone(), old);
        }
        let cwd = self.parallel.tasks.iter().find(|t| t.id == id).map_or_else(
            || self.sessions[index].info.task.initial_cwd.clone(),
            |t| t.cwd.clone(),
        );
        let mut sidebar = self
            .parallel
            .sidebars
            .remove(&id)
            .unwrap_or_else(|| crate::sidebar::Sidebar::new(cwd));
        sidebar.configure_images(self.parallel.display);
        self.sidebar = Some(sidebar);
        self.parallel.context = id;
        self.popup = self.parallel.popups.get(&self.parallel.context).copied();
        self.parallel.restore_popup = self.popup.is_some();
        self.zoom = None;
    }
    pub(super) fn select_task(&mut self, offset: isize) {
        let agents = self.agents();
        if agents.is_empty() {
            return;
        }
        let current = self.role_index(Role::Agent).unwrap_or(self.focus);
        let pos = agents.iter().position(|&i| i == current).unwrap_or(0);
        let i = agents[(pos as isize + offset).rem_euclid(agents.len() as isize) as usize];
        self.focus(i);
    }
    pub(super) fn pin_selected_task(&mut self, index: usize) {
        if self.agents().len() < 2 || self.sessions[index].info.task.role != Role::Agent {
            return;
        }
        let id = self.sessions[index].info.task.task_id.clone();
        if !self.parallel.layout.pair.is_empty() && !self.parallel.layout.pair.contains(&id) {
            self.zoom = Some(chrome::Zoom::Terminal(index));
        }
    }
    pub(super) fn toggle_task_pins(&mut self) {
        if self.parallel.layout.pair.is_empty() {
            self.parallel.layout.pair = self
                .rects()
                .into_iter()
                .filter(|(i, _)| self.sessions[*i].info.task.role == Role::Agent)
                .take(2)
                .map(|(i, _)| self.sessions[i].info.task.task_id.clone())
                .collect();
        } else {
            self.parallel.layout.pair.clear();
        }
        self.save_task_layout();
        let _ = self.attach_visible();
    }
    pub(super) fn replace_pinned_task(&mut self) {
        if self.parallel.layout.pair.len() == 2 {
            self.parallel.layout.pair[1] = self.sessions[self.focus].info.task.task_id.clone();
            self.zoom = None;
            self.save_task_layout();
            let _ = self.attach_visible();
        }
    }
    pub(super) fn cancel_task_drag(&mut self) {
        if let Some(before) = self.parallel.drag_before.take() {
            self.parallel.layout.ratio = before;
        }
        self.parallel.drag = false;
    }
    pub(super) fn save_task_layout(&mut self) {
        if uuid::Uuid::parse_str(&self.workspace_id).is_ok()
            && let Ok(bytes) = serde_json::to_vec(&self.parallel.layout)
        {
            let path = crate::config::home()
                .join("layouts")
                .join(format!("{}.json", self.workspace_id));
            if let Err(e) = crate::config::atomic_write(&path, &bytes) {
                self.message = format!("Save task layout: {e}");
            }
        }
    }
    pub(super) fn task_mouse(&mut self, event: MouseEvent) -> bool {
        if self.parallel.dialog.is_some() {
            return true;
        }
        let Some(mut rects) = self.parallel_rects() else {
            return false;
        };
        rects.retain(|(i, _)| self.sessions[*i].info.task.role == Role::Agent);
        if rects.len() != 2 || self.zoom.is_some() || self.popup.is_some() {
            self.parallel.drag = false;
            return false;
        }
        let a = rects[0].1;
        let b = rects[1].1;
        let sides = a.y == b.y;
        let point = if sides { event.column } else { event.row };
        let boundary = if sides { b.x } else { b.y };
        let in_bounds = if sides {
            event.row >= a.y && event.row < a.bottom()
        } else {
            event.column >= a.x && event.column < a.right()
        };
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && in_bounds
            && point.abs_diff(boundary) <= 1
        {
            self.parallel.drag = true;
            self.parallel.drag_before = Some(self.parallel.layout.ratio);
            return true;
        }
        if !self.parallel.drag {
            return false;
        }
        if matches!(event.kind, MouseEventKind::Drag(MouseButton::Left)) {
            let origin = if sides { a.x } else { a.y };
            let length = if sides {
                a.width + b.width
            } else {
                a.height + b.height
            };
            self.parallel.layout.ratio = ((u32::from(point.saturating_sub(origin)) * 10000)
                / u32::from(length.max(1)))
            .clamp(1000, 9000) as u16;
            let _ = self.attach_visible();
        }
        if matches!(event.kind, MouseEventKind::Up(_)) {
            self.parallel.drag = false;
            self.parallel.drag_before = None;
            self.save_task_layout();
        }
        true
    }
    pub(super) fn new_task(&mut self) {
        let source = self.sessions[self.focus].info.task.task_id.clone();
        self.parallel.dialog = Some(Dialog {
            source,
            fields: [String::new(), "HEAD".into(), String::new()],
            field: 0,
            plan: None,
            pending: None,
            error: String::new(),
        });
    }
    pub(super) fn start_task_role(&mut self, role: Role, fresh: bool) {
        let task = self.sessions[self.focus].info.task.task_id.clone();
        let expected_run = self
            .role_index(role)
            .map(|i| self.sessions[i].info.task.run_id.clone());
        let result = crate::protocol::rpc(Request::StartRole {
            workspace: self.workspace_id.clone(),
            task: task.clone(),
            role,
            fresh,
            expected_run,
        });
        match result {
            Ok(Response::Workspace(work)) => {
                if let Err(e) = self.sync_presentations(&work) {
                    self.message = format!("{e:#}");
                    return;
                }
                if let Some(i) = self.role_index(role) {
                    self.focus(i);
                }
            }
            Ok(Response::TaskOperation(op)) => {
                self.parallel.operations.push_back(op.plan.operation);
                self.parallel.role_focus = Some((task, role));
                self.message = crate::i18n::t("parallel.working").into_owned();
            }
            Err(e) => self.message = format!("{e:#}"),
            _ => self.message = "Unexpected role response".into(),
        }
    }
    pub(super) fn task_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.code == KeyCode::Esc {
            self.parallel.dialog = None;
            return;
        }
        let Some(d) = &mut self.parallel.dialog else {
            return;
        };
        if d.pending.is_some() {
            return;
        }
        match key.code {
            KeyCode::Tab => {
                d.field = (d.field + 1) % 3;
                d.plan = None;
            }
            KeyCode::BackTab => {
                d.field = (d.field + 2) % 3;
                d.plan = None;
            }
            KeyCode::Backspace => {
                d.fields[d.field].pop();
                d.plan = None;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && d.plan.is_none() =>
            {
                if d.fields[d.field].len() < 2048 && !c.is_control() {
                    d.fields[d.field].push(c);
                }
            }
            KeyCode::Enter => {
                let request = if let Some(p) = &d.plan {
                    Request::AddTask {
                        workspace: self.workspace_id.clone(),
                        operation: p.operation.clone(),
                    }
                } else {
                    Request::PlanTask {
                        workspace: self.workspace_id.clone(),
                        source: d.source.clone(),
                        name: d.fields[0].clone(),
                        base: d.fields[1].clone(),
                        path: (!d.fields[2].is_empty()).then(|| d.fields[2].clone().into()),
                    }
                };
                let (tx, rx) = mpsc::sync_channel(1);
                match std::thread::Builder::new()
                    .name("devtabs-task-form".into())
                    .spawn(move || {
                        let _ = tx.send(crate::protocol::rpc(request));
                    }) {
                    Ok(_) => d.pending = Some(rx),
                    Err(e) => d.error = e.to_string(),
                }
            }
            _ => {}
        }
    }
    pub(super) fn task_tick(&mut self) -> bool {
        let changed = self.operation_tick();
        let Some(d) = &mut self.parallel.dialog else {
            return changed;
        };
        let Some(rx) = &d.pending else {
            return changed;
        };
        let response = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return changed,
            Err(e) => Err(e.into()),
        };
        d.pending = None;
        match response {
            Ok(Response::TaskPlan(p)) => {
                d.plan = Some(p);
                d.error.clear();
            }
            Ok(Response::TaskOperation(op)) => {
                self.message = format!("{} · {}", op.plan.task.name, op.phase);
                self.parallel.operations.push_back(op.plan.operation);
                self.parallel.dialog = None;
            }
            Ok(_) => d.error = "Unexpected task response".into(),
            Err(e) => d.error = format!("{e:#}"),
        }
        true
    }
    fn operation_tick(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.parallel.operation_pending {
            match rx.try_recv() {
                Ok(Ok(Response::TaskOperation(op))) => {
                    self.parallel.operation_pending = None;
                    self.parallel.operations.pop_front();
                    if matches!(op.phase.as_str(), "Attached" | "Failed") {
                        self.message = format!(
                            "{} · {}{}",
                            op.plan.task.name,
                            op.phase,
                            op.error.map_or(String::new(), |e| format!(": {e}"))
                        );
                        changed = true;
                    } else {
                        self.parallel.operations.push_back(op.plan.operation);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                other => {
                    self.parallel.operation_pending = None;
                    self.parallel.operations.pop_front();
                    self.message = format!(
                        "Task status unavailable: {}",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            _ => "transport closed".into(),
                        }
                    );
                    changed = true;
                }
            }
        }
        if self.parallel.operation_pending.is_none()
            && self
                .parallel
                .operation_at
                .is_none_or(|at| at.elapsed() > Duration::from_millis(500))
            && let Some(operation) = self.parallel.operations.front().cloned()
        {
            let (tx, rx) = mpsc::sync_channel(1);
            if std::thread::Builder::new()
                .name("devtabs-task-progress".into())
                .spawn(move || {
                    let _ = tx.send(crate::protocol::rpc(Request::TaskOperation { operation }));
                })
                .is_ok()
            {
                self.parallel.operation_pending = Some(rx);
                self.parallel.operation_at = Some(Instant::now());
            }
        }
        changed
    }
    pub(super) fn task_paste(&mut self, text: &str) {
        if let Some(d) = &mut self.parallel.dialog
            && d.pending.is_none()
            && d.plan.is_none()
        {
            for c in text.chars().filter(|c| !c.is_control()) {
                if d.fields[d.field].len() + c.len_utf8() > 2048 {
                    break;
                }
                d.fields[d.field].push(c);
            }
        }
    }
    pub(super) fn input_target(&self) -> Option<InputTarget> {
        let i = self.role_index(Role::Agent)?;
        let info = &self.sessions[i].info;
        Some(InputTarget {
            session: info.id.clone(),
            run: info.task.run_id.clone(),
            task: info.task.task_id.clone(),
            lease: self.sessions[i].input_lease(),
        })
    }
    pub(super) fn resolve_input_target(&self, target: &InputTarget) -> Result<usize> {
        let i = self
            .sessions
            .iter()
            .position(|s| s.info.id == target.session)
            .context("Original input session disappeared")?;
        let info = &self.sessions[i].info;
        anyhow::ensure!(
            target.lease.is_some() && self.sessions[i].input_lease() == target.lease,
            "Original input lease ended; no retry was sent"
        );
        anyhow::ensure!(
            info.task.run_id == target.run
                && info.task.task_id == target.task
                && self.parallel.context == target.task,
            "Input context changed; inspect original Agent before retrying"
        );
        Ok(i)
    }
}
pub(crate) fn overlay(frame: &mut Frame, app: &App) {
    let Some(d) = &app.parallel.dialog else {
        return;
    };
    let area = frame.area();
    let width = area.width.min(100);
    let height = area.height.min(17);
    let r = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    crate::ui::clear_popup(frame, r);
    let mut lines = vec![crate::i18n::t("parallel.form_hint").into_owned()];
    for (i, key) in ["parallel.name", "parallel.base", "parallel.path"]
        .iter()
        .enumerate()
    {
        lines.push(format!(
            "{} {}: {}",
            if i == d.field { "▸" } else { " " },
            crate::i18n::t(key),
            d.fields[i]
        ));
    }
    if let Some(p) = &d.plan {
        lines.extend([
            format!("{} {}", crate::i18n::t("parallel.frozen"), p.task.base),
            p.task.branch.clone(),
            p.task.checkout.display().to_string(),
            crate::i18n::t("parallel.confirm").into_owned(),
        ]);
    }
    if d.pending.is_some() {
        lines.push(crate::i18n::t("parallel.working").into_owned());
    }
    if !d.error.is_empty() {
        lines.push(d.error.clone());
    }
    frame.render_widget(
        Paragraph::new(lines.join("\n"))
            .block(Block::bordered().title(crate::i18n::t("parallel.new_task")))
            .style(Style::default().bg(crate::appearance::palette().bg)),
        r,
    );
}
