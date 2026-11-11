#[allow(dead_code)]
mod support;
use devtabs::{
    activity::{Outcome, Phase, State},
    notifications::{Ledger, Target},
};
#[test]
fn result_read_versions_survive_host_storage_reload_without_replaying_notifications() {
    let f = support::Fixture::new();
    let mut ledger = Ledger::default();
    let target = Target {
        workspace: "w".into(),
        task: "t".into(),
        session: "s".into(),
        run: "r".into(),
        result: 2,
    };
    let state = State {
        result: 2,
        phase: Phase::Working,
        results: vec![
            Outcome {
                version: 1,
                phase: Phase::Reported,
                at: 1,
                read: false,
            },
            Outcome {
                version: 2,
                phase: Phase::Failed,
                at: 2,
                read: true,
            },
        ],
        ..Default::default()
    };
    ledger.insert(target, state).unwrap();
    devtabs::config::atomic_write(&f.0.join("activity.json"), &ledger.bytes().unwrap()).unwrap();
    let mut restored = Ledger::load(&f.0).unwrap();
    assert_eq!(restored.pending().len(), 1);
    assert_eq!(restored.pending()[0].0.result, 1);
    restored.0.get_mut("r").unwrap().1.ack(1);
    assert!(restored.pending().is_empty());
}
