use super::*;
use crossterm::event::{MouseEvent, MouseEventKind};
#[test]
fn task_rows_are_distinct_from_history_and_preserve_selection_after_reordering() {
    let mut app = overview();
    let mut options = crate::launch::Options::parse_for_view();
    options.project = "a".into();
    let task = |id: &str| crate::tasks::Task {
        id: id.into(),
        name: id.into(),
        project: "a".into(),
        checkout: format!("a-{id}").into(),
        cwd: format!("a-{id}").into(),
        branch: id.into(),
        base: "commit".into(),
        owned: true,
        archived: false,
        options: options.clone(),
    };
    let sessions = ["A", "B"]
        .into_iter()
        .map(|id| crate::protocol::SessionInfo {
            id: format!("session-{id}"),
            task: Box::new(crate::tasks::Binding {
                task_id: id.into(),
                run_id: id.into(),
                role: crate::tasks::Role::Agent,
                initial_cwd: format!("a-{id}").into(),
            }),
            name: "Agent".into(),
            shell_pid: Some(1),
            status: crate::session::Status::Running,
            command: crate::session::CommandStatus::Running,
            placement: crate::presentation::Placement::Docked,
            activity: Default::default(),
        })
        .collect();
    let work = crate::protocol::WorkspaceInfo {
        id: "w".into(),
        project: "a".into(),
        options: options.clone(),
        sessions,
        tasks: vec![task("A"), task("B")],
    };
    app.workspaces
        .insert(config::key(std::path::Path::new("a")), vec![work]);
    app.dashboard
        .expanded_projects
        .insert(config::key(std::path::Path::new("a")));
    app.dashboard.store_context(
        "a|Codex".into(),
        history::History {
            sessions: (0..40)
                .map(|i| history::Conversation {
                    id: format!("history-{i}"),
                    timestamp: String::new(),
                    prompt: String::new(),
                    turns: vec![],
                    file: format!("{i}.jsonl").into(),
                })
                .collect(),
            ..Default::default()
        },
    );
    app.rebuild();
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Task(..)))
            .count(),
        2
    );
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Project(..)))
            .count(),
        2
    );
    app.selected = app
        .rows
        .iter()
        .position(|r| matches!(r,Row::Task(_,_,id) if id=="B"))
        .unwrap();
    app.workspaces.values_mut().next().unwrap()[0]
        .tasks
        .reverse();
    app.rebuild();
    assert!(matches!(app.rows.get(app.selected),Some(Row::Task(_,_,id)) if id=="B"));
    assert_eq!(
        app.options_for(std::path::Path::new("a"))
            .workspace
            .as_deref(),
        Some("w")
    );
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Session(..)))
            .count(),
        40
    );
    assert_eq!(
        app.options_for(std::path::Path::new("a"))
            .navigation
            .as_ref()
            .unwrap()
            .task,
        "B"
    );
    app.toggle_selected_queue();
    app.selected = app
        .rows
        .iter()
        .position(|r| matches!(r,Row::Task(_,_,id) if id=="A"))
        .unwrap();
    app.toggle_selected_queue();
    assert_eq!(app.queue_options().len(), 2);
    assert_eq!(
        app.queue_options()[0].navigation.as_ref().unwrap().task,
        "B"
    );
    assert_eq!(
        app.queue_options()[1].navigation.as_ref().unwrap().task,
        "A"
    );
    app.query = "a-B".into();
    app.rebuild();
    assert!(
        app.rows
            .iter()
            .any(|r| matches!(r,Row::Task(_,_,id) if id=="B"))
    );
}
fn overview() -> Overview {
    let mut app = Overview::new(Config::default());
    app.projects = vec![PathBuf::from("a"), PathBuf::from("b")];
    app.rebuild();
    app.selected = 1;
    app
}
fn draw(app: &mut Overview, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| view::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn mouse_at(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}
#[test]
fn settings_composition_matches_implicit_custom_launch_and_cycles_from_that_value() {
    use crate::launch::Composition;
    let t = worker::isolated();
    let mut app = overview();
    let path = app.path().unwrap();
    app.config
        .projects
        .entry(config::key(&path))
        .or_default()
        .sessions
        .push(crate::config::SessionConfig {
            name: "Docs".into(),
            command: None,
            directory: None,
        });
    assert_eq!(app.options_for(&path).composition, Composition::Custom);
    app.command(&t.worker, Command::Settings).unwrap();
    assert!(
        matches!(&app.dialog, Some(Dialog::Settings { project, .. }) if project.terminal_composition() == Composition::Custom)
    );
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT),
    )
    .unwrap();
    assert!(
        matches!(&app.dialog, Some(Dialog::Settings { project, .. }) if project.composition == Some(Composition::AgentOnly) && project.sessions.len() == 1)
    );
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
    )
    .unwrap();
    assert!(
        matches!(&app.dialog, Some(Dialog::Settings { project, .. }) if project.composition == Some(Composition::Custom) && project.sessions.len() == 2)
    );
    assert!(t.edits.try_recv().is_err());
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn detail_stats_and_queue_focus_preserve_global_launch_shortcuts() {
    for focus in [
        dashboard::Focus::Detail,
        dashboard::Focus::Stats,
        dashboard::Focus::Queue,
    ] {
        let t = worker::isolated();
        let mut app = overview();
        app.marked.push("b".into());
        app.dashboard.focus = focus;
        app.key(
            &t.worker,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        )
        .unwrap();
        let Action::Launch(_, jobs) = t.launches.try_recv().unwrap() else {
            panic!("queue launch")
        };
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].1.project, PathBuf::from("b"));
        let t = worker::isolated();
        let mut app = overview();
        app.dashboard.focus = focus;
        app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))
            .unwrap();
        assert!(matches!(app.dialog, Some(Dialog::Options { .. })));
        assert!(t.launches.try_recv().is_err());
    }
}
#[test]
fn detail_text_selection_is_explicit_bounded_and_canceled_without_launching() {
    let t = worker::isolated();
    let mut app = overview();
    draw(&mut app, 200, 50);
    let inner = app
        .dashboard
        .columns
        .unwrap()
        .detail
        .unwrap()
        .inner(ratatui::layout::Margin::new(2, 1));
    let mut mouse = mouse_at(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y);
    mouse.modifiers = KeyModifiers::ALT;
    app.mouse(&t.worker, mouse).unwrap();
    assert!(app.dashboard.text_selection.is_some());
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Drag(MouseButton::Left), u16::MAX, u16::MAX),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Up(MouseButton::Left), u16::MAX, u16::MAX),
    )
    .unwrap();
    let selection = app.dashboard.text_selection.as_ref().unwrap();
    assert_eq!(selection.range.end.col, inner.right() - 1);
    assert_eq!(selection.range.end.row, inner.bottom() - 1);
    assert!(!selection.dragging);
    assert!(!selection.text().is_empty());
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 8, 1));
    buffer.set_string(0, 0, "你好 hi", ratatui::style::Style::default());
    let selection = dashboard::TextSelection {
        buffer,
        range: crate::selection::Selection {
            start: crate::selection::Point { row: 0, col: 1 },
            end: crate::selection::Point { row: 0, col: 5 },
        },
        dragging: false,
    };
    assert_eq!(selection.text(), "你好 h");
    app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.dashboard.text_selection.is_none());
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn draft_editors_stage_commands_arguments_and_extras_without_saving_defaults() {
    let t = worker::isolated();
    let mut app = overview();
    let path = app.path().unwrap();
    let defaults = serde_json::to_value(&app.config).unwrap();
    app.command(&t.worker, Command::Options).unwrap();
    for (shortcut, modifiers, value) in [
        ('d', KeyModifiers::CONTROL, "cargo test"),
        ('a', KeyModifiers::CONTROL, r#"["--model","test model"]"#),
        ('n', KeyModifiers::ALT, "Build"),
    ] {
        app.key(&t.worker, KeyEvent::new(KeyCode::Char(shortcut), modifiers))
            .unwrap();
        let Some(Dialog::Options {
            editing: Some((_, text)),
            ..
        }) = &mut app.dialog
        else {
            panic!("field editor")
        };
        *text = value.into();
        app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
    }
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
    )
    .unwrap();
    app.key(&t.worker, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    let Some(Dialog::Options {
        editing: Some((3, text)),
        ..
    }) = &mut app.dialog
    else {
        panic!("extra command")
    };
    *text = "cargo check".into();
    app.key(&t.worker, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    let Some(Dialog::Options {
        editing: Some((4, text)),
        ..
    }) = &mut app.dialog
    else {
        panic!("extra directory")
    };
    *text = "src".into();
    app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    let draft = app.options_for(&path);
    assert_eq!(draft.debug_command.as_deref(), Some("cargo test"));
    assert_eq!(draft.agent_args, ["--model", "test model"]);
    assert_eq!(draft.extra_sessions[0].name, "Build");
    assert_eq!(
        draft.extra_sessions[0].command.as_deref(),
        Some("cargo check")
    );
    assert_eq!(draft.extra_sessions[0].directory, Some("src".into()));
    assert_eq!(draft.terminal_count(), 3);
    assert_eq!(serde_json::to_value(&app.config).unwrap(), defaults);
    assert!(t.edits.try_recv().is_err());
    assert!(t.launches.try_recv().is_err());
    assert_eq!(app.marked, vec![path]);
}
#[test]
fn draft_validation_retains_input_and_escape_cancels_only_the_field() {
    let t = worker::isolated();
    let mut app = overview();
    app.command(&t.worker, Command::Options).unwrap();
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    let Some(Dialog::Options {
        editing: Some((_, value)),
        ..
    }) = &mut app.dialog
    else {
        panic!("arguments")
    };
    *value = "[invalid".into();
    assert!(
        app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .is_err()
    );
    assert!(
        matches!(&app.dialog, Some(Dialog::Options { editing: Some((1, value)), options, .. }) if value == "[invalid" && options.agent_args.is_empty())
    );
    app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        &app.dialog,
        Some(Dialog::Options { editing: None, .. })
    ));
    assert!(app.staged.is_empty());
    assert!(t.edits.try_recv().is_err());
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn instance_selection_survives_host_reordering_and_disappearance_cannot_launch() {
    let t = worker::isolated();
    let mut app = overview();
    let path = app.path().unwrap();
    let make = |id: &str| crate::protocol::WorkspaceInfo {
        tasks: vec![],
        id: id.into(),
        project: path.clone(),
        options: app.options_for(&path),
        sessions: vec![crate::protocol::SessionInfo {
            task: Box::default(),
            id: format!("{id}-agent"),
            name: "Agent".into(),
            shell_pid: Some(1),
            status: crate::session::Status::Running,
            command: crate::session::CommandStatus::Idle,
            placement: crate::presentation::Placement::Docked,
            activity: Default::default(),
        }],
    };
    let one = make("one");
    let two = make("two");
    app.workspaces = worker::active(&[one.clone(), two.clone()]);
    app.dialog = Some(Dialog::Instances {
        path: path.clone(),
        selected: 1,
    });
    t.replies
        .send(Reply::Host(Ok(vec![two.clone(), one])))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert!(matches!(
        &app.dialog,
        Some(Dialog::Instances { selected: 0, .. })
    ));
    t.replies.send(Reply::Host(Ok(vec![]))).unwrap();
    app.poll(&t.worker).unwrap();
    assert!(app.dialog.is_none());
    assert!(!app.message.is_empty());
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn queue_mouse_reordering_delete_and_undo_preserve_entry_identity() {
    let t = worker::isolated();
    let mut app = overview();
    app.marked.push("a".into());
    app.marked.push("b".into());
    app.dialog = Some(Dialog::Queue { selected: 0 });
    draw(&mut app, 160, 40);
    let (area, _) = app.dialog_list.unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), area.x + 2, area.y),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(
            MouseEventKind::Up(MouseButton::Left),
            area.x + 2,
            area.y + 1,
        ),
    )
    .unwrap();
    assert_eq!(app.marked, vec![PathBuf::from("b"), PathBuf::from("a")]);
    app.mouse(
        &t.worker,
        mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            area.right() - 1,
            area.y,
        ),
    )
    .unwrap();
    assert_eq!(app.marked, vec![PathBuf::from("a")]);
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(app.marked, vec![PathBuf::from("b"), PathBuf::from("a")]);
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn local_file_locations_preserve_drive_colons_and_reject_remote_urls() {
    let root = std::path::Path::new("D:/Projects/Example");
    assert_eq!(
        dashboard::local_reference(root, "src/main.rs:12:4"),
        Some((root.join("src/main.rs"), Some(12)))
    );
    assert_eq!(
        dashboard::local_reference(root, "C:/Projects/文件.md:23"),
        Some((PathBuf::from("C:/Projects/文件.md"), Some(23)))
    );
    assert_eq!(
        dashboard::local_reference(root, "docs/a.md#L5"),
        Some((root.join("docs/a.md"), Some(5)))
    );
    for target in [
        "https://example.com/a.rs:12",
        "http://example.org",
        "javascript:alert(1)",
        "file://remote/share/a.md",
    ] {
        assert!(
            dashboard::local_reference(root, target).is_none(),
            "{target}"
        );
    }
}
#[test]
fn file_viewer_is_read_only_and_return_retains_overview_state() {
    let t = worker::isolated();
    let mut app = overview();
    let root =
        std::env::temp_dir().join(format!("DevTabs-overview-reader-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("test.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    app.projects = vec![root.clone()];
    app.rebuild();
    app.marked.push(root.clone());
    draw(&mut app, 140, 36);
    let selected = app.path();
    let queue = app.marked.clone();
    app.dashboard.focus = dashboard::Focus::Detail;
    app.dashboard.file_selected = Some((path.clone(), None));
    app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.dashboard.viewer.as_ref().unwrap().read_only);
    let start = Instant::now();
    while app
        .dashboard
        .viewer
        .as_mut()
        .unwrap()
        .document
        .as_ref()
        .is_none_or(|d| d.loading)
    {
        app.dashboard.viewer.as_mut().unwrap().tick(true);
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.dashboard.viewer.as_mut().unwrap().save().is_err());
    app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.dashboard.viewer.is_none());
    assert_eq!(app.path(), selected);
    assert_eq!(
        app.marked.iter().collect::<Vec<_>>(),
        queue.iter().collect::<Vec<_>>()
    );
    assert!(t.launches.try_recv().is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn filter_counts_and_identity_ignore_temporary_agent_and_use_known_history_recency() {
    let t = worker::isolated();
    let mut app = overview();
    app.config.overview.filter = model::Filter::Codex;
    app.rebuild();
    let original = app.path();
    app.command(&t.worker, Command::DraftAgent(Agent::Claude))
        .unwrap();
    app.rebuild();
    assert_eq!(app.path(), original);
    assert_eq!(app.filter_counts[&model::Filter::Codex], 2);
    app.usage.insert("b".into(), config::now());
    app.rebuild();
    assert_eq!(app.path(), original);
    assert_eq!(app.rows[0], Row::Group(1));
    assert!(
        app.config
            .projects
            .get("b")
            .is_none_or(|p| p.last_opened == 0)
    );
    app.query = "b".into();
    app.rebuild();
    assert_eq!(app.filter_counts[&model::Filter::All], 1);
    assert_eq!(app.path(), Some(PathBuf::from("b")));
}
#[test]
fn divider_release_is_persisted_and_failed_save_retains_visible_size() {
    let t = worker::isolated();
    let mut app = overview();
    draw(&mut app, 200, 50);
    let rect = app.dashboard.columns.unwrap().dividers[0].unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y + 2),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(
            MouseEventKind::Drag(MouseButton::Left),
            rect.x + 8,
            rect.y + 2,
        ),
    )
    .unwrap();
    let saved = app.config.overview.list_ratio;
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Up(MouseButton::Left), 0, 0),
    )
    .unwrap();
    let Action::Edit(id, Edit::Preferences(prefs)) = t.edits.try_recv().unwrap() else {
        panic!("save widths")
    };
    assert_eq!(prefs.list_ratio, saved);
    t.replies
        .send(Reply::Config(id, Err("disk full".into())))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert_eq!(app.config.overview.list_ratio, saved);
    assert_eq!(app.message, "disk full");
}
#[test]
fn default_columns_respect_boundaries_and_preferences_without_auto_popups() {
    for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
        crate::i18n::with_language(language, || {
            let mut app = overview();
            for (w, h) in [
                (59, 9),
                (60, 10),
                (80, 24),
                (103, 30),
                (104, 30),
                (140, 36),
                (159, 40),
                (160, 40),
                (200, 50),
                (280, 65),
            ] {
                draw(&mut app, w, h);
                assert!(app.dashboard.overlay.is_none());
                if w < 60 {
                    assert!(app.dashboard.columns.is_none());
                    assert!(app.dashboard.row_hits.is_empty());
                    assert!(app.filter_hits.is_empty());
                    continue;
                }
                let c = app.dashboard.columns.unwrap();
                assert_eq!(c.detail.is_some(), w >= 104);
                assert_eq!(c.stats.is_some(), w >= 160);
                if let Some(r) = c.detail {
                    assert!(c.list.right() < r.x);
                    assert!(r.width >= 60);
                }
                if let Some(r) = c.stats {
                    assert!(c.detail.unwrap().right() < r.x);
                    assert!(r.width >= 30);
                }
            }
            app.show_detail = false;
            app.config.overview.detail = Some(false);
            draw(&mut app, 200, 50);
            assert!(app.dashboard.columns.unwrap().detail.is_none());
            draw(&mut app, 80, 24);
            assert_eq!(app.config.overview.detail, Some(false));
            draw(&mut app, 200, 50);
            assert!(!app.show_detail);
        });
    }
}
#[test]
fn hover_wheel_and_badge_click_do_not_launch_or_replace_context() {
    let t = worker::isolated();
    let mut app = overview();
    draw(&mut app, 200, 50);
    let selected = app.path();
    let hit = app
        .dashboard
        .row_hits
        .iter()
        .find(|(_, i)| *i == 2)
        .unwrap()
        .0;
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Moved, hit.x + 10, hit.y),
    )
    .unwrap();
    assert_eq!(app.path(), selected);
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::ScrollDown, hit.x + 10, hit.y),
    )
    .unwrap();
    assert_eq!(app.path(), selected);
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), hit.x + 1, hit.y),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), hit.x + 10, hit.y),
    )
    .unwrap();
    assert!(t.launches.try_recv().is_err());
    assert_eq!(app.marked.len(), 1);
    let empty = Rect::new(app.list_area.x, app.list_area.bottom() - 1, 1, 1);
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), empty.x, empty.y),
    )
    .unwrap();
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn independent_scrollbars_and_divider_cancel_do_not_change_selection() {
    let t = worker::isolated();
    let mut app = overview();
    app.config.projects.entry("a".into()).or_default().note = "note\n".repeat(100);
    app.stats.insert(
        "a".into(),
        stats::Stats {
            disk_ready: true,
            directories: (0..5).map(|i| (format!("dir{i}"), 100)).collect(),
            numstat: (0..6).map(|i| (format!("src/file{i}.rs"), 10, 2)).collect(),
            ..Default::default()
        },
    );
    draw(&mut app, 200, 22);
    let c = app.dashboard.columns.unwrap();
    let detail = c.detail.unwrap();
    let stats = c.stats.unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::ScrollDown, detail.x + 2, detail.y + 2),
    )
    .unwrap();
    assert!(app.detail_scroll > 0);
    assert_eq!(app.dashboard.stats_scroll, 0);
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::ScrollDown, stats.x + 2, stats.y + 2),
    )
    .unwrap();
    assert!(app.dashboard.stats_scroll > 0);
    let path = app.path();
    let divider = c.dividers[0].unwrap();
    app.mouse(
        &t.worker,
        mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            divider.x,
            divider.y + 2,
        ),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Drag(MouseButton::Left), 120, divider.y + 2),
    )
    .unwrap();
    assert!(app.config.overview.list_ratio.is_some());
    app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.config.overview.list_ratio.is_none());
    assert_eq!(app.path(), path);
    assert!(t.edits.try_recv().is_err());
    app.mouse(
        &t.worker,
        mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            stats.right() - 1,
            stats.y + 2,
        ),
    )
    .unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Drag(MouseButton::Left), 0, 100),
    )
    .unwrap();
    assert_eq!(app.dashboard.stats_scroll, app.dashboard.stats_max);
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Up(MouseButton::Left), 0, 100),
    )
    .unwrap();
    assert!(app.dashboard.scrollbar.is_none());
}
#[test]
fn background_sources_are_debounced_independent_and_agent_guarded() {
    let t = worker::isolated();
    let mut app = overview();
    draw(&mut app, 200, 50);
    app.request_stats(&t.worker).unwrap();
    assert!(t.details.try_recv().is_err());
    assert!(t.sources.try_recv().is_err());
    assert!(t.histories.try_recv().is_err());
    app.dashboard.selected_since = Some(("a".into(), Instant::now() - Duration::from_millis(250)));
    app.dashboard.context_since = Some(Instant::now() - Duration::from_millis(250));
    app.request_stats(&t.worker).unwrap();
    assert!(matches!(t.sources.try_recv(), Ok(Detail::Source(_, _))));
    let Detail::Context(_, agent, old) = t.histories.try_recv().unwrap() else {
        panic!("history")
    };
    assert_eq!(agent, Agent::Codex);
    assert!(t.details.try_recv().is_err());
    app.key(&t.worker, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    app.request_stats(&t.worker).unwrap();
    t.replies
        .send(Reply::Context(
            "a".into(),
            Agent::Codex,
            old,
            history::History::default(),
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert!(app.dashboard.contexts.is_empty());
    assert!(
        app.dashboard.source_pending.is_some(),
        "Agent selection cannot cancel source statistics"
    );
    app.dashboard.selected_since = Some(("a".into(), Instant::now() - Duration::from_millis(600)));
    app.request_stats(&t.worker).unwrap();
    assert!(matches!(t.details.try_recv(), Ok(Detail::Stats(_, _))));
}
#[test]
fn enter_current_and_control_enter_queue_remain_distinct_and_recovery_is_reviewable() {
    let t = worker::isolated();
    let mut app = overview();
    assert!(
        app.key(
            &t.worker,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)
        )
        .is_err()
    );
    assert!(t.launches.try_recv().is_err());
    app.marked.push("b".into());
    app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let Action::Launch(_, items) = t.launches.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].1.project, PathBuf::from("a"));
    app.batch = Batch::default();
    app.dialog = None;
    let options = app.options_for(std::path::Path::new("a"));
    app.recovery.register("first", &options);
    app.recovery.register("second", &options);
    app.command(&t.worker, Command::Restore).unwrap();
    assert_eq!(app.marked.len(), 3);
    assert!(t.launches.try_recv().is_err());
    let options = app.queue_options();
    assert_eq!(options[0].project, PathBuf::from("b"));
    assert_ne!(options[1].recovery_id, options[2].recovery_id);
    app.dialog = None;
    app.command(&t.worker, Command::Restore).unwrap();
    assert_eq!(app.marked.len(), 3);
    app.key(&t.worker, KeyEvent::new(KeyCode::Down, KeyModifiers::ALT))
        .unwrap();
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
    )
    .unwrap();
    let Action::Launch(_, items) = t.launches.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].1.project, PathBuf::from("b"));
}
#[test]
fn conversation_summary_reuses_markdown_and_never_displays_another_agent() {
    let mut app = overview();
    let h = history::History {
        sessions: vec![history::Conversation {
            id: "specific".into(),
            timestamp: "2026-11-11".into(),
            prompt: String::new(),
            turns: vec![
                history::Turn {
                    user: true,
                    text: "# Task\n\n**Important**".into(),
                },
                history::Turn {
                    user: false,
                    text: "```rust\nfn main() {}\n```".into(),
                },
            ],
            file: PathBuf::new(),
        }],
        ..Default::default()
    };
    app.dashboard.store_context("a|Codex".into(), h);
    let lines = view::detail_lines(&mut app, 60);
    let text = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Task"));
    assert!(!text.contains("**Important**"));
    assert!(app.dashboard.summary_cache.is_some());
    let mut options = app.options_for(std::path::Path::new("a"));
    options.agent = Agent::Claude;
    app.staged.insert("a".into(), options);
    let text = view::detail_lines(&mut app, 60)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("fn main"));
    for n in 0..40 {
        app.dashboard
            .store_context(format!("{n}|Codex"), history::History::default());
    }
    assert_eq!(app.dashboard.contexts.len(), 32);
    assert_eq!(app.dashboard.context_order.len(), 32);
}
#[test]
#[ignore = "release overview navigation benchmark; no PTY, host or filesystem work"]
fn overview_navigation_benchmark() {
    navigation_benchmark(false);
}
#[test]
#[ignore = "release cached-context navigation benchmark; no PTY, host or filesystem work"]
fn overview_cached_context_benchmark() {
    navigation_benchmark(true);
}
fn navigation_benchmark(cached: bool) {
    crate::i18n::with_language(crate::i18n::Language::EnUs, || {
        for count in [100, 1000] {
            let mut config = Config::default();
            let projects = (0..count)
                .map(|i| PathBuf::from(format!("D:/fixtures/项目-{i:04}")))
                .collect::<Vec<_>>();
            for (i, p) in projects.iter().enumerate() {
                config.projects.insert(
                    config::key(p),
                    Project {
                        agent: Some(if i % 2 == 0 {
                            Agent::Codex
                        } else {
                            Agent::Claude
                        }),
                        note: "Continue the implementation".into(),
                        ..Default::default()
                    },
                );
            }
            let mut app = Overview::new(config);
            app.projects = projects;
            app.scanning = false;
            app.rebuild();
            if cached {
                for path in app.projects.iter().take(32) {
                    let agent = app.options_for(path).agent;
                    app.dashboard.store_context(dashboard::context_key(path, agent), history::History {
                        sessions: vec![history::Conversation {
                            id: "fixture-session".into(), timestamp: "2026-11-11".into(), prompt: "Continue".into(), file: PathBuf::new(),
                            turns: vec![history::Turn { user: true, text: "## 下一步\n\n- 完成 **总览**\n- 测试启动队列".into() }, history::Turn { user: false, text: "### Implementation\n\n```rust\nfn main() { println!(\"DevTabs\"); }\n```\n\n| State | Result |\n|---|---|\n| Ready | Pass |".into() }],
                        }], ..Default::default()
                    });
                }
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 50)).unwrap();
            let mut samples = vec![];
            for i in 0..110 {
                app.selected = 1 + (i * 7) % if cached { 32 } else { count };
                let start = Instant::now();
                terminal.draw(|f| view::draw(f, &mut app)).unwrap();
                if i >= 10 {
                    samples.push(start.elapsed().as_micros() as u64);
                }
            }
            samples.sort_unstable();
            println!(
                "overview_navigation cached={cached} projects={count} samples={} p50_us={} p95_us={} max_us={}",
                samples.len(),
                samples[50],
                samples[95],
                samples[99]
            );
        }
    });
}
#[test]
#[ignore = "write deterministic overview TestBackend fixtures; no PTY or host"]
fn overview_fixture_snapshots() {
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/overview-fixtures");
    std::fs::create_dir_all(&output).unwrap();
    let original = crate::appearance::theme();
    for theme in [
        crate::appearance::Theme::Catppuccin,
        crate::appearance::Theme::Light,
    ] {
        crate::appearance::set(theme);
        for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
            crate::i18n::with_language(language, || {
                for (width, height) in [(80, 24), (140, 36), (200, 50)] {
                    let mut app = Overview::new(Config::default());
                    app.scanning = false;
                    app.projects = (0..20)
                        .map(|i| PathBuf::from(format!("D:/fixtures/项目-{i:02}")))
                        .collect();
                    for (i, path) in app.projects.iter().enumerate() {
                        app.config.projects.insert(
                            config::key(path),
                            Project {
                                agent: Some(if i % 2 == 0 {
                                    Agent::Codex
                                } else {
                                    Agent::Claude
                                }),
                                note: "Next: review the implementation\n下一步：验收总览".into(),
                                favorite: i % 4 == 0,
                                ..Default::default()
                            },
                        );
                    }
                    app.rebuild();
                    app.selected = 1;
                    app.marked.push(app.projects[1].clone());
                    let buffer = draw(&mut app, width, height);
                    let text = (0..height)
                        .map(|y| {
                            let mut line = String::new();
                            let mut x = 0;
                            while x < width {
                                let symbol = buffer[(x, y)].symbol();
                                line.push_str(symbol);
                                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
                            }
                            line.trim_end().to_owned()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    std::fs::write(
                        output.join(format!("{language:?}-{theme:?}-{width}x{height}.txt")),
                        text,
                    )
                    .unwrap();
                }
            });
        }
    }
    crate::appearance::set(original);
}
#[test]
fn note_keeps_original_project_when_background_results_change_selection() {
    let t = worker::isolated();
    let mut app = overview();
    let original = app.path().unwrap();
    app.command(&t.worker, Command::Note).unwrap();
    app.selected = 2;
    assert_ne!(app.path().unwrap(), original);
    app.key(&t.worker, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let Action::Edit(_, Edit::Note(path, _)) = t.edits.try_recv().unwrap() else {
        panic!("expected note save");
    };
    assert_eq!(path, original);
}
#[test]
fn settings_fields_add_remove_and_validation_keep_input() {
    let t = worker::isolated();
    let mut app = overview();
    app.command(&t.worker, Command::Settings).unwrap();
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
    )
    .unwrap();
    assert!(
        matches!(&app.dialog,Some(Dialog::Settings{project,field:7,..}) if project.sessions.len()==1)
    );
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(
        matches!(&app.dialog,Some(Dialog::Settings{project,field:6,..}) if project.sessions.is_empty())
    );
    if let Some(Dialog::Settings { project, .. }) = &mut app.dialog {
        project.sessions.push(crate::config::SessionConfig {
            name: String::new(),
            directory: None,
            command: Some("keep; 中文".into()),
        })
    }
    assert!(
        app.key(
            &t.worker,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        )
        .is_err()
    );
    assert!(
        matches!(&app.dialog,Some(Dialog::Settings{project,..}) if project.sessions[0].command.as_deref()==Some("keep; 中文"))
    );
    assert!(t.edits.try_recv().is_err());
}
#[test]
fn failed_persistence_restores_form_with_all_entered_values() {
    let t = worker::isolated();
    let mut app = overview();
    app.command(&t.worker, Command::Settings).unwrap();
    if let Some(Dialog::Settings { debug, .. }) = &mut app.dialog {
        *debug = "echo 'keep me'".into()
    }
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.dialog.is_none());
    let Action::Edit(id, _) = t.edits.try_recv().unwrap() else {
        panic!("expected save")
    };
    t.replies
        .send(Reply::Config(id, Err("disk full".into())))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert!(matches!(&app.dialog,Some(Dialog::Settings{debug,..}) if debug=="echo 'keep me'"));
    assert_eq!(app.pending_edits, 0);
}
#[test]
fn statistics_progress_keeps_request_pending_and_canceled_scan_restarts_on_return() {
    let t = worker::isolated();
    let mut app = overview();
    app.show_stats = true;
    app.dashboard.overlay = Some(dashboard::Focus::Stats);
    app.request_stats(&t.worker).unwrap();
    let Detail::Stats(path, old_ticket) = t.details.try_recv().unwrap() else {
        panic!("stats")
    };
    let key = config::key(&path);
    t.replies
        .send(Reply::Stats(
            key.clone(),
            old_ticket,
            stats::Stats {
                disk_scanning: true,
                scanned_entries: 12,
                files: 10,
                source_ready: true,
                ..Default::default()
            },
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert!(app.detail_pending);
    assert!(app.stats[&key].disk_scanning);
    assert!(!app.dashboard.stats_at.contains_key(&key));
    assert!(t.details.try_recv().is_err());
    app.dashboard.overlay = None;
    app.dashboard.columns = None;
    app.request_stats(&t.worker).unwrap();
    assert!(!app.detail_pending);
    app.dashboard.overlay = Some(dashboard::Focus::Stats);
    app.request_stats(&t.worker).unwrap();
    let Detail::Stats(_, current) = t.details.try_recv().unwrap() else {
        panic!("restart")
    };
    assert_ne!(old_ticket, current);
    t.replies
        .send(Reply::Stats(
            key.clone(),
            old_ticket,
            stats::Stats {
                disk_ready: true,
                files: 999,
                ..Default::default()
            },
        ))
        .unwrap();
    t.replies
        .send(Reply::Stats(
            key.clone(),
            current,
            stats::Stats {
                disk_ready: true,
                files: 25,
                ..Default::default()
            },
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert_eq!(app.stats[&key].files, 25);
    assert!(!app.detail_pending && !app.stats[&key].disk_scanning);
    assert!(app.dashboard.stats_at.contains_key(&key));
}
#[test]
fn stale_background_results_cannot_replace_current_details() {
    let t = worker::isolated();
    let mut app = overview();
    app.show_stats = true;
    app.dashboard.overlay = Some(dashboard::Focus::Stats);
    app.request_stats(&t.worker).unwrap();
    let Detail::Stats(_, old) = t.details.try_recv().unwrap() else {
        panic!("stats")
    };
    app.selected = 2;
    app.request_stats(&t.worker).unwrap();
    let Detail::Stats(path, current) = t.details.try_recv().unwrap() else {
        panic!("stats")
    };
    assert_ne!(old, current);
    t.replies
        .send(Reply::Stats(
            "a".into(),
            old,
            stats::Stats {
                bytes: 999,
                ..Default::default()
            },
        ))
        .unwrap();
    t.replies
        .send(Reply::Stats(
            config::key(&path),
            current,
            stats::Stats {
                bytes: 10,
                ..Default::default()
            },
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    assert!(!app.stats.contains_key("a"));
    assert_eq!(app.stats[&config::key(&path)].bytes, 10);
}
#[test]
fn native_history_fallback_uses_selected_agent_and_new_workspace() {
    let t = worker::isolated();
    let mut app = overview();
    app.dialog = Some(Dialog::History {
        path: "a".into(),
        agent: Agent::Claude,
        selected: 0,
        scroll: 0,
    });
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
    )
    .unwrap();
    assert!(t.launches.try_recv().is_err());
    assert_eq!(
        app.options_for(std::path::Path::new("a")).agent,
        Agent::Claude
    );
    app.command(&t.worker, Command::Open).unwrap();
    let Action::Launch(_, items) = t.launches.try_recv().unwrap() else {
        panic!("launch")
    };
    assert_eq!(items[0].1.agent, Agent::Claude);
    assert!(items[0].1.resume && items[0].1.new_instance);
    assert!(items[0].1.resume_id.is_none());
}
#[test]
fn command_palette_searches_both_languages_and_reports_unavailable_actions() {
    crate::i18n::with_language(crate::i18n::Language::ZhCn, || {
        let mut app = overview();
        assert!(
            app.palette_entries("settings")
                .iter()
                .any(|e| matches!(e.command, Command::Settings))
        );
        assert!(
            app.palette_entries("项目设置")
                .iter()
                .any(|e| matches!(e.command, Command::Settings))
        );
        app.selected = 0;
        assert!(
            app.entries()
                .iter()
                .any(|e| matches!(e.command, Command::Settings) && !e.enabled)
        );
    });
}
#[test]
fn batch_keeps_order_and_frontend_failure_reuses_created_workspace() {
    let t = worker::isolated();
    let mut app = overview();
    app.marked = vec!["b".into(), "a".into()].into();
    app.command(&t.worker, Command::Launch).unwrap();
    let Action::Launch(generation, items) = t.launches.try_recv().unwrap() else {
        panic!("launch")
    };
    assert_eq!(items[0].1.project, PathBuf::from("b"));
    t.replies
        .send(Reply::Attached(generation, 0, "created".into()))
        .unwrap();
    t.replies
        .send(Reply::Launch(
            generation,
            0,
            LaunchState::Failed("window".into()),
        ))
        .unwrap();
    t.replies
        .send(Reply::Launch(
            generation,
            1,
            LaunchState::Opened("second".into()),
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    app.retry(&t.worker).unwrap();
    let Action::Launch(_, retry) = t.launches.try_recv().unwrap() else {
        panic!("retry")
    };
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].1.workspace.as_deref(), Some("created"));
}
#[test]
fn chinese_popups_mouse_geometry_and_escape_do_not_reach_underlying_list() {
    let t = worker::isolated();
    crate::i18n::with_language(crate::i18n::Language::ZhCn, || {
        let mut app = overview();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 30)).unwrap();
        app.show_stats = true;
        app.dashboard.overlay = Some(dashboard::Focus::Stats);
        terminal.draw(|f| view::draw(f, &mut app)).unwrap();
        let original = app.selected;
        let rect = app.detail_areas[0];
        app.mouse(
            &t.worker,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: rect.x + 1,
                row: rect.y + 2,
                modifiers: KeyModifiers::NONE,
            },
        )
        .unwrap();
        assert_eq!(app.selected, original);
        assert_eq!(app.detail_scroll, 0);
        assert!(app.dashboard.stats_scroll <= app.dashboard.stats_max);
        assert!(matches!(
            app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .unwrap(),
            Outcome::Stay
        ));
        assert!(app.dashboard.overlay.is_none());
        assert!(app.show_stats);
        for dialog in [
            Dialog::Input(Input::Note(PathBuf::from("a")), "下一步".into()),
            Dialog::Settings {
                path: "a".into(),
                project: Project::default(),
                field: 2,
                debug: "echo 1".into(),
                arguments: "[]".into(),
            },
            Dialog::Options {
                options: LaunchSpec::from_config(PathBuf::from("a").as_path(), &app.config)
                    .options(),
                new_tab: true,
                editing: None,
            },
            Dialog::History {
                path: "a".into(),
                agent: Agent::Codex,
                selected: 0,
                scroll: 0,
            },
            Dialog::Palette {
                query: "设置".into(),
                selected: 0,
                original: crate::appearance::theme(),
            },
            Dialog::Batch,
            Dialog::Help,
            Dialog::Presets { selected: 0 },
        ] {
            app.dialog = Some(dialog);
            for width in [25, 90, 190] {
                terminal.resize(Rect::new(0, 0, width, 30)).unwrap();
                terminal.draw(|f| view::draw(f, &mut app)).unwrap();
            }
        }
    });
}

#[test]
fn preset_apply_stages_all_options_without_launch_and_options_can_be_adjusted() {
    let t = worker::isolated();
    let mut app = overview();
    let mut options = Options::parse_for_view();
    options.project = "b".into();
    options.new_window = true;
    options.fresh = true;
    options.no_profile = true;
    options.title = Some("saved title".into());
    app.config
        .presets
        .insert("test".into(), vec![LaunchSpec::from_options(&options)]);
    app.command(&t.worker, Command::Preset("test".into()))
        .unwrap();
    assert!(t.launches.try_recv().is_err());
    assert_eq!(app.marked, vec![PathBuf::from("b")]);
    app.selected = app
        .rows
        .iter()
        .position(|r| r.path() == Some(PathBuf::from("b").as_path()))
        .unwrap();
    app.command(&t.worker, Command::Options).unwrap();
    if let Some(Dialog::Options { options, .. }) = &mut app.dialog {
        assert!(options.fresh && options.new_window && options.no_profile);
        options.layout = Layout::Split;
    }
    app.key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(t.launches.try_recv().is_err());
    app.command(&t.worker, Command::Launch).unwrap();
    let Action::Launch(_, jobs) = t.launches.try_recv().unwrap() else {
        panic!("launch")
    };
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].1.layout, Layout::Split);
    assert!(jobs[0].1.fresh && jobs[0].1.new_window);
}
#[test]
fn visible_select_all_preserves_hidden_selection_and_filter_clicks_use_translated_geometry() {
    let t = worker::isolated();
    crate::i18n::with_language(crate::i18n::Language::ZhCn, || {
        let mut app = overview();
        app.marked.push("hidden".into());
        app.command(&t.worker, Command::SelectAll).unwrap();
        assert_eq!(app.marked.len(), 3);
        app.command(&t.worker, Command::SelectAll).unwrap();
        assert_eq!(app.marked, vec![PathBuf::from("hidden")]);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 32)).unwrap();
        terminal.draw(|f| view::draw(f, &mut app)).unwrap();
        let rect = app
            .filter_hits
            .iter()
            .find(|(_, f)| *f == model::Filter::Pending)
            .unwrap()
            .0;
        app.mouse(
            &t.worker,
            MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: rect.x + 1,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
        )
        .unwrap();
        assert_eq!(app.config.overview.filter, model::Filter::Pending);
        assert!(app.rows.is_empty());
    });
}
#[test]
fn malformed_arguments_and_project_profile_block_launch_without_losing_forms() {
    let t = worker::isolated();
    let mut app = overview();
    app.command(&t.worker, Command::Settings).unwrap();
    if let Some(Dialog::Settings { arguments, .. }) = &mut app.dialog {
        *arguments = "[bad JSON".into();
    }
    assert!(
        app.key(
            &t.worker,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        )
        .is_err()
    );
    assert!(matches!(&app.dialog,Some(Dialog::Settings{arguments,..}) if arguments=="[bad JSON"));
    assert!(t.edits.try_recv().is_err());
    app.dialog = None;
    let path = app.path().unwrap();
    app.profile_errors
        .insert(config::key(&path), "invalid profile".into());
    assert!(app.command(&t.worker, Command::Open).is_err());
    assert!(t.launches.try_recv().is_err());
}

#[test]
fn profile_agent_title_search_and_private_note_share_effective_settings() {
    let mut app = overview();
    let key = config::key(std::path::Path::new("a"));
    app.config.projects.entry(key.clone()).or_default().note = "private note".into();
    app.config.projects.get_mut(&key).unwrap().favorite = true;
    app.profiles.insert(
        key,
        Project {
            agent: Some(Agent::Claude),
            title: Some("团队工作台".into()),
            ..Default::default()
        },
    );
    app.config.overview.filter = model::Filter::Claude;
    for query in ["团队工作台", "private", "团台"] {
        app.query = query.into();
        app.rebuild();
        assert_eq!(
            app.rows.iter().filter_map(Row::path).collect::<Vec<_>>(),
            vec![std::path::Path::new("a")]
        );
        assert_eq!(app.rows[0], Row::Group(0));
    }
    app.config.overview.filter = model::Filter::Codex;
    app.rebuild();
    assert!(app.rows.is_empty());
}
#[test]
fn refresh_cancels_previous_results_and_waits_for_cache_invalidation() {
    let t = worker::isolated();
    let mut app = overview();
    app.show_stats = true;
    app.dashboard.overlay = Some(dashboard::Focus::Stats);
    app.request_stats(&t.worker).unwrap();
    let Detail::Stats(path, old_ticket) = t.details.try_recv().unwrap() else {
        panic!("stats")
    };
    app.command(&t.worker, Command::Refresh).unwrap();
    let Action::Reload(id) = t.edits.try_recv().unwrap() else {
        panic!("reload")
    };
    t.replies
        .send(Reply::Stats(
            config::key(&path),
            old_ticket,
            stats::Stats {
                source_bytes: 999,
                ..Default::default()
            },
        ))
        .unwrap();
    app.poll(&t.worker).unwrap();
    app.request_stats(&t.worker).unwrap();
    assert!(app.stats.is_empty());
    assert!(t.details.try_recv().is_err());
    t.replies
        .send(Reply::Config(id, Ok(app.config.clone())))
        .unwrap();
    app.poll(&t.worker).unwrap();
    app.request_stats(&t.worker).unwrap();
    assert!(matches!(t.details.try_recv(), Ok(Detail::Stats(_, ticket)) if ticket != old_ticket));
}

#[test]
fn clickable_search_filters_projects_and_keeps_modal_input_isolated() {
    for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
        crate::i18n::with_language(language, || {
            let t = worker::isolated();
            let mut app = overview();
            draw(&mut app, 140, 36);
            let search = app.dashboard.search_hit.unwrap();
            app.mouse(
                &t.worker,
                mouse_at(
                    MouseEventKind::Down(MouseButton::Left),
                    search.x + 5,
                    search.y,
                ),
            )
            .unwrap();
            assert_eq!(app.dashboard.focus, dashboard::Focus::Search);
            app.key(
                &t.worker,
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
            )
            .unwrap();
            assert_eq!(app.query, "b");
            assert_eq!(app.path(), Some(PathBuf::from("b")));
            assert_eq!(app.rows.iter().filter(|r| r.path().is_some()).count(), 1);
            app.key(
                &t.worker,
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            )
            .unwrap();
            assert_eq!(app.rows.iter().filter(|r| r.path().is_some()).count(), 2);
            // A slash typed after focusing is a literal part of a path query.
            app.key(
                &t.worker,
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            )
            .unwrap();
            assert_eq!(app.query, "/");
            app.key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .unwrap();
            assert_eq!(app.dashboard.focus, dashboard::Focus::Projects);
            app.query.clear();
            app.rebuild();
            app.dashboard.overlay = Some(dashboard::Focus::Detail);
            draw(&mut app, 140, 36);
            app.mouse(
                &t.worker,
                mouse_at(MouseEventKind::Down(MouseButton::Left), search.x, search.y),
            )
            .unwrap();
            assert_ne!(app.dashboard.focus, dashboard::Focus::Search);
            app.dashboard.overlay = None;
            app.dialog = Some(Dialog::Help);
            draw(&mut app, 140, 36);
            app.mouse(
                &t.worker,
                mouse_at(MouseEventKind::Down(MouseButton::Left), search.x, search.y),
            )
            .unwrap();
            assert_ne!(app.dashboard.focus, dashboard::Focus::Search);
            app.dialog = None;
            draw(&mut app, 59, 8);
            assert!(app.dashboard.search_hit.is_none());
            assert!(app.filter_hits.is_empty());
            assert!(t.launches.try_recv().is_err());
        });
    }
}

#[test]
fn header_keeps_long_unicode_query_cursor_and_active_filter_visible() {
    use unicode_width::UnicodeWidthStr;
    for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
        crate::i18n::with_language(language, || {
            for width in [60, 80, 104, 140, 160, 200] {
                for filter in [
                    model::Filter::All,
                    model::Filter::Pending,
                    model::Filter::Shell,
                ] {
                    let mut app = overview();
                    app.config.overview.filter = filter;
                    app.query = format!("{}e\u{301}尾", "中文/".repeat(30));
                    app.dashboard.focus = dashboard::Focus::Search;
                    let mut terminal =
                        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 36))
                            .unwrap();
                    terminal.draw(|f| view::draw(f, &mut app)).unwrap();
                    let rect = app.dashboard.search_hit.unwrap();
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert!(rect.contains((cursor.x, cursor.y).into()));
                    assert_eq!(
                        terminal.backend().buffer()[(cursor.x - 2, cursor.y)].symbol(),
                        "尾"
                    );
                    let active = app
                        .filter_hits
                        .iter()
                        .find(|(_, f)| *f == filter)
                        .expect("selected filter remains visible")
                        .0;
                    let cell = &terminal.backend().buffer()[(active.x, active.y)];
                    assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD));
                    assert!(!cell.modifier.contains(ratatui::style::Modifier::UNDERLINED));
                    assert_eq!(cell.bg, ratatui::style::Color::Rgb(237, 174, 85));
                    for (r, _) in &app.filter_hits {
                        assert!(r.right() <= width);
                    }
                    let field = (rect.x..rect.right())
                        .map(|x| terminal.backend().buffer()[(x, rect.y)].symbol())
                        .collect::<String>();
                    assert!(field.contains('尾'));
                    assert!(field.width() >= rect.width as usize);
                }
            }
        });
    }
}

#[test]
fn conversation_roles_have_distinct_badges_rails_and_preserved_markdown() {
    use ratatui::style::Modifier;
    let mut app = overview();
    app.dashboard.store_context("a|Codex".into(), history::History {
        sessions: vec![history::Conversation {
            id:"role-fixture".into(), timestamp:"2026-11-11".into(), prompt:String::new(), file:PathBuf::new(),
            turns:vec![
                history::Turn{user:true,text:"**用户需求**\n\n- 中文换行测试内容，要求有清晰的视觉区分。".into()},
                history::Turn{user:false,text:"**已完成**\n\n```rust\nfn main() { println!(\"你好\"); }\n```\n\n[源码](src/main.rs)".into()},
            ],
        }], ..Default::default()
    });
    app.dashboard.expanded = true;
    for width in [30, 56, 72] {
        let lines = view::detail_lines(&mut app, width);
        let summary = &app.dashboard.summary_cache.as_ref().unwrap().lines;
        let badges: Vec<_> = summary
            .iter()
            .filter(|l| l.spans.first().is_some_and(|s| s.style.bg.is_some()))
            .collect();
        assert_eq!(badges.len(), 2);
        let a = badges[0].spans[0].style.bg.unwrap();
        let b = badges[1].spans[0].style.bg.unwrap();
        assert_ne!(a, b);
        let rails: Vec<_> = summary
            .iter()
            .filter_map(|l| l.spans.first().filter(|s| s.content == "▎ "))
            .collect();
        assert!(rails.iter().any(|s| s.style.fg == Some(a)));
        assert!(rails.iter().any(|s| s.style.fg == Some(b)));
        assert!(summary.iter().all(|l| l.width() <= width as usize));
        assert!(summary.iter().flat_map(|l| &l.spans).any(
            |s| s.content.contains("用户需求") && s.style.add_modifier.contains(Modifier::BOLD)
        ));
        assert!(summary.iter().any(|l| l.to_string().contains("fn main")));
        assert_eq!(app.dashboard.summary_links, vec!["src/main.rs"]);
        assert!(lines.iter().any(|l| l.to_string().contains('─')
            && l.to_string().contains(t("overview.conversation").as_ref())));
        let before = summary.clone();
        view::detail_lines(&mut app, width);
        assert_eq!(before, app.dashboard.summary_cache.as_ref().unwrap().lines);
    }
}

#[test]
fn host_failure_is_unknown_clickable_and_full_error_stays_in_diagnostics() {
    let t = worker::isolated();
    let mut app = overview();
    let error = "Connect to DevTabs host: fixture permission error";
    t.replies.send(Reply::Host(Err(error.into()))).unwrap();
    app.poll(&t.worker).unwrap();
    assert_eq!(app.host_error, error);
    let buffer = draw(&mut app, 200, 50);
    let text = buffer
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!text.contains(error));
    let hit = app
        .dashboard
        .detail_hits
        .iter()
        .find(|(_, a)| matches!(a, dashboard::DetailAction::Diagnostics))
        .unwrap()
        .0;
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), hit.x, hit.y),
    )
    .unwrap();
    assert!(matches!(app.dialog, Some(Dialog::Diagnostics)));
    let buffer = draw(&mut app, 200, 50);
    assert!(
        buffer
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains(error)
    );
    app.dialog = None;
    t.replies.send(Reply::Host(Ok(vec![]))).unwrap();
    app.poll(&t.worker).unwrap();
    assert!(app.host_error.is_empty());
    assert!(app.workspaces.is_empty());
    assert!(t.launches.try_recv().is_err());
}

#[test]
fn single_line_projects_have_truthful_status_add_button_and_explicit_session_expansion() {
    let t = worker::isolated();
    let mut app = overview();
    app.config.overview.pending.insert("a".into());
    app.dashboard.store_context(
        "a|Codex".into(),
        history::History {
            sessions: (0..20)
                .map(|i| history::Conversation {
                    id: format!("s-{i}"),
                    timestamp: String::new(),
                    prompt: format!("Task {i}"),
                    turns: vec![],
                    file: PathBuf::from(format!("{i}.jsonl")),
                })
                .collect(),
            ..Default::default()
        },
    );
    app.rebuild();
    let buffer = draw(&mut app, 140, 36);
    assert!(app.dashboard.row_hits.iter().all(|(r, _)| r.height == 1));
    let rect = app.dashboard.add_hit.unwrap();
    assert!(rect.y > app.list_area.y);
    let project = app
        .rows
        .iter()
        .position(|r| matches!(r,Row::Project(p) if p==std::path::Path::new("a")))
        .unwrap();
    let (hit, _) = app
        .dashboard
        .row_hits
        .iter()
        .find(|(_, i)| *i == project)
        .unwrap();
    let text = (hit.x..hit.right())
        .map(|x| buffer[(x, hit.y)].symbol())
        .collect::<String>();
    assert!(!text.contains("Codex"));
    assert!(text.contains(crate::i18n::t("parity.pending").as_ref()));
    assert!(!app.rows.iter().any(|r| matches!(r, Row::Session(..))));
    let expand = app.dashboard.expand_hits[0].0;
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), expand.x, expand.y),
    )
    .unwrap();
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Session(..)))
            .count(),
        20
    );
    app.selected = app
        .rows
        .iter()
        .position(|r| matches!(r,Row::Session(_,_,id) if id=="s-17"))
        .unwrap();
    assert_eq!(
        app.options_for(std::path::Path::new("a"))
            .resume_id
            .as_deref(),
        Some("s-17")
    );
    app.rebuild();
    assert!(matches!(app.rows.get(app.selected),Some(Row::Session(_,_,id)) if id=="s-17"));
    draw(&mut app, 140, 36);
    let add = app.dashboard.add_hit.unwrap();
    app.mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), add.x + 2, add.y),
    )
    .unwrap();
    assert!(matches!(app.dialog, Some(Dialog::Input(Input::Add, _))));
    assert!(t.launches.try_recv().is_err());
}
#[test]
fn needs_attention_precedes_running_and_pending_without_duplicate_projects() {
    let mut app = overview();
    app.projects = (0..5).map(|i| PathBuf::from(format!("p{i}"))).collect();
    app.config
        .overview
        .pending
        .extend(["p0".into(), "p1".into()]);
    app.profile_errors
        .insert("p0".into(), "invalid profile".into());
    app.workspaces.insert("p1".into(), Vec::new());
    app.config.projects.entry("p2".into()).or_default().favorite = true;
    app.config
        .projects
        .entry("p3".into())
        .or_default()
        .last_opened = config::now();
    app.rebuild();
    let projects = app
        .rows
        .iter()
        .filter_map(|r| {
            if let Row::Project(p) = r {
                Some(p.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(projects, app.projects);
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r, Row::Project(_)))
            .count(),
        5
    );
    let groups = app
        .rows
        .iter()
        .filter_map(|r| {
            if let Row::Group(g) = r {
                Some(*g)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(groups, vec![6, 7, 0, 1, 5]);
    assert_eq!(app.filter_counts[&model::Filter::Attention], 1);
}
#[test]
fn latest_user_is_always_in_summary_and_session_search_stages_exact_id() {
    let workers = worker::isolated();
    let mut app = overview();
    app.dashboard.store_context(
        "a|Codex".into(),
        history::History {
            sessions: vec![
                history::Conversation {
                    id: "first".into(),
                    timestamp: String::new(),
                    prompt: "First question".into(),
                    file: PathBuf::new(),
                    turns: vec![
                        history::Turn {
                            user: true,
                            text: "Latest user request".into(),
                        },
                        history::Turn {
                            user: false,
                            text: "Latest assistant response".into(),
                        },
                    ],
                },
                history::Conversation {
                    id: "other-123".into(),
                    timestamp: String::new(),
                    prompt: "Interesting experiment".into(),
                    file: PathBuf::from("other.jsonl"),
                    turns: vec![],
                },
            ],
            ..Default::default()
        },
    );
    let text = view::detail_lines(&mut app, 60)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Latest user request") && text.contains("Latest assistant response"));
    app.dashboard.expanded_projects.insert("a".into());
    app.rebuild();
    app.selected = app
        .rows
        .iter()
        .position(|r| matches!(r, Row::Session(_,_,id) if id == "first"))
        .unwrap();
    app.command(&workers.worker, Command::History).unwrap();
    app.dashboard.history_pending = false;
    for c in "experiment".chars() {
        app.key(
            &workers.worker,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .unwrap();
    }
    assert_eq!(app.dashboard.history_query, "experiment");
    app.key(
        &workers.worker,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(
        app.options_for(std::path::Path::new("a"))
            .resume_id
            .as_deref(),
        Some("other-123")
    );
    assert!(matches!(app.rows.get(app.selected), Some(Row::Session(_,_,id)) if id == "other-123"));
    for (key, fresh) in [('n', true), ('p', false)] {
        while workers.histories.try_recv().is_ok() {}
        app.command(&workers.worker, Command::History).unwrap();
        app.key(
            &workers.worker,
            KeyEvent::new(KeyCode::Char(key), KeyModifiers::ALT),
        )
        .unwrap();
        let options = app.options_for(std::path::Path::new("a"));
        assert_eq!(options.fresh, fresh);
        assert_eq!(options.resume, !fresh);
        assert!(options.resume_id.is_none() && options.workspace.is_none());
        assert!(matches!(app.rows.get(app.selected), Some(Row::Project(_))));
    }
    assert!(workers.launches.try_recv().is_err());
}
#[test]
fn selecting_an_unloaded_session_fetches_body_in_worker_and_rejects_stale_results() {
    let workers = worker::isolated();
    let mut app = overview();
    let h = history::History {
        sessions: vec![history::Conversation {
            id: "lazy".into(),
            prompt: "Lazy".into(),
            timestamp: String::new(),
            turns: vec![],
            file: PathBuf::from("lazy.jsonl"),
        }],
        ..Default::default()
    };
    app.dashboard.store_context("a|Codex".into(), h);
    draw(&mut app, 140, 36);
    app.request_stats(&workers.worker).unwrap();
    let Detail::Session(path, agent, file, ticket) = workers.histories.try_recv().unwrap() else {
        panic!("lazy worker expected")
    };
    assert_eq!(path, PathBuf::from("a"));
    assert_eq!(file, PathBuf::from("lazy.jsonl"));
    let c = history::Conversation {
        id: "lazy".into(),
        prompt: "Lazy".into(),
        timestamp: String::new(),
        turns: vec![history::Turn {
            user: true,
            text: "Recovered".into(),
        }],
        file,
    };
    workers
        .replies
        .send(Reply::Session(
            "a".into(),
            agent,
            ticket + 1,
            Ok(Some(c.clone())),
        ))
        .unwrap();
    app.poll(&workers.worker).unwrap();
    assert!(
        app.dashboard.contexts["a|Codex"].sessions[0]
            .turns
            .is_empty()
    );
    workers
        .replies
        .send(Reply::Session("a".into(), agent, ticket, Ok(Some(c))))
        .unwrap();
    app.poll(&workers.worker).unwrap();
    assert_eq!(
        app.dashboard.contexts["a|Codex"].sessions[0].turns[0].text,
        "Recovered"
    );
}

#[test]
fn returning_to_overview_reveals_origin_hidden_by_filter_and_collapsed_group() {
    let workers = worker::isolated();
    let mut app = overview();
    app.config.overview.filter = model::Filter::Running;
    app.config.overview.collapsed.insert(6);
    app.profile_errors
        .insert("a".into(), "invalid profile".into());
    app.query = "unrelated".into();
    app.focus_after_scan = Some(PathBuf::from("a"));
    workers
        .replies
        .send(Reply::Paths(app.scan_ticket, app.projects.clone()))
        .unwrap();
    app.poll(&workers.worker).unwrap();
    assert_eq!(app.path().as_deref(), Some(std::path::Path::new("a")));
    assert_eq!(app.config.overview.filter, model::Filter::All);
    assert!(!app.config.overview.collapsed.contains(&6));
    assert!(matches!(app.rows.first(), Some(Row::Group(6))));
    app.rebuild();
    assert_eq!(app.path().as_deref(), Some(std::path::Path::new("a")));
}

#[test]
fn contribution_calendar_switches_period_and_hits_only_visible_cells_after_scroll_and_resize() {
    use dashboard::{Focus, HeatmapAction};
    let t = worker::isolated();
    let mut app = overview();
    let path = app.path().unwrap();
    let today = crate::config::now() / 86400;
    let first_day = today - (today + 3) % 7 - 364;
    app.stats.insert(
        config::key(&path),
        stats::Stats {
            source_ready: true,
            disk_ready: true,
            weeks: vec![0; 12],
            days: vec![0; 84],
            calendar: Some(stats::Calendar {
                first_day,
                today,
                days: vec![0; 371],
            }),
            ..Default::default()
        },
    );
    app.dashboard.focus = Focus::Stats;
    app.dashboard_key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    )
    .unwrap();
    assert!(app.dashboard.stats_year);
    assert_eq!(app.dashboard.overlay, Some(Focus::Stats));
    draw(&mut app, 200, 48);
    assert_eq!(
        app.dashboard
            .heatmap_hits
            .iter()
            .filter(|(_, action)| matches!(action, HeatmapAction::Day { .. }))
            .count(),
        371
    );
    let (rect, action) = app
        .dashboard
        .heatmap_hits
        .iter()
        .find(|(_, action)| {
            matches!(
                action,
                HeatmapAction::Day {
                    commits: Some(_),
                    ..
                }
            )
        })
        .copied()
        .unwrap();
    app.dashboard_mouse(&t.worker, mouse_at(MouseEventKind::Moved, rect.x, rect.y))
        .unwrap();
    assert!(
        app.dashboard
            .heatmap_hover
            .as_ref()
            .unwrap()
            .1
            .contains("UTC")
    );
    assert!(matches!(action, HeatmapAction::Day { .. }));
    app.dashboard_mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y),
    )
    .unwrap();
    assert!(app.message.contains("UTC"));
    assert!(app.dashboard.heatmap_hover.is_none());
    let (toggle, _) = app
        .dashboard
        .heatmap_hits
        .iter()
        .find(|(_, action)| *action == HeatmapAction::Period(false))
        .copied()
        .unwrap();
    app.dashboard_mouse(
        &t.worker,
        mouse_at(MouseEventKind::Down(MouseButton::Left), toggle.x, toggle.y),
    )
    .unwrap();
    assert!(!app.dashboard.stats_year);
    draw(&mut app, 200, 28);
    assert_eq!(
        app.dashboard
            .heatmap_hits
            .iter()
            .filter(|(_, action)| matches!(action, HeatmapAction::Day { .. }))
            .count(),
        84
    );
    app.dashboard.stats_scroll = 6;
    draw(&mut app, 200, 20);
    let scroll = app.dashboard.stats_scroll;
    assert!(scroll > 0);
    assert!(
        !app.dashboard
            .heatmap_hits
            .iter()
            .any(|(_, action)| matches!(action, HeatmapAction::Period(_)))
    );
    let inner = app.detail_areas[0].inner(ratatui::layout::Margin::new(2, 1));
    assert!(
        app.dashboard
            .heatmap_hits
            .iter()
            .all(|(rect, _)| inner.contains(rect.as_position())
                && rect.right() <= inner.right()
                && rect.bottom() <= inner.bottom())
    );
    // Restore width after a narrow redraw; hidden/offscreen hits are rebuilt.
    draw(&mut app, 70, 20);
    assert!(
        app.dashboard
            .heatmap_hits
            .iter()
            .all(|(rect, _)| rect.right() <= 70)
    );
    app.dashboard_key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.dashboard.overlay.is_none());
    app.dashboard.focus = Focus::Stats;
    app.dashboard_key(
        &t.worker,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(app.dashboard.overlay, Some(Focus::Stats));
    assert_eq!(app.dashboard.stats_scroll, 0);
}

#[test]
fn details_overlay_clears_calendar_hits_underneath_it() {
    let mut app = overview();
    let path = app.path().unwrap();
    app.stats.insert(
        config::key(&path),
        stats::Stats {
            source_ready: true,
            disk_ready: true,
            weeks: vec![0; 12],
            days: vec![0; 84],
            ..Default::default()
        },
    );
    app.show_stats = true;
    draw(&mut app, 200, 40);
    assert!(!app.dashboard.heatmap_hits.is_empty());
    app.dashboard.overlay = Some(dashboard::Focus::Detail);
    draw(&mut app, 200, 40);
    assert!(app.dashboard.heatmap_hits.is_empty());
}

#[test]
fn statistics_modal_covers_background_and_restores_it_after_mouse_close() {
    use dashboard::{Focus, HeatmapAction};
    let t = worker::isolated();
    for width in [120, 200, 260] {
        for action in [HeatmapAction::Period(true), HeatmapAction::Expand] {
            let mut app = overview();
            app.show_stats = true;
            app.show_detail = false;
            let path = app.path().unwrap();
            let today = config::now() / 86400;
            app.stats.insert(
                config::key(&path),
                stats::Stats {
                    source_ready: true,
                    disk_ready: true,
                    weeks: vec![0; 12],
                    days: vec![0; 84],
                    calendar: Some(stats::Calendar {
                        first_day: today - (today + 3) % 7 - 364,
                        today,
                        days: vec![0; 371],
                    }),
                    hotspots: vec![stats::Hotspot {
                        path: "unique-hotspot.rs".into(),
                        commits: 7531,
                        added: 98765,
                        removed: 4321,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
            let original = draw(&mut app, width, 42);
            let (button, _) = app
                .dashboard
                .heatmap_hits
                .iter()
                .find(|(_, hit)| *hit == action)
                .copied()
                .unwrap();
            app.dashboard_mouse(
                &t.worker,
                mouse_at(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
            )
            .unwrap();
            assert_eq!(app.dashboard.overlay, Some(Focus::Stats));
            let modal = draw(&mut app, width, 42);
            let rect = app.detail_areas[0];
            // A wide terminal used to leave another copy of the statistics
            // visible on both sides of the centered popup, including numbers.
            for y in 2..40 {
                for x in 0..width {
                    if !rect.contains((x, y).into()) {
                        assert_eq!(
                            modal[(x, y)].symbol(),
                            " ",
                            "background leak at {width} {x},{y}"
                        );
                    }
                }
            }
            let text = (rect.y..rect.bottom())
                .map(|y| {
                    (rect.x..rect.right())
                        .map(|x| modal[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(text.matches("7531").count(), 1);
            let expected = if action == HeatmapAction::Period(true) {
                371
            } else {
                84
            };
            assert_eq!(
                app.dashboard
                    .heatmap_hits
                    .iter()
                    .filter(|(_, hit)| matches!(hit, HeatmapAction::Day { .. }))
                    .count(),
                expected
            );
            // Click the popup's expand button again to close, independent of
            // the underlying sidebar's old screen coordinates.
            let (close, _) = app
                .dashboard
                .heatmap_hits
                .iter()
                .find(|(_, hit)| *hit == HeatmapAction::Expand)
                .copied()
                .unwrap();
            app.dashboard_mouse(
                &t.worker,
                mouse_at(MouseEventKind::Down(MouseButton::Left), close.x, close.y),
            )
            .unwrap();
            assert!(app.dashboard.overlay.is_none());
            app.dashboard.stats_year = false;
            app.dashboard.focus = Focus::Projects;
            let restored = draw(&mut app, width, 42);
            // Compare the body after restoration; transient focus and footer
            // messages do not change the returned layout or statistics.
            for y in 2..40 {
                for x in 0..width {
                    assert_eq!(
                        original[(x, y)],
                        restored[(x, y)],
                        "restore {width} {x},{y}"
                    );
                }
            }
        }
    }
}

#[test]
fn minimum_statistics_width_keeps_the_full_heatmap_after_dragging_to_the_limit() {
    let t = worker::isolated();
    for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
        crate::i18n::with_language(language, || {
            let mut app = overview();
            app.show_detail = true;
            app.show_stats = true;
            app.config.overview.detail_ratio = Some(10000);
            let path = app.path().unwrap();
            app.stats.insert(
                config::key(&path),
                stats::Stats {
                    source_ready: true,
                    disk_ready: true,
                    weeks: vec![0; 12],
                    days: vec![0; 84],
                    ..Default::default()
                },
            );
            for width in [160, 180, 200, 260] {
                draw(&mut app, width, 42);
                let columns = app.dashboard.columns.unwrap();
                let rect = columns.stats.unwrap();
                assert!(
                    rect.width >= 32,
                    "statistics must reserve the heatmap and padding"
                );
                let divider = columns.dividers[1].unwrap();
                app.dashboard_mouse(
                    &t.worker,
                    mouse_at(
                        MouseEventKind::Down(MouseButton::Left),
                        divider.x,
                        divider.y + 2,
                    ),
                )
                .unwrap();
                app.dashboard_mouse(
                    &t.worker,
                    mouse_at(
                        MouseEventKind::Drag(MouseButton::Left),
                        width - 1,
                        divider.y + 2,
                    ),
                )
                .unwrap();
                draw(&mut app, width, 42);
                assert!(app.dashboard.columns.unwrap().stats.unwrap().width >= 32);
                assert_eq!(
                    app.dashboard
                        .heatmap_hits
                        .iter()
                        .filter(|(_, action)| matches!(
                            action,
                            dashboard::HeatmapAction::Day { .. }
                        ))
                        .count(),
                    84
                );
                // Cancel the transient drag without editing any real config.
                app.dashboard_key(&t.worker, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                    .unwrap();
            }
        });
    }
}
