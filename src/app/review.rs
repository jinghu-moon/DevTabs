//! Read-only round review and derived attention inbox. Never forwards keys to an Agent.
use super::*;
use crate::{
    protocol::{Request, Response},
    rounds::service::Action,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use std::sync::mpsc;
#[derive(Default)]
pub struct State {
    pub resources: bool,
    pub catalog: Vec<crate::tasks::Task>,
    pub target: usize,
    pub operation: Option<crate::lifecycle::Operation>,
    pub require_check: bool,
    pub archive_pending: bool,
    pub open: bool,
    pub inbox: bool,
    pub task: String,
    pub data: crate::rounds::service::View,
    pub notices: crate::notifications::View,
    pub pending: Option<mpsc::Receiver<Result<Response>>>,
    pub error: String,
    pub check: Option<crate::checks::Report>,
    pub round: usize,
    pub file: usize,
    pub scroll: usize,
    pub search: bool,
    pub query: String,
    pub last_poll: Option<Instant>,
    pub scope: String,
    pub editing_scope: bool,
    pub deferred: std::collections::VecDeque<Request>,
    pub viewed: Option<crate::notifications::Target>,
    pub marks_pending: bool,
}
impl App {
    pub(super) fn open_review(&mut self, inbox: bool) {
        self.parallel.review.resources = false;
        self.parallel.review.open = true;
        self.parallel.review.inbox = inbox;
        self.parallel.review.task = self.parallel.context.clone();
        self.parallel.review.error.clear();
        self.parallel.review.scroll = 0;
        if inbox {
            self.parallel.review.file = 0;
            self.parallel.review.data.offset = 0;
            self.review_request(Request::Notifications {
                action: crate::notifications::Action::List,
            });
        } else {
            self.review_action(Action::List);
        }
    }
    pub(super) fn open_resources(&mut self) {
        let s = &mut self.parallel.review;
        s.open = true;
        s.resources = true;
        s.inbox = false;
        s.task = self.parallel.context.clone();
        s.error.clear();
        s.operation = None;
        s.archive_pending = false;
        self.review_request(Request::Lifecycle {
            action: crate::lifecycle::Action::List,
        });
    }
    fn review_request(&mut self, request: Request) {
        if self.parallel.review.pending.is_some() {
            if self.parallel.review.deferred.len() < 8 {
                self.parallel.review.deferred.push_back(request);
            } else {
                self.parallel.review.error = "Review request queue full".into();
            }
            return;
        }
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("devtabs-review".into())
            .spawn(move || {
                let _ = tx.send(crate::protocol::rpc(request));
            }) {
            Ok(_) => self.parallel.review.pending = Some(rx),
            Err(e) => self.parallel.review.error = e.to_string(),
        }
    }
    fn review_action(&mut self, action: Action) {
        self.review_request(Request::Review {
            workspace: self.workspace_id.clone(),
            task: self.parallel.review.task.clone(),
            action,
        });
    }
    pub(super) fn review_tick(&mut self) -> bool {
        let s = &mut self.parallel.review;
        if let Some(rx) = &s.pending {
            match rx.try_recv() {
                Ok(result) => {
                    s.pending = None;
                    match result {
                        Ok(Response::TaskCatalog(c)) => {
                            s.catalog = c
                                .tasks
                                .into_values()
                                .filter(|t| {
                                    t.id != s.task
                                        && !t.archived
                                        && self
                                            .parallel
                                            .tasks
                                            .iter()
                                            .find(|t| t.id == s.task)
                                            .is_some_and(|source| source.project == t.project)
                                })
                                .collect();
                            s.target = s.target.min(s.catalog.len().saturating_sub(1));
                        }
                        Ok(Response::Lifecycle(op)) => {
                            s.operation = Some(*op);
                            s.error.clear();
                        }
                        Ok(Response::Review(v)) => {
                            s.marks_pending = true;
                            let selected = s.data.records.get(s.round).map(|r| r.id.clone());
                            if v.detail
                                && let Some(record) = v.records.first()
                                && let Some(i) =
                                    s.data.records.iter().position(|r| r.id == record.id)
                            {
                                s.data.records[i] = record.clone();
                                s.round = i;
                                s.data.offset = v.offset;
                                s.data.paths = v.paths;
                                s.data.diff = v.diff;
                                s.data.error = v.error;
                            } else {
                                s.data = *v;
                                s.round = selected
                                    .and_then(|id| s.data.records.iter().position(|r| r.id == id))
                                    .unwrap_or(0);
                            }
                            s.file = s.file.min(
                                s.data
                                    .records
                                    .get(s.round)
                                    .map_or(0, |r| r.file_count.saturating_sub(1)),
                            );
                            s.error = s.data.error.clone().unwrap_or_default();
                            if let Some(target) = s.viewed.take() {
                                s.deferred.push_back(Request::Notifications {
                                    action: crate::notifications::Action::Ack { target },
                                });
                            }
                        }
                        Ok(Response::Check(v)) => {
                            s.check = *v;
                        }
                        Ok(Response::Ok) => {}
                        Ok(Response::Notifications(v)) => {
                            let selected = s
                                .notices
                                .pending
                                .get(s.file)
                                .map(|(t, _)| (t.run.clone(), t.result));
                            s.notices = *v;
                            if s.inbox {
                                s.file = selected
                                    .and_then(|(run, result)| {
                                        s.notices
                                            .pending
                                            .iter()
                                            .position(|(t, _)| t.run == run && t.result == result)
                                    })
                                    .unwrap_or(
                                        s.file.min(s.notices.pending.len().saturating_sub(1)),
                                    );
                            }
                            s.error.clear();
                        }
                        Ok(_) => s.error = "Unexpected response".into(),
                        Err(e) => {
                            s.viewed = None;
                            s.error = format!("{e:#}");
                        }
                    };
                    return true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    s.pending = None;
                    s.error = "Worker disconnected; query before retry".into();
                    return true;
                }
                Err(_) => {}
            }
        }
        if s.marks_pending
            && let Some(sidebar) = if self.parallel.context == s.task {
                self.sidebar.as_mut()
            } else {
                self.parallel.sidebars.get_mut(&s.task)
            }
        {
            sidebar
                .files
                .mark_round(s.data.paths.iter().cloned().collect());
            s.marks_pending = false;
        }
        if s.pending.is_none()
            && let Some(request) = s.deferred.pop_front()
        {
            self.review_request(request);
            return true;
        }
        if s.pending.is_none()
            && s.last_poll
                .is_none_or(|t| t.elapsed() > Duration::from_millis(500))
        {
            s.last_poll = Some(Instant::now());
            self.review_request(Request::Notifications {
                action: crate::notifications::Action::Poll {
                    workspace: self.workspace_id.clone(),
                    clients: self
                        .sessions
                        .iter()
                        .filter_map(|s| s.input_lease())
                        .collect(),
                },
            });
        }
        let nav = self.parallel.review.notices.navigation.take();
        if let Some(t) = nav {
            // A click is an explicit navigation request. Do not take another view's input lease.
            if let Some(i) = self.sessions.iter().position(|s| {
                s.info.id == t.session
                    && s.info.task.run_id == t.run
                    && s.info.task.task_id == t.task
            }) {
                self.focus(i);
                self.parallel.review.task = t.task.clone();
                self.parallel.review.open = true;
                self.parallel.review.inbox = false;
                self.parallel.review.data = Default::default();
                self.parallel.review.viewed = Some(t.clone());
                self.review_action(Action::Result {
                    run: t.run,
                    result: t.result,
                });
            } else {
                self.message = crate::i18n::t("review.expired").into_owned();
            }
            return true;
        }
        false
    }
    pub(super) fn review_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let s = &mut self.parallel.review;
        if key.code == KeyCode::Esc {
            if s.search || s.editing_scope {
                s.search = false;
                s.editing_scope = false;
            } else {
                s.open = false;
            }
            return;
        }
        if s.resources {
            use crate::lifecycle::Action as L;
            if s.pending.is_some() {
                return;
            }
            let action = match key.code {
                KeyCode::Down => {
                    s.target = (s.target + 1).min(s.catalog.len().saturating_sub(1));
                    s.operation = None;
                    s.archive_pending = false;
                    None
                }
                KeyCode::Up => {
                    s.target = s.target.saturating_sub(1);
                    s.operation = None;
                    s.archive_pending = false;
                    None
                }
                KeyCode::Char('m' | 'd') => {
                    s.archive_pending = false;
                    s.catalog.get(s.target).map(|t| L::Plan {
                        source: s.task.clone(),
                        target: t.id.clone(),
                        cleanup: key.code == KeyCode::Char('d'),
                        require_check: s.require_check,
                    })
                }
                KeyCode::Char('p') => {
                    s.require_check = !s.require_check;
                    s.operation = None;
                    None
                }
                KeyCode::Char('a') => {
                    s.archive_pending = true;
                    s.operation = None;
                    None
                }
                KeyCode::Char('l') => Some(L::List),
                KeyCode::Char('q') => s.operation.as_ref().map(|op| L::Query {
                    operation: op.id.clone(),
                }),
                KeyCode::Char('y') => {
                    if let Some(op) = &s.operation {
                        let result = std::env::current_exe()
                            .map_err(anyhow::Error::from)
                            .and_then(|exe| {
                                crate::files::copy(format!(
                                    "& '{}' task apply '{}' --confirm '{}'",
                                    exe.display().to_string().replace('\'', "''"),
                                    op.id,
                                    op.id
                                ))
                            });
                        if let Err(e) = result {
                            s.error = e.to_string();
                        }
                    }
                    None
                }
                KeyCode::Enter if s.archive_pending => {
                    s.archive_pending = false;
                    Some(L::Archive {
                        task: s.task.clone(),
                        confirm: s.task.clone(),
                    })
                }
                KeyCode::Enter => s.operation.as_ref().map(|op| L::Apply {
                    operation: op.id.clone(),
                    confirm: op.id.clone(),
                }),
                _ => None,
            };
            if let Some(action) = action {
                self.review_request(Request::Lifecycle { action });
            }
            return;
        }
        if s.search || s.editing_scope {
            let text = if s.search { &mut s.query } else { &mut s.scope };
            match key.code {
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if text.len() < 1024 => text.push(c),
                KeyCode::Enter => {
                    s.search = false;
                    s.editing_scope = false;
                }
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Char('/') {
            s.search = true;
            s.query.clear();
            return;
        }
        if s.inbox {
            use crate::notifications::Action as N;
            let selected = s.notices.pending.get(s.file).cloned();
            let action = match key.code {
                KeyCode::Down => {
                    s.file = (s.file + 1).min(s.notices.pending.len().saturating_sub(1));
                    None
                }
                KeyCode::Up => {
                    s.file = s.file.saturating_sub(1);
                    None
                }
                KeyCode::Enter => selected.map(|(target, _)| N::Navigate { target }),
                KeyCode::Char('a') => selected.map(|(target, _)| N::Ack { target }),
                KeyCode::Char('m') => selected.map(|(target, _)| N::Mute {
                    task: target.task,
                    muted: true,
                }),
                KeyCode::Char('u') => selected.map(|(target, _)| N::Mute {
                    task: target.task,
                    muted: false,
                }),
                KeyCode::Char('1') => selected.map(|(target, _)| N::Once { task: target.task }),
                KeyCode::Char('5') => selected.map(|(target, _)| N::Snooze {
                    target,
                    seconds: 300,
                }),
                KeyCode::Char('3') => selected.map(|(target, _)| N::Snooze {
                    target,
                    seconds: 1800,
                }),
                KeyCode::Char('0') => selected.map(|(target, _)| N::Snooze { target, seconds: 0 }),
                KeyCode::Char('t') => Some(N::Test),
                KeyCode::Char('d') => Some(N::Diagnostics),
                KeyCode::Char('y') => {
                    let text = s.notices.diagnostic.join("\n");
                    if let Err(e) = crate::files::copy(text) {
                        s.error = e.to_string();
                    }
                    None
                }
                KeyCode::Char('g') => Some(N::Global {
                    enabled: !s.notices.enabled,
                }),
                _ => None,
            };
            if let Some(action) = action {
                self.review_request(Request::Notifications { action });
            }
            return;
        }
        if key.code == KeyCode::Char('c') {
            let task = s.task.clone();
            self.review_request(Request::Check {
                workspace: self.workspace_id.clone(),
                task,
                spec: None,
                initialize: false,
            });
            return;
        }
        if key.code == KeyCode::Char('k') {
            let task = s.task.clone();
            self.review_request(Request::CheckStatus { task });
            return;
        }
        let record = s
            .data
            .records
            .get(s.round.min(s.data.records.len().saturating_sub(1)))
            .cloned();
        let action = match key.code {
            KeyCode::Char('b') => Some(Action::Begin {
                scope: s
                    .scope
                    .split(';')
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| p.trim().into())
                    .collect(),
            }),
            KeyCode::Char('e') => Some(Action::End),
            KeyCode::Char('l') => Some(Action::List),
            KeyCode::Char('s') => {
                s.editing_scope = true;
                None
            }
            KeyCode::Char('[') => {
                s.round = s.round.saturating_sub(1);
                s.data.offset = 0;
                s.file = 0;
                s.data.diff = None;
                s.data.records.get(s.round).map(|r| Action::Page {
                    round: r.id.clone(),
                    offset: 0,
                })
            }
            KeyCode::Char(']') => {
                s.round = (s.round + 1).min(s.data.records.len().saturating_sub(1));
                s.data.offset = 0;
                s.file = 0;
                s.data.diff = None;
                s.data.records.get(s.round).map(|r| Action::Page {
                    round: r.id.clone(),
                    offset: 0,
                })
            }
            KeyCode::Up => {
                s.file = s.file.saturating_sub(1);
                if s.file < s.data.offset {
                    record.map(|r| Action::Page {
                        round: r.id,
                        offset: s.file / 100 * 100,
                    })
                } else {
                    None
                }
            }
            KeyCode::Down => {
                s.file = (s.file + 1).min(
                    record
                        .as_ref()
                        .map_or(0, |r| r.file_count.saturating_sub(1)),
                );
                if s.file >= s.data.offset + 100 {
                    record.map(|r| Action::Page {
                        round: r.id,
                        offset: s.file / 100 * 100,
                    })
                } else {
                    None
                }
            }
            KeyCode::PageDown => {
                s.scroll = s.scroll.saturating_add(12);
                None
            }
            KeyCode::PageUp => {
                s.scroll = s.scroll.saturating_sub(12);
                None
            }
            KeyCode::Home => {
                s.scroll = 0;
                None
            }
            KeyCode::Enter => {
                s.scroll = 0;
                record.map(|r| Action::Open {
                    round: r.id,
                    path: Some(s.file),
                })
            }
            KeyCode::Char('a') => record.map(|r| Action::Ack {
                round: r.id,
                revision: r.revision,
            }),
            KeyCode::Char('n') => {
                if let Some(diff) = &s.data.diff
                    && let Some((i, _)) =
                        diff.lines().enumerate().skip(s.scroll + 1).find(|(_, l)| {
                            l.starts_with("@@") || (!s.query.is_empty() && l.contains(&s.query))
                        })
                {
                    s.scroll = i;
                }
                None
            }
            KeyCode::Char('o') => {
                let path = record.and_then(|r| {
                    r.changes
                        .get(s.file.saturating_sub(s.data.offset))
                        .map(|c| r.root.join(&c.path))
                });
                if let Some(p) = path {
                    if p.is_file()
                        && dunce::canonicalize(&p).ok().is_some_and(|p| {
                            self.parallel
                                .tasks
                                .iter()
                                .find(|t| t.id == s.task)
                                .is_some_and(|t| p.starts_with(&t.checkout))
                        })
                    {
                        s.open = false;
                        if let Some(sidebar) = &mut self.sidebar
                            && let Err(e) = sidebar.open(p, true)
                        {
                            self.message = format!("{e:#}");
                        }
                    } else {
                        s.error = crate::i18n::t("review.expired").into_owned();
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(action) = action {
            self.review_action(action);
        }
    }
    pub(super) fn review_mouse(&mut self, event: MouseEvent) -> bool {
        let count = self.parallel.review.notices.pending.len();
        let button = Rect::new(
            self.size.width.saturating_sub(22),
            self.size.height.saturating_sub(1),
            22,
            1,
        );
        if !self.parallel.review.open {
            if event.kind == MouseEventKind::Down(MouseButton::Left)
                && button.contains((event.column, event.row).into())
            {
                self.open_review(true);
                return true;
            }
            return false;
        }
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.parallel.review.scroll = self.parallel.review.scroll.saturating_add(4)
            }
            MouseEventKind::ScrollUp => {
                self.parallel.review.scroll = self.parallel.review.scroll.saturating_sub(4)
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let row = usize::from(event.row.saturating_sub(6));
                if self.parallel.review.inbox {
                    self.parallel.review.file = row.min(count.saturating_sub(1));
                } else if let Some(r) = self
                    .parallel
                    .review
                    .data
                    .records
                    .get(self.parallel.review.round)
                {
                    self.parallel.review.file = self.parallel.review.data.offset
                        + row.min(r.changes.len().saturating_sub(1));
                }
            }
            _ => {}
        }
        true
    }
}
pub(crate) fn draw(frame: &mut Frame, app: &App) {
    let s = &app.parallel.review;
    if s.open && s.resources {
        draw_resources(frame, app);
        return;
    }
    let area = frame.area();
    if !s.open {
        let text = format!(
            " {} {} ",
            crate::i18n::t("review.inbox"),
            s.notices.pending.len()
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(if s.notices.pending.is_empty() {
                Color::Gray
            } else {
                Color::Yellow
            })),
            Rect::new(
                area.right().saturating_sub(22),
                area.bottom().saturating_sub(1),
                22,
                1,
            ),
        );
        return;
    }
    let r = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 2,
    });
    crate::ui::clear_popup(frame, r);
    frame.render_widget(
        Block::bordered()
            .title(crate::i18n::t(if s.inbox {
                "review.inbox"
            } else {
                "review.title"
            }))
            .style(crate::appearance::style()),
        r,
    );
    let inner = crate::ui::content(r);
    let mut lines = vec![Line::from(
        crate::i18n::t(if s.inbox {
            "review.inbox_keys"
        } else {
            "review.keys"
        })
        .into_owned(),
    )];
    if s.pending.is_some() {
        lines.push(Line::from(crate::i18n::t("parallel.working").into_owned()));
    }
    if !s.error.is_empty() {
        lines.push(Line::styled(
            crate::preview::safe_text(&s.error),
            Style::default().fg(Color::Red),
        ));
    }
    if let Some(c) = &s.check {
        lines.push(Line::from(format!(
            "{} · exit {:?} · {} ms · {}",
            crate::i18n::t("review.check"),
            c.exit,
            c.elapsed_ms,
            if c.stale {
                crate::i18n::t("review.stale")
            } else {
                crate::i18n::t("review.bound")
            }
        )));
    }
    if s.inbox {
        lines.push(Line::from(format!(
            "{}: {}",
            crate::i18n::t("review.notifications"),
            s.notices.enabled
        )));
        for (i, (t, a)) in s.notices.pending.iter().enumerate() {
            let name = app
                .parallel
                .tasks
                .iter()
                .find(|x| x.id == t.task)
                .map_or(t.task.as_str(), |t| t.name.as_str());
            lines.push(Line::styled(
                format!(
                    "{} {} · {} · #{}",
                    if i + s.data.offset == s.file {
                        "▸"
                    } else {
                        " "
                    },
                    name,
                    crate::i18n::t(a.phase.label()),
                    a.result
                ),
                Style::default().fg(if a.phase == crate::activity::Phase::Failed {
                    Color::Red
                } else {
                    Color::Yellow
                }),
            ));
        }
        lines.extend(
            s.notices
                .diagnostic
                .iter()
                .rev()
                .take(5)
                .map(|s| Line::from(crate::preview::safe_text(s))),
        );
    } else if let Some(record) = s
        .data
        .records
        .get(s.round.min(s.data.records.len().saturating_sub(1)))
    {
        lines.push(Line::from(format!(
            "{} · {:?} · {} {} · {}{}",
            s.round + 1,
            record.mode,
            record.file_count,
            crate::i18n::t("review.files"),
            if record.late {
                crate::i18n::t("review.late")
            } else {
                crate::i18n::t("review.manual")
            },
            if record.ended.is_none() { " …" } else { "" }
        )));
        for reason in record.reasons.iter().take(2) {
            lines.push(Line::styled(
                crate::preview::safe_text(reason),
                Style::default().fg(Color::Yellow),
            ));
        }
        if let Some(diff) = &s.data.diff {
            let start = lines.len() as u16;
            frame.render_widget(Paragraph::new(lines), inner);
            let viewport = Rect::new(
                inner.x,
                inner.y + start,
                inner.width,
                inner.height.saturating_sub(start),
            );
            let total = diff.lines().count();
            let offset = s.scroll.min(total.saturating_sub(1));
            let visible = diff
                .lines()
                .skip(offset)
                .take(usize::from(viewport.height))
                .map(|l| {
                    Line::styled(
                        crate::preview::safe_text(l),
                        Style::default().fg(if l.starts_with('+') {
                            Color::Green
                        } else if l.starts_with('-') {
                            Color::Red
                        } else if l.starts_with("@@") {
                            Color::Cyan
                        } else {
                            crate::appearance::palette().fg
                        }),
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(visible), viewport);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                viewport,
                &mut ScrollbarState::new(total).position(offset),
            );
            return;
        }
        for (i, c) in record
            .changes
            .iter()
            .enumerate()
            .skip(s.file.saturating_sub(s.data.offset).saturating_sub(3))
            .take(inner.height as usize)
        {
            let (symbol, color) = if c.reason.is_some() {
                ("?", Color::Yellow)
            } else if c.before.is_none() {
                ("+", Color::Green)
            } else if c.after.is_none() {
                ("-", Color::Red)
            } else {
                ("M", Color::Cyan)
            };
            lines.push(Line::styled(
                format!(
                    "{} {symbol} {}",
                    if i + s.data.offset == s.file {
                        "▸"
                    } else {
                        " "
                    },
                    crate::preview::safe_text(&c.path.display().to_string())
                ),
                Style::default().fg(color),
            ));
        }
    }
    if s.search || s.editing_scope {
        lines.push(Line::from(format!(
            "{}{}",
            if s.editing_scope { "scope: " } else { "/" },
            if s.editing_scope { &s.scope } else { &s.query }
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
fn draw_resources(frame: &mut Frame, app: &App) {
    let s = &app.parallel.review;
    let r = frame.area().inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 2,
    });
    crate::ui::clear_popup(frame, r);
    let block = Block::bordered().title(crate::i18n::t("lifecycle.title"));
    let inner = block.inner(r);
    frame.render_widget(block, r);
    let mut lines = vec![
        Line::from(crate::i18n::t("lifecycle.keys").into_owned()),
        Line::from(format!(
            "{}: {}",
            crate::i18n::t("lifecycle.source"),
            s.task
        )),
        Line::from(format!(
            "{}: {}",
            crate::i18n::t("lifecycle.check"),
            s.require_check
        )),
    ];
    if s.pending.is_some() {
        lines.push(Line::from(crate::i18n::t("parallel.working").into_owned()));
    }
    if !s.error.is_empty() {
        lines.push(Line::styled(
            crate::preview::safe_text(&s.error),
            Style::default().fg(Color::Red),
        ));
    }
    if s.archive_pending {
        lines.push(Line::styled(
            crate::i18n::t("lifecycle.archive_confirm").into_owned(),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(op) = &s.operation {
        lines.push(Line::from(format!(
            "{:?} · {} → {} · {}",
            op.kind, op.source.name, op.target.name, op.phase
        )));
        lines.push(Line::from(format!(
            "{} → {}",
            &op.target_head[..op.target_head.len().min(12)],
            &op.source_head[..op.source_head.len().min(12)]
        )));
        lines.push(Line::from(crate::preview::safe_text(
            &op.source.checkout.display().to_string(),
        )));
        lines.push(Line::from(format!("{} · {}", op.source.branch, op.id)));
        lines.push(Line::from(
            crate::i18n::t(if op.check.is_some() {
                "lifecycle.checked"
            } else {
                "lifecycle.unchecked"
            })
            .into_owned(),
        ));
        if let Some(e) = &op.error {
            lines.push(Line::styled(
                crate::preview::safe_text(e),
                Style::default().fg(Color::Red),
            ));
        }
        lines.push(Line::styled(
            crate::i18n::t("lifecycle.confirm").into_owned(),
            Style::default().fg(Color::Yellow),
        ));
    } else {
        for (i, t) in s.catalog.iter().enumerate().take(12) {
            lines.push(Line::from(crate::preview::safe_text(&format!(
                "{} {} · {}",
                if i == s.target { "▸" } else { " " },
                t.name,
                t.branch
            ))));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).scroll((s.scroll.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
}
