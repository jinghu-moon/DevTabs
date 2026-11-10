use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
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
        height: area.height.saturating_sub(2),
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
    let area = frame.area();
    frame.render_widget(
        Block::default().style(
            Style::new()
                .fg(Color::Rgb(212, 212, 212))
                .bg(Color::Rgb(16, 18, 23)),
        ),
        area,
    );
    let rects = app.rects();
    if rects.is_empty()
        && !app
            .sidebar
            .as_ref()
            .is_some_and(|s| s.showing() && s.geometry.preview.is_some())
    {
        let minimum = if app.options.layout == crate::launch::Layout::Split {
            "94 × 12"
        } else {
            "62 × 12"
        };
        frame.render_widget(Paragraph::new(format!("DevTabs — enlarge the terminal to at least {minimum}.\nSessions continue running. Ctrl+G, Q closes the workspace.")), area);
        if let Some(s) = &app.sidebar {
            s.draw_overlay(frame);
        }
        confirm(frame, app);
        return;
    }
    for (index, rect) in rects {
        let state = app.sessions[index].state();
        let label = if index == 0 {
            format!("Agent · {:?}", app.options.agent)
        } else {
            app.sessions[index].info.name.clone()
        };
        let focused = app.focus == index
            && app
                .sidebar
                .as_ref()
                .is_none_or(|s| s.focus == crate::sidebar::Focus::Terminal);
        let title = format!(
            " {label}{} · shell {} · command {:?} · {} ",
            if focused { " [focus]" } else { "" },
            state.status,
            state.command,
            app.project_name()
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::new().fg(if focused {
                Color::Cyan
            } else {
                Color::DarkGray
            }));
        let snapshot = if let Some(selection) = &app.selection {
            if selection.pane == index {
                Snapshot::from_screen(&selection.screen)
            } else {
                state.terminal.snapshot()
            }
        } else {
            state.terminal.snapshot()
        };
        frame.render_widget(
            PseudoTerminal::new(&snapshot)
                .cursor(Cursor::default().visibility(false))
                .block(block),
            rect,
        );
        let inner = content(rect);
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
        } else if focused && !snapshot.hide_cursor() {
            let (row, col) = snapshot.cursor_position();
            if row < inner.height && col < inner.width {
                frame.set_cursor_position((inner.x + col, inner.y + row));
            }
        }
    }
    if let Some(sidebar) = &mut app.sidebar {
        sidebar.draw(frame);
    }
    let hint = match app.mode {
        Mode::Normal if app.selection.is_some() => {
            "SELECT (frozen view) · drag text · wheel/PgUp/PgDn history · Ctrl+G Y copy · Ctrl+G S live"
        }
        Mode::Normal => {
            "Ctrl+G → A Agent / D Debug / B Files / F Tree / I Insert / O Preview / L Layout / X Detach / Q Close"
        }
        Mode::Prefix => {
            "A Agent · D Debug · L Layout · N Session · K Stop · Shift+F Force · f Files · R Rerun · S Select · Y Copy · X Detach · Q Close"
        }
        Mode::Confirm => {
            "Close this workspace? Enter: request interrupt · F: force after 3s · Esc: return to sessions"
        }
    };
    let footer = Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 2);
    let mut status = app.message.clone();
    if let Some(s) = &app.sidebar {
        let text = s.status();
        if !text.is_empty() {
            status = text;
        }
    }
    if let Some(error) = app
        .sessions
        .iter()
        .enumerate()
        .find_map(|(index, session)| {
            session.state().io_error.as_ref().map(|error| {
                format!(
                    "{} I/O error: {error}",
                    if index == 0 { "Agent" } else { "Debug" }
                )
            })
        })
    {
        status = error;
    }
    if status.is_empty() {
        status = format!(
            "DevTabs · {} · Ctrl+G X detaches; sessions keep running · [Debug: Ctrl+G D]",
            app.options.project.display()
        );
    }
    if app
        .sidebar
        .as_ref()
        .is_some_and(|s| s.geometry.toggle.is_some())
    {
        status = format!("        {status}");
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(hint, Style::new().fg(Color::Cyan))),
            Line::from(status),
        ]),
        footer,
    );
    if (app.view.is_none() || app.view.as_deref() == Some("debug-shared"))
        && app.options.layout == crate::launch::Layout::Float
    {
        let button = Rect::new(
            area.right().saturating_sub(18),
            area.bottom().saturating_sub(1),
            area.width.min(18),
            1,
        );
        frame.render_widget(
            Paragraph::new(if app.view.is_none() {
                " [ Open Debug ] "
            } else {
                "[ Collapse Debug ]"
            })
            .style(Style::new().fg(Color::Black).bg(Color::Cyan)),
            button,
        );
    }
    if let Some(sidebar) = &app.sidebar {
        sidebar.draw_overlay(frame);
    }
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
        frame.render_widget(Paragraph::new(format!("Close every session in this workspace, including background commands.\nEnter: send Ctrl+C; then finish programs and exit the shells.\nF: force all workspace session trees (available in {delay}s).\nEsc: cancel and return. Other terminals are not managed."))
            .block(Block::bordered().title(" Close DevTabs workspace ").border_style(Style::new().fg(Color::Yellow))),popup);
    }
}
