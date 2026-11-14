#![cfg(windows)]
#[allow(dead_code)]
mod support;
use devtabs::checks::Spec;
use std::fs;
#[test]
fn revision_hash_reads_nested_untracked_files_and_ignores_ignored_tree_badges() {
    let f = support::Fixture::new();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&f.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "--quiet"]);
    fs::write(f.0.join(".gitignore"), "ignored/\n").unwrap();
    fs::write(f.0.join("tracked"), "original").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "-m",
        "baseline",
    ]);
    let hash = || devtabs::checks::fingerprint(&f.0).unwrap();
    let clean = hash();
    fs::create_dir_all(f.0.join("ignored/nested")).unwrap();
    fs::write(f.0.join("ignored/nested/file"), "ignored").unwrap();
    assert_eq!(
        hash(),
        clean,
        "ignored file-tree directory badges are not revision files"
    );
    fs::create_dir_all(f.0.join("new/nested")).unwrap();
    let untracked = f.0.join("new/nested/file");
    fs::write(&untracked, "one").unwrap();
    let first = hash();
    assert_ne!(first, clean);
    fs::write(&untracked, "two").unwrap();
    let second = hash();
    assert_ne!(second, first, "nested untracked content is hashed");
    fs::rename(f.0.join("tracked"), f.0.join("renamed")).unwrap();
    git(&["add", "tracked", "renamed"]);
    let renamed = hash();
    assert_ne!(
        renamed, second,
        "staged rename and removed path are represented"
    );
    fs::write(f.0.join("renamed"), "worktree edit").unwrap();
    assert_ne!(
        hash(),
        renamed,
        "unstaged edit over the index is represented"
    );
}

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
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_devtabs"));
    let home = f.0.join("home");
    assert!(
        devtabs::checks::initialize_with_executable("B", &root, &home, exe, &profile.prepare)
            .is_err()
    );
    fs::write(root.join("ready"), "").unwrap();
    devtabs::checks::initialize_with_executable("B", &root, &home, exe, &profile.prepare).unwrap();
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
