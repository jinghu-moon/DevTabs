#[allow(dead_code)]
mod support;
use devtabs::rounds::{Limits, Mode, Recorder};
use std::fs;
#[test]
fn budget_unknown_is_not_a_deletion_and_corrupt_history_is_rejected() {
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a"), "old").unwrap();
    let mut rec = Recorder::new(f.0.join("history"));
    rec.limits = Limits {
        file: 8,
        ..Limits::default()
    };
    let start = rec
        .begin(
            "task",
            "run",
            &root,
            Mode::Snapshot(vec!["src".into()]),
            true,
        )
        .unwrap();
    assert!(start.late);
    fs::write(root.join("src/a"), "way over budget").unwrap();
    let done = rec.finish(start).unwrap();
    assert!(done.changes[0].reason.is_some());
    assert!(
        !done.reasons.is_empty(),
        "Partial capture needs a visible reason"
    );
    let key = done.changes[0].before.as_ref().unwrap();
    fs::write(rec.home.join("objects").join(key), "corrupt").unwrap();
    assert!(rec.content(key).is_err());
    assert!(rec.diff(&done.changes[0]).is_err());
}
#[test]
fn bounded_history_never_drops_an_active_round_and_task_quota_prevents_unbounded_storage() {
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a"), "old").unwrap();
    let mut rec = Recorder::new(f.0.join("history"));
    rec.limits = Limits {
        task: 4,
        ..Limits::default()
    };
    let start = rec
        .begin(
            "task",
            "run",
            &root,
            Mode::Snapshot(vec!["src".into()]),
            false,
        )
        .unwrap();
    fs::write(root.join("src/a"), "new").unwrap();
    let done = rec.finish(start).unwrap();
    assert!(
        done.changes[0].reason.is_some(),
        "Per-task byte limit must include prior content"
    );
}
#[test]
fn collection_preserves_active_and_pinned_versions_and_reclaims_only_unreferenced_content() {
    use std::collections::BTreeSet;
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a"), "start").unwrap();
    let rec = Recorder::new(f.0.join("history"));
    let mut pin = None;
    for i in 0..22 {
        let start = rec
            .begin(
                "task",
                "run",
                &root,
                Mode::Snapshot(vec!["src".into()]),
                false,
            )
            .unwrap();
        fs::write(root.join("src/a"), format!("version {i}")).unwrap();
        let done = rec.finish(start).unwrap();
        if i == 0 {
            pin = Some(done);
        }
    }
    let active = rec
        .begin(
            "task",
            "run",
            &root,
            Mode::Snapshot(vec!["src".into()]),
            false,
        )
        .unwrap();
    let pin = pin.unwrap();
    rec.collect("task", &BTreeSet::from([pin.id.clone()]))
        .unwrap();
    assert!(rec.load("task", &active.id).is_ok());
    let loaded = rec.load("task", &pin.id).unwrap();
    assert_eq!(
        rec.content(loaded.changes[0].before.as_ref().unwrap())
            .unwrap(),
        b"start"
    );
    assert_eq!(rec.list("task").unwrap().len(), 22);
}
