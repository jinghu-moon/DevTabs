#[allow(dead_code)]
mod support;
use clap::Parser;
use devtabs::{lifecycle, tasks::Catalog};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().to_owned()
}
struct Repo {
    _f: support::Fixture,
    home: std::path::PathBuf,
    a: devtabs::tasks::Task,
    b: devtabs::tasks::Task,
}
impl Repo {
    fn new() -> Self {
        let f = support::Fixture::new();
        let root = f.0.join("repo");
        let home = f.0.join("home");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        for (k, v) in [
            ("user.name", "fixture"),
            ("user.email", "fixture@localhost"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", "NUL"),
        ] {
            git(&root, &["config", k, v]);
        }
        fs::write(root.join("a"), "base").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        let options = devtabs::launch::Options::try_parse_from([
            "devtabs",
            "--project",
            root.to_str().unwrap(),
            "--agent",
            "shell",
            "--no-profile",
        ])
        .unwrap();
        let mut c = Catalog::default();
        let a = c.main_task(&options).unwrap();
        let p = c.plan(&a.id, "B", "HEAD", Some(f.0.join("B"))).unwrap();
        let b = c.create(&home, &p).unwrap();
        Self { _f: f, home, a, b }
    }
    fn progress(&self) {
        fs::write(self.b.checkout.join("b"), "feature").unwrap();
        git(&self.b.checkout, &["add", "."]);
        git(&self.b.checkout, &["commit", "-m", "feature"]);
    }
}
#[test]
fn fast_forward_freezes_both_heads_and_rejects_dirty_stale_and_non_ff() {
    let r = Repo::new();
    r.progress();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, false).unwrap();
    fs::write(r.a.checkout.join("local"), "dirty").unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(git(&r.a.checkout, &["rev-parse", "HEAD"]), p.target_head);
    fs::remove_file(r.a.checkout.join("local")).unwrap();
    let result = lifecycle::execute(&r.home, &p.id).unwrap();
    assert_eq!(result.phase, "Complete");
    assert_eq!(git(&r.a.checkout, &["rev-parse", "HEAD"]), p.source_head);
    assert_eq!(
        fs::read_to_string(r.a.checkout.join("b")).unwrap(),
        "feature"
    );
    let r = Repo::new();
    r.progress();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, false).unwrap();
    fs::write(r.a.checkout.join("independent"), "A").unwrap();
    git(&r.a.checkout, &["add", "."]);
    git(&r.a.checkout, &["commit", "-m", "A"]);
    let head = git(&r.a.checkout, &["rev-parse", "HEAD"]);
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(git(&r.a.checkout, &["rev-parse", "HEAD"]), head);
    assert!(lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, false).is_err());
}
#[test]
fn cleanup_never_deletes_main_dirty_untracked_or_ignored_content() {
    let r = Repo::new();
    assert!(lifecycle::plan(&r.home, &r.a.id, &r.b.id, true, false).is_err());
    fs::write(r.b.checkout.join(".gitignore"), "private/\n").unwrap();
    git(&r.b.checkout, &["add", "."]);
    git(&r.b.checkout, &["commit", "-m", "ignore"]);
    git(&r.a.checkout, &["merge", "--ff-only", &r.b.branch]);
    fs::create_dir(r.b.checkout.join("private")).unwrap();
    fs::write(r.b.checkout.join("private/valuable"), "user").unwrap();
    assert!(lifecycle::plan(&r.home, &r.b.id, &r.a.id, true, false).is_err());
    assert!(r.b.checkout.join("private/valuable").is_file());
    fs::remove_dir_all(r.b.checkout.join("private")).unwrap();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, true, false).unwrap();
    let done = lifecycle::execute(&r.home, &p.id).unwrap();
    assert_eq!(done.phase, "Complete");
    assert!(!r.b.checkout.exists());
    assert!(r.a.checkout.is_dir());
    assert!(Catalog::load(&r.home).unwrap().tasks[&r.b.id].archived);
}
#[test]
fn partial_cleanup_is_journaled_retryable_and_recreated_paths_are_protected() {
    let r = Repo::new();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, true, false).unwrap();
    let repo = devtabs::tasks::Repository::discover(&r.a.checkout).unwrap();
    let lock = repo
        .common
        .join("refs/heads")
        .join(format!("{}.lock", r.b.branch));
    fs::write(&lock, "").unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    let partial = lifecycle::load(&r.home, &p.id).unwrap();
    assert_eq!(partial.phase, "DeletingBranch");
    assert!(
        partial.error.as_ref().is_some_and(|e| e.contains(".lock")),
        "Git rejection reason must explain the locked resource: {:?}",
        partial.error
    );
    assert!(!r.b.checkout.exists());
    fs::create_dir(&r.b.checkout).unwrap();
    fs::write(r.b.checkout.join("valuable"), "new owner").unwrap();
    fs::remove_file(lock).unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(
        fs::read_to_string(r.b.checkout.join("valuable")).unwrap(),
        "new owner"
    );
    fs::remove_file(r.b.checkout.join("valuable")).unwrap();
    fs::remove_dir(&r.b.checkout).unwrap();
    assert_eq!(
        lifecycle::execute(&r.home, &p.id).unwrap().phase,
        "Complete"
    );
    fs::create_dir(&r.b.checkout).unwrap();
    fs::write(r.b.checkout.join("after-complete"), "keep").unwrap();
    assert_eq!(
        lifecycle::execute(&r.home, &p.id).unwrap().phase,
        "Complete"
    );
    assert!(r.b.checkout.join("after-complete").is_file());
}
#[test]
fn recreated_branch_at_same_commit_is_not_deleted_by_old_cleanup() {
    let r = Repo::new();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, true, false).unwrap();
    let repo = devtabs::tasks::Repository::discover(&r.a.checkout).unwrap();
    let lock = repo
        .common
        .join("refs/heads")
        .join(format!("{}.lock", r.b.branch));
    fs::write(&lock, "").unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    fs::remove_file(lock).unwrap();
    git(&r.a.checkout, &["branch", "-d", &r.b.branch]);
    git(&r.a.checkout, &["branch", &r.b.branch, &p.source_head]);
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(
        git(&r.a.checkout, &["rev-parse", &r.b.branch]),
        p.source_head
    );
}
#[test]
fn ignored_collision_is_not_overwritten_and_archiving_keeps_original_files() {
    let r = Repo::new();
    fs::write(r.b.checkout.join("collision"), "feature").unwrap();
    git(&r.b.checkout, &["add", "."]);
    git(&r.b.checkout, &["commit", "-m", "feature"]);
    let repo = devtabs::tasks::Repository::discover(&r.a.checkout).unwrap();
    fs::write(repo.common.join("info/exclude"), "collision\n").unwrap();
    fs::write(r.a.checkout.join("collision"), "valuable ignored").unwrap();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, false).unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(
        fs::read_to_string(r.a.checkout.join("collision")).unwrap(),
        "valuable ignored"
    );
    assert!(lifecycle::archive(&r.home, &r.a.id).unwrap().archived);
    assert!(r.a.checkout.join("a").is_file());
    assert!(r.a.checkout.join("collision").is_file());
}
#[test]
fn required_check_is_bound_to_the_exact_source_revision() {
    let r = Repo::new();
    r.progress();
    assert!(lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, true).is_err());
    let spec = devtabs::checks::Spec {
        argv: vec!["cmd.exe".into(), "/c".into(), "exit 0".into()],
        cwd: ".".into(),
        env: Default::default(),
        timeout_ms: 3000,
    };
    let mut report = devtabs::checks::run_with_executable(
        &r.b.id,
        1,
        &r.b.checkout,
        &spec,
        Path::new(env!("CARGO_BIN_EXE_devtabs")),
    )
    .unwrap();
    report.run = "fixture-run".into();
    devtabs::config::atomic_write(
        &r.home.join("checks").join(format!("{}.json", r.b.id)),
        &serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, true).unwrap();
    assert!(p.check.is_some());
    fs::write(r.b.checkout.join("later"), "later").unwrap();
    git(&r.b.checkout, &["add", "."]);
    git(&r.b.checkout, &["commit", "-m", "later"]);
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert!(lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, true).is_err());
}
#[test]
fn other_host_terminal_and_view_leases_block_resource_writes_until_released() {
    let r = Repo::new();
    r.progress();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, false, false).unwrap();
    let terminal = lifecycle::lease(&r.home, &r.b.id, false, false).unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert_eq!(git(&r.a.checkout, &["rev-parse", "HEAD"]), p.target_head);
    drop(terminal);
    lifecycle::execute(&r.home, &p.id).unwrap();
    let p = lifecycle::plan(&r.home, &r.b.id, &r.a.id, true, false).unwrap();
    let view = lifecycle::lease(&r.home, &r.b.id, true, false).unwrap();
    assert!(lifecycle::execute(&r.home, &p.id).is_err());
    assert!(r.b.checkout.is_dir());
    drop(view);
    assert_eq!(
        lifecycle::execute(&r.home, &p.id).unwrap().phase,
        "Complete"
    );
}
