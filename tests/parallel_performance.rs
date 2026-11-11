#[allow(dead_code)]
mod support;
use devtabs::rounds::{Mode, Recorder};
use std::{fs, time::Instant};
#[test]
#[ignore = "explicit bounded snapshot I/O cost comparison; writes fixture content only"]
fn snapshot_storage_workload() {
    let mut samples = vec![];
    for count in [1, 2, 4] {
        let mut times = vec![];
        for _ in 0..5 {
            let f = support::Fixture::new();
            let root = f.0.join("repo");
            fs::create_dir_all(root.join("src")).unwrap();
            for i in 0..500 {
                fs::write(
                    root.join("src").join(format!("{i}.txt")),
                    format!("{i} {}", "data".repeat(256)),
                )
                .unwrap();
            }
            let rec = Recorder::new(f.0.join("history"));
            let start = Instant::now();
            for i in 0..count {
                let r = rec
                    .begin(
                        &format!("task-{i}"),
                        "run",
                        &root,
                        Mode::Snapshot(vec!["src".into()]),
                        false,
                    )
                    .unwrap();
                assert_eq!(r.baseline.len(), 500);
                assert!(r.reasons.is_empty(), "{:?}", r.reasons);
            }
            times.push(start.elapsed().as_micros() as u64);
        }
        times.sort();
        samples.push(serde_json::json!({"tasks":count,"files_per_task":500,"samples_us":times,"p50_us":times[2],"p95_us":times[4]}));
    }
    println!("{}", serde_json::to_string(&samples).unwrap());
}
#[test]
#[ignore = "explicit 100k-file Git and non-Git mode budget fixture"]
fn clean_repository_copies_only_dirty_exceptions_for_one_two_four_tasks() {
    use std::process::Command;
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir_all(root.join("src")).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "fixture"),
        ("user.email", "fixture@localhost"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("core.hooksPath", "NUL"),
    ] {
        git(&["config", k, v]);
    }
    for i in 0..100_000 {
        fs::write(root.join("src").join(format!("{i}.txt")), "base\n").unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-m", "fixture"]);
    for i in 0..5 {
        fs::write(
            root.join("src").join(format!("{i}.txt")),
            format!("dirty {i}"),
        )
        .unwrap();
    }
    for i in 0..2 {
        fs::write(root.join(format!("untracked{i}")), format!("untracked {i}")).unwrap();
    }
    let rec = Recorder::new(f.0.join("history"));
    let mut rows = vec![];
    for count in [1, 2, 4] {
        let start = Instant::now();
        for i in 0..count {
            let r = rec
                .begin(&format!("git-{count}-{i}"), "run", &root, Mode::Git, false)
                .unwrap();
            assert_eq!(r.baseline.len(), 7);
            assert!(r.reasons.is_empty(), "{:?}", r.reasons);
        }
        rows.push(serde_json::json!({"tasks":count,"mode":"Git","tracked":100000,"exception_copies":7,"elapsed_ms":start.elapsed().as_millis()}));
    }
    let objects = fs::read_dir(rec.home.join("objects")).unwrap().count();
    assert_eq!(objects, 7, "100k clean files must not be copied");
    let nongit = f.0.join("non-git");
    fs::create_dir_all(nongit.join("src")).unwrap();
    for i in 0..5 {
        fs::write(nongit.join("src").join(format!("{i}")), "scope").unwrap();
    }
    let mut service = devtabs::rounds::service::Service::new(f.0.join("observed"));
    let start = Instant::now();
    service
        .act(
            "observed",
            "run",
            &nongit,
            devtabs::rounds::service::Action::Begin { scope: vec![] },
            false,
        )
        .unwrap();
    rows.push(serde_json::json!({"mode":"Observe","elapsed_ms":start.elapsed().as_millis(),"prior_content":false}));
    let start = Instant::now();
    let r = rec
        .begin(
            "snapshot",
            "run",
            &nongit,
            Mode::Snapshot(vec!["src".into()]),
            false,
        )
        .unwrap();
    assert_eq!(r.baseline.len(), 5);
    rows.push(serde_json::json!({"mode":"Snapshot","files":5,"elapsed_ms":start.elapsed().as_millis(),"prior_content":true}));
    println!("{}", serde_json::to_string(&rows).unwrap());
}
