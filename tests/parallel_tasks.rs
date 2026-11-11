use clap::Parser;
use devtabs::{
    launch::Options,
    tasks::{Catalog, Repository},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs tasks {}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn git(&self, args: &[&str]) -> String {
        git(&self.0, args)
    }
    fn init(&self) {
        self.git(&["init", "-b", "main"]);
        self.git(&["config", "core.autocrlf", "false"]);
        self.git(&["config", "user.name", "Fixture"]);
        self.git(&["config", "user.email", "fixture@localhost"]);
        self.git(&["config", "commit.gpgsign", "false"]);
        fs::write(self.0.join("same.txt"), "base\n").unwrap();
        self.git(&["add", "."]);
        self.git(&["commit", "-m", "base"]);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn options(root: &Path) -> Options {
    Options::try_parse_from([
        "devtabs",
        "--project",
        root.to_str().unwrap(),
        "--agent",
        "shell",
        "--no-profile",
    ])
    .unwrap()
}
#[test]
fn dirty_source_frozen_base_and_idempotent_worktree_creation() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut catalog = Catalog::default();
    let main = catalog.main_task(&options(&f.0)).unwrap();
    fs::write(f.0.join("same.txt"), "source dirty\n").unwrap();
    let plan = catalog
        .plan(&main.id, "Feature B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    let frozen = plan.task.base.clone();
    fs::write(f.0.join("later.txt"), "later").unwrap();
    f.git(&["add", "later.txt"]);
    f.git(&["commit", "-m", "later"]);
    let b = catalog.create(&home.0, &plan).unwrap();
    assert_eq!(git(&b.checkout, &["rev-parse", "HEAD"]), frozen);
    assert_eq!(
        fs::read_to_string(b.cwd.join("same.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        fs::read_to_string(f.0.join("same.txt")).unwrap(),
        "source dirty\n"
    );
    fs::write(b.cwd.join("same.txt"), "B only").unwrap();
    assert_eq!(catalog.create(&home.0, &plan).unwrap().id, b.id);
    assert_eq!(Catalog::load(&home.0).unwrap().tasks.len(), 2);
    assert_ne!(
        Repository::discover(&b.cwd).unwrap().git_dir,
        Repository::discover(&f.0).unwrap().git_dir
    );
    assert_eq!(
        Repository::discover(&b.cwd).unwrap().common,
        Repository::discover(&f.0).unwrap().common
    );
}
#[test]
fn existing_target_and_unknown_source_never_overwrite_resources() {
    let f = Fixture::new();
    f.init();
    let mut c = Catalog::default();
    let m = c.main_task(&options(&f.0)).unwrap();
    assert!(c.plan("unknown", "B", "HEAD", None).is_err());
    assert!(c.plan(&m.id, "B", "HEAD", Some(f.0.clone())).is_err());
    assert_eq!(fs::read_to_string(f.0.join("same.txt")).unwrap(), "base\n");
}

#[test]
fn parent_traversal_cannot_hide_a_nested_worktree() {
    let f = Fixture::new();
    f.init();
    let outside = Fixture::new();
    let mut c = Catalog::default();
    let m = c.main_task(&options(&f.0)).unwrap();
    let disguised = outside
        .0
        .join("..")
        .join(f.0.file_name().unwrap())
        .join("nested");
    assert!(c.plan(&m.id, "B", "HEAD", Some(disguised)).is_err());
    assert!(!f.0.join("nested").exists());
}

#[test]
fn failed_creation_is_queryable_and_leaves_source_and_external_files_intact() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut c = Catalog::default();
    let main = c.main_task(&options(&f.0)).unwrap();
    let plan = c
        .plan(&main.id, "B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    f.git(&["branch", &plan.task.branch]);
    fs::write(f.0.join("same.txt"), "A stays dirty").unwrap();
    assert!(c.create(&home.0, &plan).is_err());
    let c = Catalog::load(&home.0).unwrap();
    assert_eq!(c.operations[&plan.operation].phase, "Failed");
    assert!(c.operations[&plan.operation].error.is_some());
    assert_eq!(
        fs::read_to_string(f.0.join("same.txt")).unwrap(),
        "A stays dirty"
    );
    assert!(!plan.task.checkout.exists());
    assert_eq!(f.git(&["branch", "--show-current"]), "main");
}

#[test]
fn persisted_intent_reconciles_exact_worktree_and_preserves_changed_resources() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut c = Catalog::default();
    let main = c.main_task(&options(&f.0)).unwrap();
    let p = c
        .plan(&main.id, "B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    c.operations.insert(
        p.operation.clone(),
        devtabs::tasks::Operation {
            plan: p.clone(),
            phase: "Prepared".into(),
            error: None,
        },
    );
    c.save(&home.0).unwrap();
    f.git(&[
        "worktree",
        "add",
        "-b",
        &p.task.branch,
        p.task.checkout.to_str().unwrap(),
        &p.task.base,
    ]);
    let mut restored = Catalog::load(&home.0).unwrap();
    assert_eq!(restored.create(&home.0, &p).unwrap().id, p.task.id);
    fs::write(p.task.cwd.join("same.txt"), "keep user changes").unwrap();
    assert_eq!(restored.create(&home.0, &p).unwrap().id, p.task.id);
    assert_eq!(
        fs::read_to_string(p.task.cwd.join("same.txt")).unwrap(),
        "keep user changes"
    );
    let mut conflicting = p.clone();
    conflicting.task.base = "other".into();
    assert!(restored.create(&home.0, &conflicting).is_err());
}

#[test]
fn linked_checkout_index_and_common_ref_changes_update_the_correct_stamp() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut c = Catalog::default();
    let m = c.main_task(&options(&f.0)).unwrap();
    let p = c
        .plan(&m.id, "B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    let b = c.create(&home.0, &p).unwrap();
    let a_repo = Repository::discover(&f.0).unwrap();
    let b_repo = Repository::discover(&b.cwd).unwrap();
    let a_before = a_repo.stamp();
    let b_before = b_repo.stamp();
    fs::write(b.cwd.join("new.txt"), "new").unwrap();
    git(&b.cwd, &["add", "new.txt"]);
    assert_eq!(a_repo.stamp(), a_before);
    assert_ne!(b_repo.stamp(), b_before);
    let a_before = a_repo.stamp();
    f.git(&["branch", "another"]);
    f.git(&["pack-refs", "--all"]);
    assert_ne!(a_repo.stamp(), a_before);
}

#[test]
fn recovery_never_adopts_an_unrelated_clone_at_the_intended_path() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut c = Catalog::default();
    let main = c.main_task(&options(&f.0)).unwrap();
    let p = c
        .plan(&main.id, "B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    c.operations.insert(
        p.operation.clone(),
        devtabs::tasks::Operation {
            plan: p.clone(),
            phase: "Prepared".into(),
            error: None,
        },
    );
    c.save(&home.0).unwrap();
    git(&targets.0, &["clone", f.0.to_str().unwrap(), "B"]);
    git(&p.task.checkout, &["checkout", "-b", &p.task.branch]);
    assert!(c.create(&home.0, &p).is_err());
    assert!(p.task.checkout.join("same.txt").exists());
}

#[test]
fn overlap_uses_all_paths_beyond_hotspot_rank_and_includes_committed_progress() {
    let f = Fixture::new();
    f.init();
    let home = Fixture::new();
    let targets = Fixture::new();
    let mut c = Catalog::default();
    let a = c.main_task(&options(&f.0)).unwrap();
    let plan = c
        .plan(&a.id, "B", "HEAD", Some(targets.0.join("B")))
        .unwrap();
    let b = c.create(&home.0, &plan).unwrap();
    for i in 0..9 {
        fs::write(a.checkout.join(format!("file{i}.txt")), "A").unwrap();
        fs::write(b.checkout.join(format!("file{i}.txt")), "B").unwrap();
    }
    git(&b.checkout, &["add", "."]);
    git(&b.checkout, &["commit", "-m", "B progress"]);
    let overlap = devtabs::tasks::overlaps(&[a, b]).unwrap();
    assert_eq!(overlap.len(), 9);
    assert!(overlap.contains_key(Path::new("file8.txt")));
}
