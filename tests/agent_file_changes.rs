#[allow(dead_code)]
mod support;
use devtabs::rounds::{
    Recorder,
    service::{Action, Service},
};
use std::{fs, path::Path};
#[test]
fn panel_result_is_exact_and_clicks_beyond_first_metadata_page_open_correct_diff() {
    let fixture = support::Fixture::new();
    let root = fixture.0.join("project");
    fs::create_dir(&root).unwrap();
    for n in 0..120 {
        fs::write(
            root.join(format!("file-{n:03}.rs")),
            format!("before {n}\n"),
        )
        .unwrap();
    }
    let home = fixture.0.join("history");
    let mut service = Service::new(home.clone());
    service
        .act(
            "task",
            "run",
            &root,
            Action::Begin {
                scope: (0..120).map(|n| format!("file-{n:03}.rs").into()).collect(),
            },
            true,
        )
        .unwrap();
    for n in 0..120 {
        fs::write(root.join(format!("file-{n:03}.rs")), format!("after {n}\n")).unwrap();
    }
    service
        .automatic("task", "run", &root, false, false, 1)
        .unwrap();
    let first = service
        .act(
            "task",
            "run",
            &root,
            Action::Result {
                run: "run".into(),
                result: 1,
            },
            false,
        )
        .unwrap();
    assert_eq!(first.paths.len(), 120);
    assert_eq!(first.records[0].file_count, 120);
    assert_eq!(first.records[0].changes.len(), 100);
    assert!(first.paths.iter().all(|p| p.starts_with(&root)));
    let round = first.records[0].id.clone();
    let diff = service
        .act(
            "task",
            "run",
            &root,
            Action::Open {
                round: round.clone(),
                path: Some(119),
            },
            false,
        )
        .unwrap();
    assert_eq!(diff.offset, 100);
    assert_eq!(diff.records[0].changes[19].path, Path::new("file-119.rs"));
    assert!(diff.diff.as_ref().unwrap().contains("-before 119"));
    assert!(diff.diff.as_ref().unwrap().contains("+after 119"));
    assert!(
        service
            .act(
                "task",
                "run",
                &root,
                Action::Result {
                    run: "wrong-run".into(),
                    result: 1
                },
                false
            )
            .is_err()
    );
    assert!(
        service
            .act(
                "task",
                "run",
                &root,
                Action::Result {
                    run: "run".into(),
                    result: 2
                },
                false
            )
            .is_err()
    );
    fs::write(root.join("file-119.rs"), "future disk contents\n").unwrap();
    let historical = service
        .act(
            "task",
            "run",
            &root,
            Action::Open {
                round,
                path: Some(119),
            },
            false,
        )
        .unwrap();
    assert!(!historical.diff.unwrap().contains("future disk contents"));
    assert!(
        !Recorder::new(home)
            .list("task")
            .unwrap()
            .iter()
            .find(|r| r.result == Some(1))
            .unwrap()
            .read
    );
}
#[test]
fn parallel_task_results_keep_their_own_checkout_even_with_the_same_filename() {
    let fixture = support::Fixture::new();
    let mut service = Service::new(fixture.0.join("history"));
    for task in ["left", "right"] {
        let root = fixture.0.join(task);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("same.rs"), "before\n").unwrap();
        service
            .act(
                task,
                task,
                &root,
                Action::Begin {
                    scope: vec!["same.rs".into()],
                },
                true,
            )
            .unwrap();
        fs::write(root.join("same.rs"), format!("{task}\n")).unwrap();
        service
            .automatic(task, task, &root, false, false, 1)
            .unwrap();
    }
    for task in ["left", "right"] {
        let root = fixture.0.join(task);
        let view = service
            .act(
                task,
                task,
                &root,
                Action::Result {
                    run: task.into(),
                    result: 1,
                },
                false,
            )
            .unwrap();
        assert_eq!(view.paths, vec![root.join("same.rs")]);
        let diff = service
            .act(
                task,
                task,
                &root,
                Action::Open {
                    round: view.records[0].id.clone(),
                    path: Some(0),
                },
                false,
            )
            .unwrap();
        assert!(diff.diff.unwrap().contains(&format!("+{task}")));
    }
}
