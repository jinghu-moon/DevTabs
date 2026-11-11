#[allow(dead_code)]
mod support;
use devtabs::rounds::{Mode, Recorder};
use std::{fs, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn dirty_start_mid_round_commit_and_later_edits_preserve_exact_history() {
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "fixture"]);
    git(&root, &["config", "user.email", "fixture@localhost"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    git(&root, &["config", "core.hooksPath", "NUL"]);
    fs::write(root.join("a.txt"), "base\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base"]);
    fs::write(root.join("a.txt"), "dirty before\n").unwrap();
    fs::write(root.join("new.txt"), "already staged\n").unwrap();
    git(&root, &["add", "new.txt"]);
    let rec = Recorder::new(f.0.join("history"));
    let begin = rec.begin("task", "run", &root, Mode::Git, false).unwrap();
    assert_eq!(begin.baseline.len(), 2, "Only dirty exceptions need copies");
    fs::write(root.join("a.txt"), "after\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "during round"]);
    let done = rec.finish(begin).unwrap();
    assert_eq!(done.changes.len(), 1);
    assert!(done.reasons.is_empty(), "{:?}", done.reasons);
    fs::write(root.join("a.txt"), "future").unwrap();
    let c = &done.changes[0];
    assert_eq!(c.path, Path::new("a.txt"));
    assert_eq!(
        rec.content(c.before.as_ref().unwrap()).unwrap(),
        b"dirty before\n"
    );
    assert_eq!(rec.content(c.after.as_ref().unwrap()).unwrap(), b"after\n");
    git(&root, &["gc", "--prune=now"]);
    assert_eq!(
        rec.content(c.before.as_ref().unwrap()).unwrap(),
        b"dirty before\n"
    );
}
#[test]
fn snapshot_scope_is_frozen_and_missing_content_is_never_filled_from_current_disk() {
    let f = support::Fixture::new();
    let root = f.0.join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a"), "old").unwrap();
    fs::write(root.join("outside"), "outside").unwrap();
    let rec = Recorder::new(f.0.join("history"));
    let start = rec
        .begin(
            "task",
            "run",
            &root,
            Mode::Snapshot(vec!["src".into()]),
            false,
        )
        .unwrap();
    fs::remove_file(root.join("src/a")).unwrap();
    fs::write(root.join("outside"), "changed").unwrap();
    let done = rec.finish(start).unwrap();
    assert_eq!(done.changes.len(), 1);
    assert!(done.changes[0].after.is_none());
    assert_eq!(
        rec.content(done.changes[0].before.as_ref().unwrap())
            .unwrap(),
        b"old"
    );
    assert!(!root.join(".git").exists());
    assert!(rec.content("../../outside").is_err());
}
#[test]
fn crlf_clean_baseline_and_literal_bracket_paths_have_truthful_before_contents() {
    let f = support::Fixture::new();
    let root = f.0.join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "user.email", "fixture@localhost"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.autocrlf", "false"]);
    fs::write(root.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
    fs::write(root.join("[a].txt"), "old\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base"]);
    fs::remove_file(root.join("[a].txt")).unwrap();
    git(&root, &["checkout", "--", "[a].txt"]);
    assert_eq!(fs::read(root.join("[a].txt")).unwrap(), b"old\r\n");
    let rec = Recorder::new(f.0.join("history"));
    let start = rec.begin("task", "run", &root, Mode::Git, false).unwrap();
    assert!(start.baseline.is_empty());
    fs::write(root.join("[a].txt"), "new\r\n").unwrap();
    let done = rec.finish(start).unwrap();
    assert_eq!(done.changes.len(), 1);
    assert!(done.reasons.is_empty(), "{:?}", done.reasons);
    assert_eq!(
        rec.content(done.changes[0].before.as_ref().unwrap())
            .unwrap(),
        b"old\r\n"
    );
}
