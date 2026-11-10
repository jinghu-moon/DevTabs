use devtabs::perf::{self, Metric};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn profiling_is_opt_in_exports_live_and_final_reports_and_does_not_break_on_io_errors() {
    let root = std::env::temp_dir().join(format!("DevTabs-perf-export-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    for mode in ["disabled", "enabled", "error"] {
        let target = root.join(mode);
        if mode == "error" {
            fs::write(&target, "not a directory").unwrap();
        }
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "export_child", "--ignored", "--nocapture"])
            .env("DEVTABS_PERF_TEST", mode)
            .env("DEVTABS_PERF_TEST_TARGET", &target);
        if mode == "disabled" {
            child.env_remove("DEVTABS_PERF_DIR");
        } else {
            child.env("DEVTABS_PERF_DIR", &target);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if mode == "disabled" {
            assert!(!target.exists());
        }
        if mode == "enabled" {
            let files = fs::read_dir(&target)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            assert_eq!(
                files.len(),
                1,
                "one bounded report per process, no sample logs or leftover temp files"
            );
            let text = fs::read_to_string(&files[0]).unwrap();
            let report: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(report["timings"][0]["calls"], 3);
            assert!(
                !text.contains(root.to_str().unwrap()),
                "reports must not capture file paths"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
fn read_calls(root: &Path) -> u64 {
    fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| {
            if entry.path().extension().is_none_or(|e| e != "json") {
                return None;
            }
            let text = fs::read(entry.path()).ok()?;
            let report: serde_json::Value = serde_json::from_slice(&text).ok()?;
            report["timings"][0]["calls"].as_u64()
        })
        .unwrap_or(0)
}
#[test]
#[ignore = "subprocess fixture for isolated environment initialization"]
fn export_child() {
    let mode = std::env::var("DEVTABS_PERF_TEST").unwrap();
    let target = PathBuf::from(std::env::var_os("DEVTABS_PERF_TEST_TARGET").unwrap());
    let _exit = perf::ExitGuard::new();
    perf::record(Metric::Read, Duration::from_millis(1));
    if mode == "disabled" {
        assert!(!perf::enabled());
        assert!(perf::snapshot().timings.is_empty());
        perf::flush().unwrap();
        return;
    }
    if mode == "error" {
        assert!(perf::flush().is_err());
        return;
    }
    perf::flush().unwrap();
    assert_eq!(read_calls(&target), 1);
    perf::record(Metric::Read, Duration::from_millis(2));
    let start = Instant::now();
    while read_calls(&target) != 2 {
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "background periodic export"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    perf::record(Metric::Read, Duration::from_millis(3)); // ExitGuard flushes the final interval.
}
