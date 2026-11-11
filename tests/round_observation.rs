#[allow(dead_code)]
mod support;
use devtabs::rounds::{
    Mode,
    service::{Action, Service},
};
use std::{
    fs,
    time::{Duration, Instant},
};
#[test]
fn non_git_observation_uses_real_events_without_inventing_old_content_or_initializing_git() {
    let f = support::Fixture::new();
    let root = f.0.join("project");
    fs::create_dir(&root).unwrap();
    let mut service = Service::new(f.0.join("history"));
    let begin = service
        .act("task", "run", &root, Action::Begin { scope: vec![] }, false)
        .unwrap();
    assert_eq!(begin.records[0].mode, Mode::Observe);
    fs::write(root.join("new.txt"), "new").unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let view = service
            .act("task", "run", &root, Action::List, false)
            .unwrap();
        if view.records[0]
            .changes
            .iter()
            .any(|c| c.path == std::path::Path::new("new.txt"))
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "Watcher did not deliver the known file event"
        );
        std::thread::yield_now();
    }
    let end = service
        .act("task", "run", &root, Action::End, false)
        .unwrap();
    let r = &end.records[0];
    assert!(
        r.changes
            .iter()
            .any(|c| c.path == std::path::Path::new("new.txt"))
    );
    assert!(r.changes.iter().all(|c| c.before.is_none()));
    assert!(!root.join(".git").exists());
}
