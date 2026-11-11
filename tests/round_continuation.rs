#[allow(dead_code)]
mod support;
use devtabs::rounds::{
    Recorder,
    service::{Action, Service},
};
#[test]
fn observed_stop_versions_keep_the_same_capture_window_when_provider_continues() {
    let f = support::Fixture::new();
    let home = f.0.join("history");
    let root = f.0.join("project");
    std::fs::create_dir(&root).unwrap();
    let mut s = Service::new(home.clone());
    s.automatic("task", "run", &root, true, true, 0).unwrap();
    s.act(
        "task",
        "run",
        &root,
        Action::Observe {
            paths: vec!["first.txt".into()],
            gap: false,
        },
        true,
    )
    .unwrap();
    s.automatic("task", "run", &root, false, false, 1).unwrap();
    s.automatic("task", "run", &root, true, false, 1).unwrap();
    s.act(
        "task",
        "run",
        &root,
        Action::Observe {
            paths: vec!["second.txt".into()],
            gap: false,
        },
        true,
    )
    .unwrap();
    s.automatic("task", "run", &root, false, false, 2).unwrap();
    let r = Recorder::new(home).list("task").unwrap();
    let latest = r.iter().find(|r| r.result == Some(2)).unwrap();
    assert!(
        latest
            .changes
            .iter()
            .any(|c| c.path == std::path::Path::new("first.txt")),
        "continued Stop must retain prior observed paths"
    );
    assert_eq!(
        r.iter().find(|r| r.result == Some(1)).unwrap().started,
        latest.started
    );
}
