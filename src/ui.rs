use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};
use tui_term::widget::{Cursor, PseudoTerminal, Screen};

use crate::{
    app::{App, Mode},
    selection::Point,
    terminal::Snapshot,
};

pub fn panes(area: Rect) -> Option<[Rect; 2]> {
    if area.width < 60 || area.height < 10 {
        return None;
    }
    let body = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let chunks =
        Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)]).split(body);
    Some([chunks[0], chunks[1]])
}

pub fn content(rect: Rect) -> Rect {
    rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    })
}

/// Read-only document viewport: one blank cell inside each border.
pub fn preview_content(rect: Rect) -> Rect {
    content(rect).inner(Margin {
        horizontal: 1,
        vertical: 1,
    })
}

/// Erase glyphs straddling the left edge before clearing a popup's cells.
pub fn clear_popup(frame: &mut Frame, rect: Rect) {
    use unicode_width::UnicodeWidthStr;
    let rect = rect.intersection(frame.area());
    if rect.x > frame.area().x {
        for y in rect.y..rect.bottom() {
            let cell = &mut frame.buffer_mut()[(rect.x - 1, y)];
            if cell.symbol().width() > 1 {
                cell.set_symbol(" ");
            }
        }
    }
    frame.render_widget(Clear, rect);
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let _perf = crate::perf::span(crate::perf::Metric::Ui);
    app.clear_terminal_hits();
    let area = frame.area();
    frame.render_widget(Block::default().style(crate::appearance::style()), area);
    let rects = app.rects();
    if area.width < 60 || area.height < 10 {
        frame.render_widget(Paragraph::new(crate::i18n::t("terminal.minimum")), area);
        return;
    }
    for (index, rect) in rects {
        if app.popup != Some(index) || app.zoom.is_some() {
            terminal(frame, app, index, rect);
        }
    }
    if let Some(sidebar) = &mut app.sidebar {
        sidebar.draw(frame);
    }
    crate::app::files::draw(frame, app);
    if let Some(i) = app.popup.filter(|_| app.zoom.is_none()) {
        let r = app.popup_window.rect(area);
        prepare_popup(frame, r);
        terminal(frame, app, i, r);
        frame.render_widget(
            Paragraph::new("◢"),
            Rect::new(r.right() - 1, r.bottom() - 1, 1, 1),
        );
        if app.popup_window.editing() {
            frame.render_widget(
                Paragraph::new(crate::i18n::t("terminal.geometry_keys")),
                Rect::new(r.x + 1, r.bottom() - 1, r.width.saturating_sub(2), 1),
            );
        }
    }
    crate::app::terminals::draw(frame, app);
    if let Some(sidebar) = &app.sidebar {
        sidebar.draw_modal_overlays(frame);
    }
    crate::app::terminals::overlay(frame, app);
    crate::app::chrome::overlay(frame, app);
    crate::app::parallel::overlay(frame, app);
    crate::app::review::draw(frame, app);
    confirm(frame, app);
}

fn confirm(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if app.mode == Mode::Confirm {
        let width = area.width.min(78);
        let height = area.height.min(8);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let delay = app
            .quit_requested
            .map_or(3, |at| 3u64.saturating_sub(at.elapsed().as_secs()));
        frame.render_widget(
            Paragraph::new(crate::i18n::format(
                "close_prompt",
                &[("delay", delay.to_string())],
            ))
            .block(
                Block::bordered()
                    .title(crate::i18n::t("close_workspace"))
                    .border_style(Style::new().fg(Color::Yellow)),
            ),
            popup,
        );
    }
}

fn terminal(frame: &mut Frame, app: &App, index: usize, rect: Rect) {
    let state = app.sessions[index].state();
    let info = &app.sessions[index].info;
    let label = app
        .parallel
        .tasks
        .iter()
        .find(|t| t.id == info.task.task_id)
        .map_or_else(
            || info.name.clone(),
            |t| format!("{} · {} · {}", t.name, info.name, t.branch),
        );
    let focused = app.terminal_focused(index);
    let mut title = format!(" {label} ");
    if info.task.role == crate::tasks::Role::Agent
        && info.activity.phase != crate::activity::Phase::Unknown
    {
        title.push_str(&format!(
            "· {} {} ",
            if info.activity.pending() { "◆" } else { "" },
            crate::i18n::t(info.activity.phase.label())
        ));
    }
    match &state.status {
        crate::session::Status::Running => {}
        crate::session::Status::Exited(c) => {
            title.push_str(&format!("· {} {c} ", crate::i18n::t("exited")))
        }
        crate::session::Status::Failed(e) => {
            title.push_str(&format!("· {}: {e} ", crate::i18n::t("failed")))
        }
    }
    if let crate::session::CommandStatus::Exited(c) = state.command {
        title.push_str(&format!("· {} {c} ", crate::i18n::t("exited")));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(if app.popup == Some(index) {
            Style::new().bg(popup_background())
        } else {
            Style::default()
        })
        .border_style(
            Style::new().fg(
                if !focused
                    && info.activity.pending()
                    && info
                        .activity
                        .ended
                        .is_some_and(|at| crate::config::now().saturating_sub(at) < 3)
                {
                    Color::Yellow
                } else if focused {
                    crate::appearance::palette().accent
                } else {
                    crate::appearance::palette().muted
                },
            ),
        );
    let block = if let Some(task) = app
        .parallel
        .tasks
        .iter()
        .find(|t| t.id == info.task.task_id)
    {
        let module = task.cwd.strip_prefix(&task.checkout).unwrap_or(&task.cwd);
        block.title_bottom(format!(" {} · {} · — ", task.branch, module.display()))
    } else {
        block
    };
    let snapshot = if let Some(selection) = &app.selection {
        if selection.pane == index {
            Snapshot::from_screen(&selection.screen)
        } else {
            state.terminal.snapshot()
        }
    } else {
        state.terminal.snapshot()
    };
    if app.popup == Some(index) {
        frame.render_widget(
            Block::default().style(Style::new().bg(popup_background())),
            rect,
        );
    }
    frame.render_widget(
        PseudoTerminal::new(&snapshot)
            .cursor(Cursor::default().visibility(false))
            .block(block),
        rect,
    );
    let inner = content(rect);
    if app.popup == Some(index) {
        for y in inner.y..inner.bottom() {
            for x in inner.x..inner.right() {
                let cell = &mut frame.buffer_mut()[(x, y)];
                if cell.bg == Color::Reset {
                    cell.set_bg(popup_background());
                }
            }
        }
    }
    if let Some(selection) = &app.selection {
        if selection.pane == index
            && let Some(range) = selection.range
        {
            for row in 0..inner.height {
                for col in 0..inner.width {
                    if range.contains(Point { row, col }) {
                        frame.buffer_mut()[(inner.x + col, inner.y + row)]
                            .set_style(Style::new().bg(Color::Cyan).fg(Color::Black));
                    }
                }
            }
        }
    } else if focused
        && !snapshot.hide_cursor()
        && app.mode == Mode::Normal
        && !app.terminal_menu_open()
        && !app.popup_window.editing()
        && app.sidebar.as_ref().is_none_or(|s| !s.modal())
    {
        let (row, col) = snapshot.cursor_position();
        let covered = app.zoom.is_none()
            && app.popup.is_some_and(|i| {
                i != index
                    && crate::app::chrome::cover(app.popup_window.rect(frame.area()), frame.area())
                        .contains((inner.x + col, inner.y + row).into())
            });
        if row < inner.height && col < inner.width && !covered {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}

fn popup_background() -> Color {
    if crate::appearance::theme() == crate::appearance::Theme::Light {
        Color::Rgb(230, 236, 245)
    } else {
        Color::Rgb(21, 29, 47)
    }
}
fn prepare_popup(frame: &mut Frame, panel: Rect) {
    let cover = crate::app::chrome::cover(panel, frame.area());
    clear_popup(frame, cover);
    for y in cover.y..cover.bottom() {
        for x in cover.x..cover.right() {
            if !panel.contains((x, y).into()) {
                frame.buffer_mut()[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::new().bg(
                        if crate::appearance::theme() == crate::appearance::Theme::Light {
                            Color::Gray
                        } else {
                            Color::Black
                        },
                    ));
            }
        }
    }
}
