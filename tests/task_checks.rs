#![cfg(windows)]
#[allow(dead_code)]
mod support;
use devtabs::checks::Spec;
use std::fs;
#[test]
fn checks_run_in_explicit_cwd_and_mark_a_mutating_check_stale_without_using_agent_input() {
    let f = support::Fixture::new();
    fs::write(f.0.join("a.txt"), "before").unwrap();
    let spec = Spec {
        argv: vec![
            "pwsh.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            "[IO.File]::WriteAllText('a.txt','after'); Write-Output checked".into(),
        ],
        cwd: f.0.clone(),
        env: Default::default(),
        timeout_ms: 3000,
    };
    let report = devtabs::checks::run_with_executable(
        "task",
        4,
        &f.0,
        &spec,
        std::path::Path::new(env!("CARGO_BIN_EXE_devtabs")),
    )
    .unwrap();
    assert_eq!(report.exit, Some(0));
    assert!(report.stale);
    assert!(report.output.contains("checked"));
    assert_eq!(fs::read_to_string(f.0.join("a.txt")).unwrap(), "after");
}
#[test]
fn timeout_cleans_only_the_checks_job_and_never_reports_success() {
    let f = support::Fixture::new();
    let spec = Spec {
        argv: vec![
            "pwsh.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 60".into(),
        ],
        cwd: f.0.clone(),
        env: Default::default(),
        timeout_ms: 100,
    };
    let start = std::time::Instant::now();
    let report = devtabs::checks::run_with_executable(
        "task",
        1,
        &f.0,
        &spec,
        std::path::Path::new(env!("CARGO_BIN_EXE_devtabs")),
    )
    .unwrap();
    assert!(report.timeout);
    assert_ne!(report.exit, Some(0));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}
#[test]
fn initialization_retry_skips_successful_steps_and_does_not_initialize_other_tasks() {
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir(&root).unwrap();
    let spec = |command: &str| Spec {
        argv: vec![
            "pwsh.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            command.into(),
        ],
        cwd: ".".into(),
        env: Default::default(),
        timeout_ms: 3000,
    };
    let profile = devtabs::project_profile::Profile {
        prepare: vec![
            spec("Add-Content one.txt first"),
            spec("if (!(Test-Path ready)) {exit 1}; Add-Content two.txt second"),
        ],
        ..Default::default()
    };
    profile.save(&root).unwrap();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_devtabs"));
    let home = f.0.join("home");
    assert!(devtabs::checks::initialize_with_executable("B", &root, &home, exe).is_err());
    fs::write(root.join("ready"), "").unwrap();
    devtabs::checks::initialize_with_executable("B", &root, &home, exe).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("one.txt"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(root.join("two.txt").is_file());
    assert!(!home.join("preparation/A").exists());
}
