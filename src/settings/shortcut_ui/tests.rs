use super::*;
fn draft() -> Draft {
    Draft::new(crate::settings::store::Snapshot::parse("schema=1".into()).unwrap())
}
fn select(m: &mut Manager, d: &Draft, id: &str) {
    m.selected = m.commands(d).iter().position(|c| c.id == id).unwrap();
    m.scope = 0;
}
fn key(m: &mut Manager, d: &mut Draft, text: &str) {
    m.key(Chord::parse(text).unwrap().event_value(), d).unwrap();
}
#[test]
fn aliases_preference_disable_delete_notes_and_restore_are_only_drafts() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    m.append(&mut d, Sequence(vec![Chord::parse("Ctrl+J").unwrap()]))
        .unwrap();
    assert!(d.dirty());
    assert_eq!(d.base.effective.shortcuts.bindings.len(), 0);
    let index = Index::build(&settings(&d).unwrap()).unwrap();
    assert_eq!(
        index.resolve("overview", &[Chord::parse("F3").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    m.action(Action::Prefer(2), &mut d).unwrap();
    assert_eq!(
        Index::build(&settings(&d).unwrap())
            .unwrap()
            .preferred("overview.note.edit", "overview"),
        "Ctrl+J"
    );
    let f3 = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "F3")
        .unwrap();
    m.action(Action::Remove(f3), &mut d).unwrap();
    m.mode = Mode::Note(TextArea::new(vec!["截图软件使用 F3".into()]));
    m.finish(&mut d).unwrap();
    let p = settings(&d).unwrap();
    assert_eq!(
        p.bindings[0].notes.get("F3").map(String::as_str),
        Some("截图软件使用 F3")
    );
    m.action(Action::Disable, &mut d).unwrap();
    assert_eq!(
        Index::build(&settings(&d).unwrap())
            .unwrap()
            .resolve("overview", &[Chord::parse("Ctrl+J").unwrap()]),
        crate::shortcuts::Resolution::TextInput
    );
    m.action(Action::Disable, &mut d).unwrap();
    m.action(Action::RestoreKey(0), &mut d).unwrap();
    assert!(settings(&d).unwrap().bindings[0].remove.is_empty());
    assert!(settings(&d).unwrap().bindings[0].notes.contains_key("F3"));
    m.action(Action::ClearNotes, &mut d).unwrap();
    assert!(settings(&d).unwrap().bindings[0].notes.is_empty());
    m.action(Action::Restore, &mut d).unwrap();
    assert!(settings(&d).unwrap().bindings.is_empty());
}

#[test]
fn modifying_one_binding_is_atomic_preserves_aliases_and_never_duplicates_defaults() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    let sequence = |text: &str| Sequence(vec![Chord::parse(text).unwrap()]);
    m.append(&mut d, sequence("F3")).unwrap();
    assert!(
        !d.dirty(),
        "adding an effective default is a read-only duplicate notice"
    );
    m.binding = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "F3")
        .unwrap();
    m.action(Action::Modify, &mut d).unwrap();
    m.mode = Mode::Text(TextArea::new(vec!["F2".into()]));
    assert!(m.finish(&mut d).is_err());
    assert!(
        !d.dirty(),
        "conflicting replacement keeps the old binding and all aliases"
    );
    m.action(Action::Cancel, &mut d).unwrap();
    assert!(m.effective_keys(&d).contains(&sequence("F3")));
    m.action(Action::Modify, &mut d).unwrap();
    m.mode = Mode::Text(TextArea::new(vec!["Ctrl+J".into()]));
    m.finish(&mut d).unwrap();
    let i = Index::build(&settings(&d).unwrap()).unwrap();
    assert_eq!(
        i.resolve("overview", &sequence("Ctrl+J").0),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    assert_eq!(
        i.resolve("overview", &sequence("Ctrl+F3").0),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    assert_eq!(
        i.resolve("overview", &sequence("F3").0),
        crate::shortcuts::Resolution::TextInput
    );
    m.binding = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "Ctrl+J")
        .unwrap();
    m.action(Action::Prefer(m.binding), &mut d).unwrap();
    m.action(Action::Modify, &mut d).unwrap();
    m.mode = Mode::Text(TextArea::new(vec!["Ctrl+K".into()]));
    m.finish(&mut d).unwrap();
    assert_eq!(
        Index::build(&settings(&d).unwrap())
            .unwrap()
            .preferred("overview.note.edit", "overview"),
        "Ctrl+K"
    );
    m.binding = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "Ctrl+K")
        .unwrap();
    m.action(Action::Modify, &mut d).unwrap();
    m.mode = Mode::Text(TextArea::new(vec!["Ctrl+F3".into()]));
    m.finish(&mut d).unwrap();
    assert_eq!(
        m.effective_keys(&d),
        vec![sequence("Ctrl+F3")],
        "replacement with an existing alias merges without duplication"
    );
    assert!(d.base.effective.shortcuts.bindings.is_empty());
}

#[test]
fn explicit_conflict_replacement_also_removes_only_the_original_selected_alias() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    m.binding = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "F3")
        .unwrap();
    m.action(Action::Modify, &mut d).unwrap();
    m.mode = Mode::Text(TextArea::new(vec!["F2".into()]));
    assert!(m.finish(&mut d).is_err());
    m.action(Action::ReplaceConflict, &mut d).unwrap();
    let i = Index::build(&settings(&d).unwrap()).unwrap();
    for text in ["F2", "Ctrl+F3"] {
        assert_eq!(
            i.resolve("overview", &[Chord::parse(text).unwrap()]),
            crate::shortcuts::Resolution::Execute("overview.note.edit")
        );
    }
    assert_eq!(
        i.resolve("overview", &[Chord::parse("F3").unwrap()]),
        crate::shortcuts::Resolution::TextInput
    );
    assert_eq!(i.preferred("overview.favorite", "overview"), "");
}

#[test]
fn narrow_details_scroll_to_defaults_sources_allowed_scopes_and_deletion_notes() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    let f3 = m
        .effective_keys(&d)
        .iter()
        .position(|s| s.display() == "F3")
        .unwrap();
    m.action(Action::Remove(f3), &mut d).unwrap();
    m.mode = Mode::Note(TextArea::new(vec!["Screenshot tool".into()]));
    m.finish(&mut d).unwrap();
    m.action(Action::Details, &mut d).unwrap();
    let before = d.tree.clone();
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(48, 14)).unwrap();
    let mut frames = String::new();
    for _ in 0..20 {
        let mut hits = vec![];
        t.draw(|f| hits = m.draw(f, f.area(), &d)).unwrap();
        frames.extend(
            t.backend()
                .buffer()
                .content
                .iter()
                .flat_map(|c| c.symbol().chars()),
        );
        assert!(
            hits.iter()
                .all(|(r, _)| r.bottom() <= 14 && r.right() <= 48)
        );
        m.action(Action::Scroll(3), &mut d).unwrap();
    }
    for value in [
        "Default",
        "Source",
        "custom",
        "Status",
        "Allowed",
        "Notes",
        "Screenshot tool",
    ] {
        assert!(frames.contains(value), "missing detail {value}");
    }
    assert_eq!(d.tree, before, "viewing details never edits preferences");
}
#[test]
fn conflicting_defaults_require_explicit_replacement_and_preserve_other_aliases() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.favorite");
    assert!(
        m.append(&mut d, Sequence(vec![Chord::parse("F3").unwrap()]))
            .is_err()
    );
    assert!(!d.dirty());
    assert!(m.conflict.is_some());
    m.action(Action::ReplaceConflict, &mut d).unwrap();
    let index = Index::build(&settings(&d).unwrap()).unwrap();
    assert_eq!(
        index.resolve("overview", &[Chord::parse("F3").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.favorite")
    );
    assert_eq!(
        index.resolve("overview", &[Chord::parse("Ctrl+F3").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    assert_eq!(
        index.resolve("overview", &[Chord::parse("F2").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.favorite")
    );
}
#[test]
fn test_mode_resolves_sequences_without_executing_and_can_prefer_tested_alias() {
    let mut d = draft();
    let mut m = Manager::default();
    m.action(Action::Test, &mut d).unwrap();
    key(&mut m, &mut d, "Ctrl+G");
    key(&mut m, &mut d, "Ctrl+G");
    let Mode::Test { received } = &m.mode else {
        panic!()
    };
    assert!(received.last().unwrap().contains("terminal.send_prefix"));
    assert!(!d.dirty());
    m.context = 1;
    key(&mut m, &mut d, "F3");
    m.action(Action::PreferTest, &mut d).unwrap();
    let index = Index::build(&settings(&d).unwrap()).unwrap();
    assert_eq!(index.preferred("overview.note.edit", "overview"), "F3");
    assert_eq!(
        index.resolve("overview", &[Chord::parse("Ctrl+F3").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    assert_eq!(d.base.effective.shortcuts.bindings.len(), 0);
}
#[test]
fn record_esc_enter_and_search_by_key_are_supported() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    m.action(Action::Record, &mut d).unwrap();
    key(&mut m, &mut d, "Esc");
    let Mode::Record(chords) = &m.mode else {
        panic!()
    };
    assert_eq!(chords.len(), 1);
    assert_eq!(chords[0], Chord::parse("Esc").unwrap());
    m.action(Action::Cancel, &mut d).unwrap();
    m.action(Action::Record, &mut d).unwrap();
    key(&mut m, &mut d, "Enter");
    assert!(
        matches!(&m.mode,Mode::Record(chords) if chords==&vec![Chord::parse("Enter").unwrap()])
    );
    m.action(Action::Cancel, &mut d).unwrap();
    assert!(!d.dirty());
    m.search = "ctrl+f3".into();
    assert!(m.commands(&d).iter().any(|c| c.id == "overview.note.edit"));
    m.search.clear();
    select(&mut m, &d, "workspace.quit");
    m.action(Action::Scope, &mut d).unwrap();
    assert_eq!(m.command(&d).unwrap().context, "terminal.input");
    m.mode = Mode::Text(TextArea::new(vec!["Ctrl+Space".into()]));
    m.finish(&mut d).unwrap();
    assert!(m.message.contains("input-method"));
}
#[test]
fn mouse_controls_and_keyboard_actions_are_available_in_narrow_layouts() {
    let mut d = draft();
    let mut m = Manager::default();
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 15)).unwrap();
    let mut hits = vec![];
    t.draw(|f| hits = m.draw(f, f.area(), &d)).unwrap();
    assert!(!hits.is_empty());
    select(&mut m, &d, "overview.note.edit");
    key(&mut m, &mut d, "E");
    assert!(matches!(m.mode, Mode::Text(_)));
    assert!(m.paste("Ctrl+J"));
    key(&mut m, &mut d, "Enter");
    assert!(d.dirty());
    key(&mut m, &mut d, "T");
    key(&mut m, &mut d, "Ctrl+S");
    assert!(matches!(m.mode, Mode::Test { .. }));
}

#[test]
fn physical_test_records_down_up_and_repeats_in_a_bounded_diagnostic() {
    let mut d = draft();
    let mut m = Manager::default();
    m.action(Action::Test, &mut d).unwrap();
    for i in 0..50 {
        let raw = crate::shortcuts::ledger::RawKey::parse(
            format!("\x1b[114;61;0;{};8;{}_", i % 2, i + 1).as_bytes(),
        )
        .unwrap();
        m.observe_raw(&raw);
    }
    assert_eq!(m.raw_events.len(), 8);
    assert!(m.raw_events.back().unwrap().contains("repeat=50"));
    assert!(!d.dirty());
}
#[test]
fn asynchronous_import_is_only_a_draft_and_export_uses_committed_values() {
    let root = std::env::temp_dir().join(format!("DevTabs-shortcut-io-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("config.toml"),
        "schema=1\n[shortcuts]\nprefix='Ctrl+G'\n",
    )
    .unwrap();
    let incoming = root.join("import.toml");
    std::fs::write(&incoming, "schema=1\n[shortcuts]\nprefix='Ctrl+K'\n").unwrap();
    let mut d = Draft::new(
        super::super::store::SettingsStore::new(&root)
            .read()
            .unwrap(),
    );
    let mut m = Manager::default();
    m.root = root.clone();
    m.mode = Mode::Import(TextArea::new(vec![incoming.display().to_string()]));
    m.finish(&mut d).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while m.pending() {
        m.tick(&mut d);
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(settings(&d).unwrap().prefix.as_deref(), Some("Ctrl+G"));
    assert!(
        m.candidate.is_some(),
        "imports are reviewed before changing the draft"
    );
    m.action(Action::ConfirmRisk, &mut d).unwrap();
    assert_eq!(settings(&d).unwrap().prefix.as_deref(), Some("Ctrl+K"));
    assert!(
        std::fs::read_to_string(root.join("config.toml"))
            .unwrap()
            .contains("Ctrl+G")
    );
    let export = root.join("export.toml");
    m.export_confirm = Some(export.display().to_string());
    m.action(Action::ConfirmExport, &mut d).unwrap();
    while m.pending() {
        m.tick(&mut d);
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert!(std::fs::read_to_string(export).unwrap().contains("Ctrl+G"));
    assert!(d.dirty());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn native_recording_waits_for_release_and_risks_require_explicit_confirmation() {
    use crate::shortcuts::ledger::RawKey;
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    m.action(Action::Record, &mut d).unwrap();
    let raw = RawKey::parse(b"\x1b[9;15;9;1;2;1_").unwrap();
    m.observe_raw(&raw);
    key(&mut m, &mut d, "Alt+Tab");
    assert!(matches!(&m.mode,Mode::Record(keys) if keys.is_empty()));
    assert!(m.finish(&mut d).is_err());
    assert!(!d.dirty());
    m.observe_raw(&RawKey::parse(&raw.release(2)).unwrap());
    assert!(matches!(&m.mode,Mode::Record(keys) if keys.len()==1));
    m.finish(&mut d).unwrap();
    assert!(m.candidate.is_some());
    assert!(!d.dirty());
    key(&mut m, &mut d, "Enter");
    assert!(!d.dirty(), "Enter is not risk acknowledgement");
    m.action(Action::ConfirmRisk, &mut d).unwrap();
    assert!(d.dirty());
    assert_eq!(
        Index::build(&settings(&d).unwrap())
            .unwrap()
            .resolve("overview", &[Chord::parse("Alt+Tab").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
    m.mode = Mode::Text(TextArea::new(vec!["Ctrl+F4".into()]));
    m.finish(&mut d).unwrap();
    assert!(m.candidate.is_some());
    let before = toml::Value::try_from(settings(&d).unwrap()).unwrap();
    m.action(Action::Cancel, &mut d).unwrap();
    assert_eq!(
        toml::Value::try_from(settings(&d).unwrap()).unwrap(),
        before
    );
}
#[test]
fn removed_custom_alias_can_keep_history_without_becoming_a_binding_mask() {
    let mut d = draft();
    let mut m = Manager::default();
    select(&mut m, &d, "overview.note.edit");
    let alias = Sequence(vec![Chord::parse("Ctrl+J").unwrap()]);
    m.append(&mut d, alias.clone()).unwrap();
    let n = m
        .effective_keys(&d)
        .iter()
        .position(|s| s == &alias)
        .unwrap();
    m.action(Action::Remove(n), &mut d).unwrap();
    m.mode = Mode::Note(TextArea::new(vec!["history".into()]));
    m.finish(&mut d).unwrap();
    let b = &settings(&d).unwrap().bindings[0];
    assert!(b.add.is_empty());
    assert!(b.remove.is_empty());
    assert_eq!(b.notes.get("Ctrl+J").map(String::as_str), Some("history"));
    m.append(&mut d, alias).unwrap();
    assert!(
        settings(&d).unwrap().bindings[0]
            .notes
            .contains_key("Ctrl+J")
    );
    m.action(Action::Restore, &mut d).unwrap();
    assert!(
        settings(&d).unwrap().bindings[0]
            .notes
            .contains_key("Ctrl+J")
    );
    assert_eq!(
        Index::build(&settings(&d).unwrap())
            .unwrap()
            .resolve("overview", &[Chord::parse("F3").unwrap()]),
        crate::shortcuts::Resolution::Execute("overview.note.edit")
    );
}
#[test]
fn effective_export_is_separate_from_importable_differences_and_does_not_write_settings() {
    let root = std::env::temp_dir().join(format!("devtabs-catalog-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("catalog.json");
    super::super::operations::execute_at(
        &root,
        &super::super::operations::Action::ExportEffectiveShortcuts,
        &path.display().to_string(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["kind"], "devtabs.effective-shortcut-catalog");
    assert!(
        report["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["command"] == "overview.note.edit" && b["preferred"] == "Ctrl+F3")
    );
    assert!(super::super::store::Snapshot::parse(text).is_err());
    assert!(!root.join("config.toml").exists());
    std::fs::remove_dir_all(root).unwrap();
}
