//! DevTabs-owned file tables beneath docked Agent PTYs.
use super::*;
use crate::{
    activity::Phase,
    protocol::{Request, Response},
    references::{Cursor, Reference},
    rounds::service::{Action, View},
    tasks::Role,
};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{
        Block, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    },
};
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub height: u16,
    pub collapsed: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            height: 7,
            collapsed: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    run: String,
    conversation: String,
    generation: u64,
    result: u64,
    turn: Option<String>,
    transcript: Option<PathBuf>,
    root: PathBuf,
}
struct Reply {
    identity: Identity,
    cursor: Cursor,
    references: Result<Vec<Reference>>,
    changes: Result<View>,
    partial: bool,
}
#[derive(Default)]
struct Entry {
    identity: Option<Identity>,
    cursor: Cursor,
    pending: Option<mpsc::Receiver<Reply>>,
    references: Vec<Reference>,
    view: View,
    reference_error: Option<String>,
    change_error: Option<String>,
    partial: bool,
    attempts: u8,
    retry_at: Option<Instant>,
    offsets: [usize; 2],
}
impl Entry {
    fn receive(&mut self, wanted: Option<&Identity>, reply: Reply) -> bool {
        if wanted == Some(&reply.identity) {
            self.cursor = reply.cursor;
            self.partial = reply.partial;
            match reply.references {
                Ok(refs) => {
                    self.references = refs;
                    self.reference_error = None;
                }
                Err(err) => {
                    self.references.clear();
                    self.reference_error = Some(err.to_string());
                }
            }
            match reply.changes {
                Ok(view) => {
                    self.change_error = view.error.clone();
                    self.view = view;
                }
                Err(err) => {
                    self.view = View::default();
                    self.change_error = Some(err.to_string());
                }
            }
            if self.attempts < 2 || self.change_error.is_some() || self.reference_error.is_some() {
                self.retry_at = Some(
                    Instant::now() + Duration::from_millis(500 * u64::from(self.attempts.max(1))),
                );
            }
        } else {
            self.identity = None;
            return false;
        }
        true
    }
}
#[derive(Default)]
pub struct State {
    entries: BTreeMap<String, Entry>,
    drag: Option<Drag>,
    pointer: Option<Pointer>,
}
struct Pointer {
    key: String,
    side: usize,
    body: Rect,
    scroll: bool,
}
struct Drag {
    key: String,
    area: Rect,
    before: Settings,
}

/// The panel gets real rows; even a tiny workspace preserves an eight-row terminal.
pub fn split(area: Rect, settings: Settings) -> (Rect, Rect) {
    let available = area.height.saturating_sub(8);
    let height = if available == 0 {
        0
    } else if settings.collapsed || available < 5 {
        1
    } else {
        settings.height.clamp(5, available)
    };
    (
        Rect {
            height: area.height - height,
            ..area
        },
        Rect::new(area.x, area.bottom() - height, area.width, height),
    )
}
fn key(info: &crate::protocol::SessionInfo) -> String {
    if info.task.task_id.is_empty() {
        info.id.clone()
    } else {
        info.task.task_id.clone()
    }
}
fn identity(info: &crate::protocol::SessionInfo) -> Option<Identity> {
    let a = &info.activity;
    if info.task.role != Role::Agent
        || a.result == 0
        || a.ended.is_none()
        || matches!(
            a.phase,
            Phase::Working | Phase::Permission | Phase::Input | Phase::Unknown
        )
    {
        return None;
    }
    Some(Identity {
        run: info.task.run_id.clone(),
        conversation: a.conversation.clone()?,
        generation: a.generation,
        result: a.result,
        turn: a.turn.clone(),
        transcript: a.transcript.clone(),
        root: info.task.initial_cwd.clone(),
    })
}
impl App {
    pub(super) fn file_terminal_rect(&self, i: usize, area: Rect) -> Rect {
        if self.file_panel_enabled(i) {
            split(area, self.file_settings(i)).0
        } else {
            area
        }
    }
    fn file_settings(&self, i: usize) -> Settings {
        self.parallel
            .layout
            .file_panels
            .get(&key(&self.sessions[i].info))
            .copied()
            .unwrap_or_default()
    }
    fn file_panel_enabled(&self, i: usize) -> bool {
        self.view.is_none() && self.zoom.is_none() && self.sessions[i].info.task.role == Role::Agent
    }
    pub(crate) fn file_panels(&self) -> Vec<(usize, Rect)> {
        self.docked_rects()
            .into_iter()
            .filter(|(i, _)| self.file_panel_enabled(*i))
            .map(|(i, r)| (i, split(r, self.file_settings(i)).1))
            .filter(|(_, r)| r.height > 0)
            .collect()
    }
    pub(super) fn toggle_file_panel(&mut self) {
        let Some(i) = self.role_index(Role::Agent) else {
            return;
        };
        let settings = self
            .parallel
            .layout
            .file_panels
            .entry(key(&self.sessions[i].info))
            .or_default();
        settings.collapsed = !settings.collapsed;
        self.selection = None;
        self.save_task_layout();
        let _ = self.attach_visible();
    }
    pub(super) fn cancel_file_drag(&mut self) -> bool {
        let pointer = self.parallel.files.pointer.take().is_some();
        if let Some(drag) = self.parallel.files.drag.take() {
            self.parallel
                .layout
                .file_panels
                .insert(drag.key, drag.before);
            return true;
        }
        pointer
    }
    pub(super) fn file_dragging(&self) -> bool {
        self.parallel.files.drag.is_some() || self.parallel.files.pointer.is_some()
    }
    pub(super) fn files_tick(&mut self) -> bool {
        let mut changed = false;
        let live: std::collections::BTreeSet<_> =
            self.sessions.iter().map(|s| key(&s.info)).collect();
        self.parallel.files.entries.retain(|k, _| live.contains(k));
        let mut running = self
            .parallel
            .files
            .entries
            .values()
            .filter(|e| e.pending.is_some())
            .count();
        for session in &self.sessions {
            if session.info.task.role != Role::Agent {
                continue;
            }
            let k = key(&session.info);
            let wanted = identity(&session.info);
            let e = self.parallel.files.entries.entry(k.clone()).or_default();
            // A restarted Agent must never inherit another run's paths or reader cursor.
            if e.identity
                .as_ref()
                .is_some_and(|old| old.run != session.info.task.run_id)
            {
                *e = Entry::default();
                changed = true;
            }
            if let Some(rx) = &e.pending {
                match rx.try_recv() {
                    Ok(reply) => {
                        e.pending = None;
                        running = running.saturating_sub(1);
                        // Late replies are discarded after task/run/turn changes.
                        changed |= e.receive(wanted.as_ref(), reply);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        e.pending = None;
                        running = running.saturating_sub(1);
                        e.change_error = Some("Worker disconnected".into());
                        changed = true;
                    }
                    Err(_) => {}
                }
            }
            let Some(wanted) = wanted else { continue };
            if e.identity.as_ref() != Some(&wanted) && e.pending.is_none() {
                // Keep the previous completed turn visible while the next one runs;
                // once its result arrives replace both tables together.
                e.identity = Some(wanted.clone());
                e.attempts = 0;
                e.retry_at = None;
                e.reference_error = None;
                e.change_error = None;
                e.references.clear();
                e.view = View::default();
                e.offsets = [0, 0];
                changed = true;
            }
            let retry = e.attempts == 0
                || (e.attempts < 4 && e.retry_at.is_some_and(|at| Instant::now() >= at));
            if e.pending.is_some() || running >= 2 || !retry {
                continue;
            }
            let cursor = std::mem::take(&mut e.cursor);
            let agent = self
                .parallel
                .tasks
                .iter()
                .find(|t| t.id == session.info.task.task_id)
                .map_or(self.options.agent, |t| t.options.agent);
            let workspace = self.workspace_id.clone();
            let task = session.info.task.task_id.clone();
            let (tx, rx) = mpsc::channel();
            let wanted = wanted.clone();
            e.attempts += 1;
            e.retry_at = None;
            match std::thread::Builder::new()
                .name("devtabs-file-panel".into())
                .spawn(move || {
                    let mut cursor = cursor;
                    let references = wanted
                        .transcript
                        .as_ref()
                        .context("No transcript path reported by Agent hooks")
                        .and_then(|path| {
                            cursor.read(
                                path,
                                &wanted.root,
                                agent,
                                &wanted.conversation,
                                wanted.turn.as_deref(),
                            )
                        });
                    let partial = cursor.partial;
                    let changes = crate::protocol::rpc(Request::Review {
                        workspace,
                        task,
                        action: Action::Result {
                            run: wanted.run.clone(),
                            result: wanted.result,
                        },
                    })
                    .and_then(|r| match r {
                        Response::Review(v) => Ok(*v),
                        Response::Error(e) => anyhow::bail!("{e}"),
                        _ => anyhow::bail!("Unexpected review response"),
                    });
                    let _ = tx.send(Reply {
                        identity: wanted,
                        cursor,
                        references,
                        changes,
                        partial,
                    });
                }) {
                Ok(_) => {
                    e.pending = Some(rx);
                    running += 1;
                    changed = true;
                }
                Err(err) => {
                    e.change_error = Some(err.to_string());
                    changed = true;
                }
            }
        }
        changed
    }
    pub(super) fn files_mouse(&mut self, event: MouseEvent) -> bool {
        if let Some(pointer) = &self.parallel.files.pointer
            && matches!(event.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_))
        {
            if pointer.scroll
                && let Some(e) = self.parallel.files.entries.get_mut(&pointer.key)
            {
                let count = if pointer.side == 0 {
                    e.references.len()
                } else {
                    e.view.paths.len()
                };
                let max = count.saturating_sub(usize::from(pointer.body.height));
                let row = event
                    .row
                    .saturating_sub(pointer.body.y)
                    .min(pointer.body.height.saturating_sub(1));
                e.offsets[pointer.side] = usize::from(row) * max
                    / usize::from(pointer.body.height.saturating_sub(1).max(1));
            }
            if matches!(event.kind, MouseEventKind::Up(_)) {
                self.parallel.files.pointer = None;
            }
            return true;
        }
        // A fresh press starts a new gesture even if a lost release left capture active.
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.parallel.files.pointer = None;
        }
        if self.parallel.dialog.is_some()
            || self.terminal_menu.is_some()
            || self.mode != Mode::Normal
            || self.sidebar.as_ref().is_some_and(|s| s.modal())
        {
            return false;
        }
        if let Some(drag) = &self.parallel.files.drag {
            if matches!(event.kind, MouseEventKind::Drag(MouseButton::Left)) {
                let height = drag
                    .area
                    .bottom()
                    .saturating_sub(event.row)
                    .clamp(5, drag.area.height.saturating_sub(8).max(5));
                let settings = self
                    .parallel
                    .layout
                    .file_panels
                    .entry(drag.key.clone())
                    .or_default();
                settings.height = height;
                settings.collapsed = false;
                self.selection = None;
                let _ = self.attach_visible();
            }
            if matches!(event.kind, MouseEventKind::Up(_)) {
                self.parallel.files.drag = None;
                self.save_task_layout();
            }
            return true;
        }
        let Some((i, area)) = self
            .file_panels()
            .into_iter()
            .find(|(_, r)| r.contains((event.column, event.row).into()))
        else {
            return false;
        };
        let k = key(&self.sessions[i].info);
        if matches!(event.kind, MouseEventKind::Down(_)) {
            let side = usize::from(event.column >= area.x + area.width / 2);
            self.parallel.files.pointer = Some(Pointer {
                key: k.clone(),
                side,
                body: table_body(area, side),
                scroll: false,
            });
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) && event.row == area.y {
            let controls_right = area.right().saturating_sub(u16::from(area.height > 1));
            if event.column >= controls_right {
                return true;
            }
            if event.column >= controls_right.saturating_sub(3) {
                let settings = self.parallel.layout.file_panels.entry(k).or_default();
                settings.collapsed = !settings.collapsed;
                self.selection = None;
                self.save_task_layout();
                let _ = self.attach_visible();
            } else if event.column >= controls_right.saturating_sub(7) {
                if let Some(e) = self.parallel.files.entries.get_mut(&k) {
                    e.attempts = 0;
                }
            } else if let Some((_, r)) = self.docked_rects().into_iter().find(|(n, _)| *n == i) {
                self.parallel.files.pointer = None;
                self.parallel.files.drag = Some(Drag {
                    key: k,
                    area: r,
                    before: self.file_settings(i),
                });
            }
            return true;
        }
        let Some(e) = self.parallel.files.entries.get_mut(&k) else {
            return true;
        };
        let side = usize::from(event.column >= area.x + area.width / 2);
        let body = table_body(area, side);
        let count = if side == 0 {
            e.references.len()
        } else {
            e.view.paths.len()
        };
        e.offsets[side] = e.offsets[side].min(count.saturating_sub(usize::from(body.height)));
        if count > usize::from(body.height)
            && event.column == body.right()
            && event.row >= body.y
            && event.row < body.bottom()
            && matches!(
                event.kind,
                MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
            )
        {
            if let Some(pointer) = &mut self.parallel.files.pointer {
                pointer.scroll = true;
            }
            let max = count.saturating_sub(usize::from(body.height));
            e.offsets[side] = usize::from(event.row - body.y) * max
                / usize::from(body.height.saturating_sub(1).max(1));
            return true;
        }
        match event.kind {
            MouseEventKind::ScrollDown => {
                e.offsets[side] =
                    (e.offsets[side] + 3).min(count.saturating_sub(usize::from(body.height)))
            }
            MouseEventKind::ScrollUp => e.offsets[side] = e.offsets[side].saturating_sub(3),
            MouseEventKind::Down(MouseButton::Left)
                if body.contains((event.column, event.row).into()) =>
            {
                let n = e.offsets[side] + usize::from(event.row - body.y);
                if side == 0 {
                    let Some(reference) = e.references.get(n).cloned() else {
                        return true;
                    };
                    self.switch_context(i);
                    self.focus = i;
                    if self.sidebar.is_none() {
                        self.sidebar = Some(crate::sidebar::Sidebar::new(
                            self.sessions[i].info.task.initial_cwd.clone(),
                        ));
                    }
                    if let Some(s) = &mut self.sidebar
                        && let Err(error) = s.open(reference.path, true).and_then(|_| {
                            if let Some(line) = reference.line {
                                s.reveal_line(line)?;
                            }
                            Ok(())
                        })
                    {
                        s.say(error.to_string(), false);
                    }
                    let _ = self.attach_visible();
                } else if n < e.view.paths.len()
                    && let Some(r) = e.view.records.first()
                {
                    let round = r.id.clone();
                    let task = self.sessions[i].info.task.task_id.clone();
                    self.open_file_change(task, round, n);
                }
            }
            _ => {}
        }
        true // Never leak panel mouse events into the Agent's PTY.
    }
}
fn table_body(area: Rect, side: usize) -> Rect {
    let half = area.width / 2;
    Rect::new(
        area.x + if side == 0 { 1 } else { half + 1 },
        area.y + 2,
        if side == 0 {
            half.saturating_sub(3)
        } else {
            area.width.saturating_sub(half + 3)
        },
        area.height.saturating_sub(4),
    )
}
fn status(change: Option<&crate::rounds::Change>, observe: bool) -> (&'static str, Color) {
    if observe {
        return ("observed", Color::Yellow);
    }
    match change {
        Some(c) if c.reason.is_some() => ("unavailable", Color::Yellow),
        Some(c) if c.before.is_none() && c.after.is_some() => ("added", Color::Green),
        Some(c) if c.before.is_some() && c.after.is_none() => ("deleted", Color::Red),
        Some(_) => ("modified", Color::Cyan),
        None => ("recorded", crate::appearance::palette().muted),
    }
}
pub(crate) fn draw(frame: &mut Frame, app: &mut App) {
    let _perf = crate::perf::span(crate::perf::Metric::FilePanelDraw);
    let palette = crate::appearance::palette();
    for (i, area) in app.file_panels() {
        let border_style = crate::ui::terminal_border_style(app, i);
        let k = key(&app.sessions[i].info);
        let e = app.parallel.files.entries.entry(k).or_default();
        frame.render_widget(
            if area.height > 1 {
                Block::bordered().border_style(border_style)
            } else {
                Block::default()
            }
            .style(crate::appearance::style()),
            area,
        );
        if area.height == 1 {
            frame.render_widget(
                Paragraph::new("─".repeat(usize::from(area.width))).style(border_style),
                area,
            );
        }
        if area.height > 1 {
            for y in area.y + 1..area.bottom() - 1 {
                frame.render_widget(
                    Paragraph::new("│").style(border_style),
                    Rect::new(area.x + area.width / 2, y, 1, 1),
                );
            }
        }
        if area.height > 1 {
            for (y, symbol) in [(area.y, "┬"), (area.bottom() - 1, "┴")] {
                frame.render_widget(
                    Paragraph::new(symbol).style(border_style),
                    Rect::new(area.x + area.width / 2, y, 1, 1),
                );
            }
        }
        let refs = e.references.len();
        let changes = e.view.records.first().map_or(0, |r| r.file_count);
        let generation = e.identity.as_ref().map_or(0, |id| id.generation);
        let half = area.width / 2;
        let inset = u16::from(area.height > 1);
        let heading = |label: &str, count| format!("─ {} {count} ", crate::i18n::t(label));
        let left = if area.height == 1 {
            format!(
                "{} · {} {changes}",
                heading("files_panel.references", refs),
                crate::i18n::t("files_panel.changes")
            )
        } else {
            heading("files_panel.references", refs)
        };
        frame.render_widget(
            Paragraph::new(left).style(Style::new().fg(palette.accent)),
            Rect::new(
                area.x + inset,
                area.y,
                if area.height == 1 {
                    area.width.saturating_sub(8)
                } else {
                    half.saturating_sub(inset)
                },
                1,
            ),
        );
        if area.height > 1 {
            frame.render_widget(
                Paragraph::new(format!(
                    "{} · #{}",
                    heading("files_panel.changes", changes),
                    generation
                ))
                .style(Style::new().fg(palette.accent)),
                Rect::new(
                    area.x + half + inset,
                    area.y,
                    area.width.saturating_sub(half + 8 + inset),
                    1,
                ),
            );
        }
        frame.render_widget(
            Paragraph::new(if area.height == 1 {
                "[↻] [+]"
            } else {
                "[↻] [−]"
            })
            .style(Style::new().fg(palette.accent)),
            Rect::new(area.right().saturating_sub(7 + inset), area.y, 7, 1),
        );
        if area.height == 1 {
            continue;
        }
        for side in 0..2 {
            let body = table_body(area, side);
            let count = if side == 0 { refs } else { e.view.paths.len() };
            e.offsets[side] = e.offsets[side].min(count.saturating_sub(usize::from(body.height)));
            let offset = e.offsets[side];
            let root = &app.sessions[i].info.task.initial_cwd;
            let rows: Vec<Row> = if side == 0 {
                e.references
                    .iter()
                    .skip(offset)
                    .take(usize::from(body.height))
                    .map(|r| {
                        Row::new([
                            Cell::from(crate::preview::safe_text(
                                &r.path
                                    .strip_prefix(root)
                                    .unwrap_or(&r.path)
                                    .to_string_lossy(),
                            )),
                            Cell::from(r.line.map_or_else(|| "—".into(), |n| n.to_string())),
                        ])
                    })
                    .collect()
            } else {
                let record = e.view.records.first();
                e.view
                    .paths
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(usize::from(body.height))
                    .map(|(n, path)| {
                        let change = record.and_then(|r| {
                            n.checked_sub(e.view.offset).and_then(|n| r.changes.get(n))
                        });
                        let (label, color) = status(
                            change,
                            record.is_some_and(|r| r.mode == crate::rounds::Mode::Observe),
                        );
                        Row::new([
                            Cell::from(crate::preview::safe_text(
                                &path.strip_prefix(root).unwrap_or(path).to_string_lossy(),
                            )),
                            Cell::from(
                                crate::i18n::t(&format!("files_panel.{label}")).into_owned(),
                            )
                            .style(Style::new().fg(color)),
                        ])
                    })
                    .collect()
            };
            let header = Row::new([
                crate::i18n::t("files_panel.file").into_owned(),
                crate::i18n::t(if side == 0 {
                    "files_panel.line"
                } else {
                    "files_panel.status"
                })
                .into_owned(),
            ])
            .style(Style::new().fg(palette.muted));
            frame.render_widget(
                Table::new(
                    rows,
                    [
                        Constraint::Min(1),
                        Constraint::Length(if side == 0 { 5 } else { 8 }),
                    ],
                )
                .column_spacing(1)
                .header(header),
                Rect::new(body.x, area.y + 1, body.width, body.height + 1),
            );
            if count > usize::from(body.height) && body.height > 0 {
                let mut state =
                    ScrollbarState::new(count.saturating_sub(usize::from(body.height)) + 1)
                        .position(offset);
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None),
                    Rect::new(body.right(), body.y, 1, body.height),
                    &mut state,
                );
            }
            if count == 0 && body.height > 0 {
                let label = if e.pending.is_some() {
                    "files_panel.loading"
                } else if (side == 0 && e.reference_error.is_some())
                    || (side == 1 && e.change_error.is_some())
                {
                    "files_panel.unavailable_hint"
                } else if e.identity.is_none() {
                    "files_panel.waiting"
                } else {
                    "files_panel.empty"
                };
                frame.render_widget(
                    Paragraph::new(crate::i18n::t(label)).style(Style::new().fg(palette.muted)),
                    body,
                );
            }
        }
        let error = e.reference_error.as_ref().or(e.change_error.as_ref());
        let label = if e.partial {
            "files_panel.partial"
        } else if e
            .view
            .records
            .first()
            .is_some_and(|r| r.late || !r.reasons.is_empty())
        {
            "files_panel.limited"
        } else {
            "files_panel.hint"
        };
        frame.render_widget(
            Paragraph::new(Line::from(if let Some(error) = error {
                format!(
                    "{}: {}",
                    crate::i18n::t("files_panel.error"),
                    crate::preview::safe_text(error)
                )
            } else {
                crate::i18n::t(label).into_owned()
            }))
            .style(Style::new().fg(palette.muted)),
            Rect::new(area.x + 1, area.bottom() - 2, half.saturating_sub(2), 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        i18n::{self, Language},
        presentation::Placement,
    };
    fn app() -> App {
        let mut app = super::super::terminals::tests::app();
        app.sessions.truncate(2);
        app.size = Rect::new(0, 0, 200, 50);
        app.options.layout = Layout::Float;
        for (n, session) in app.sessions.iter_mut().enumerate() {
            session.info.placement = Placement::Docked;
            session.info.task.task_id = format!("task-{n}");
            session.info.task.run_id = format!("run-{n}");
            session.info.task.initial_cwd = PathBuf::from(format!("D:/work/task-{n}"));
            *session.info.activity = crate::activity::State {
                phase: Phase::Reported,
                conversation: Some(format!("conv-{n}")),
                generation: 1,
                result: 1,
                ended: Some(10),
                ..Default::default()
            };
        }
        app
    }
    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }
    fn row_text(buffer: &ratatui::buffer::Buffer, area: Rect) -> String {
        use unicode_width::UnicodeWidthStr;
        (area.y..area.bottom())
            .map(|y| {
                let mut text = String::new();
                let mut x = area.x;
                while x < area.right() {
                    let symbol = buffer[(x, y)].symbol();
                    text.push_str(symbol);
                    x += symbol.width().max(1) as u16;
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn real_geometry_partitions_without_overlap_and_keeps_minimum_pty_rows() {
        for width in [20, 40, 80, 200] {
            for height in 0..80 {
                for requested in [0, 7, 40, u16::MAX] {
                    for collapsed in [true, false] {
                        let area = Rect::new(3, 5, width, height);
                        let (pty, panel) = split(
                            area,
                            Settings {
                                height: requested,
                                collapsed,
                            },
                        );
                        assert_eq!(pty.height + panel.height, area.height);
                        assert_eq!(panel.bottom(), area.bottom());
                        assert!(pty.intersection(panel).is_empty());
                        if height >= 8 {
                            assert!(pty.height >= 8);
                        }
                        if collapsed && height > 8 {
                            assert_eq!(panel.height, 1);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn parallel_panels_preserve_task_layouts_and_hide_for_zoom_and_floating_terminal() {
        let mut app = app();
        let panels = app.file_panels();
        assert_eq!(panels.len(), 2);
        assert!(panels[0].1.intersection(panels[1].1).is_empty());
        let terminals = app.rects();
        for (i, panel) in &panels {
            let pty = terminals.iter().find(|(n, _)| n == i).unwrap().1;
            assert_eq!(pty.bottom(), panel.y);
        }
        app.parallel.layout.file_panels.insert(
            "task-1".into(),
            Settings {
                collapsed: true,
                height: 12,
            },
        );
        let panels = app.file_panels();
        assert_eq!(panels[0].1.height, 7);
        assert_eq!(panels[1].1.height, 1);
        let json = serde_json::to_string(&app.parallel.layout).unwrap();
        let restored: super::super::parallel::LayoutState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.file_panels["task-1"].height, 12);
        app.zoom = Some(super::super::chrome::Zoom::Terminal(0));
        assert!(app.file_panels().is_empty());
        app.zoom = None;
        app.sessions[1].info.placement = Placement::Collapsed;
        app.popup = Some(1);
        assert!(app.file_panels().iter().all(|(i, _)| *i == 0));
    }
    #[test]
    fn stopped_turns_only_and_matching_replies_never_mix_runs_or_tasks() {
        let mut app = app();
        let info = &mut app.sessions[0].info;
        let wanted = identity(info).unwrap();
        info.activity.phase = Phase::Working;
        assert!(identity(info).is_none());
        info.activity.phase = Phase::Permission;
        assert!(identity(info).is_none());
        info.activity.phase = Phase::Reported;
        let reply = |id: Identity| Reply {
            identity: id,
            cursor: Cursor::default(),
            references: Ok(vec![Reference {
                path: "confirmed.rs".into(),
                line: None,
            }]),
            changes: Ok(View::default()),
            partial: false,
        };
        let mut e = Entry {
            identity: Some(wanted.clone()),
            attempts: 1,
            ..Default::default()
        };
        assert!(e.receive(Some(&wanted), reply(wanted.clone())));
        assert_eq!(e.references.len(), 1);
        let mut other = wanted.clone();
        other.run = "different-run".into();
        assert!(!e.receive(Some(&other), reply(wanted.clone())));
        assert_eq!(e.references.len(), 1);
        assert!(!e.receive(None, reply(wanted.clone())));
        other = wanted.clone();
        other.result += 1;
        assert!(!e.receive(Some(&other), reply(wanted.clone())));
        app.parallel.files.entries.insert(
            "task-0".into(),
            Entry {
                identity: Some(wanted.clone()),
                attempts: 1,
                ..Default::default()
            },
        );
        let other = identity(&app.sessions[1].info).unwrap();
        app.parallel.files.entries.insert(
            "task-1".into(),
            Entry {
                identity: Some(other),
                attempts: 1,
                ..Default::default()
            },
        );
        for _ in 0..100 {
            assert!(!app.files_tick());
        }
        assert!(
            app.parallel
                .files
                .entries
                .values()
                .all(|e| e.pending.is_none())
        );
    }
    #[test]
    fn expanded_file_panels_keep_the_outer_border_and_table_cells_inside_it() {
        let mut app = app();
        for (width, height) in [(200, 50), (100, 32), (80, 24)] {
            app.size = Rect::new(0, 0, width, height);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            for (_, area) in app.file_panels() {
                if area.height <= 1 {
                    continue;
                }
                let buffer = terminal.backend().buffer();
                assert_eq!(buffer[(area.x, area.y)].symbol(), "┌");
                assert_eq!(buffer[(area.right() - 1, area.y)].symbol(), "┐");
                assert_eq!(buffer[(area.x, area.bottom() - 1)].symbol(), "└");
                assert_eq!(buffer[(area.right() - 1, area.bottom() - 1)].symbol(), "┘");
                for y in area.y + 1..area.bottom() - 1 {
                    for x in [area.x, area.x + area.width / 2, area.right() - 1] {
                        assert_eq!(buffer[(x, y)].symbol(), "│");
                    }
                }
                for side in 0..2 {
                    let body = table_body(area, side);
                    assert!(body.x > area.x && body.right() < area.right());
                    assert!(body.y > area.y && body.bottom() < area.bottom());
                }
                for x in area.x + 1..area.right() - 1 {
                    assert_eq!(
                        buffer[(x, area.bottom() - 1)].symbol(),
                        if x == area.x + area.width / 2 {
                            "┴"
                        } else {
                            "─"
                        }
                    );
                }
            }
        }
    }
    #[test]
    fn file_panel_border_colors_follow_the_agent_focus_and_attention_state() {
        let mut app = app();
        for focus in [0, 1, 0, 1] {
            for session in &mut app.sessions {
                session.info.activity.phase = Phase::Input;
                session.info.activity.ended = Some(crate::config::now());
            }
            app.focus = focus;
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 50)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            for (i, area) in app.file_panels() {
                let terminal_area = app.rects().into_iter().find(|(n, _)| *n == i).unwrap().1;
                let color = terminal.backend().buffer()[(terminal_area.x, terminal_area.y + 1)].fg;
                for (x, y) in [
                    (area.x, area.y + 1),
                    (area.right() - 1, area.y + 1),
                    (area.x + 2, area.bottom() - 1),
                    (area.x + area.width / 2, area.y + 1),
                ] {
                    assert_eq!(terminal.backend().buffer()[(x, y)].fg, color);
                }
            }
        }
    }
    #[test]
    fn bordered_panel_controls_remain_clickable_without_treating_the_corner_as_a_button() {
        let mut app = app();
        let area = app.file_panels()[0].1;
        app.parallel.files.entries.insert(
            "task-0".into(),
            Entry {
                attempts: 3,
                ..Default::default()
            },
        );
        assert!(app.files_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.right() - 1,
            area.y,
        )));
        assert!(!app.file_settings(0).collapsed);
        app.cancel_file_drag();
        assert!(app.files_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.right() - 7,
            area.y,
        )));
        assert_eq!(app.parallel.files.entries["task-0"].attempts, 0);
        app.cancel_file_drag();
        assert!(app.files_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.right() - 2,
            area.y,
        )));
        assert!(app.file_settings(0).collapsed);
        assert!(!app.file_settings(1).collapsed);
        app.cancel_file_drag();
        let summary = app.file_panels()[0].1;
        assert_eq!(summary.height, 1);
        assert!(app.files_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            summary.right() - 2,
            summary.y,
        )));
        assert!(!app.file_settings(0).collapsed);
    }
    #[test]
    fn tables_have_headers_isolated_scroll_bars_and_mouse_never_reaches_pty() {
        i18n::with_language(Language::ZhCn, || {
            let mut app = app();
            for (n, s) in app.sessions.iter().enumerate() {
                let identity = identity(&s.info).unwrap();
                let references = (0..30)
                    .map(|i| Reference {
                        path: identity.root.join(format!("task{n}-file{i}.rs")),
                        line: Some(i + 1),
                    })
                    .collect();
                app.parallel.files.entries.insert(
                    key(&s.info),
                    Entry {
                        identity: Some(identity),
                        references,
                        attempts: 1,
                        ..Default::default()
                    },
                );
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 50)).unwrap();
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let panels = app.file_panels();
            let area = panels[0].1;
            let text = row_text(terminal.backend().buffer(), area);
            assert!(text.contains("引用文件 30"));
            assert!(text.contains("修改文件"));
            assert!(text.contains("文件"));
            assert!(text.contains("行号"));
            assert!(text.contains("状态"));
            let body = table_body(area, 0);
            assert!(app.files_mouse(mouse(MouseEventKind::ScrollDown, body.x, body.y)));
            assert_eq!(app.parallel.files.entries["task-0"].offsets, [3, 0]);
            assert_eq!(app.parallel.files.entries["task-1"].offsets, [0, 0]);
            assert!(app.files_mouse(mouse(
                MouseEventKind::Down(MouseButton::Left),
                body.right(),
                body.bottom() - 1
            )));
            assert_eq!(
                app.parallel.files.entries["task-0"].offsets[0],
                30 - usize::from(body.height)
            );
            // Header clicks do not open files, switch focus or inject terminal input.
            let focus = app.focus;
            assert!(app.files_mouse(mouse(
                MouseEventKind::Down(MouseButton::Left),
                body.x,
                area.y + 1
            )));
            assert!(app.sidebar.is_none());
            assert_eq!(app.focus, focus);
            app.parallel.layout.file_panels.insert(
                "task-0".into(),
                Settings {
                    collapsed: true,
                    ..Default::default()
                },
            );
            terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            let summary = row_text(terminal.backend().buffer(), app.file_panels()[0].1);
            assert!(summary.contains("引用文件 30") && summary.contains("[+]"));
        });
    }
    #[test]
    fn scrollbar_capture_consumes_drag_and_release_outside_panel_and_popups_block_hits() {
        let mut app = app();
        let area = app.file_panels()[0].1;
        let body = table_body(area, 0);
        app.parallel.files.entries.insert(
            "task-0".into(),
            Entry {
                references: (0..30)
                    .map(|n| Reference {
                        path: format!("{n}.rs").into(),
                        line: None,
                    })
                    .collect(),
                ..Default::default()
            },
        );
        app.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            body.right(),
            body.y,
        ));
        assert!(app.file_dragging());
        app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), body.x, 0));
        assert_eq!(app.parallel.files.entries["task-0"].offsets[0], 0);
        let screen = app.sessions[0].state().terminal.screen().clone();
        app.selection = Some(SelectionView {
            pane: 0,
            screen,
            range: Some(Selection {
                start: Point { row: 0, col: 0 },
                end: Point { row: 0, col: 0 },
            }),
        });
        app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), body.x, 10));
        assert!(!app.file_dragging());
        assert_eq!(
            app.selection
                .as_ref()
                .unwrap()
                .range
                .as_ref()
                .unwrap()
                .end
                .row,
            0
        );
        app.selection = None;
        // A foreground terminal popup owns these cells, even above the table.
        app.popup = Some(1);
        app.popup_window = super::super::popup::Window::default();
        let popup = app.popup_window.rect(app.size);
        let Some(overlap) = app
            .file_panels()
            .into_iter()
            .map(|(_, r)| r.intersection(popup))
            .find(|r| !r.is_empty())
        else {
            return;
        };
        let before = app.parallel.files.entries["task-0"].offsets;
        app.mouse(mouse(MouseEventKind::ScrollDown, overlap.x + 1, overlap.y));
        assert_eq!(app.parallel.files.entries["task-0"].offsets, before);
    }
    #[test]
    fn drag_cancel_restores_only_its_tasks_settings() {
        let mut app = app();
        let area = app.file_panels()[0].1;
        assert!(app.files_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 2,
            area.y
        )));
        assert!(app.file_dragging());
        app.parallel.layout.file_panels.insert(
            "task-0".into(),
            Settings {
                height: 14,
                collapsed: false,
            },
        );
        assert!(app.cancel_file_drag());
        assert!(!app.file_dragging());
        assert_eq!(app.file_settings(0).height, 7);
        assert_eq!(app.file_settings(1).height, 7);
    }
    #[test]
    fn observed_and_unloaded_changes_never_claim_a_comparable_diff() {
        let c = crate::rounds::Change {
            path: "file.rs".into(),
            before: None,
            after: Some("hash".into()),
            reason: None,
        };
        assert_eq!(status(Some(&c), false).0, "added");
        assert_eq!(status(Some(&c), true).0, "observed");
        assert_eq!(status(None, false).0, "recorded");
    }
}
