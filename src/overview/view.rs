mod statistics;
use super::*;
use crate::appearance::{palette, style};
use ratatui::{
    Frame,
    layout::{Constraint, Layout as UiLayout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use unicode_width::UnicodeWidthStr;
fn block(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::bordered()
        .title(title)
        .border_style(Style::new().fg(palette().muted))
}
pub(super) fn popup(frame: &mut Frame, width: u16, height: u16) -> Rect {
    let area = frame.area();
    let width = width.min(area.width);
    let height = height.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    crate::ui::clear_popup(frame, rect);
    frame.render_widget(Block::default().style(style()), rect);
    rect
}
fn info_line(label: impl AsRef<str>, value: impl std::fmt::Display) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}: ", label.as_ref()),
            Style::new().fg(palette().muted),
        ),
        Span::raw(crate::preview::safe_text(&value.to_string())),
    ])
}
fn pane(
    frame: &mut Frame,
    title: impl AsRef<str>,
    lines: Vec<Line<'static>>,
    rect: Rect,
    scroll: u16,
) {
    let inner = rect.inner(Margin::new(1, 1));
    let content = inner.inner(Margin::new(1, 0));
    frame.render_widget(block(format!(" {} ", title.as_ref())), rect);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let count = paragraph.line_count(content.width);
    let max = count.saturating_sub(content.height.into());
    let scroll = usize::from(scroll).min(max);
    frame.render_widget(paragraph.scroll((scroll as u16, 0)), content);
    if max > 0 {
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            inner,
            &mut ScrollbarState::new(max + 1)
                .position(scroll)
                .viewport_content_length(content.height.into()),
        )
    }
}
pub(super) fn draw(frame: &mut Frame, app: &mut Overview) {
    if let Some(viewer) = app.dashboard.viewer.as_mut() {
        frame.render_widget(Block::default().style(style()), frame.area());
        viewer.update_layout(frame.area(), false);
        viewer.draw(frame);
        let area = frame.area();
        if area.height > 0 {
            frame.render_widget(
                Paragraph::new(t("overview.viewer_hint")).style(Style::new().fg(palette().muted)),
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
            );
        }
        viewer.draw_overlay(frame);
        return;
    }
    let area = frame.area();
    if matches!(app.dialog, Some(Dialog::History { .. })) {
        app.dashboard.size = area;
        app.dialog_list = None;
        app.dialog_preview = Rect::default();
        app.dashboard.code_hits.clear();
        app.dashboard.detail_hits.clear();
        app.dashboard.heatmap_hits.clear();
        crate::ui::clear_popup(frame, area);
        frame.render_widget(Block::default().style(style()), area);
    } else {
        super::dashboard::draw(frame, app);
    }
    if let Some(dialog) = &app.dialog {
        match dialog {
            Dialog::Instances { path, selected } => {
                let rect = popup(frame, 100, 18);
                let key = config::key(path);
                let mut lines = vec![Line::from(t("overview.instances_hint"))];
                let page = rect.height.saturating_sub(3).max(1) as usize;
                let first = selected.saturating_sub(page - 1);
                if let Some(work) = app.workspaces.get(&key) {
                    lines.extend(
                        work.iter()
                            .enumerate()
                            .skip(first)
                            .take(page)
                            .map(|(i, w)| {
                                Line::from(format!(
                                    "{} {}. {} · {} {}",
                                    if i == *selected { "▸" } else { " " },
                                    i + 1,
                                    opening(&w.options),
                                    w.sessions.len(),
                                    t("sessions")
                                ))
                            }),
                    );
                }
                app.dialog_list = Some((
                    Rect::new(
                        rect.x + 2,
                        rect.y + 2,
                        rect.width.saturating_sub(4),
                        rect.height.saturating_sub(3),
                    ),
                    first,
                ));
                let lines = lines
                    .into_iter()
                    .map(|l| {
                        Line::from(super::dashboard::fit(
                            &l.to_string(),
                            rect.width.saturating_sub(4) as usize,
                        ))
                    })
                    .collect();
                pane(frame, t("overview.instances"), lines, rect, 0);
            }
            Dialog::Attention { selected } => {
                let rect = popup(frame, 100, 24);
                let rows = app.attention();
                let lines = rows
                    .iter()
                    .enumerate()
                    .map(|(i, (target, state))| {
                        let name = app
                            .workspaces
                            .values()
                            .flatten()
                            .flat_map(|w| w.tasks.iter())
                            .find(|t| t.id == target.task)
                            .map_or(target.task.as_str(), |t| t.name.as_str());
                        Line::from(format!(
                            "{} {} · {} · #{}",
                            if i == *selected { "▸" } else { " " },
                            name,
                            t(state.phase.label()),
                            state.result
                        ))
                    })
                    .collect();
                pane(frame, t("review.inbox"), lines, rect, 0);
            }
            Dialog::Diagnostics => {
                let rect = popup(frame, 110, area.height.saturating_sub(4));
                let mut lines = vec![info_line("Build", crate::protocol::BUILD_ID)];
                if !app.host_error.is_empty() {
                    lines.push(info_line(t("overview.host_unknown"), &app.host_error));
                }
                if let Some(path) = app.path()
                    && let Some(work) = app.workspaces.get(&config::key(&path))
                {
                    for w in work {
                        lines.push(Line::from(w.id.clone()));
                        for s in &w.sessions {
                            lines.push(Line::from(format!(
                                "{} · PID {:?} · {:?} · {:?}",
                                s.name, s.shell_pid, s.status, s.command
                            )));
                        }
                    }
                }
                pane(
                    frame,
                    t("overview.diagnostics"),
                    lines,
                    rect,
                    app.detail_scroll,
                );
            }
            Dialog::Queue { selected } => {
                let rect = popup(frame, 110, area.height.saturating_sub(4));
                frame.render_widget(block(format!(" {} ", t("overview.queue"))), rect);
                frame.render_widget(
                    Paragraph::new(super::dashboard::fit(
                        &t("overview.queue_hint"),
                        rect.width.saturating_sub(4) as usize,
                    )),
                    Rect::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), 1),
                );
                let list = Rect::new(
                    rect.x + 2,
                    rect.y + 3,
                    rect.width.saturating_sub(4),
                    rect.height.saturating_sub(6),
                );
                let page = list.height.max(1) as usize;
                let first = selected.saturating_sub(page - 1);
                app.dialog_list = Some((list, first));
                let options = app.queue_options();
                for (i, o) in options.iter().enumerate().skip(first).take(page) {
                    let value = format!(
                        "{} {} {} · {}",
                        if i == *selected { "▸" } else { " " },
                        i + 1,
                        o.project.file_name().unwrap_or_default().to_string_lossy(),
                        app.launch_description(o)
                    );
                    let line = format!(
                        "{} [×]",
                        super::dashboard::fit(&value, list.width.saturating_sub(4) as usize)
                    );
                    frame.render_widget(
                        Paragraph::new(line).style(if i == *selected {
                            Style::new().fg(palette().accent).bg(palette().selected)
                        } else {
                            style()
                        }),
                        Rect::new(list.x, list.y + (i - first) as u16, list.width, 1),
                    );
                }
                if let Some(o) = options.get(*selected) {
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{:?} · {}",
                            o.layout,
                            t(if o.new_window {
                                "parity.new_window"
                            } else {
                                "parity.current_window"
                            })
                        ))
                        .style(Style::new().fg(palette().muted)),
                        Rect::new(
                            rect.x + 2,
                            rect.bottom().saturating_sub(2),
                            rect.width.saturating_sub(4),
                            1,
                        ),
                    );
                }
            }

            Dialog::Help => {
                let rect = popup(frame, 100, 24);
                pane(
                    frame,
                    t("parity.help"),
                    vec![
                        Line::from(t("footer")),
                        Line::from(t("footer2")),
                        Line::from(t("footer3")),
                        Line::from(t("parity.help_extra")),
                        Line::from(t("overview.help_navigation")),
                        Line::from(t("settings_hint")),
                        Line::from(t("history_hint")),
                        Line::from(t("parity.help_limits")),
                    ],
                    rect,
                    0,
                );
            }
            Dialog::Presets { selected } => {
                let rect = popup(frame, 90, 22);
                let inner = rect.inner(Margin::new(2, 2));
                frame.render_widget(block(t("parity.presets")), rect);
                frame.render_widget(
                    Paragraph::new(t("parity.preset_hint")),
                    Rect::new(inner.x, inner.y, inner.width, 1),
                );
                let names = app.config.presets.keys().collect::<Vec<_>>();
                let page = inner.height.saturating_sub(1).max(1) as usize;
                let first = selected.saturating_sub(page - 1);
                let list = Rect::new(
                    inner.x,
                    inner.y + 1,
                    inner.width,
                    inner.height.saturating_sub(1),
                );
                app.dialog_list = Some((list, first));
                for (i, name) in names.iter().enumerate().skip(first).take(page) {
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} {name}",
                            if i == *selected { "▶" } else { " " }
                        ))
                        .style(if i == *selected {
                            Style::new().fg(palette().accent)
                        } else {
                            Style::new()
                        }),
                        Rect::new(list.x, list.y + (i - first) as u16, list.width, 1),
                    );
                }
            }
            Dialog::Input(kind, value) => {
                let title = t(match kind {
                    Input::NewProject(_) => "parity.new_project",
                    Input::RenamePreset(_) => "parity.rename_preset",
                    Input::Root => "root",
                    Input::Add => "add",
                    Input::Exclude => "exclude",
                    Input::Note(_) => "edit_note",
                    Input::Preset => "preset_name",
                });
                let rect = popup(frame, 86, 9);
                pane(
                    frame,
                    title,
                    vec![
                        Line::from(value.clone()),
                        Line::from(""),
                        Line::from(t("confirm")),
                    ],
                    rect,
                    0,
                )
            }
            Dialog::Settings {
                project,
                field,
                debug,
                arguments,
                ..
            } => {
                let rect = popup(frame, 100, 22);
                let total = 7 + project.sessions.len() * 3;
                let mut lines = vec![
                    Line::from(t("settings_hint")),
                    info_line(
                        format!("Alt+M · {}", t("overview.composition")),
                        project.terminal_composition().label(),
                    ),
                    Line::from(""),
                ];
                for i in 0..total {
                    let label = match i {
                        0 => t("agent").into_owned(),
                        1 => t("layout").into_owned(),
                        2 => t("debug").into_owned(),
                        3 => t("parity.title").into_owned(),
                        4 => t("parity.color").into_owned(),
                        5 => t("parity.arguments").into_owned(),
                        6 => t("parity.continue_latest").into_owned(),
                        _ => format!(
                            "{} {} · {}",
                            t("sessions"),
                            (i - 7) / 3 + 1,
                            t(match (i - 7) % 3 {
                                0 => "name",
                                1 => "command",
                                _ => "directory",
                            })
                        ),
                    };
                    lines.push(Line::from(Span::styled(
                        format!(
                            "{} {label}: {}",
                            if i == *field { "▶" } else { " " },
                            if i == 5 {
                                arguments.clone()
                            } else {
                                settings_value(project, debug, i)
                            }
                        ),
                        if i == *field {
                            Style::new().fg(palette().accent)
                        } else {
                            Style::new()
                        },
                    )));
                }
                pane(
                    frame,
                    t("settings"),
                    lines,
                    rect,
                    field.saturating_sub(rect.height.saturating_sub(6).max(1) as usize) as u16,
                );
            }
            Dialog::Options {
                options,
                new_tab,
                editing,
            } => {
                let rect = popup(frame, 100, 23);
                pane(
                    frame,
                    t("options"),
                    vec![
                        Line::from(t("options_hint")),
                        Line::from(""),
                        info_line(t("agent"), format!("{:?}", options.agent)),
                        info_line(t("layout"), format!("{:?}", options.layout)),
                        info_line(
                            format!("M · {}", t("overview.composition")),
                            options.effective_composition().label(),
                        ),
                        info_line(
                            t("parity.window"),
                            t(if options.new_window {
                                "parity.new_window"
                            } else {
                                "parity.current_window"
                            }),
                        ),
                        info_line(
                            t("parity.continue_latest"),
                            t(if options.continue_latest { "yes" } else { "no" }),
                        ),
                        info_line(t("resume"), t(if options.resume { "yes" } else { "no" })),
                        info_line(
                            t("new_instance"),
                            t(if options.new_instance { "yes" } else { "no" }),
                        ),
                        Line::from(t(if *new_tab { "new_tab" } else { "current_tab" })),
                        info_line("Ctrl+D", options.debug_command.as_deref().unwrap_or("—")),
                        info_line(
                            format!("Ctrl+A · {}", t("parity.arguments")),
                            serde_json::to_string(&options.agent_args).unwrap_or_default(),
                        ),
                        info_line(
                            "Ctrl+E",
                            options
                                .extra_sessions
                                .iter()
                                .map(|s| {
                                    format!(
                                        "{}: {} ({})",
                                        s.name,
                                        s.command.as_deref().unwrap_or("—"),
                                        s.directory
                                            .as_ref()
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_else(|| ".".into())
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ),
                        Line::from(t("overview.draft_fields")),
                        Line::from(if let Some((field, value)) = editing {
                            format!(
                                "{} → {}",
                                t(match *field {
                                    0 => "debug",
                                    1 => "parity.arguments",
                                    _ => match (field - 2) % 3 {
                                        0 => "name",
                                        1 => "command",
                                        _ => "directory",
                                    },
                                }),
                                value
                            )
                        } else {
                            String::new()
                        }),
                    ],
                    rect,
                    0,
                )
            }
            Dialog::Palette {
                query, selected, ..
            } => {
                let rect = popup(frame, 100, 22);
                frame.render_widget(
                    block(format!(" {} · {} ", t("palette"), t("palette_hint"))),
                    rect,
                );
                let pieces = UiLayout::vertical([Constraint::Length(2), Constraint::Min(1)])
                    .split(rect.inner(Margin::new(2, 1)));
                frame.render_widget(Paragraph::new(format!("> {query}")), pieces[0]);
                let entries = app.palette_entries(query);
                let items = entries
                    .iter()
                    .map(|e| {
                        ListItem::new(format!(
                            "{}  {}  {}",
                            e.label,
                            e.hint,
                            if e.enabled {
                                "".into()
                            } else {
                                t("unavailable")
                            }
                        ))
                        .style(Style::new().fg(if e.enabled {
                            palette().fg
                        } else {
                            palette().muted
                        }))
                    })
                    .collect::<Vec<_>>();
                let mut state = ListState::default().with_selected(Some(*selected));
                frame.render_stateful_widget(
                    List::new(items)
                        .highlight_style(Style::new().bg(palette().selected).fg(palette().accent)),
                    pieces[1],
                    &mut state,
                );
                app.dialog_list = Some((pieces[1], state.offset()));
            }
            Dialog::History {
                selected, scroll, ..
            } => {
                let rect = popup(frame, 110, area.height.saturating_sub(3));
                frame.render_widget(block(format!(" {} ", t("history"))), rect);
                let sessions = app
                    .history
                    .as_ref()
                    .map(|h| {
                        h.sessions
                            .iter()
                            .filter(|c| history::matches(c, &app.dashboard.history_query))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let inner = rect.inner(Margin::new(2, 1));
                let list_height = (sessions.len().min(8) as u16)
                    .max(1)
                    .min((inner.height / 3).max(1));
                let sections = UiLayout::vertical([
                    Constraint::Length(
                        Paragraph::new(format!(
                            "{}\n/ {}",
                            t("history_hint"),
                            if app.dashboard.history_query.is_empty() {
                                t("overview.session_search").into_owned()
                            } else {
                                app.dashboard.history_query.clone()
                            }
                        ))
                        .wrap(Wrap { trim: false })
                        .line_count(inner.width)
                        .min(u16::MAX as usize) as u16,
                    ),
                    Constraint::Length(list_height.saturating_add(1)),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n/ {}",
                        t("history_hint"),
                        if app.dashboard.history_query.is_empty() {
                            t("overview.session_search").into_owned()
                        } else {
                            app.dashboard.history_query.clone()
                        }
                    ))
                    .wrap(Wrap { trim: false }),
                    sections[0],
                );
                // Separate the selected session row from the conversation heading.
                let list_rect = Rect {
                    height: sections[1].height.saturating_sub(1),
                    ..sections[1]
                };
                let rows = sessions
                    .iter()
                    .map(|s| {
                        let updated = app
                            .history
                            .as_ref()
                            .and_then(|h| h.updated.get(&s.id))
                            .copied()
                            .unwrap_or(0);
                        ListItem::new(crate::preview::safe_text(&format!(
                            "{} · {}",
                            history::age_label(updated),
                            if s.prompt.is_empty() {
                                s.id.clone()
                            } else {
                                s.prompt.replace('\n', " ")
                            }
                        )))
                    })
                    .collect::<Vec<_>>();
                let mut state = ListState::default().with_selected(Some(*selected));
                frame.render_stateful_widget(
                    List::new(rows)
                        .highlight_style(Style::new().bg(palette().selected).fg(palette().accent)),
                    list_rect,
                    &mut state,
                );
                app.dialog_list = Some((list_rect, state.offset()));
                let preview = Rect::new(
                    sections[2].x,
                    sections[2].y.saturating_add(1),
                    sections[2].width.saturating_sub(1),
                    sections[2].height.saturating_sub(1),
                );
                app.dialog_preview = preview;
                frame.render_widget(
                    Block::default()
                        .borders(ratatui::widgets::Borders::TOP)
                        .title(format!(" {} ", t("overview.conversation")))
                        .border_style(Style::new().fg(palette().muted)),
                    sections[2],
                );
                let lines = if let Some(session) = sessions.get(*selected) {
                    let key = format!(
                        "{:?}|{}|{}|{:?}|{:?}",
                        session.file,
                        session.id,
                        preview.width,
                        crate::appearance::theme(),
                        crate::i18n::language()
                    );
                    if app
                        .dashboard
                        .history_preview
                        .as_ref()
                        .is_none_or(|(cached, _)| *cached != key)
                    {
                        let mut output = crate::preview::MarkdownText::default();
                        let light = crate::appearance::theme() == crate::appearance::Theme::Light;
                        for turn in &session.turns {
                            output.lines.push(role_heading(turn.user));
                            let color = role_color(turn.user);
                            let mut rendered = crate::preview::markdown_summary(
                                &turn.text,
                                preview.width.saturating_sub(2).max(1),
                                light,
                            );
                            for copy in &mut rendered.copies {
                                copy.row += output.lines.len();
                            }
                            output.copies.extend(rendered.copies);
                            for mut line in rendered.lines {
                                line.spans
                                    .insert(0, Span::styled("▎ ", Style::new().fg(color)));
                                output.lines.push(line);
                            }
                            output.lines.push(Line::default());
                        }
                        app.dashboard.history_preview = Some((key, output));
                    }
                    app.dashboard.history_preview.as_ref().unwrap().1.clone()
                } else {
                    crate::preview::MarkdownText {
                        lines: vec![Line::from(t(if app.dashboard.history_pending {
                            "loading"
                        } else {
                            "history_empty"
                        }))],
                        copies: Vec::new(),
                    }
                };
                let copies = lines.copies;
                let paragraph = Paragraph::new(lines.lines).wrap(Wrap { trim: false });
                let count = paragraph.line_count(preview.width).min(u16::MAX as usize) as u16;
                let visible_scroll = (*scroll).min(count.saturating_sub(preview.height));
                frame.render_widget(paragraph.scroll((visible_scroll, 0)), preview);
                let copy_width = t("ui.copy").width() as u16;
                for copy in copies {
                    if copy.row >= visible_scroll as usize
                        && copy.row - (visible_scroll as usize) < preview.height as usize
                        && preview.width >= copy_width
                    {
                        app.dashboard.code_hits.push((
                            Rect::new(
                                preview.right() - copy_width,
                                preview.y + (copy.row - visible_scroll as usize) as u16,
                                copy_width,
                                1,
                            ),
                            copy.text,
                        ));
                    }
                }
                if count > preview.height && preview.height > 0 {
                    frame.render_stateful_widget(
                        Scrollbar::new(ScrollbarOrientation::VerticalRight),
                        Rect::new(sections[2].x, preview.y, sections[2].width, preview.height),
                        &mut ScrollbarState::new(
                            usize::from(count.saturating_sub(preview.height)) + 1,
                        )
                        .position(visible_scroll.into())
                        .viewport_content_length(preview.height.into()),
                    );
                }
                if let Some(h) = &app.history {
                    frame.render_widget(
                        Paragraph::new(
                            if app
                                .dashboard
                                .copied_code
                                .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
                            {
                                t("ui.copied_code_block").into_owned()
                            } else {
                                tf(
                                    "history_status",
                                    &[
                                        ("count", h.sessions.len().to_string()),
                                        ("skipped", h.skipped.to_string()),
                                        ("limited", t(if h.limited { "yes" } else { "no" }).into()),
                                    ],
                                )
                            },
                        ),
                        sections[3],
                    )
                }
                if let Some(Dialog::History { scroll, .. }) = &mut app.dialog {
                    *scroll = visible_scroll;
                }
            }
            Dialog::Batch => {
                let rect = popup(frame, 104, 22);
                app.dialog_preview = rect;
                let mut lines = vec![Line::from(t("batch_hint")), Line::from("")];
                for (i, item) in app.batch.items.iter().enumerate() {
                    let label = match &item.state {
                        LaunchState::Queued => t("queued"),
                        LaunchState::Opening => t("opening"),
                        LaunchState::Opened(_) => t("opened"),
                        LaunchState::Failed(_) => t("failed"),
                    };
                    lines.push(Line::from(format!(
                        "{}. {} · {label}",
                        i + 1,
                        item.options.project.display()
                    )));
                    if let LaunchState::Failed(error) = &item.state {
                        lines.extend(error.lines().map(|e| {
                            Line::from(Span::styled(
                                format!("  {e}"),
                                Style::new().fg(palette().error),
                            ))
                        }));
                    }
                }
                pane(frame, t("batch"), lines, rect, app.batch_scroll)
            }
        }
    }
}
pub(super) fn opening(options: &Options) -> String {
    format!(
        "{:?} · {} · {} · {:?}",
        options.agent,
        options.effective_composition().label(),
        t(if options.fresh {
            "overview.new"
        } else if options.resume_id.is_some() {
            "overview.specific"
        } else if options.resume {
            "overview.native"
        } else if options.continue_latest {
            "overview.latest"
        } else {
            "overview.new"
        }),
        options.layout
    )
}
pub(super) fn detail_lines(app: &mut Overview, width: u16) -> Vec<Line<'static>> {
    app.dashboard.detail_copies.clear();
    use super::dashboard::DetailAction;
    app.dashboard.detail_actions.clear();
    if let Some(task) = app.selected_task() {
        let mut lines = vec![
            section(task.name.clone(), width),
            info_line(t("agent"), format!("{:?}", task.options.agent)),
            info_line(t("branch"), task.branch.clone()),
            Line::from(task.cwd.display().to_string()),
            Line::from(t("parallel.connect_existing")),
            Line::from(""),
            section(t("history").into_owned(), width),
        ];
        if let Some(id) = &task.options.resume_id {
            lines.push(Line::from(id.clone()));
            lines.extend(conversation(app, &task.cwd, &task.options, width));
        } else {
            lines.push(Line::from(t("parallel.conversation_unbound")));
        }
        return lines;
    }
    let Some(path) = app.path() else {
        return vec![Line::from(t("no_project"))];
    };
    let key = config::key(&path);
    let project = app
        .profiles
        .get(&key)
        .or_else(|| app.config.projects.get(&key))
        .cloned()
        .unwrap_or_default();
    let options = app.options_for(&path);
    let mut lines =
        vec![
            Line::from(vec![
                Span::styled(
                    "■ ",
                    Style::new().fg(project
                        .color
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| stable_color(&key))),
                ),
                Span::styled(
                    crate::preview::safe_text(project.title.as_deref().unwrap_or_else(|| {
                        path.file_name().and_then(|n| n.to_str()).unwrap_or("")
                    })),
                    Style::new().fg(palette().fg).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                crate::preview::safe_text(&path.display().to_string()),
                Style::new().fg(palette().muted),
            )),
        ];
    match app.git.get(&key) {
        Some(Ok(g)) => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}: ", t("branch")),
                    Style::new().fg(palette().muted),
                ),
                Span::styled(
                    crate::preview::safe_text(&g.branch),
                    Style::new().fg(palette().accent),
                ),
                Span::raw(" · "),
                Span::styled(
                    if g.changed == 0 {
                        t("overview.clean").into_owned()
                    } else {
                        format!("{} {}", g.changed, t("overview.changes"))
                    },
                    Style::new().fg(if g.changed == 0 {
                        ratatui::style::Color::Green
                    } else {
                        ratatui::style::Color::Yellow
                    }),
                ),
                Span::styled(
                    if g.conflicts > 0 {
                        format!(" · !{} {}", g.conflicts, t("overview.conflict"))
                    } else {
                        String::new()
                    },
                    Style::new().fg(palette().error),
                ),
                Span::styled(
                    if g.ahead > 0 {
                        format!(" · ↑{}", g.ahead)
                    } else {
                        String::new()
                    },
                    Style::new().fg(ratatui::style::Color::Green),
                ),
                Span::styled(
                    if g.behind > 0 {
                        format!(" ↓{}", g.behind)
                    } else {
                        String::new()
                    },
                    Style::new().fg(palette().error),
                ),
            ]));
        }
        Some(Err(error)) => lines.push(info_line(t("not_git"), crate::preview::safe_text(error))),
        None => lines.push(Line::from(t("loading"))),
    }
    if let Some(error) = app.profile_errors.get(&key) {
        lines.push(info_line(
            t("parity.source_error"),
            crate::preview::safe_text(error),
        ));
    }
    lines.push(Line::from(""));
    app.dashboard
        .detail_actions
        .push((lines.len(), DetailAction::Note));
    lines.push(section(format!("{}  [F3]", t("overview.next_step")), width));
    lines.extend(if project.note.is_empty() {
        vec![Line::from(t("overview.no_note"))]
    } else {
        project
            .note
            .lines()
            .take(if app.dashboard.expanded { 64 } else { 3 })
            .map(|l| Line::from(crate::preview::safe_text(l)))
            .collect()
    });
    lines.push(Line::from(""));
    app.dashboard
        .detail_actions
        .push((lines.len(), DetailAction::Options));
    lines.push(section(format!("{}  [Alt+Enter]", t("options")), width));
    lines.push(Line::from(Span::styled(
        opening(&options),
        Style::new().fg(palette().accent),
    )));
    if app.changed_defaults.contains(&key) {
        lines.push(Line::from(t("overview.defaults_changed")));
    }
    lines.push(info_line(
        t("parity.window"),
        t(if options.new_window {
            "parity.new_window"
        } else {
            "parity.current_window"
        }),
    ));
    if options.composition == crate::launch::Composition::Custom
        && !options.extra_sessions.is_empty()
    {
        lines.push(info_line(
            t("extra"),
            options
                .extra_sessions
                .iter()
                .map(|s| crate::preview::safe_text(&s.name))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if !app.host_error.is_empty() {
        app.dashboard
            .detail_actions
            .push((lines.len(), DetailAction::Diagnostics));
        lines.push(Line::from(Span::styled(
            t("overview.host_retry"),
            Style::new().fg(palette().error),
        )));
    } else if let Some(work) = app.workspaces.get(&key) {
        app.dashboard
            .detail_actions
            .push((lines.len(), DetailAction::Instances));
        lines.push(info_line(
            t("running"),
            format!("{} · {}", work.len(), t("overview.activate")),
        ));
        if work.iter().any(|w| {
            w.options.agent != options.agent
                || w.options.composition != options.composition
                || w.options.layout != options.layout
                || w.options.resume_id != options.resume_id
        }) {
            lines.push(Line::from(t("overview.reuse_warning")));
        }
    }
    if let Some(g) = app
        .git
        .get(&key)
        .and_then(|r| r.as_ref().ok())
        .filter(|g| !g.files.is_empty())
    {
        lines.push(Line::from(""));
        app.dashboard
            .detail_actions
            .push((lines.len(), DetailAction::Expand));
        lines.push(section(
            format!("{} · {}  [E]", t("dirty"), g.changed),
            width,
        ));
        for file in g.files.iter().take(if app.dashboard.expanded {
            usize::MAX
        } else {
            5
        }) {
            app.dashboard
                .detail_actions
                .push((lines.len(), DetailAction::File(path.join(file))));
            let status = g.statuses.get(file).map_or("M", String::as_str);
            let color = if status.contains('D') {
                palette().error
            } else if status.contains('A') || status == "?" {
                ratatui::style::Color::Green
            } else {
                ratatui::style::Color::Yellow
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {status} "), Style::new().fg(color)),
                Span::raw(super::dashboard::fit(
                    file,
                    width.saturating_sub(status.len() as u16 + 3) as usize,
                )),
            ]));
        }
    }
    let context_key = super::dashboard::context_key(&path, options.agent);
    if let Some(history) = app
        .dashboard
        .contexts
        .get(&context_key)
        .filter(|h| !h.sessions.is_empty())
    {
        lines.push(Line::default());
        app.dashboard
            .detail_actions
            .push((lines.len(), DetailAction::History));
        lines.push(section(
            format!(
                "{} · {} [F9]",
                t("overview.sessions"),
                history.sessions.len()
            ),
            width,
        ));
        let width = usize::from(width);
        let age_width = t("overview.session_updated").width().max(6);
        let wide = width >= 36 + age_width + 12;
        let name_width = width
            .saturating_sub(if wide {
                36 + age_width + 4
            } else {
                age_width + 2
            })
            .max(1);
        let fit = super::dashboard::fit;
        let header = if wide {
            format!(
                "{}  {}  {}",
                fit(&t("name"), name_width),
                fit(&t("overview.session_id"), 36),
                t("overview.session_updated")
            )
        } else {
            format!(
                "{}  {}",
                fit(&t("name"), name_width),
                t("overview.session_updated")
            )
        };
        lines.push(Line::from(Span::styled(
            fit(&header, width),
            Style::new().fg(palette().muted).bold(),
        )));
        if !wide {
            lines.push(Line::from(Span::styled(
                fit(&t("overview.session_id"), width),
                Style::new().fg(palette().muted).bold(),
            )));
        }
        lines.push(Line::from(Span::styled(
            "─".repeat(width),
            Style::new().fg(palette().muted),
        )));
        for (i, c) in history
            .sessions
            .iter()
            .enumerate()
            .take(if app.dashboard.expanded {
                usize::MAX
            } else {
                4
            })
        {
            let current = options.resume_id.as_ref().map_or(i == 0, |id| *id == c.id);
            let style = if current {
                Style::new().fg(palette().accent).bold()
            } else {
                Style::new().fg(palette().fg)
            };
            let label = if c.prompt.is_empty() {
                &c.id
            } else {
                &c.prompt
            };
            let age = history
                .updated
                .get(&c.id)
                .copied()
                .map(history::age_label)
                .unwrap_or_else(|| "—".into());
            app.dashboard
                .detail_actions
                .push((lines.len(), DetailAction::Resume(c.id.clone())));
            let row = if wide {
                format!(
                    "{}  {}  {}",
                    fit(
                        &format!("{} {}", if current { "▸" } else { " " }, label),
                        name_width
                    ),
                    fit(&c.id, 36),
                    fit(&age, age_width)
                )
            } else {
                format!(
                    "{}  {}",
                    fit(
                        &format!("{} {}", if current { "▸" } else { " " }, label),
                        name_width
                    ),
                    fit(&age, age_width)
                )
            };
            lines.push(Line::from(Span::styled(fit(&row, width), style)));
            if !wide || c.id.width() > 36 {
                let id = crate::preview::safe_text(&c.id);
                let chars = id.chars().collect::<Vec<_>>();
                for chunk in chars.chunks(width.saturating_sub(2).max(1)) {
                    app.dashboard
                        .detail_actions
                        .push((lines.len(), DetailAction::Resume(c.id.clone())));
                    lines.push(Line::from(Span::styled(
                        fit(&format!("  {}", chunk.iter().collect::<String>()), width),
                        Style::new().fg(palette().muted),
                    )));
                }
            }
        }
        if !app.dashboard.expanded && history.sessions.len() > 4 {
            lines.push(Line::from(Span::styled(
                t("overview.more_sessions"),
                Style::new().fg(palette().muted),
            )));
        }
    }
    lines.push(Line::from(""));
    app.dashboard
        .detail_actions
        .push((lines.len(), DetailAction::History));
    lines.push(section(
        format!("{}  [F9]", t("overview.conversation")),
        width,
    ));
    let offset = lines.len();
    lines.extend(conversation(app, &path, &options, width));
    app.dashboard.detail_copies = app
        .dashboard
        .summary_copies
        .iter()
        .cloned()
        .map(|mut copy| {
            copy.row += offset;
            copy
        })
        .collect();
    for target in &app.dashboard.summary_links {
        if super::dashboard::local_reference(&path, target).is_some() {
            app.dashboard
                .detail_actions
                .push((lines.len(), DetailAction::Link(target.clone())));
            lines.push(Line::from(Span::styled(
                format!("↗ {}", crate::preview::safe_text(target)),
                Style::new()
                    .fg(palette().accent)
                    .add_modifier(Modifier::UNDERLINED),
            )));
        }
    }
    lines.push(Line::from(""));
    app.dashboard
        .detail_actions
        .push((lines.len(), DetailAction::Expand));
    lines.push(section(format!("{}  [E]", t("commits")), width));
    let stats = app
        .stats
        .get(&key)
        .or_else(|| app.dashboard.sources.get(&key));
    if let Some(s) = stats {
        if let Some(error) = &s.git_error {
            lines.push(Line::from(error.clone()));
        } else if s.commits.is_empty() {
            lines.push(Line::from(t("overview.no_commits")));
        } else {
            lines.extend(
                s.commits
                    .iter()
                    .take(if app.dashboard.expanded { 6 } else { 3 })
                    .map(|s| Line::from(crate::preview::safe_text(s))),
            );
        }
    } else {
        lines.push(Line::from(t("loading")));
    }
    lines
}
fn section(title: String, width: u16) -> Line<'static> {
    use unicode_width::UnicodeWidthStr;
    let title = crate::preview::truncate_name(&title, width as usize);
    let used = title.width();
    let mut spans = vec![Span::styled(title, Style::new().fg(palette().fg).bold())];
    if used < width as usize {
        spans.push(Span::styled(
            format!(" {}", "─".repeat(width as usize - used - 1)),
            Style::new().fg(palette().muted),
        ));
    }
    Line::from(spans)
}
fn role_color(user: bool) -> ratatui::style::Color {
    use ratatui::style::Color;
    let light = crate::appearance::theme() == crate::appearance::Theme::Light;
    match (user, light) {
        (true, true) => Color::Rgb(145, 84, 0),
        (true, false) => Color::Rgb(237, 174, 85),
        (false, true) => Color::Rgb(0, 82, 160),
        (false, false) => Color::Rgb(125, 207, 255),
    }
}
fn role_heading(user: bool) -> Line<'static> {
    use ratatui::style::Color;
    let light = crate::appearance::theme() == crate::appearance::Theme::Light;
    Line::from(Span::styled(
        format!(" {} ", t(if user { "user" } else { "overview.role_ai" })),
        Style::new()
            .fg(if light { Color::White } else { Color::Black })
            .bg(role_color(user))
            .bold(),
    ))
}
fn conversation(
    app: &mut Overview,
    path: &std::path::Path,
    options: &Options,
    width: u16,
) -> Vec<Line<'static>> {
    app.dashboard.summary_links.clear();
    app.dashboard.summary_copies.clear();
    if options.agent == Agent::Shell {
        return vec![Line::from(t("history_shell"))];
    }
    if options.fresh || (!options.continue_latest && !options.resume && options.resume_id.is_none())
    {
        return vec![Line::from(t("overview.new"))];
    }
    let key = super::dashboard::context_key(path, options.agent);
    let Some(history) = app.dashboard.contexts.get(&key) else {
        return vec![Line::from(t("loading"))];
    };
    let selected = options
        .resume_id
        .as_ref()
        .and_then(|id| history.sessions.iter().find(|s| s.id == *id))
        .or_else(|| {
            if options.resume_id.is_none() {
                history.sessions.first()
            } else {
                None
            }
        });
    let Some(session) = selected else {
        return vec![Line::from(t(
            if history.skipped > 0 || history.limited || options.resume_id.is_some() {
                "overview.history_unknown"
            } else {
                "history_empty"
            },
        ))];
    };
    if session.turns.is_empty() {
        let identity = format!("{key}|{}", session.id);
        return vec![Line::from(t(
            if app.dashboard.session_loaded.contains(&identity) {
                "overview.user_unavailable"
            } else {
                "loading"
            },
        ))];
    }
    let cache_key = format!(
        "{key}|{}|{width}|{}|{:?}|{:?}",
        session.id,
        app.dashboard.expanded,
        crate::appearance::theme(),
        crate::i18n::language()
    );
    if let Some(cached) = &app.dashboard.summary_cache
        && cached.key == cache_key
    {
        crate::perf::add(crate::perf::Count::OverviewContextHits, 1);
        app.dashboard.summary_links = cached.links.clone();
        app.dashboard.summary_copies = cached.copies.clone();
        return cached.lines.clone();
    }
    let mut lines = vec![Line::from(Span::styled(
        history.updated.get(&session.id).map_or_else(
            || crate::preview::safe_text(&session.timestamp),
            |updated| {
                format!(
                    "{} · {}",
                    t("overview.updated"),
                    history::age_label(*updated)
                )
            },
        ),
        Style::new().fg(palette().muted),
    ))];
    let mut selected: Vec<_> = if app.dashboard.expanded {
        session.turns.iter().rev().take(6).collect()
    } else {
        let user = session.turns.iter().rposition(|t| t.user);
        let ai = session.turns.iter().rposition(|t| !t.user);
        let mut indices = user.into_iter().chain(ai).collect::<Vec<_>>();
        indices.sort_unstable();
        indices.into_iter().map(|i| &session.turns[i]).collect()
    };
    if app.dashboard.expanded {
        selected.reverse();
    }
    if !session.turns.iter().any(|t| t.user) {
        lines.push(Line::from(Span::styled(
            t("overview.user_unavailable"),
            Style::new().fg(palette().muted),
        )));
    }
    for turn in selected {
        let light = crate::appearance::theme() == crate::appearance::Theme::Light;
        let role_color = role_color(turn.user);
        lines.push(role_heading(turn.user));
        let limit = if app.dashboard.expanded { 8192 } else { 2048 };
        let text = if turn.user {
            turn.text.chars().take(limit).collect::<String>()
        } else {
            turn.text
                .chars()
                .rev()
                .take(limit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        };
        for event in pulldown_cmark::Parser::new(&text) {
            if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) = event
                && app.dashboard.summary_links.len() < 8
                && !app
                    .dashboard
                    .summary_links
                    .iter()
                    .any(|s| s == dest_url.as_ref())
            {
                app.dashboard.summary_links.push(dest_url.into_string());
            }
        }
        let rendered =
            crate::preview::markdown_summary(&text, width.saturating_sub(2).max(1), light);
        let copies = rendered.copies;
        let mut rendered = rendered.lines;
        let original_length = rendered.len();
        let mut removed = 0;
        let max = if app.dashboard.expanded { 120 } else { 5 };
        if rendered.len() > max {
            if turn.user {
                rendered.truncate(max);
                rendered.push(Line::from("…"));
            } else {
                removed = rendered.len() - max;
                rendered = rendered.split_off(removed);
                rendered.insert(0, Line::from("…"));
            }
        }
        for mut copy in copies {
            let retained = if original_length > max {
                if turn.user {
                    copy.row < max
                } else {
                    copy.row >= removed
                }
            } else {
                true
            };
            if retained {
                copy.row =
                    lines.len() + copy.row - removed + usize::from(!turn.user && removed > 0);
                app.dashboard.summary_copies.push(copy);
            }
        }
        for mut line in rendered {
            line.spans
                .insert(0, Span::styled("▎ ", Style::new().fg(role_color)));
            lines.push(line);
        }
        lines.push(Line::from(""));
    }
    if history.skipped > 0 || history.limited {
        lines.push(Line::from(t("overview.history_unknown")));
    }
    app.dashboard.summary_cache = Some(super::dashboard::Summary {
        key: cache_key,
        lines: lines.clone(),
        links: app.dashboard.summary_links.clone(),
        copies: app.dashboard.summary_copies.clone(),
    });
    lines
}
pub(super) fn human_bytes(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut i = 0;
    while value >= 1024.0 && i < units.len() - 1 {
        value /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[i])
    }
}
pub(super) fn stats_lines(app: &Overview, width: u16) -> Vec<Line<'static>> {
    let Some(path) = app.active_path() else {
        return vec![Line::from(t("no_project"))];
    };
    let key = config::key(&path);
    match app
        .stats
        .get(&key)
        .or_else(|| app.dashboard.sources.get(&key))
    {
        Some(stats) => statistics::lines_for(stats, width as usize, app.dashboard.stats_year),
        None => vec![Line::from(t("loading"))],
    }
}
pub(super) fn stats_chart_hits(
    app: &Overview,
    width: u16,
) -> Vec<(usize, usize, usize, dashboard::HeatmapAction)> {
    let Some(path) = app.active_path() else {
        return Vec::new();
    };
    let key = config::key(&path);
    app.stats
        .get(&key)
        .or_else(|| app.dashboard.sources.get(&key))
        .map(|s| statistics::chart_hits(s, width as usize, app.dashboard.stats_year))
        .unwrap_or_default()
}
pub(super) fn stable_color(key: &str) -> ratatui::style::Color {
    use ratatui::style::Color;
    let colors = if crate::appearance::theme() == crate::appearance::Theme::Light {
        [
            Color::Blue,
            Color::Magenta,
            Color::Rgb(0, 110, 90),
            Color::Rgb(150, 70, 0),
        ]
    } else {
        [
            Color::Cyan,
            Color::LightMagenta,
            Color::LightGreen,
            Color::LightYellow,
        ]
    };
    let hash = key.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    colors[hash as usize % colors.len()]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responsive_layout_renders_all_languages_without_panics() {
        for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
            crate::i18n::with_language(language, || {
                for (width, height) in [(1, 1), (35, 10), (90, 24), (130, 40), (190, 50)] {
                    let mut app = Overview::new(Config::default());
                    app.projects = vec![PathBuf::from(r"D:\项目\hello")];
                    app.rebuild();
                    app.show_stats = true;
                    let mut terminal =
                        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                            .unwrap();
                    terminal.draw(|f| draw(f, &mut app)).unwrap();
                }
            });
        }
    }
}
