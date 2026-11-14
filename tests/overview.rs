//! Overview contracts use generated fixtures; no interactive windows or personal history.
use devtabs::{
    config::{self, Config, Project, SessionConfig},
    i18n::Language,
    launch::{Agent, Layout},
    overview::{
        history,
        model::{self, Batch, Filter, LaunchSpec, LaunchState, Row, Sort},
        stats,
        worker::Edit,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicU64},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-overview-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn write(&self, name: &str, data: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, data).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn paths(rows: Vec<Row>) -> Vec<PathBuf> {
    rows.into_iter()
        .filter_map(|r| match r {
            Row::Project(p) => Some(p),
            _ => None,
        })
        .collect()
}
#[test]
fn filters_notes_groups_and_statistics_sort_use_project_identities() {
    let a = PathBuf::from("a");
    let b = PathBuf::from("b");
    let mut c = Config::default();
    c.projects.insert(
        config::key(&a),
        Project {
            agent: Some(Agent::Codex),
            note: "支持中英文".into(),
            ..Default::default()
        },
    );
    c.projects.insert(
        config::key(&b),
        Project {
            agent: Some(Agent::Claude),
            ..Default::default()
        },
    );
    let projects = [b.clone(), a.clone()];
    let active = BTreeSet::from([config::key(&b)]);
    let git = BTreeMap::from([(
        config::key(&a),
        Ok(model::GitSummary {
            changed: 3,
            ..Default::default()
        }),
    )]);
    let sizes = BTreeMap::from([(config::key(&a), 100), (config::key(&b), 200)]);
    let activity = BTreeMap::from([(config::key(&a), 5), (config::key(&b), 1)]);
    for (filter, expected) in [
        (Filter::Running, vec![b.clone()]),
        (Filter::Dirty, vec![a.clone()]),
        (Filter::Codex, vec![a.clone()]),
        (Filter::Claude, vec![b.clone()]),
    ] {
        c.overview.filter = filter;
        assert_eq!(
            paths(model::rows(
                &projects, &c, "", &active, &git, &sizes, &activity
            )),
            expected
        )
    }
    c.overview.filter = Filter::All;
    c.overview.sort = Sort::Size;
    assert_eq!(
        paths(model::rows(
            &projects, &c, "", &active, &git, &sizes, &activity
        )),
        vec![b.clone(), a.clone()]
    );
    c.overview.sort = Sort::Activity;
    assert_eq!(
        paths(model::rows(
            &projects, &c, "", &active, &git, &sizes, &activity
        )),
        vec![b.clone(), a.clone()]
    );
    c.overview.collapsed.insert(5);
    assert_eq!(
        paths(model::rows(
            &projects, &c, "", &active, &git, &sizes, &activity
        )),
        vec![b.clone()],
        "running projects remain visible when Other projects is collapsed"
    );
    assert_eq!(
        paths(model::rows(
            &projects,
            &c,
            "中英文",
            &active,
            &git,
            &sizes,
            &activity
        )),
        vec![a]
    );
}
#[test]
fn settings_edit_merges_and_preset_snapshots_all_launch_fields() {
    let mut c = Config::default();
    let a = PathBuf::from("a");
    let b = PathBuf::from("b");
    c.projects.insert(
        config::key(&a),
        Project {
            favorite: true,
            last_opened: 42,
            note: "保留便签".into(),
            ..Default::default()
        },
    );
    let settings = Project {
        agent: Some(Agent::Claude),
        layout: Some(Layout::Split),
        debug_command: Some("npm run dev; Write-Output '中文'".into()),
        sessions: vec![SessionConfig {
            name: "Docs".into(),
            command: Some("cargo doc".into()),
            directory: Some("docs".into()),
        }],
        ..Default::default()
    };
    Edit::Settings(a.clone(), settings).apply(&mut c).unwrap();
    assert!(c.projects[&config::key(&a)].favorite);
    assert_eq!(c.projects[&config::key(&a)].note, "保留便签");
    Edit::Note(a.clone(), String::new(), "下一步".into())
        .apply(&mut c)
        .unwrap();
    let items = vec![
        LaunchSpec::from_config(&b, &c),
        LaunchSpec::from_config(&a, &c),
    ];
    Edit::Preset("日常开发".into(), items)
        .apply(&mut c)
        .unwrap();
    Edit::Language(Language::ZhCn).apply(&mut c).unwrap();
    let c: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
    let specs = &c.presets["日常开发"];
    assert_eq!(specs[0].project, b);
    let options = specs[1].options();
    assert_eq!(options.project, a);
    assert_eq!(options.agent, Agent::Claude);
    assert_eq!(options.layout, Layout::Split);
    assert_eq!(options.extra_sessions[0].directory, Some("docs".into()));
    assert_eq!(options.source(2).as_deref(), Some("cargo doc"));
    assert!(options.debug_command.unwrap().contains("中文"));
}
#[test]
fn failed_settings_validation_never_mutates_current_config() {
    let mut c = Config::default();
    let path = PathBuf::from("p");
    Edit::Note(path.clone(), String::new(), "retain".into())
        .apply(&mut c)
        .unwrap();
    let before = toml::to_string(&c).unwrap();
    let p = Project {
        sessions: vec![SessionConfig {
            name: "".into(),
            command: None,
            directory: None,
        }],
        ..Default::default()
    };
    assert!(Edit::Settings(path, p).apply(&mut c).is_err());
    assert_eq!(toml::to_string(&c).unwrap(), before);
    assert!(Edit::Preset(" ".into(), vec![]).apply(&mut c).is_err());
}
#[test]
fn retry_keeps_workspace_after_frontend_failure_and_rejects_old_progress() {
    let c = Config::default();
    let mut options = LaunchSpec::from_config(Path::new("."), &c).options();
    options.new_instance = true;
    let mut b = Batch::default();
    b.start(vec![options.clone(), options]).unwrap();
    let old = b.generation;
    b.items[0].options.workspace = Some("created-host-workspace".into());
    b.update(old, 0, LaunchState::Failed("frontend failed".into()));
    b.update(old, 1, LaunchState::Opened("second".into()));
    let retry = b.retry();
    assert_eq!(retry.len(), 1);
    assert_eq!(
        retry[0].1.workspace.as_deref(),
        Some("created-host-workspace")
    );
    b.update(old, 0, LaunchState::Opened("stale".into()));
    assert_eq!(b.items[0].state, LaunchState::Queued);
}
#[test]
fn git_summary_matches_real_rename_untracked_and_branch_output() {
    let f = Fixture::new();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(&f.0)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    f.write("old name.rs", "fn main() {}\n");
    git(&["add", "."]);
    git(&["commit", "-m", "fixture"]);
    git(&["mv", "old name.rs", "新 name.rs"]);
    f.write("untracked.txt", "x");
    let summary = stats::git(&f.0).unwrap();
    assert_eq!(summary.branch, "main");
    assert_eq!(summary.changed, 2);
    assert_eq!(summary.conflicts, 0);
    assert!(summary.files.contains(&"新 name.rs".into()));
    assert!(summary.files.contains(&"untracked.txt".into()));
}
#[test]
fn stats_separates_logical_bytes_from_source_and_honors_cancel() {
    let f = Fixture::new();
    f.write("src/main.rs", "fn main() {}\n");
    f.write("assets/blob.bin", [1; 400]);
    f.write("target/generated.rs", [b'x'; 1000]);
    let flag = Arc::new(AtomicU64::new(1));
    let stats = stats::collect(&f.0, &flag, 1);
    assert_eq!(stats.files, 3);
    assert_eq!(stats.bytes, 1413);
    assert_eq!(stats.source_files, 1);
    assert_eq!(stats.source_bytes, 13);
    assert!(
        stats
            .languages
            .iter()
            .any(|(name, n)| name == "Rust" && *n == 13)
    );
    assert!(!stats.partial);
    let cancelled = stats::collect(&f.0, &flag, 2);
    assert_eq!(cancelled.files, 0);
}
#[test]
fn history_filters_cwd_bounds_tail_ignores_tools_and_reads_both_agents() {
    let f = Fixture::new();
    let project = f.0.join("project");
    fs::create_dir(&project).unwrap();
    let cwd = serde_json::to_string(&project).unwrap();
    let codex = format!(
        r#"{{"type":"session_meta","payload":{{"id":"codex-fixture","cwd":{cwd},"timestamp":"2026-11-11T10:00:00Z"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<environment_context>ignore</environment_context>"}},{{"type":"input_text","text":"继续实现"}}]}}}}
{{"type":"event_msg","payload":{{"type":"user_message","message":"继续实现"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","phase":"commentary","content":[{{"type":"output_text","text":"skip progress"}}]}}}}
{{"type":"event_msg","payload":{{"type":"agent_message","message":"完成"}}}}
"#
    );
    let p = f.write("sessions/codex.jsonl", codex);
    let c = history::read(&p, &project, Agent::Codex).unwrap().unwrap();
    assert_eq!(c.turns.len(), 2);
    assert_eq!(c.prompt, "继续实现");
    assert_eq!(c.turns[1].text, "完成");
    assert!(history::read(&p, &f.0, Agent::Codex).unwrap().is_none());
    let claude = format!(
        r#"{{"type":"user","sessionId":"claude-fixture","cwd":{cwd},"timestamp":"2026-11-11T11:00:00Z","message":{{"content":"问题"}}}}
{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"bash","input":{{"command":"private"}}}},{{"type":"text","text":"回答"}}]}}}}
"#
    );
    f.write("sessions/claude.jsonl", claude);
    let flag = Arc::new(AtomicU64::new(3));
    let h = history::load(&f.0.join("sessions"), &project, Agent::Codex, &flag, 3);
    assert!(!h.sessions.is_empty());
    assert_eq!(h.sessions.len(), 1);
    assert_eq!(h.sessions[0].id, "codex-fixture");
    let claude = history::load(&f.0.join("sessions"), &project, Agent::Claude, &flag, 3);
    assert_eq!(claude.sessions.len(), 1);
    assert_eq!(claude.sessions[0].id, "claude-fixture");
    assert!(
        h.sessions
            .iter()
            .flat_map(|s| &s.turns)
            .all(|t| !t.text.contains("private"))
    );
    let cancelled = history::load(&f.0, &project, Agent::Codex, &flag, 4);
    assert!(cancelled.sessions.is_empty());
}
#[test]
fn cli_persists_language_in_isolated_home() {
    let f = Fixture::new();
    let exe = env!("CARGO_BIN_EXE_devtabs");
    for (arg, expected) in [
        ("zh-cn", Language::ZhCn),
        ("en-us", Language::EnUs),
        ("auto", Language::Auto),
    ] {
        let o = std::process::Command::new(exe)
            .args(["config", "language", arg])
            .env("DEVTABS_HOME", &f.0)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let c: devtabs::settings::model::Preferences =
            toml::from_str(&fs::read_to_string(f.0.join("config.toml")).unwrap()).unwrap();
        assert_eq!(c.general.language, Some(expected));
    }
}
