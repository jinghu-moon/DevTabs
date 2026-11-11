mod support;
#[test]
#[cfg(windows)]
fn isolated_job_releases_descendant_output_pipes_when_the_wrapper_is_terminated() {
    use std::io::{Read, Write};
    let f = support::Fixture::new();
    let marker = f.0.join("started");
    let job = devtabs::platform::Job::new(0).unwrap();
    let script = format!(
        "$null=[Console]::ReadLine(); & pwsh.exe -NoProfile -Command \"[IO.File]::WriteAllText('{}','ready'); Start-Sleep -Seconds 60\"",
        marker.display().to_string().replace('\'', "''")
    );
    let mut command = Command::new("pwsh.exe");
    command
        .args(["-NoProfile", "-Command", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = support::ChildGuard::spawn(&mut command);
    job.assign(&child.0).unwrap();
    child.0.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let mut pipe = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = pipe.read_to_end(&mut bytes);
        let _ = tx.send(());
    });
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < until,
            "descendant did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    job.terminate().unwrap();
    assert!(child.wait(Duration::from_secs(3)).unwrap().is_some());
    rx.recv_timeout(Duration::from_secs(3))
        .expect("Descendant must release inherited pipes");
}
use devtabs::task_runtime::{Budget, Limits, RevisionGate};
use std::{
    process::{Command, Stdio},
    time::Duration,
};

#[test]
fn byte_and_entry_limits_are_checked_before_accounting() {
    let mut b = Budget::new(Limits {
        bytes: 5,
        entries: 2,
        time: Duration::from_secs(5),
    });
    assert!(b.charge(3));
    assert!(
        !b.charge(3),
        "over-budget content must be rejected before storage"
    );
    assert_eq!(b.used(), (3, 1));
    assert!(b.charge(2));
    assert!(!b.charge(0));
    assert_eq!(b.used(), (5, 2));
    let mut expired = Budget::new(Limits {
        bytes: 9,
        entries: 9,
        time: Duration::ZERO,
    });
    assert!(expired.expired());
    assert!(!expired.charge(1));
}

#[test]
fn stale_generation_and_revision_never_replace_current_results() {
    let mut gate = RevisionGate {
        generation: 2,
        revision: 3,
    };
    assert!(!gate.accept(1, 99));
    assert!(!gate.accept(2, 3));
    assert!(gate.accept(2, 4));
    assert_eq!(gate.revision, 4);
    assert!(
        !gate.accept(3, 5),
        "generation must be explicitly changed by its owner"
    );
}

#[test]
fn timed_out_fixture_only_cleans_up_its_own_child_and_directory() {
    let other = support::Fixture::new();
    let file = other.0.join("keep.txt");
    std::fs::write(&file, "keep").unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "fixture_block_child", "--ignored"])
        .env("DEVTABS_TEST_BLOCK", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = support::ChildGuard::spawn(&mut command);
    assert!(child.wait(Duration::from_millis(30)).unwrap().is_none());
    drop(child);
    assert_eq!(std::fs::read_to_string(file).unwrap(), "keep");
}

#[test]
#[ignore = "isolated bounded-child fixture; invoked by its parent test"]
fn fixture_block_child() {
    assert!(std::env::var_os("DEVTABS_TEST_BLOCK").is_some());
    std::thread::sleep(Duration::from_secs(20));
}
