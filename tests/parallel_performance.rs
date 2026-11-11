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
