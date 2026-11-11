//! Workspace chrome uses the same prefix dispatcher as keyboard commands.
use super::*;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zoom {
    Tree,
    Preview,
    Terminal(usize),
}

pub(crate) const COMMANDS: &[(char, &str)] = &[
    ('+', "new_task"),
    ('g', "round_review"),
    ('!', "attention"),
    ('m', "task_resources"),
    ('[', "previous_task"),
    (']', "next_task"),
    ('v', "agent_layout"),
    ('c', "task_pins"),
    ('e', "replace_task"),
    ('j', "new_conversation"),
    ('h', "resume_task"),
    ('a', "agent"),
    ('d', "debug"),
    ('f', "tree"),
    ('o', "preview"),
    ('w', "sessions"),
    ('b', "files"),
    ('i', "insert"),
    ('z', "zoom"),
    ('l', "layout"),
    ('n', "next"),
    ('t', "move"),
    ('r', "rerun"),
    ('k', "stop"),
    ('F', "force"),
    ('s', "select"),
    ('y', "copy"),
    ('u', "language"),
    ('p', "overview"),
    ('x', "detach"),
    ('q', "close"),
];

pub(crate) fn body(area: Rect) -> Rect {
    Rect {
        height: area.height.saturating_sub(1),
        ..area
    }
}
/// The shadow is real occlusion, not merely decoration over native images.
pub(crate) fn cover(panel: Rect, area: Rect) -> Rect {
    Rect {
        width: panel.width.saturating_add(1),
        height: panel.height.saturating_add(1),
        ..panel
    }
    .intersection(body(area))
}

impl App {
    pub(super) fn back_to_overview(&mut self) {
        if self.sidebar.as_mut().is_some_and(|s| !s.leave('p')) {
            return;
        }
        self.return_overview = true;
        self.detach();
    }
    pub(crate) fn terminal_focused(&self, index: usize) -> bool {
        self.focus == index
            && self
                .sidebar
                .as_ref()
                .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal)
    }
    pub(crate) fn hovered(&self, rect: Rect) -> bool {
        self.hover.is_some_and(|p| rect.contains(p.into()))
    }
    pub(super) fn refresh_message(&mut self, now: Instant) -> bool {
        if self.message.is_empty() {
            self.message_seen = None;
            return false;
        }
        if self
            .message_seen
            .as_ref()
            .is_none_or(|(last, _)| last != &self.message)
        {
            self.message_seen = Some((self.message.clone(), now));
        } else if self
            .message_seen
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) >= Duration::from_secs(6))
        {
            self.message.clear();
            self.message_seen = None;
            return true;
        }
        false
    }
    pub(super) fn toggle_zoom(&mut self) {
        if self.sidebar.as_ref().is_some_and(|s| s.resizing()) {
            return;
        }
        self.popup_window.cancel_drag();
        self.zoom = if self.zoom.is_some() {
            None
        } else {
            Some(match self.sidebar.as_ref().map(|s| s.focus) {
                Some(crate::sidebar::Focus::Tree) => Zoom::Tree,
                Some(crate::sidebar::Focus::Preview | crate::sidebar::Focus::Editor) => {
                    Zoom::Preview
                }
                _ if self.allowed(self.focus) => Zoom::Terminal(self.focus),
                _ => return,
            })
        };
        if let Err(error) = self.attach_visible() {
            self.message = format!("{error:#}");
        }
    }
    pub(super) fn prefix_mouse(&mut self, event: MouseEvent) -> bool {
        if self.chrome_capture {
            if matches!(event.kind, MouseEventKind::Up(_)) {
                self.chrome_capture = false;
            }
            return true;
        }
        if self.mode != Mode::Prefix {
            return false;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            self.chrome_capture = true;
            if let Some((_, key)) = command_rows(self.size)
                .into_iter()
                .find(|(r, _)| r.contains((event.column, event.row).into()))
            {
                self.key(
                    KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                    None,
                    1,
                );
            } else {
                self.mode = Mode::Normal;
            }
        }
        true
    }
}

fn command_rect(area: Rect) -> Rect {
    let width = area.width.min(76);
    let height = area.height.saturating_sub(1).min(13);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
pub(crate) fn command_rows(area: Rect) -> Vec<(Rect, char)> {
    let r = command_rect(area);
    let rows = r.height.saturating_sub(3) as usize;
    let columns = COMMANDS.len().div_ceil(rows.max(1)) as u16;
    let width = r.width.saturating_sub(4) / columns.max(1);
    if rows == 0 || width == 0 {
        return vec![];
    }
    COMMANDS
        .iter()
        .enumerate()
        .take(rows * columns as usize)
        .map(|(i, (key, _))| {
            (
                Rect::new(
                    r.x + 2 + (i / rows) as u16 * width,
                    r.y + 1 + (i % rows) as u16,
                    width,
                    1,
                ),
                *key,
            )
        })
        .collect()
}
pub(crate) fn overlay(frame: &mut Frame, app: &App) {
    if app.mode != Mode::Prefix {
        return;
    }
    let p = crate::appearance::palette();
    let r = command_rect(frame.area());
    crate::ui::clear_popup(frame, r);
    frame.render_widget(
        Block::bordered()
            .title(crate::i18n::t("chrome.commands"))
            .style(crate::appearance::style())
            .border_style(Style::new().fg(p.accent)),
        r,
    );
    for (rect, key) in command_rows(frame.area()) {
        let unavailable = key == 'd' && app.sessions.len() < 2;
        let label = COMMANDS.iter().find(|(k, _)| *k == key).unwrap().1;
        let key = if key == 'F' {
            "Shift+F".into()
        } else {
            key.to_string()
        };
        let label = format!(
            "{key:>7}  {}",
            crate::i18n::t(&format!("chrome.command.{label}"))
        );
        frame.render_widget(
            Paragraph::new(label).style(if unavailable {
                Style::new().fg(p.muted)
            } else if app.hovered(rect) {
                Style::new().bg(p.selected)
            } else {
                Style::default()
            }),
            rect,
        );
    }
    frame.render_widget(
        Paragraph::new(crate::i18n::t("chrome.cancel")),
        Rect::new(
            r.x + 2,
            r.bottom().saturating_sub(2),
            r.width.saturating_sub(4),
            1,
        ),
    );
}

/// Returns the space occupied by session controls, leaving the rest for context.
pub(crate) fn footer_controls(frame: &mut Frame, app: &mut App) -> u16 {
    use super::terminals::{Action, footer_hit};
    let area = frame.area();
    let left = overview_button(area, app);
    footer_hit(
        frame,
        app,
        left,
        format!("[← {}]", crate::i18n::t("overview")),
        Action::Overview,
        false,
    );
    let new_label = format!("[+ {}]", crate::i18n::t("parallel.new_task"));
    let new_width = new_label.width() as u16;
    footer_hit(
        frame,
        app,
        Rect::new(left.right() + 1, area.bottom() - 1, new_width, 1),
        new_label,
        Action::NewTask,
        false,
    );
    let more = format!("[{}]", crate::i18n::t("terminal.more"));
    let w = (more.width() as u16).min(area.width);
    let mut x = area.right().saturating_sub(w);
    footer_hit(
        frame,
        app,
        Rect::new(x, area.bottom() - 1, w, 1),
        more,
        Action::Menu(app.focus),
        false,
    );
    for i in 0..app.sessions.len() {
        let info = &app.sessions[i].info;
        let state = match info.placement {
            crate::presentation::Placement::Collapsed => "−",
            crate::presentation::Placement::Floating => "◇",
            _ => "▣",
        };
        let task = app
            .parallel
            .tasks
            .iter()
            .find(|t| t.id == info.task.task_id);
        let label = task.map_or_else(
            || info.name.clone(),
            |t| format!("{}:{}", t.name, info.name),
        );
        let name = super::terminals::short_name(&label, 16);
        let status = match &info.status {
            crate::session::Status::Exited(code) => format!(":{code}"),
            crate::session::Status::Failed(_) => "!".into(),
            crate::session::Status::Running => match info.command {
                crate::session::CommandStatus::Exited(code) => format!(":{code}"),
                crate::session::CommandStatus::Running
                | crate::session::CommandStatus::Starting => "▶".into(),
                _ => "·".into(),
            },
        };
        let label = format!("[{name}{state}{status}] ");
        let w = label.width() as u16;
        // Context and menu remain usable even with many sessions.
        if x.saturating_sub(area.x) < w + area.width / 2 {
            break;
        }
        x -= w;
        footer_hit(
            frame,
            app,
            Rect::new(x, area.bottom() - 1, w, 1),
            label,
            Action::Activate(i),
            app.terminal_focused(i) && app.allowed(i),
        );
    }
    x
}

pub(crate) fn footer(frame: &mut Frame, app: &App, right: u16) {
    let area = frame.area();
    let (name, hint) = match app.sidebar.as_ref().map(|s| s.focus) {
        Some(crate::sidebar::Focus::Tree) => {
            (crate::i18n::t("chrome.command.tree"), "chrome.hint.tree")
        }
        Some(crate::sidebar::Focus::Preview)
            if app
                .sidebar
                .as_ref()
                .is_some_and(|s| s.document.as_ref().is_some_and(|d| d.is_image())) =>
        {
            (
                crate::i18n::t("chrome.command.preview"),
                "chrome.hint.image",
            )
        }
        Some(crate::sidebar::Focus::Preview) => (
            crate::i18n::t("chrome.command.preview"),
            "chrome.hint.preview",
        ),
        Some(crate::sidebar::Focus::Editor) => {
            (crate::i18n::t("chrome.editor"), "chrome.hint.editor")
        }
        _ => (
            std::borrow::Cow::Owned(app.sessions[app.focus].info.name.clone()),
            "chrome.hint.terminal",
        ),
    };
    let error = app
        .sessions
        .iter()
        .filter(|s| s.attached())
        .find_map(|s| s.state().io_error.clone());
    let notice = app.sidebar.as_ref().and_then(|s| s.chrome_notice());
    let text = error
        .as_deref()
        .or(notice)
        .or((!app.message.is_empty()).then_some(app.message.as_str()));
    let text = text.map_or_else(
        || {
            format!(
                "[{name}] {}",
                crate::i18n::t(if app.mode == Mode::Prefix {
                    "chrome.cancel"
                } else if app.selection.is_some() {
                    "select_hint"
                } else {
                    hint
                })
            )
        },
        str::to_owned,
    );
    let x = overview_button(area, app)
        .right()
        .saturating_add(format!("[+ {}]", crate::i18n::t("parallel.new_task")).width() as u16 + 2)
        .min(right);
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(crate::appearance::palette().accent)),
        Rect::new(x, area.bottom() - 1, right.saturating_sub(x), 1),
    );
}

fn overview_button(area: Rect, app: &App) -> Rect {
    let x = area.x
        + if app
            .sidebar
            .as_ref()
            .is_some_and(|s| s.geometry.toggle.is_some())
        {
            8
        } else {
            0
        };
    Rect::new(
        x,
        area.bottom().saturating_sub(1),
        format!("[← {}]", crate::i18n::t("overview")).width() as u16,
        1,
    )
}
