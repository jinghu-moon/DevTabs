use super::*;
use std::fs;
struct Fixture(PathBuf);
impl Fixture {
    fn new(text: &str) -> Self {
        let f = Self(
            std::env::temp_dir().join(format!("DevTabs-settings-ui-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir_all(&f.0).unwrap();
        if !text.is_empty() {
            fs::write(f.0.join("config.toml"), text).unwrap();
        }
        f
    }
    fn ui(&self) -> SettingsUi {
        SettingsUi::at_root(self.0.clone(), None)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn path(keys: &[&str]) -> Path {
    keys.iter().map(|s| Part::Key((*s).into())).collect()
}
fn key(ui: &mut SettingsUi, code: KeyCode, mods: Mods) {
    ui.key(KeyEvent::new(code, mods)).unwrap();
}
fn settle(ui: &mut SettingsUi) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while ui.pending.is_some() {
        ui.tick();
        assert!(
            std::time::Instant::now() < until,
            "settings worker timed out"
        );
        std::thread::yield_now();
    }
}
fn draw(ui: &mut SettingsUi, w: u16, h: u16) -> String {
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    t.draw(|f| ui.draw(f)).unwrap();
    t.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn disabled_danger_hover_and_primary_states_are_distinct_and_non_mutating() {
    let fixture = Fixture::new("schema=1\n[terminal]\nhistory_lines=0\n");
    let mut ui = fixture.ui();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 34)).unwrap();
    terminal.draw(|f| ui.draw(f)).unwrap();
    let save = ui
        .hits
        .iter()
        .find(|(_, h)| matches!(h, Hit::Save))
        .unwrap()
        .0;
    let reset = ui
        .hits
        .iter()
        .find(|(_, h)| matches!(h, Hit::Reset))
        .unwrap()
        .0;
    let p = crate::appearance::palette();
    assert_eq!(terminal.backend().buffer()[(save.x, save.y)].fg, p.muted);
    assert_eq!(terminal.backend().buffer()[(reset.x, reset.y)].fg, p.error);
    ui.activate(Some(Hit::Save)).unwrap();
    assert!(ui.pending.is_none(), "disabled Save does not write");
    let before = ui.draft.as_ref().unwrap().tree.clone();
    let focus = (ui.category, ui.nav, ui.footer_focus);
    ui.mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: reset.x,
        row: reset.y,
        modifiers: Mods::NONE,
    })
    .unwrap();
    terminal.draw(|f| ui.draw(f)).unwrap();
    assert!(
        terminal.backend().buffer()[(reset.x, reset.y)]
            .modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert_eq!(focus, (ui.category, ui.nav, ui.footer_focus));
    assert_eq!(before, ui.draft.as_ref().unwrap().tree);
    ui.focus_path(&path(&["terminal", "history_lines"]))
        .unwrap();
    assert!(!ui.hit_enabled(&Hit::Step(-1)));
    ui.activate(Some(Hit::Step(-1))).unwrap();
    assert_eq!(before, ui.draft.as_ref().unwrap().tree);
    ui.activate(Some(Hit::Step(1))).unwrap();
    assert!(ui.hit_enabled(&Hit::Save));
    terminal.draw(|f| ui.draw(f)).unwrap();
    let save = ui
        .hits
        .iter()
        .find(|(_, h)| matches!(h, Hit::Save))
        .unwrap()
        .0;
    assert_eq!(terminal.backend().buffer()[(save.x, save.y)].bg, p.accent);
}
#[test]
fn browsing_every_category_and_f4_placeholder_is_read_only() {
    let f = Fixture::new("");
    let mut ui = f.ui();
    for i in 0..ui.pages.len() {
        ui.category = i;
        ui.stack.clear();
        draw(&mut ui, 120, 36);
        assert!(!ui.dirty());
    }
    assert!(!f.0.join("config.toml").exists());
    let mut ui = SettingsUi::at_root(f.0.clone(), Some(PathBuf::from("D:/中文 project")));
    assert!(!ui.dirty());
    draw(&mut ui, 70, 22);
    key(&mut ui, KeyCode::Esc, Mods::NONE);
    assert!(!ui.dirty());
    assert!(!f.0.join("config.toml").exists());
}
#[test]
fn inherited_lists_require_override_and_empty_is_distinct_from_inherit() {
    let f =
        Fixture::new("schema=1\n[launch]\nagent_args=['','中文 空格']\n[[project]]\npath='D:/p'\n");
    let mut ui = f.ui();
    let p = vec![
        Part::Key("project".into()),
        Part::Index(0),
        Part::Key("launch".into()),
        Part::Key("agent_args".into()),
    ];
    ui.focus_path(&p).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    assert!(!ui.dirty());
    assert_eq!(ui.page().field.key, "launch");
    ui.activate(Some(Hit::Override)).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    assert_eq!(ui.children(&ui.page()).len(), 2);
    key(&mut ui, KeyCode::Delete, Mods::NONE);
    key(&mut ui, KeyCode::Delete, Mods::NONE);
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &p)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    key(&mut ui, KeyCode::Enter, Mods::CONTROL);
    key(&mut ui, KeyCode::F(2), Mods::NONE);
    assert!(draft::get(&ui.draft.as_ref().unwrap().tree, &p).is_none());
    assert!(!ui.dirty());
}
#[test]
fn child_cancel_and_ctrl_s_preserve_literal_arguments_and_current_input() {
    let f = Fixture::new("schema=1");
    let mut ui = f.ui();
    let p = path(&["launch", "agent_args"]);
    ui.focus_path(&p).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    key(&mut ui, KeyCode::Insert, Mods::NONE);
    ui.paste("中文 \"带引号\"");
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    key(&mut ui, KeyCode::Insert, Mods::NONE);
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &p)
            .unwrap()
            .as_array()
            .unwrap()[1]
            .as_str(),
        Some("")
    );
    key(&mut ui, KeyCode::Esc, Mods::NONE);
    assert!(!ui.dirty());
    ui.focus_path(&path(&["launch", "debug_command"])).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    ui.paste("echo 中文\necho second");
    key(&mut ui, KeyCode::Char('s'), Mods::CONTROL);
    settle(&mut ui);
    assert!(ui.error.is_none(), "{:?}", ui.error);
    assert_eq!(
        SettingsStore::new(&f.0)
            .read()
            .unwrap()
            .effective
            .launch
            .debug_command
            .as_deref(),
        Some("echo 中文\necho second")
    );
}
#[test]
fn all_drafts_are_atomic_and_invalid_edits_are_recoverable() {
    let f = Fixture::new("schema=1\n# do not lose this\n");
    let mut ui = f.ui();
    ui.set_field(&path(&["appearance", "theme"]), Some("light".into()))
        .unwrap();
    ui.set_field(&path(&["terminal", "history_lines"]), Some(20001.into()))
        .unwrap();
    key(&mut ui, KeyCode::Char('s'), Mods::CONTROL);
    settle(&mut ui);
    assert!(ui.error.is_some());
    assert!(ui.dirty());
    assert_eq!(
        fs::read_to_string(f.0.join("config.toml")).unwrap(),
        "schema=1\n# do not lose this\n"
    );
    key(&mut ui, KeyCode::F(8), Mods::NONE);
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    assert_eq!(ui.row().unwrap().1.key, "history_lines");
    ui.set_field(&path(&["terminal", "history_lines"]), Some(0.into()))
        .unwrap();
    key(&mut ui, KeyCode::Char('s'), Mods::CONTROL);
    settle(&mut ui);
    assert!(ui.error.is_none());
    assert!(!ui.dirty());
}
#[test]
fn every_numeric_preference_accepts_its_endpoints_and_rejects_outside_and_overflow_input() {
    let fixture = Fixture::new("schema=1");
    for (section, name, min, max, percent) in [
        ("overview", "list_ratio", 1, 9999, true),
        ("overview", "detail_ratio", 1, 9999, true),
        ("sidebar", "agent_ratio", 1, 9999, true),
        ("sidebar", "width", 20, 80, false),
        ("terminal", "history_lines", 0, 20000, false),
    ] {
        for value in [min - 1, min, max, max + 1] {
            let mut ui = fixture.ui();
            let p = path(&[section, name]);
            ui.focus_path(&p).unwrap();
            let (_, field) = ui.row().unwrap();
            let text = if percent {
                format!("{}.{:02}", value / 100, value % 100)
            } else {
                value.to_string()
            };
            ui.start_edit(p.clone(), field, text.into(), None);
            let accepted = (min..=max).contains(&value);
            assert_eq!(
                ui.finish_edit().is_ok(),
                accepted,
                "{section}.{name} {value}"
            );
            if accepted {
                assert_eq!(
                    draft::get(&ui.draft.as_ref().unwrap().tree, &p)
                        .unwrap()
                        .as_integer(),
                    Some(value)
                );
                let prefs: super::super::model::Preferences =
                    ui.draft.as_ref().unwrap().tree.clone().try_into().unwrap();
                prefs.validate().unwrap();
            } else {
                assert!(ui.edit.is_some(), "invalid input stays editable");
            }
        }
    }
    let mut ui = fixture.ui();
    let p = path(&["sidebar", "agent_ratio"]);
    ui.focus_path(&p).unwrap();
    for invalid in ["-0.01", "0.001", "9223372036854775807", "1.2.3", "NaN"] {
        let (_, field) = ui.row().unwrap();
        ui.start_edit(p.clone(), field, invalid.into(), None);
        assert!(ui.finish_edit().is_err(), "{invalid}");
        assert!(ui.edit.is_some());
    }
}
#[test]
fn precise_percentages_and_number_boundaries_are_not_quantized() {
    let f = Fixture::new("schema=1");
    let mut ui = f.ui();
    let p = path(&["sidebar", "agent_ratio"]);
    for (input, expected) in [("0.01", 1), ("99.99", 9999), ("44.37", 4437)] {
        ui.focus_path(&p).unwrap();
        let (_, field) = ui.row().unwrap();
        ui.start_edit(p.clone(), field, input.into(), None);
        ui.finish_edit().unwrap();
        assert_eq!(
            draft::get(&ui.draft.as_ref().unwrap().tree, &p)
                .unwrap()
                .as_integer(),
            Some(expected)
        );
    }
    let (_, field) = ui.row().unwrap();
    ui.start_edit(p.clone(), field, "100.00".into(), None);
    assert!(ui.finish_edit().is_err());
    assert!(ui.edit.is_some());
}
#[test]
fn dropdown_cancel_does_not_change_theme_and_discard_uses_latest_commit() {
    let f = Fixture::new("schema=1\n[appearance]\ntheme='terminal'\n");
    let mut ui = f.ui();
    ui.focus_path(&path(&["appearance", "theme"])).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    key(&mut ui, KeyCode::Down, Mods::NONE);
    key(&mut ui, KeyCode::Esc, Mods::NONE);
    assert!(!ui.dirty());
    ui.set_field(&path(&["appearance", "theme"]), Some("tokyo".into()))
        .unwrap();
    fs::write(
        f.0.join("config.toml"),
        "schema=1\n[appearance]\ntheme='light'",
    )
    .unwrap();
    ui.discard();
    assert_eq!(crate::appearance::theme(), crate::appearance::Theme::Light);
    assert!(
        fs::read_to_string(f.0.join("config.toml"))
            .unwrap()
            .contains("light")
    );
}
#[test]
fn responsive_forms_and_exclusive_popups_keep_controls_reachable() {
    let f = Fixture::new("schema=1");
    for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
        crate::i18n::with_language(language, || {
            for theme in [
                crate::appearance::Theme::Light,
                crate::appearance::Theme::Terminal,
            ] {
                crate::appearance::set(theme);
                for (w, h) in [(2, 2), (40, 12), (70, 24), (130, 40)] {
                    let mut ui = f.ui();
                    for category in 0..13 {
                        ui.category = category;
                        ui.stack.clear();
                        draw(&mut ui, w, h);
                    }
                    if w > 2 {
                        ui.category = 0;
                        ui.nav = true;
                        key(&mut ui, KeyCode::Tab, Mods::NONE);
                        key(&mut ui, KeyCode::Tab, Mods::NONE);
                        assert_eq!(
                            ui.footer_focus,
                            Some(1),
                            "pristine Save is disabled and skipped"
                        );
                        for _ in 0..4 {
                            key(&mut ui, KeyCode::Right, Mods::NONE);
                        }
                        assert_eq!(ui.footer_focus, Some(4));
                        draw(&mut ui, w, h);
                    }
                }
            }
        });
    }
}
#[test]
fn stale_path_probe_does_not_overwrite_new_input() {
    let f = Fixture::new("schema=1\n[launch]\nshell='pwsh'\n");
    let mut ui = f.ui();
    let p = path(&["launch", "shell"]);
    let (tx, rx) = mpsc::channel();
    ui.probe = Some(rx);
    ui.set_field(&p, Some("powershell".into())).unwrap();
    tx.send(PathProbe {
        path: p,
        value: "pwsh".into(),
        result: "outdated result".into(),
    })
    .unwrap();
    assert!(!ui.poll_probe());
    assert!(ui.probes.is_empty());
}

#[test]
fn concrete_entities_are_searchable_but_environment_values_are_private() {
    let f = Fixture::new(
        "schema=1\n[[project]]\npath='D:/中文 project'\n[project.checks.check]\nargv=['pwsh']\ncwd='.'\ntimeout_ms=1001\n[project.checks.check.env]\nTOKEN='secret-value'\n",
    );
    let mut ui = f.ui();
    ui.search = "中文 project".into();
    let found = ui.search_results();
    assert!(
        found
            .iter()
            .any(|(p, _)| p == &vec![Part::Key("project".into()), Part::Index(0)])
    );
    ui.focus_path(&found[0].0).unwrap();
    ui.search = "secret-value".into();
    assert!(ui.search_results().is_empty());
    ui.search = "timeout".into();
    let found = ui.search_results();
    assert_eq!(found.len(), 1);
    ui.focus_path(&found[0].0).unwrap();
    assert_eq!(ui.row().unwrap().1.key, "timeout_ms");
}
#[test]
fn environment_errors_keep_input_and_milliseconds_and_program_are_literal() {
    let f = Fixture::new(
        "schema=1\n[[project]]\npath='D:/p'\n[project.checks.check]\nargv=['pwsh','中文 空格','']\ncwd='.'\ntimeout_ms=1001\n[project.checks.check.env]\nPATH='original'\n",
    );
    let mut ui = f.ui();
    let check = vec![
        Part::Key("project".into()),
        Part::Index(0),
        Part::Key("checks".into()),
        Part::Key("check".into()),
    ];
    let env = draft::key(&check, "env");
    ui.focus_path(&env).unwrap();
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    ui.add().unwrap();
    ui.edit.as_mut().unwrap().area = TextArea::new(vec!["path".into()]);
    assert!(ui.finish_edit().is_err());
    assert_eq!(ui.edit.as_ref().unwrap().area.lines(), &["path"]);
    assert!(!ui.draft.as_ref().unwrap().dirty());
    ui.edit.as_mut().unwrap().area = TextArea::new(vec!["EMPTY".into()]);
    ui.finish_edit().unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &draft::key(&env, "EMPTY"))
            .and_then(Value::as_str),
        Some("")
    );
    let timeout = draft::key(&check, "timeout_ms");
    ui.focus_path(&timeout).unwrap();
    ui.fine_step(1).unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &timeout).and_then(Value::as_integer),
        Some(1002)
    );
    let argv = draft::key(&check, "argv");
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &argv)
            .unwrap()
            .as_array()
            .unwrap(),
        &vec![Value::from("pwsh"), "中文 空格".into(), "".into()]
    );
}
#[test]
#[cfg(windows)]
fn lock_timeout_and_live_file_replace_failure_preserve_source_and_draft_for_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new("schema=1\n[appearance]\ntheme='terminal'\n");
    let mut ui = fixture.ui();
    ui.set_field(&path(&["appearance", "theme"]), Some("light".into()))
        .unwrap();
    let source = fixture.0.join("config.toml");
    let original = fs::read(&source).unwrap();
    let (ready, await_ready) = mpsc::channel();
    let (release, await_release) = mpsc::channel();
    std::thread::scope(|scope| {
        let root = fixture.0.clone();
        let holding = scope.spawn(move || {
            let _lock = crate::config::ConfigLock::acquire_at(&root).unwrap();
            ready.send(()).unwrap();
            let _ = await_release.recv_timeout(std::time::Duration::from_secs(20));
        });
        await_ready.recv().unwrap();
        ui.save().unwrap();
        draw(&mut ui, 48, 16);
        assert!(
            ui.pending.is_some(),
            "drawing does not block on the writer lock"
        );
        settle(&mut ui);
        assert!(ui.error.as_ref().is_some_and(|e| e.contains("busy")));
        assert!(ui.dirty());
        assert_eq!(fs::read(&source).unwrap(), original);
        release.send(()).unwrap();
        holding.join().unwrap();
    });
    // Allow reading/writing the source, but deny deletion/replacement. This
    // reaches the real atomic replace failure after preparation succeeds.
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&source)
        .unwrap();
    ui.save().unwrap();
    settle(&mut ui);
    assert!(ui.error.is_some());
    assert!(ui.dirty());
    assert_eq!(fs::read(&source).unwrap(), original);
    drop(handle);
    ui.save().unwrap();
    settle(&mut ui);
    assert!(ui.error.is_none());
    assert!(!ui.dirty());
    assert_eq!(
        SettingsStore::new(&fixture.0)
            .read()
            .unwrap()
            .effective
            .appearance
            .theme,
        Some(crate::appearance::Theme::Light)
    );
}
#[test]
fn conflicts_and_io_failures_keep_all_drafts_and_never_overwrite_source() {
    let f = Fixture::new("schema=1\n[appearance]\ntheme='terminal'\n");
    let mut ui = f.ui();
    let theme = path(&["appearance", "theme"]);
    ui.set_field(&theme, Some("tokyo".into())).unwrap();
    ui.set_field(&path(&["terminal", "history_lines"]), Some(42.into()))
        .unwrap();
    fs::write(
        f.0.join("config.toml"),
        "schema=1\n[appearance]\ntheme='light'\n",
    )
    .unwrap();
    ui.save().unwrap();
    settle(&mut ui);
    assert!(ui.conflict.is_some());
    assert!(ui.dirty());
    assert!(
        !fs::read_to_string(f.0.join("config.toml"))
            .unwrap()
            .contains("42")
    );
    ui.activate(Some(Hit::KeepMine)).unwrap();
    fs::create_dir(f.0.join("config.previous.toml")).unwrap();
    let before = fs::read(f.0.join("config.toml")).unwrap();
    ui.save().unwrap();
    settle(&mut ui);
    assert!(ui.error.is_some());
    assert!(ui.dirty());
    assert_eq!(fs::read(f.0.join("config.toml")).unwrap(), before);
    fs::remove_dir(f.0.join("config.previous.toml")).unwrap();
    ui.save().unwrap();
    settle(&mut ui);
    assert!(ui.error.is_none());
    assert!(!ui.dirty());
    assert!(
        fs::read_to_string(f.0.join("config.toml"))
            .unwrap()
            .contains("42")
    );
}
#[test]
fn popup_wheel_never_changes_background_and_shortcut_reset_is_draft_only() {
    let f = Fixture::new("schema=1\n[shortcuts]\nprefix='Ctrl+K'\n");
    let mut ui = f.ui();
    ui.focus_path(&path(&["appearance", "theme"])).unwrap();
    ui.open_row().unwrap();
    let selected = ui.selected();
    ui.mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 2,
        modifiers: Mods::NONE,
    })
    .unwrap();
    assert_eq!(ui.selected(), selected);
    assert!(!ui.dirty());
    ui.choice = None;
    ui.shortcuts_page();
    ui.reset_page().unwrap();
    assert!(ui.dirty());
    assert!(ui.draft.as_ref().unwrap().tree.get("shortcuts").is_none());
    assert!(
        fs::read_to_string(f.0.join("config.toml"))
            .unwrap()
            .contains("Ctrl+K")
    );
}
#[test]
fn custom_colors_shell_paths_and_default_layout_are_real_values() {
    let f = Fixture::new("schema=1\n[overview]\nstatistics=false\n[[project]]\npath='D:/p'\n");
    let mut ui = f.ui();
    let ratio = path(&["overview", "list_ratio"]);
    ui.focus_path(&ratio).unwrap();
    let (p, field) = ui.row().unwrap();
    assert_eq!(ui.value(&p, &field).as_integer(), Some(4400));
    let shell = path(&["launch", "shell"]);
    ui.focus_path(&shell).unwrap();
    ui.open_row().unwrap();
    ui.choose(2).unwrap();
    ui.edit.as_mut().unwrap().area = TextArea::new(vec!["D:/my shell/pwsh.exe".into()]);
    ui.finish_edit().unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &shell).and_then(Value::as_str),
        Some("D:/my shell/pwsh.exe")
    );
    let color = vec![
        Part::Key("project".into()),
        Part::Index(0),
        Part::Key("launch".into()),
        Part::Key("color".into()),
    ];
    ui.focus_path(&color).unwrap();
    ui.activate(Some(Hit::Override)).unwrap();
    ui.open_row().unwrap();
    ui.choose(6).unwrap();
    ui.edit.as_mut().unwrap().area = TextArea::new(vec!["#a1b2c3".into()]);
    ui.finish_edit().unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &color).and_then(Value::as_str),
        Some("#a1b2c3")
    );
    assert!(draw(&mut ui, 80, 24).contains("■■"));
}
#[test]
fn immediate_cache_operation_survives_draft_discard() {
    let f = Fixture::new("schema=1");
    fs::create_dir_all(f.0.join("cache/stats")).unwrap();
    fs::write(f.0.join("cache/stats/x.json"), "{}").unwrap();
    let mut ui = f.ui();
    ui.set_field(&path(&["appearance", "theme"]), Some("tokyo".into()))
        .unwrap();
    ui.run_operation(super::super::operations::Action::ClearStats)
        .unwrap();
    assert!(ui.operation_confirm.is_some());
    key(&mut ui, KeyCode::Enter, Mods::CONTROL);
    settle(&mut ui);
    assert!(!f.0.join("cache/stats/x.json").exists());
    ui.discard();
    assert_eq!(
        fs::read_to_string(f.0.join("config.toml")).unwrap(),
        "schema=1"
    );
}

#[test]
fn canceling_new_rows_and_project_subtree_restores_a_pristine_draft() {
    let f = Fixture::new("schema=1");
    let mut ui = f.ui();
    ui.focus_path(&path(&["discovery", "roots"])).unwrap();
    ui.open_row().unwrap();
    ui.add().unwrap();
    key(&mut ui, KeyCode::Esc, Mods::NONE);
    assert!(!ui.dirty());
    ui.category = 8;
    ui.stack.clear();
    ui.add().unwrap();
    assert!(ui.dirty());
    ui.leave_child(true).unwrap();
    assert!(!ui.dirty());
    assert!(ui.draft.as_ref().unwrap().tree.get("project").is_none());
}
#[test]
fn preset_same_project_entries_reorder_and_path_decisions_preserve_or_recapture() {
    use crate::settings::model as m;
    let mut p = m::Preferences::default();
    p.launch.agent = Some(crate::launch::Agent::Claude);
    let mut a = p
        .resolve(std::path::Path::new("D:/a"), &Default::default())
        .snapshot;
    a.launch.agent_args = vec!["".into(), "中文 空格".into()];
    a.terminal.history_lines = 42;
    let mut b = a.clone();
    b.launch.new_instance = true;
    b.launch.conversation = m::Conversation::Picker;
    p.preset.push(m::Preset {
        name: "日常".into(),
        entries: vec![a.clone(), b.clone()],
    });
    let f = Fixture::new(&toml::to_string(&p).unwrap());
    let mut ui = f.ui();
    let entries = vec![
        Part::Key("preset".into()),
        Part::Index(0),
        Part::Key("entries".into()),
    ];
    let profile = draft::key(
        &draft::key(&draft::index(&entries, 0), "launch"),
        "no_profile",
    );
    ui.focus_path(&profile).unwrap();
    let (_, field) = ui.row().unwrap();
    assert!(
        display(&ui.value(&profile, &field), &field).contains("[x]"),
        "Load Profile is on when no_profile=false"
    );
    ui.open_row().unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &profile).and_then(Value::as_bool),
        Some(true)
    );
    assert!(display(&ui.value(&profile, &field), &field).contains("[ ]"));
    ui.open_row().unwrap();
    ui.focus_path(&entries).unwrap();
    ui.open_row().unwrap();
    ui.select(1);
    ui.move_row(-1).unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &draft::index(&entries, 0)).unwrap()["launch"]
            ["new_instance"]
            .as_bool(),
        Some(true)
    );
    ui.select(0);
    ui.open_row().unwrap();
    let project = draft::key(&draft::index(&entries, 0), "project");
    ui.focus_path(&project).unwrap();
    let (_, field) = ui.row().unwrap();
    ui.start_edit(project.clone(), field, "D:/b".into(), None);
    ui.finish_edit().unwrap();
    assert!(ui.path_decision.is_some());
    ui.decide_path(2).unwrap();
    assert_eq!(
        draft::get(&ui.draft.as_ref().unwrap().tree, &project).and_then(Value::as_str),
        Some("D:/a")
    );
    let (_, field) = ui.row().unwrap();
    ui.start_edit(project.clone(), field, "D:/b".into(), None);
    ui.finish_edit().unwrap();
    ui.decide_path(0).unwrap();
    let entry = draft::get(&ui.draft.as_ref().unwrap().tree, &draft::index(&entries, 0)).unwrap();
    assert_eq!(entry["terminal"]["history_lines"].as_integer(), Some(42));
    assert_eq!(entry["launch"]["agent_args"][0].as_str(), Some(""));
    let (_, field) = ui.row().unwrap();
    ui.start_edit(project.clone(), field, "D:/c".into(), None);
    ui.finish_edit().unwrap();
    ui.decide_path(1).unwrap();
    let entry = draft::get(&ui.draft.as_ref().unwrap().tree, &draft::index(&entries, 0)).unwrap();
    assert_eq!(entry["terminal"]["history_lines"].as_integer(), Some(5000));
    assert_eq!(entry["launch"]["agent"].as_str(), Some("claude"));
    let before = ui.draft.as_ref().unwrap().tree.clone();
    ui.reset_page().unwrap();
    assert_eq!(
        ui.draft.as_ref().unwrap().tree,
        before,
        "reset cannot delete frozen values"
    );
}

#[test]
fn project_picker_is_explicit_background_work_and_preserves_cancel() {
    let f = Fixture::new("schema=1\n[[project]]\npath='D:/known project'\n");
    let mut ui = f.ui();
    ui.focus_path(&path(&["discovery", "excluded"])).unwrap();
    ui.open_row().unwrap();
    ui.pick_project().unwrap();
    settle(&mut ui);
    assert!(!ui.dirty());
    let values = &ui.project_picker.as_ref().unwrap().2;
    assert_eq!(values, &vec!["D:/known project".to_owned()]);
    key(&mut ui, KeyCode::Esc, Mods::NONE);
    assert!(!ui.dirty());
    ui.pick_project().unwrap();
    settle(&mut ui);
    key(&mut ui, KeyCode::Enter, Mods::NONE);
    assert_eq!(
        ui.draft.as_ref().unwrap().tree["discovery"]["excluded"][0].as_str(),
        Some("D:/known project")
    );
    assert!(
        !fs::read_to_string(f.0.join("config.toml"))
            .unwrap()
            .contains("excluded")
    );
}

#[test]
fn acceptance_frames_include_both_languages_themes_and_narrow_action_menu() {
    let f = Fixture::new("schema=1\n[[project]]\npath='D:/中文 project'\n");
    for language in [crate::i18n::Language::EnUs, crate::i18n::Language::ZhCn] {
        crate::i18n::with_language(language, || {
            for theme in [
                crate::appearance::Theme::Terminal,
                crate::appearance::Theme::Light,
            ] {
                crate::appearance::set(theme);
                for (w, h) in [(120, 34), (48, 16)] {
                    let mut ui = f.ui();
                    ui.focus_path(&path(&["appearance", "theme"])).unwrap();
                    ui.open_row().unwrap();
                    for scene in ["themes", "project", "actions", "shortcuts"] {
                        if scene == "project" {
                            ui.choice = None;
                            let p = vec![
                                Part::Key("project".into()),
                                Part::Index(0),
                                Part::Key("launch".into()),
                                Part::Key("agent".into()),
                            ];
                            ui.focus_path(&p).unwrap();
                        }
                        if scene == "actions" {
                            ui.control_menu = Some(ui.controls().len().saturating_sub(1));
                        }
                        if scene == "shortcuts" {
                            ui.control_menu = None;
                            ui.shortcuts_page();
                        }
                        let mut t =
                            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h))
                                .unwrap();
                        t.draw(|frame| ui.draw(frame)).unwrap();
                        let buffer = t.backend().buffer();
                        assert_eq!(buffer.content.len(), (w * h) as usize);
                        assert!(
                            ui.hits
                                .iter()
                                .all(|(r, _)| r.right() <= w && r.bottom() <= h)
                        );
                        if let Some(dir) = std::env::var_os("DEVTABS_SETTINGS_FRAMES") {
                            let dir = PathBuf::from(dir);
                            fs::create_dir_all(&dir).unwrap();
                            let cells=buffer.content.iter().map(|c|serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect::<Vec<_>>();
                            fs::write(
                                dir.join(format!("{language:?}-{theme:?}-{w}-{scene}.json")),
                                serde_json::to_vec(
                                    &serde_json::json!({"width":w,"height":h,"cells":cells}),
                                )
                                .unwrap(),
                            )
                            .unwrap();
                        }
                    }
                }
            }
        });
    }
}
