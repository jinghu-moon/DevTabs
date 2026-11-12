use devtabs::{launch::Agent, references::Cursor};
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf};
struct Fixture {
    root: PathBuf,
    log: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("devtabs-refs-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let log = root.join("session.jsonl");
        let f = Self { root, log };
        f.append(json!({"type":"session_meta","payload":{"id":"session-1","cwd":f.root}}));
        f
    }
    fn append(&self, value: Value) {
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&self.log)
                .unwrap(),
            "{value}"
        )
        .unwrap();
    }
    fn start(&self, turn: &str) {
        self.append(json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}}));
        self.append(json!({"type":"event_msg","payload":{"type":"user_message","message":"work"}}));
    }
    fn answer(&self, text: &str) {
        self.append(json!({"type":"event_msg","payload":{"type":"agent_message","message":text}}));
    }
    fn read(&self, c: &mut Cursor, turn: &str) -> Vec<devtabs::references::Reference> {
        c.read(&self.log, &self.root, Agent::Codex, "session-1", Some(turn))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn explicit_links_only_deduplicated_scoped_to_requested_turn_and_task() {
    let f = Fixture::new();
    f.start("old");
    f.answer("[old](old.rs)");
    f.start("t1");
    f.append(json!({"type":"response_item","payload":{"type":"function_call_output","output":"[tool](tool.rs)"}}));
    f.answer("[a](src/main.rs:12:4) [again](src/main.rs#L12) [web](https://example.com) `fake.rs` [no](javascript:alert)");
    f.start("t2");
    f.answer("[next](next.rs)");
    let mut c = Cursor::default();
    let refs = f.read(&mut c, "t1");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, f.root.join("src/main.rs"));
    assert_eq!(refs[0].line, Some(12));
    assert!(
        c.read(
            &f.log,
            &f.root.join("other-task"),
            Agent::Codex,
            "session-1",
            Some("t1")
        )
        .is_err()
    );
}
#[test]
fn incremental_reads_wait_for_complete_utf8_and_do_not_reread_history() {
    let f = Fixture::new();
    for _ in 0..256 {
        f.append(json!({"type":"tool","text":"x".repeat(32768)}));
    }
    f.start("t1");
    f.answer("[first](first.rs)");
    let mut c = Cursor::default();
    assert_eq!(f.read(&mut c, "t1").len(), 1);
    assert!(
        c.bytes < 600_000,
        "bootstrap read {} bytes of an 8 MiB log",
        c.bytes
    );
    let before = c.bytes;
    let text = format!(
        "{}\n",
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"[文件](文件.rs)"}})
    );
    let cut = text.find('文').unwrap() + 1;
    let mut out = fs::OpenOptions::new().append(true).open(&f.log).unwrap();
    out.write_all(&text.as_bytes()[..cut]).unwrap();
    assert_eq!(f.read(&mut c, "t1").len(), 1);
    out.write_all(&text.as_bytes()[cut..]).unwrap();
    assert_eq!(f.read(&mut c, "t1").len(), 2);
    assert!(
        c.bytes - before < 4096,
        "incremental read {}",
        c.bytes - before
    );
    f.start("t2");
    f.answer("[second](second.rs)");
    let refs = f.read(&mut c, "t2");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, f.root.join("second.rs"));
}
#[test]
fn full_assistant_text_is_used_and_injected_user_or_child_content_is_ignored() {
    let f = Fixture::new();
    f.start("t1");
    f.append(
        json!({"type":"assistant","isSidechain":true,"message":{"content":"[child](child.rs)"}}),
    );
    f.append(
        json!({"type":"event_msg","payload":{"type":"user_message","message":"[user](user.rs)"}}),
    );
    f.answer(&format!("{} [tail](tail.rs)", "x".repeat(12000)));
    let refs = f.read(&mut Cursor::default(), "t1");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, f.root.join("tail.rs"));
}
#[test]
fn claude_uses_latest_user_boundary_and_skips_sidechain_and_meta() {
    let f = Fixture::new();
    fs::write(&f.log, b"").unwrap();
    let user = |text: &str| json!({"type":"user","cwd":f.root,"sessionId":"session-1","message":{"content":text}});
    f.append(user("old"));
    f.append(json!({"type":"assistant","message":{"content":"[old](old.rs)"}}));
    f.append(user("new"));
    f.append(json!({"type":"user","isMeta":true,"message":{"content":"system"}}));
    f.append(json!({"type":"assistant","message":{"content":[{"type":"text","text":"[new](new.rs#L3)"},{"type":"tool_use","text":"[tool](tool.rs)"}]}}));
    let refs = Cursor::default()
        .read(&f.log, &f.root, Agent::Claude, "session-1", None)
        .unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, f.root.join("new.rs"));
    assert_eq!(refs[0].line, Some(3));
}
#[test]
fn truncated_replaced_and_mismatched_logs_are_not_reused() {
    let f = Fixture::new();
    f.start("t1");
    f.answer("[old](old.rs)");
    let mut c = Cursor::default();
    assert_eq!(f.read(&mut c, "t1").len(), 1);
    fs::write(&f.log, b"").unwrap();
    f.append(json!({"type":"session_meta","payload":{"id":"session-1","cwd":f.root}}));
    f.start("t2");
    f.answer("[new](new.rs)");
    let refs = f.read(&mut c, "t2");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, f.root.join("new.rs"));
    assert!(
        c.read(&f.log, &f.root, Agent::Codex, "wrong-session", Some("t2"))
            .is_err()
    );
}

#[test]
fn reconnect_can_advance_to_a_completed_turn_already_read_past() {
    let f = Fixture::new();
    for turn in ["t1", "t2", "t3"] {
        f.start(turn);
        f.answer(&format!("[{turn}]({turn}.rs)"));
    }
    let mut c = Cursor::default();
    assert_eq!(f.read(&mut c, "t1")[0].path, f.root.join("t1.rs"));
    assert_eq!(f.read(&mut c, "t2")[0].path, f.root.join("t2.rs"));
    assert_eq!(f.read(&mut c, "t3")[0].path, f.root.join("t3.rs"));
    assert!(f.read(&mut c, "missing-turn").is_empty());
    assert!(c.partial);
}

#[test]
#[ignore = "Release-only incremental transcript I/O comparison; temporary JSONL fixtures only"]
fn incremental_reference_workload() {
    use std::time::Instant;
    let f = Fixture::new();
    for _ in 0..256 {
        f.append(json!({"type":"tool","text":"x".repeat(32768)}));
    }
    f.start("warmup");
    f.answer("[warm](warm.rs)");
    let mut cursor = Cursor::default();
    f.read(&mut cursor, "warmup");
    let before = cursor.bytes;
    let mut fresh_bytes = 0;
    let mut fresh_times = Vec::new();
    let mut incremental_times = Vec::new();
    for n in 0..30 {
        let turn = format!("turn-{n}");
        f.start(&turn);
        f.answer(&format!("[file](src/file{n}.rs:12)"));
        let mut fresh = Cursor::default();
        let start = Instant::now();
        let expected = f.read(&mut fresh, &turn);
        fresh_times.push(start.elapsed().as_nanos() as u64);
        fresh_bytes += fresh.bytes;
        let start = Instant::now();
        let actual = f.read(&mut cursor, &turn);
        incremental_times.push(start.elapsed().as_nanos() as u64);
        assert_eq!(actual, expected);
    }
    fresh_times.sort();
    incremental_times.sort();
    let bytes = cursor.bytes - before;
    assert!(bytes * 50 < fresh_bytes);
    println!(
        "{}",
        json!({"rounds":30,"log_bytes":fs::metadata(&f.log).unwrap().len(),"fresh_cursor":{"bytes":fresh_bytes,"p50_ns":fresh_times[15],"p95_ns":fresh_times[28]},"incremental":{"bytes":bytes,"p50_ns":incremental_times[15],"p95_ns":incremental_times[28]}})
    );
}
