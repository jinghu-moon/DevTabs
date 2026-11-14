use super::*;
use crate::settings::model::{Binding as Edit, Sequence as S, Shortcuts};
fn edit(command: &str, context: &str) -> Edit {
    Edit {
        command: command.into(),
        context: context.into(),
        add: vec![],
        remove: vec![],
        preferred: None,
        disabled: false,
        notes: Default::default(),
    }
}
fn resolve(i: &Index, context: &str, key: &str) -> Resolution {
    i.resolve(context, &[Chord::parse(key).unwrap()])
}
#[test]
fn parent_effective_changes_precede_child_differences_regardless_of_file_order() {
    let mut parent = edit("overview.note.edit", "overview");
    parent.add.push(S::Chord("Ctrl+J".into()));
    parent.remove.push(S::Chord("F3".into()));
    parent.preferred = Some(S::Chord("Ctrl+J".into()));
    let mut child = edit("overview.note.edit", "overview.projects");
    child.remove.push(S::Chord("Ctrl+F3".into()));
    for bindings in [vec![parent.clone(), child.clone()], vec![child, parent]] {
        let i = Index::build(&Shortcuts {
            prefix: None,
            bindings,
        })
        .unwrap();
        assert_eq!(
            resolve(&i, "overview.projects", "Ctrl+J"),
            Resolution::Execute("overview.note.edit")
        );
        assert_eq!(
            resolve(&i, "overview.projects", "F3"),
            Resolution::TextInput
        );
        assert_eq!(
            resolve(&i, "overview.projects", "Ctrl+F3"),
            Resolution::TextInput
        );
        assert_eq!(
            resolve(&i, "overview.detail", "Ctrl+F3"),
            Resolution::Execute("overview.note.edit")
        );
        assert_eq!(
            i.preferred("overview.note.edit", "overview.projects"),
            "Ctrl+J"
        );
    }
}
#[test]
fn scoped_explicit_choice_suppresses_only_overlapping_defaults_and_invalid_preference_is_visible() {
    let mut b = edit("overview.note.edit", "overview.projects");
    b.add.push(S::Chord("F2".into()));
    b.preferred = Some(S::Chord("F24".into()));
    let i = Index::build(&Shortcuts {
        prefix: None,
        bindings: vec![b],
    })
    .unwrap();
    assert_eq!(
        resolve(&i, "overview.projects", "F2"),
        Resolution::Execute("overview.note.edit")
    );
    assert_eq!(
        resolve(&i, "overview.detail", "F2"),
        Resolution::Execute("overview.favorite")
    );
    assert!(i.diagnostics.iter().any(|s| s.contains("suppressed")));
    assert!(i.diagnostics.iter().any(|s| s.contains("Preferred")));
    assert_eq!(
        i.preferred("overview.note.edit", "overview.projects"),
        "Ctrl+F3"
    );
}
#[test]
fn contradictory_diff_fails_while_retired_entries_stay_inactive_with_diagnostics() {
    let mut b = edit("overview.note.edit", "overview");
    b.add.push(S::Chord("F3".into()));
    b.remove = b.add.clone();
    assert!(
        Index::build(&Shortcuts {
            prefix: None,
            bindings: vec![b]
        })
        .is_err()
    );
    let i = Index::build(&Shortcuts {
        prefix: None,
        bindings: vec![
            edit("retired.command", "overview"),
            edit("overview.note.edit", "overview.made_up"),
        ],
    })
    .unwrap();
    assert_eq!(i.diagnostics.len(), 2);
    assert_eq!(resolve(&i, "overview.made_up", "F3"), Resolution::TextInput);
}
#[test]
fn punctuation_shift_and_aliases_have_one_canonical_spelling() {
    for (a, b) in [
        ("Plus", "+"),
        ("Ctrl+Plus", "Ctrl+Shift++"),
        ("BackTab", "Shift+Tab"),
        ("Control+A", "Ctrl+a"),
    ] {
        assert_eq!(Chord::parse(a).unwrap(), Chord::parse(b).unwrap());
    }
    assert_ne!(Chord::parse("F").unwrap(), Chord::parse("Shift+F").unwrap());
    assert_eq!(Chord::parse("Ctrl+Plus").unwrap().to_string(), "Ctrl+Plus");
    for text in ["Ctrl+Alt+Delete", "Ctrl+Shift", "Alt+Shift", "中"] {
        assert!(Chord::parse(text).is_err(), "{text}");
    }
    for text in [
        "Ctrl+Space",
        "Ctrl+Shift+Space",
        "Shift+Space",
        "Ctrl+Period",
        "Alt+Esc",
        "Ctrl+Esc",
        "Ctrl+F4",
        "Alt+F4",
        "Win+E",
    ] {
        assert!(Chord::parse(text).unwrap().warning().is_some(), "{text}");
    }
}
#[test]
fn release_or_repeat_does_not_complete_prefix_and_zero_bindings_means_zero_ownership() {
    let i = Index::build(&Default::default()).unwrap();
    let mut r = Router::default();
    let key = i.prefix.event_value();
    assert_eq!(
        r.route(&i, "terminal.input", key),
        Resolution::AwaitSequence
    );
    for kind in [
        crossterm::event::KeyEventKind::Release,
        crossterm::event::KeyEventKind::Repeat,
    ] {
        assert_eq!(
            r.route(&i, "terminal.input", KeyEvent { kind, ..key }),
            Resolution::Handled
        );
        assert!(r.waiting());
    }
    let bindings = i
        .commands
        .iter()
        .filter(|c| c.context == "workspace")
        .map(|c| {
            let mut e = edit(c.id, c.context);
            e.disabled = true;
            e
        })
        .collect();
    let i = Index::build(&Shortcuts {
        prefix: None,
        bindings,
    })
    .unwrap();
    assert!(i.terminal_owned().is_empty());
    assert_eq!(
        resolve(&i, "terminal.input", "Ctrl+G"),
        Resolution::ForwardTerminal
    );
}
