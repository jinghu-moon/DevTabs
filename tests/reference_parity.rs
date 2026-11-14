use clap::Parser;
use devtabs::{
    config::{Config, Project},
    launch::{Agent, Options},
    overview::{
        model::{self, LaunchSpec},
        stats,
        worker::Edit,
    },
    project_profile::{self, Profile},
    recovery::{Journal, restore_options},
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicU64},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-parity-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn write(&self, name: &str, data: &str) {
        let p = self.0.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn initialized_empty_repository_reports_zero_commits_and_detects_first_commit_after_refresh() {
    let f = Fixture::new();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(&f.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--quiet", "--template="]);
    f.write("src/main.rs", "fn main() {}\n");
    let flag = Arc::new(AtomicU64::new(1));
    let empty = stats::collect(&f.0, &flag, 1);
    assert_eq!(empty.weeks, vec![0; 12]);
    assert!(empty.commits.is_empty() && empty.numstat.is_empty() && empty.git_error.is_none());
    git(&["add", "src/main.rs"]);
    git(&[
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "first",
    ]);
    git(&["pack-refs", "--all"]);
    let populated = stats::collect(&f.0, &flag, 1);
    assert_eq!(populated.weeks.iter().sum::<u32>(), 1);
    assert_eq!(populated.commits.len(), 1);
    assert!(populated.git_error.is_none());
}
#[test]
fn full_statistics_complete_cached_source_without_rescanning_and_force_refreshes_both() {
    let project = Fixture::new();
    let home = Fixture::new();
    project.write("src/main.rs", "fn main() {}\n");
    project.write("target/build.bin", "build");
    let flag = Arc::new(AtomicU64::new(1));
    let source = stats::source_at(&project.0, &home.0, &flag, 1);
    project.write("src/main.rs", "fn main() { println!(\"new content\"); }\n");
    let full = stats::cached_at(&project.0, &home.0, &flag, 1, false);
    assert!(full.disk_ready && full.source_ready);
    assert_eq!(
        full.source_bytes, source.source_bytes,
        "source retains its valid TTL/version"
    );
    assert!(full.bytes > full.source_bytes);
    assert_eq!(full.files, 2);
    let refreshed = stats::cached_at(&project.0, &home.0, &flag, 1, true);
    assert!(refreshed.source_bytes > full.source_bytes);
    assert_eq!(refreshed.bytes, full.bytes);
    let fresh = stats::collect(&project.0, &flag, 1);
    assert_eq!(
        serde_json::to_value(refreshed).unwrap(),
        serde_json::to_value(fresh).unwrap()
    );
}
#[test]
fn native_disk_scan_counts_root_deep_and_zero_length_files_without_double_counting() {
    let f = Fixture::new();
    f.write("root.bin", "root");
    f.write("src/deep/file.rs", "fn main() {}\n");
    f.write("src/empty.rs", "");
    f.write("target/deep/build.bin", "1234567890");
    let flag = Arc::new(AtomicU64::new(1));
    let s = stats::collect(&f.0, &flag, 1);
    assert_eq!((s.files, s.bytes), (4, 27));
    assert_eq!(s.directories, vec![("src".into(), 13), (".".into(), 4)]);
    assert_eq!(s.artifacts, vec![("target".into(), 10)]);
    assert_eq!(s.artifact_bytes, 10);
    assert_eq!((s.source_files, s.source_bytes), (2, 13));
    assert!(!s.partial);
}
#[test]
fn combined_git_summary_preserves_old_commits_and_recent_week_counts_in_subdirectories() {
    let f = Fixture::new();
    let git = |args: &[&str], timestamp: u64| {
        let mut c = std::process::Command::new("git");
        c.current_dir(&f.0)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        if timestamp > 0 {
            c.env("GIT_AUTHOR_DATE", format!("{timestamp} +0000"))
                .env("GIT_COMMITTER_DATE", format!("{timestamp} +0000"));
        }
        let output = c.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--quiet", "--template="], 0);
    let now = devtabs::config::now();
    for i in 0..7 {
        f.write("src/main.rs", &format!("// version {i}\n"));
        git(&["add", "src/main.rs"], 0);
        git(
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                &format!("commit-{i}"),
            ],
            if i == 0 { now - 100 * 86400 } else { now - 60 },
        );
        if i >= 5 {
            let s = stats::collect(&f.0.join("src"), &Arc::new(AtomicU64::new(1)), 1);
            assert_eq!(s.weeks.iter().sum::<u32>(), i);
            assert_eq!(s.commits.len(), 6);
            assert_eq!(s.commits.iter().any(|c| c.contains("commit-0")), i == 5);
            assert!(s.git_error.is_none());
        }
    }
}
#[test]
fn source_cache_does_not_wait_for_disk_and_cancellation_keeps_saved_result() {
    use std::sync::atomic::Ordering;
    let project = Fixture::new();
    let home = Fixture::new();
    project.write("src/main.rs", "fn main() {}\n");
    project.write("target/cache.bin", "build output");
    let flag = Arc::new(AtomicU64::new(1));
    let first = stats::source_at(&project.0, &home.0, &flag, 1);
    assert!(first.source_ready);
    assert!(!first.disk_ready);
    assert_eq!(first.bytes, 0);
    assert!(first.source_files >= 1);
    let path = fs::read_dir(home.0.join("stats"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let bytes = fs::read(&path).unwrap();
    let second = stats::source_at(&project.0, &home.0, &flag, 1);
    assert_eq!(second.source_bytes, first.source_bytes);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    flag.store(2, Ordering::Relaxed);
    let canceled = stats::source_at(&project.0, &home.0, &flag, 1);
    assert!(!canceled.source_ready);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn usage_index_accepts_known_agent_records_and_ignores_directory_timestamps() {
    let f = Fixture::new();
    let project = f.0.join("项目");
    fs::create_dir(&project).unwrap();
    let record = serde_json::json!({"type":"session_meta","payload":{"id":"known-session","cwd":project,"timestamp":"2026-11-11T00:00:00Z"}});
    f.write("history/one.jsonl", &format!("{record}\n"));
    f.write(
        "history/tool.jsonl",
        &format!(
            "{}\n",
            serde_json::json!({"type":"tool_result","cwd":f.0.join("fake")})
        ),
    );
    f.write("history/invalid.jsonl", &format!("{}\n", serde_json::json!({"type":"session_meta","payload":{"id":"../bad","cwd":f.0.join("fake")}})));
    let flag = Arc::new(AtomicU64::new(1));
    let index =
        devtabs::overview::history::usage_index(&f.0.join("history"), Agent::Codex, &flag, 1);
    assert_eq!(index.len(), 1);
    assert!(index.values().all(|time| *time > 0));
    assert!(index.keys().all(|key| key.contains("项目")));
    flag.store(2, std::sync::atomic::Ordering::Relaxed);
    assert!(
        devtabs::overview::history::usage_index(&f.0.join("history"), Agent::Codex, &flag, 1)
            .is_empty()
    );
}
#[test]
fn terminal_composition_preserves_extra_configuration_and_persists_through_all_launch_formats() {
    use devtabs::launch::Composition;
    let f = Fixture::new();
    let project = Project {
        agent: Some(Agent::Shell),
        composition: Some(Composition::AgentOnly),
        sessions: vec![devtabs::config::SessionConfig {
            name: "Docs".into(),
            directory: Some("docs".into()),
            command: Some("cargo doc".into()),
        }],
        ..Default::default()
    };
    let config = central(&f.0, &project);
    let prefs = project_profile::effective(&config, &f.0).unwrap();
    assert_eq!(prefs.composition, Some(Composition::AgentOnly));
    assert_eq!(prefs.sessions.len(), 1);
    let spec = LaunchSpec::from_project(&f.0, Some(Agent::Shell), prefs);
    let mut options = spec.options();
    assert_eq!(options.terminal_count(), 1);
    assert_eq!(options.extra_sessions.len(), 1);
    options.composition = Composition::AgentDebug;
    assert_eq!(options.terminal_count(), 2);
    assert_eq!(options.extra_sessions.len(), 1);
    options.composition = Composition::AgentOnly;
    let restored: devtabs::launch::Options =
        serde_json::from_slice(&serde_json::to_vec(&options).unwrap()).unwrap();
    assert_eq!(restored.terminal_count(), 1);
    options.composition = Composition::Custom;
    assert_eq!(options.terminal_count(), 3);
    assert_eq!(options.source(2).as_deref(), Some("cargo doc"));
}
#[test]
fn journal_roundtrip_preserves_instances_order_and_replaces_only_restored_record() {
    let f = Fixture::new();
    let mut j = Journal::default();
    let mut a = Options::parse_from([
        "devtabs",
        "--agent",
        "shell",
        "--new-instance",
        "--title",
        "中文标题",
        "--no-profile",
    ]);
    a.project = f.0.clone();
    a.workspace = Some("ephemeral".into());
    a.view = Some("main".into());
    a.headless = true;
    j.register("one", &a);
    j.register("two", &a);
    j.last = vec![a.clone(), a.clone()];
    j.save_at(&f.0).unwrap();
    let mut j = Journal::load_at(&f.0).unwrap();
    assert_eq!(j.entries.len(), 2);
    assert_eq!(j.last.len(), 2);
    let pending = j.pending(&["one".to_string()].into());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "two");
    let restored = restore_options(&pending[0]);
    assert!(restored.workspace.is_none());
    assert!(restored.view.is_none());
    assert!(!restored.headless);
    assert!(restored.new_instance);
    assert_eq!(restored.title.as_deref(), Some("中文标题"));
    assert_eq!(restored.recovery_id.as_deref(), Some("two"));
    j.register("replacement", &restored);
    assert!(j.entries.contains_key("one"));
    assert!(!j.entries.contains_key("two"));
    assert!(j.entries.contains_key("replacement"));
}
#[test]
fn corrupt_journal_is_reported_and_never_silently_replaced() {
    let f = Fixture::new();
    f.write("launches.json", "broken");
    assert!(Journal::load_at(&f.0).is_err());
    assert_eq!(
        fs::read_to_string(f.0.join("launches.json")).unwrap(),
        "broken"
    );
}

#[test]
fn journal_without_task_ids_remains_readable_without_inventing_task_ownership() {
    let f = Fixture::new();
    let mut journal = devtabs::recovery::Journal::default();
    let spec = LaunchSpec::from_project(&f.0, Some(Agent::Shell), Project::default());
    journal.register("old-workspace", &spec.options());
    let mut value = serde_json::to_value(&journal).unwrap();
    value["entries"]["old-workspace"]
        .as_object_mut()
        .unwrap()
        .remove("task_ids");
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(f.0.join("launches.json"), &bytes).unwrap();
    let loaded = devtabs::recovery::Journal::load_at(&f.0).unwrap();
    let record = &loaded.entries["old-workspace"];
    assert!(record.task_ids.is_empty());
    assert_eq!(record.options.project, f.0);
    assert_eq!(record.opened, journal.entries["old-workspace"].opened);
    assert_eq!(
        fs::read(f.0.join("launches.json")).unwrap(),
        bytes,
        "reading must not rewrite old records"
    );
    value["entries"]["old-workspace"]["task_ids"] = serde_json::json!("invalid");
    assert!(serde_json::from_value::<devtabs::recovery::Journal>(value).is_err());
}
#[test]
fn central_project_launch_keeps_private_metadata_and_ignores_project_files() {
    let f = Fixture::new();
    let mut c = Config::default();
    let key = devtabs::config::key(&f.0);
    c.projects.insert(
        key.clone(),
        Project {
            favorite: true,
            note: "私有便签".into(),
            agent: Some(Agent::Codex),
            ..Default::default()
        },
    );
    let p = Project {
        agent: Some(Agent::Claude),
        title: Some("项目标题".into()),
        color: Some("#12AbEF".into()),
        agent_args: vec!["--model".into(), "a'; $(whoami); 中文".into()],
        resume_latest: Some(false),
        ..Default::default()
    };
    let mut central = central(&f.0, &p);
    central.projects.get_mut(&key).unwrap().favorite = true;
    central.projects.get_mut(&key).unwrap().note = "私有便签".into();
    let text = &central.source.as_ref().unwrap().text;
    assert!(!text.contains("私有便签"));
    let effective = project_profile::effective(&central, &f.0).unwrap();
    assert!(effective.favorite);
    assert_eq!(effective.note, "私有便签");
    assert_eq!(effective.agent, Some(Agent::Claude));
    assert_eq!(effective.title, p.title);
    assert_eq!(effective.agent_args, p.agent_args);
    f.write(".devtabs.toml", "typo_setting = true");
    assert_eq!(
        project_profile::effective(&central, &f.0).unwrap().agent,
        Some(Agent::Claude)
    );
}
#[test]
fn presets_capture_every_launch_option_and_rename_delete_without_losing_order() {
    let mut o = Options::parse_from([
        "devtabs",
        "--agent",
        "claude",
        "--fresh",
        "--new-window",
        "--no-profile",
        "--history",
        "42",
        "--agent-arg=--model",
        "--agent-arg=中文",
    ]);
    o.project = "a".into();
    o.debug_command = Some("Write-Output '中文'".into());
    o.new_instance = true;
    let spec = LaunchSpec::from_options(&o);
    let roundtrip: LaunchSpec = toml::from_str(&toml::to_string(&spec).unwrap()).unwrap();
    let result = roundtrip.options();
    assert!(result.fresh && result.new_instance && result.new_window && result.no_profile);
    assert_eq!(result.history, 42);
    assert_eq!(result.agent_args, o.agent_args);
    assert_eq!(result.source(0), o.source(0));
    let mut c = Config::default();
    Edit::Preset("old".into(), vec![spec])
        .apply(&mut c)
        .unwrap();
    Edit::RenamePreset("old".into(), "新".into())
        .apply(&mut c)
        .unwrap();
    assert!(!c.presets.contains_key("old"));
    assert!(c.presets["新"][0].options().fresh);
    Edit::Preset("occupied".into(), c.presets["新"].clone())
        .apply(&mut c)
        .unwrap();
    assert!(
        Edit::RenamePreset("新".into(), "occupied".into())
            .apply(&mut c)
            .is_err()
    );
    assert!(c.presets.contains_key("新"));
    Edit::DeletePreset("新".into()).apply(&mut c).unwrap();
    assert!(!c.presets.contains_key("新"));
}
#[test]
fn new_project_validates_names_initializes_git_and_never_touches_existing_directory() {
    let f = Fixture::new();
    for invalid in ["", "..", "../escape", "a/b", "CON", "LPT1", "bad.", "a\nb"] {
        assert!(
            project_profile::create_project(&f.0, invalid).is_err(),
            "{invalid}"
        );
    }
    let p = project_profile::create_project(&f.0, "新项目 with spaces").unwrap();
    assert!(p.join(".git").is_dir());
    fs::write(p.join("keep"), "preserved").unwrap();
    assert!(project_profile::create_project(&f.0, "新项目 with spaces").is_err());
    assert_eq!(fs::read_to_string(p.join("keep")).unwrap(), "preserved");
}
#[test]
fn pending_projects_group_and_filter_without_changing_saved_preferences() {
    let p = PathBuf::from("pending");
    let mut c = Config::default();
    c.overview.pending.insert(devtabs::config::key(&p));
    c.overview.filter = model::Filter::Pending;
    let rows = model::rows(
        std::slice::from_ref(&p),
        &c,
        "",
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(rows, vec![model::Row::Group(4), model::Row::Project(p)]);
    assert!(!toml::to_string(&c).unwrap().contains("pending ="));
}
#[test]
fn statistics_cache_persists_valid_results_force_refresh_and_cancel_keeps_previous_cache() {
    let project = Fixture::new();
    let home = Fixture::new();
    project.write("src/main.rs", "fn main() {}\n");
    let g = Arc::new(AtomicU64::new(1));
    let first = stats::cached_at(&project.0, &home.0, &g, 1, false);
    assert_eq!(first.source_bytes, 13);
    project.write("src/main.rs", "fn main() { /* changed */ }\n");
    let warm = stats::cached_at(&project.0, &home.0, &g, 1, false);
    assert_eq!(warm.source_bytes, first.source_bytes);
    let refreshed = stats::cached_at(&project.0, &home.0, &g, 1, true);
    assert!(refreshed.source_bytes > first.source_bytes);
    assert_eq!(
        stats::cached_at(&project.0, &home.0, &g, 2, true).source_bytes,
        0
    );
    assert_eq!(
        stats::cached_at(&project.0, &home.0, &g, 1, false).source_bytes,
        refreshed.source_bytes
    );
    let path = fs::read_dir(home.0.join("stats"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut cache: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    cache["source_at"] = serde_json::json!(0);
    cache["disk_at"] = serde_json::json!(0);
    fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
    project.write("src/main.rs", "short\n");
    let expired = stats::cached_at(&project.0, &home.0, &g, 1, false);
    assert_eq!(expired.source_bytes, 6);
    assert_eq!(expired.bytes, 6);
}
#[test]
fn numstat_keeps_unicode_tabs_and_binary_paths_separate() {
    let items = stats::parse_numstat(b"2\t3\tsrc/a\tb.rs\0-\t-\timage.png\x0010\t1\tother.rs\0");
    assert_eq!(items[0], ("other.rs".into(), 10, 1));
    assert!(items.contains(&("src/a\tb.rs".into(), 2, 3)));
    assert!(items.contains(&("image.png".into(), 0, 0)));
}

#[test]
fn oversized_profiles_and_journals_are_rejected_without_overwriting() {
    let f = Fixture::new();
    fs::write(f.0.join(".devtabs.toml"), vec![b' '; 128 * 1024 + 1]).unwrap();
    assert!(
        Profile::load(&f.0)
            .unwrap_err()
            .to_string()
            .contains("128 KiB")
    );
    fs::write(f.0.join("launches.json"), vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(
        Journal::load_at(&f.0)
            .unwrap_err()
            .to_string()
            .contains("4 MiB")
    );
    assert_eq!(
        fs::metadata(f.0.join("launches.json")).unwrap().len(),
        4 * 1024 * 1024 + 1
    );
}
#[test]
fn git_weekly_trend_and_numstat_include_staged_and_unstaged_changes() {
    let f = Fixture::new();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .current_dir(&f.0)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init"]);
    f.write("a.rs", "first\n");
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "fixture",
    ]);
    f.write("a.rs", "first\nstaged\n");
    git(&["add", "."]);
    f.write("a.rs", "first\nstaged\nunstaged\n");
    let flag = Arc::new(AtomicU64::new(1));
    let result = stats::collect(&f.0, &flag, 1);
    assert_eq!(result.weeks.len(), 12);
    assert_eq!(result.weeks.iter().sum::<u32>(), 1);
    assert_eq!(result.weeks[11], 1);
    assert_eq!(result.activity, 1);
    assert!(result.numstat.contains(&("a.rs".into(), 2, 0)));
}
#[test]
#[ignore = "release statistics cache benchmark; writes isolated fixture files"]
fn statistics_cache_benchmark() {
    use std::time::Instant;
    let project = Fixture::new();
    let home = Fixture::new();
    for i in 0..2000 {
        project.write(&format!("src/part{}/file{i}.rs", i / 100), "fn main() {}\n");
    }
    let flag = Arc::new(AtomicU64::new(1));
    let start = Instant::now();
    let cold = stats::cached_at(&project.0, &home.0, &flag, 1, true);
    let cold_us = start.elapsed().as_micros();
    let mut uncached_us = vec![];
    for _ in 0..5 {
        let start = Instant::now();
        let uncached = stats::collect(&project.0, &flag, 1);
        uncached_us.push(start.elapsed().as_micros());
        assert_eq!(
            serde_json::to_value(&cold).unwrap(),
            serde_json::to_value(&uncached).unwrap()
        );
    }
    uncached_us.sort();
    let mut warm_us = vec![];
    for _ in 0..7 {
        let start = Instant::now();
        let warm = stats::cached_at(&project.0, &home.0, &flag, 1, false);
        warm_us.push(start.elapsed().as_micros());
        assert_eq!(
            serde_json::to_value(&cold).unwrap(),
            serde_json::to_value(&warm).unwrap()
        );
    }
    warm_us.sort();
    assert!(!cold.partial);
    assert_eq!(cold.source_files, 2000);
    let result = serde_json::json!({"files":2000,"cold_us":cold_us,"warm_median_us":warm_us[3],"warm_samples_us":warm_us,"identical_results":true,"uncached_median_us":uncached_us[2],"uncached_samples_us":uncached_us,"speedup":uncached_us[2] as f64 / warm_us[3].max(1) as f64});
    fs::create_dir_all("artifacts").unwrap();
    fs::write(
        "artifacts/parity-cache-benchmark.json",
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
    assert!(
        warm_us[3] < uncached_us[2],
        "Cache must outperform an uncached scan after syntax initialization"
    );
}

fn central(path: &std::path::Path, p: &Project) -> Config {
    use devtabs::settings::{model as m, projection, store::Snapshot};
    let mut prefs = m::Preferences::default();
    prefs.project.push(m::Project {
        path: path.into(),
        favorite: p.favorite,
        sidebar: Default::default(),
        checks: Default::default(),
        launch: m::LaunchOverrides {
            agent: p.agent,
            layout: p.layout,
            composition: p.composition,
            title: p.title.clone(),
            color: p.color.clone(),
            agent_args: Some(p.agent_args.clone()),
            resume_latest: p.resume_latest,
            debug_command: p.debug_command.clone(),
            sessions: Some(p.sessions.clone()),
            ..Default::default()
        },
    });
    let source = Snapshot::parse(toml::to_string_pretty(&prefs).unwrap()).unwrap();
    let mut config = projection::from_preferences(&source.effective);
    config.source = Some(source);
    config
}
