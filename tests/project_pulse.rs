//! Project pulse/history contracts use isolated synthetic rollouts and Git repos.
use devtabs::{
    launch::Agent,
    overview::{history, stats},
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{Arc, atomic::AtomicU64},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-pulse-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn meta(id: &str, root: &std::path::Path) -> String {
    format!(
        "{}\n",
        serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":root,"timestamp":"2026-11-11T00:00:00Z"}})
    )
}
#[test]
fn reverse_history_recovers_latest_user_across_megabytes_of_tool_output_and_xml_image_prompt() {
    let f = Fixture::new();
    let path = f.0.join("large.jsonl");
    let mut text = meta("large-session", &f.0);
    text.push_str(&format!("{}\n",serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>metadata</environment_context>\n<image name=test>photo</image> 请检查最新图片"}})));
    let tool = format!(
        "{}\n",
        serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","output":"tool output only ".repeat(3000)}})
    );
    for _ in 0..35 {
        text.push_str(&tool);
    }
    text.push_str(&format!("{}\n",serde_json::json!({"type":"event_msg","payload":{"type":"agent_message","message":"图片分析完成"}})));
    fs::write(&path, text).unwrap();
    let c = history::read(&path, &f.0, Agent::Codex).unwrap().unwrap();
    assert_eq!(c.turns.len(), 2);
    assert!(c.turns[0].user);
    assert!(c.turns[0].text.contains("请检查最新图片"));
    assert!(!c.turns[0].text.contains("metadata"));
    assert_eq!(c.turns[1].text, "图片分析完成");
    assert!(!c.turns.iter().any(|t| t.text.contains("tool output")));
}
#[test]
fn catalogue_lists_more_than_fifteen_sessions_names_are_overlaid_only_after_cwd_validation() {
    let f = Fixture::new();
    let root = f.0.join("sessions");
    fs::create_dir(&root).unwrap();
    for i in 0..40 {
        let id = format!("session-{i:02}");
        let mut text = meta(&id, &f.0);
        text.push_str(&format!("{}\n",serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":format!("task {i}")}})));
        fs::write(root.join(format!("{i}.jsonl")), text).unwrap();
    }
    fs::write(
        root.join("foreign.jsonl"),
        meta("foreign", &f.0.join("other")),
    )
    .unwrap();
    fs::write(f.0.join("session_index.jsonl"),format!("{}\n{}\n{}\n",serde_json::json!({"id":"session-01","thread_name":"old title","updated_at":"2026-01-01"}),serde_json::json!({"id":"session-01","thread_name":"最新会话标题","updated_at":"2026-11-11"}),serde_json::json!({"id":"foreign","thread_name":"must not leak","updated_at":"2026-11-11"}))).unwrap();
    let h = history::load(&root, &f.0, Agent::Codex, &Arc::new(AtomicU64::new(1)), 1);
    assert_eq!(h.sessions.len(), 40);
    assert!(!h.limited);
    assert_eq!(
        h.sessions
            .iter()
            .find(|c| c.id == "session-01")
            .unwrap()
            .prompt,
        "最新会话标题"
    );
    assert!(h.sessions.iter().all(|c| c.id != "foreign"));
    assert!(
        h.sessions.iter().skip(1).all(|c| c.turns.is_empty()),
        "only first body is read eagerly"
    );
    assert!(history::matches(
        h.sessions.iter().find(|c| c.id == "session-01").unwrap(),
        "会话"
    ));
}
#[test]
fn hotspot_parser_counts_distinct_commits_and_preserves_literal_paths() {
    let data=b"\0DT:200\0\n4\t2\tsrc/auth.rs\0\n1\t0\tsrc/config.rs\0\0DT:100\0\n2\t1\tsrc/auth.rs\0\n1\t1\tdocs/name\twith\nnewline.md\0";
    let (files, dirs, limited) = stats::parse_hotspots(data);
    assert!(!limited);
    let auth = files.iter().find(|h| h.path == "src/auth.rs").unwrap();
    assert_eq!(
        (auth.commits, auth.added, auth.removed, auth.latest),
        (2, 6, 3, 200)
    );
    let src = dirs.iter().find(|h| h.path == "src").unwrap();
    assert_eq!((src.commits, src.added, src.removed), (2, 7, 3));
    assert!(
        files
            .iter()
            .any(|h| h.path == "docs/name\twith\nnewline.md")
    );
}
fn git(root: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn real_git_pulse_separates_artifacts_and_counts_activity_hotspots_and_categories() {
    let f = Fixture::new();
    git(&f.0, &["init", "-q"]);
    git(&f.0, &["config", "user.email", "fixture@example.test"]);
    git(&f.0, &["config", "user.name", "Fixture"]);
    for d in ["src", "tests", "docs", "target", "vendor", "tools/target"] {
        fs::create_dir_all(f.0.join(d)).unwrap();
    }
    fs::write(f.0.join(".gitignore"), "target/\nvendor/\n").unwrap();
    fs::write(f.0.join("src/auth.rs"), "fn auth() {}\n").unwrap();
    fs::write(
        f.0.join("tests/auth_test.rs"),
        "#[test] fn auth_test() {}\n",
    )
    .unwrap();
    fs::write(f.0.join("docs/guide.md"), "# Guide\n").unwrap();
    fs::write(f.0.join("target/output.bin"), vec![0; 3000]).unwrap();
    fs::write(f.0.join("vendor/dependency.txt"), vec![0; 2000]).unwrap();
    fs::write(f.0.join("tools/target/nested.bin"), vec![0; 500]).unwrap();
    fs::write(f.0.join("tools/script.ps1"), "Write-Output ok").unwrap();
    git(&f.0, &["add", "."]);
    git(&f.0, &["commit", "-qm", "first"]);
    fs::write(
        f.0.join("src/auth.rs"),
        "fn auth() { println!(\"changed\"); }\n",
    )
    .unwrap();
    git(&f.0, &["add", "."]);
    git(&f.0, &["commit", "-qm", "second"]);
    let s = stats::collect(&f.0, &Arc::new(AtomicU64::new(1)), 1);
    assert!(!s.partial);
    assert_eq!(s.artifact_bytes, 3500);
    assert_eq!(s.dependency_bytes, 2000);
    assert_eq!(s.total_commits, Some(2));
    assert_eq!(s.days.iter().sum::<u32>(), 2);
    assert!(!s.directories.iter().any(|(n, _)| n == "target"));
    assert!(s.artifacts.iter().any(|(n, b)| n == "target" && *b == 3000));
    assert!(s.artifacts.iter().any(|(n, b)| n == "tools" && *b == 500));
    assert!(s.directories.iter().any(|(n, b)| n == "tools" && *b == 15));
    assert_eq!(s.categories.values().sum::<u64>(), s.source_bytes);
    assert!(s.categories["tests"] > 0 && s.categories["docs"] > 0);
    let auth = s.hotspots.iter().find(|h| h.path == "src/auth.rs").unwrap();
    assert_eq!(auth.commits, 2);
    assert!(auth.latest > 0);
    assert!(s.hotspots_error.is_none());
}

#[test]
fn fixed_entry_limit_does_not_cause_repeated_heavy_scans() {
    let mut s = stats::Stats::default();
    assert_eq!(stats::refresh_secs(&s), 600);
    s.disk_issues.push(stats::ScanIssue::EntryLimit);
    assert_eq!(stats::refresh_secs(&s), 600);
    s.source_issues.push(stats::ScanIssue::TimeLimit);
    assert_eq!(stats::refresh_secs(&s), 30);
    s.source_issues = vec![stats::ScanIssue::ReadError];
    assert_eq!(stats::refresh_secs(&s), 30);
}
