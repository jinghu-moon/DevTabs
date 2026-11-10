use crate::{
    config::{self, Config},
    events::Events,
    launch::{Agent, Layout, Options},
};
use anyhow::Result;
use clap::Parser;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout as UiLayout, Rect},
    style::{Color, Style},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
};
use std::{path::PathBuf, sync::mpsc, time::Duration};

enum Dialog {
    Root,
    Add,
    Exclude,
    Debug,
    Options,
    Agent,
}
pub fn run(terminal: &mut DefaultTerminal) -> Result<Option<Options>> {
    let mut config = Config::load()?;
    let mut projects = Vec::<PathBuf>::new();
    let mut active = std::collections::HashSet::<String>::new();
    let mut last_active = std::time::Instant::now() - Duration::from_secs(2);
    let mut query = String::new();
    let mut selected = 0usize;
    let mut dialog = None;
    let mut value = String::new();
    let mut message = String::new();
    let mut events = Events::new()?;
    let mut pending = None;
    let mut changed = true;
    let mut scanning = true;
    let start_scan = |c: &Config| {
        let c = c.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(c.scan());
        });
        rx
    };
    let mut scan = start_scan(&config);
    let mut opened: Option<Options> = None;
    let mut list_offset = 0;
    let mut list_area = Rect::default();
    let mut last_click: Option<(usize, std::time::Instant)> = None;
    loop {
        if last_active.elapsed() > Duration::from_secs(1) {
            last_active = std::time::Instant::now();
            if let Ok(crate::protocol::Response::Workspaces(work)) =
                crate::protocol::rpc(crate::protocol::Request::List)
            {
                let next = work.iter().map(|w| config::key(&w.project)).collect();
                if active != next {
                    active = next;
                    changed = true;
                }
            } else if !active.is_empty() {
                active.clear();
                changed = true;
            }
        }
        if let Ok(found) = scan.try_recv() {
            projects = found;
            scanning = false;
            changed = true;
        }
        let _filter_perf = crate::perf::span(crate::perf::Metric::OverviewFilter);
        let lower_query = query.to_lowercase();
        let matches = projects
            .iter()
            .filter(|p| p.to_string_lossy().to_lowercase().contains(&lower_query))
            .cloned()
            .collect::<Vec<_>>();
        drop(_filter_perf);
        selected = selected.min(matches.len().saturating_sub(1));
        if changed {
            let _perf = crate::perf::span(crate::perf::Metric::OverviewFrame);
            terminal.draw(|frame| {
                let area = frame.area();
                let pieces = UiLayout::vertical([
                    Constraint::Length(3), Constraint::Min(3), Constraint::Length(3),
                ]).split(area);
                frame.render_widget(
                    Paragraph::new(format!("Search all projects: {query}"))
                        .block(Block::bordered().title(" DevTabs · Projects ")), pieces[0]);
                let rows = matches.iter().map(|path| {
                    let settings = config.projects.get(&config::key(path));
                    let favorite = if settings.is_some_and(|s| s.favorite) { "★" } else { " " };
                    let marker = if active.contains(&config::key(path)) { "   [active]" } else { "" };
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    ListItem::new(format!("{favorite} {name}   {}{marker}", path.display()))
                }).collect::<Vec<_>>();
                let mut state = ListState::default().with_selected(
                    if matches.is_empty() { None } else { Some(selected) });
                *state.offset_mut() = list_offset;
                let title = if scanning { " Discovering… " } else { " Favorite → Recent → Other " };
                frame.render_stateful_widget(List::new(rows)
                    .highlight_style(Style::new().bg(Color::DarkGray).fg(Color::Cyan))
                    .block(Block::bordered().title(title)), pieces[1], &mut state);
                list_offset = state.offset();
                list_area = pieces[1].inner(ratatui::layout::Margin::new(1,1));
                frame.render_widget(Paragraph::new(format!(
                    "Enter Open · Alt+Enter Options · F2 Favorite · F3 Debug command · F4 Config · F5 Refresh\nCtrl+R Root · Ctrl+A Add · Ctrl+E Exclude · Esc Quit\n{message}")), pieces[2]);
                if let Some(mode) = &dialog {
                    let text = match mode {
                        Dialog::Root => "Add project root (scan direct children)",
                        Dialog::Add => "Add existing project directory",
                        Dialog::Exclude => "Exclude project directory",
                        Dialog::Debug => "Project Debug PowerShell source",
                        Dialog::Options => "Launch: 1 Codex  2 Claude  R Resume  L Layout  N New instance  T Current/new tab  Enter Open",
                        Dialog::Agent => "Choose global default once: 1 Codex / 2 Claude",
                    };
                    let popup = Rect::new(area.width/10, area.height/3,
                        area.width.saturating_mul(8)/10, area.height.min(7));
                    frame.render_widget(Clear, popup);
                    frame.render_widget(Paragraph::new(format!("{text}\n{value}\nEsc cancels"))
                        .block(Block::bordered().title(" Settings ")), popup);
                }
            })?;
            changed = false;
        }
        if let Some(event) = events.read(Duration::from_millis(30))? {
            let mut event = event.event;
            if dialog.is_none()
                && let Event::Mouse(mouse) = event
            {
                match mouse.kind {
                    MouseEventKind::ScrollUp => selected = selected.saturating_sub(3),
                    MouseEventKind::ScrollDown => {
                        selected = (selected + 3).min(matches.len().saturating_sub(1))
                    }
                    MouseEventKind::Down(MouseButton::Left)
                        if list_area.contains((mouse.column, mouse.row).into()) =>
                    {
                        let index = list_offset + usize::from(mouse.row - list_area.y);
                        if index < matches.len() {
                            selected = index;
                            if last_click.is_some_and(|(i, at)| {
                                i == index && at.elapsed() < Duration::from_millis(400)
                            }) {
                                event =
                                    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                                last_click = None;
                            } else {
                                last_click = Some((index, std::time::Instant::now()));
                            }
                        }
                    }
                    _ => {}
                }
                changed = true;
            }
            match event {
                Event::Resize(_, _) => changed = true,
                Event::Paste(text) if dialog.is_some() => {
                    value.push_str(&text);
                    changed = true;
                }
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    changed = true;
                    if let Some(mode) = &dialog {
                        match key.code {
                            KeyCode::Esc => {
                                dialog = None;
                                opened = None;
                            }
                            KeyCode::Char('1' | '2') if matches!(mode, Dialog::Agent) => {
                                let agent = Some(if key.code == KeyCode::Char('1') {
                                    Agent::Codex
                                } else {
                                    Agent::Claude
                                });
                                config = Config::update(|c| {
                                    c.default_agent = agent;
                                    Ok(())
                                })?;
                                dialog = None;
                            }
                            code if matches!(mode, Dialog::Options) => {
                                let options = opened.as_mut().unwrap();
                                match code {
                                    KeyCode::Char('1') => options.agent = Agent::Codex,
                                    KeyCode::Char('2') => options.agent = Agent::Claude,
                                    KeyCode::Char('r' | 'R') => options.resume = !options.resume,
                                    KeyCode::Char('n' | 'N') => {
                                        options.new_instance = !options.new_instance
                                    }
                                    KeyCode::Char('t' | 'T') => {
                                        pending = Some(!pending.unwrap_or(false));
                                    }
                                    KeyCode::Char('l' | 'L') => {
                                        options.layout = match options.layout {
                                            Layout::Float => Layout::Split,
                                            Layout::Split => Layout::Tabs,
                                            Layout::Tabs => Layout::Float,
                                        }
                                    }
                                    KeyCode::Enter => {
                                        let options = opened.take().unwrap();
                                        if pending == Some(true) {
                                            launch_tab(&options)?;
                                            dialog = None;
                                        } else {
                                            return Ok(Some(options));
                                        }
                                    }
                                    _ => {}
                                }
                                if let Some(options) = &opened {
                                    value = format!(
                                        "{:?} · resume {} · {:?} · new {} · {}",
                                        options.agent,
                                        options.resume,
                                        options.layout,
                                        options.new_instance,
                                        if pending == Some(true) {
                                            "new tab"
                                        } else {
                                            "current tab"
                                        }
                                    );
                                }
                            }
                            KeyCode::Enter => {
                                let outcome = (|| -> Result<()> {
                                    config = Config::update(|config| {
                                        match mode {
                                            Dialog::Root => {
                                                config.roots.push(dunce::canonicalize(&value)?)
                                            }
                                            Dialog::Add => {
                                                config.added.push(dunce::canonicalize(&value)?)
                                            }
                                            Dialog::Exclude => {
                                                config.excluded.push(dunce::canonicalize(&value)?)
                                            }
                                            Dialog::Debug => {
                                                if let Some(path) = matches.get(selected) {
                                                    config
                                                        .projects
                                                        .entry(config::key(path))
                                                        .or_default()
                                                        .debug_command = Some(value.clone());
                                                }
                                            }
                                            _ => {}
                                        }
                                        Ok(())
                                    })?;
                                    Ok(())
                                })();
                                match outcome {
                                    Ok(()) => {
                                        dialog = None;
                                        scan = start_scan(&config);
                                        scanning = true;
                                    }
                                    Err(error) => message = format!("{error:#}"),
                                }
                            }
                            KeyCode::Backspace => {
                                value.pop();
                            }
                            KeyCode::Char(c) => value.push(c),
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Esc => return Ok(None),
                        KeyCode::Up => selected = selected.saturating_sub(1),
                        KeyCode::Down => {
                            selected = (selected + 1).min(matches.len().saturating_sub(1))
                        }
                        KeyCode::F(2) => {
                            if let Some(path) = matches.get(selected) {
                                config = Config::update(|c| {
                                    let project = c.projects.entry(config::key(path)).or_default();
                                    project.favorite = !project.favorite;
                                    Ok(())
                                })?;
                                scan = start_scan(&config);
                            }
                        }
                        KeyCode::F(3) => {
                            if let Some(path) = matches.get(selected) {
                                dialog = Some(Dialog::Debug);
                                value = config
                                    .projects
                                    .get(&config::key(path))
                                    .and_then(|p| p.debug_command.clone())
                                    .unwrap_or_default();
                            }
                        }
                        KeyCode::F(4) => {
                            if !config::home().join("config.toml").exists() {
                                config = Config::update(|_| Ok(()))?;
                            }
                            std::process::Command::new("notepad.exe")
                                .arg(config::home().join("config.toml"))
                                .spawn()?;
                            message="Edit global Agent, project overrides and named sessions; F5 reloads after saving".into();
                        }
                        KeyCode::F(5) => match Config::load() {
                            Ok(c) => {
                                config = c;
                                scan = start_scan(&config);
                                scanning = true;
                            }
                            Err(error) => message = format!("{error:#}"),
                        },
                        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            dialog = Some(Dialog::Root);
                            value.clear();
                        }
                        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            dialog = Some(Dialog::Add);
                            value.clear();
                        }
                        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            dialog = Some(Dialog::Exclude);
                            value = matches
                                .get(selected)
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                        }
                        KeyCode::Backspace => {
                            query.pop();
                            selected = 0;
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            query.push(c);
                            selected = 0;
                        }
                        KeyCode::Enter => {
                            if let Some(path) = matches.get(selected) {
                                if config.default_agent.is_none() {
                                    dialog = Some(Dialog::Agent);
                                    continue;
                                }
                                config = Config::update(|c| {
                                    c.projects.entry(config::key(path)).or_default().last_opened =
                                        config::now();
                                    Ok(())
                                })?;
                                let project = config
                                    .projects
                                    .get(&config::key(path))
                                    .cloned()
                                    .unwrap_or_default();
                                let mut options =
                                    Options::parse_from(["devtabs", "--agent", "shell"]);
                                options.project = path.clone();
                                options.agent = project.agent.or(config.default_agent).unwrap();
                                options.debug_command = project.debug_command;
                                options.layout = project.layout.unwrap_or_default();
                                options.extra_sessions = project.sessions;
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    opened = Some(options);
                                    pending = Some(true);
                                    dialog = Some(Dialog::Options);
                                    value = "Choose launch options".into();
                                } else {
                                    message = match launch_tab(&options) {
                                        Ok(()) => {
                                            "Opened/reused workspace; Alt+Enter offers current tab"
                                                .into()
                                        }
                                        Err(error) => format!("{error:#}"),
                                    };
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}
pub fn launch_tab(options: &Options) -> Result<()> {
    crate::host::ensure()?;
    let info = match crate::protocol::rpc(crate::protocol::Request::Open {
        options: options.clone(),
    })? {
        crate::protocol::Response::Workspace(info) => info,
        _ => anyhow::bail!("No workspace returned"),
    };
    if crate::windows::focus_workspace(&info.id).is_ok() {
        return Ok(());
    }
    crate::windows::spawn_view(&info.id, "main", false)
}
