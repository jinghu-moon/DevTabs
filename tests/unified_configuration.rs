use clap::{CommandFactory, FromArgMatches};
use devtabs::settings::{
    data::{UiChange, UiState, UserData},
    model as m,
    store::SettingsStore,
};
use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-unified-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn isolated(name: &str, run: impl FnOnce()) {
    if std::env::var_os("DEVTABS_UNIFIED_TEST").is_some() {
        run();
        return;
    }
    let home = Fixture::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("DEVTABS_UNIFIED_TEST", "1")
        .env("DEVTABS_HOME", &home.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn seed(project: &std::path::Path) -> m::Preferences {
    let mut p = m::Preferences::default();
    p.launch.agent = Some(devtabs::launch::Agent::Claude);
    p.launch.layout = Some(devtabs::launch::Layout::Split);
    p.launch.new_window = Some(true);
    p.launch.agent_args = Some(vec!["inherited".into()]);
    p.launch.debug_command = Some("inherited".into());
    p.terminal.history_lines = Some(42);
    p.project.push(m::Project {
        path: project.into(),
        favorite: false,
        sidebar: Default::default(),
        checks: Default::default(),
        launch: m::LaunchOverrides {
            agent_args: Some(vec![]),
            debug_command: Some(String::new()),
            resume_latest: Some(false),
            ..Default::default()
        },
    });
    p
}
#[test]
fn cli_provenance_content_and_snapshots_use_the_same_central_preferences() {
    isolated(
        "cli_provenance_content_and_snapshots_use_the_same_central_preferences",
        || {
            let project = Fixture::new();
            let store = SettingsStore::current();
            let base = store.read().unwrap();
            let p = seed(&project.0);
            let draft = toml::Value::try_from(&p).unwrap();
            store.save(&base, &draft).unwrap();
            fs::write(
                project.0.join(".devtabs.toml"),
                "agent='shell'\ninvalid=true",
            )
            .unwrap();
            let args = vec![
                "devtabs".to_owned(),
                "--project".into(),
                project.0.to_string_lossy().into_owned(),
                "--layout=float".into(),
                "--new-window=false".into(),
                "--agent-arg=--agent=shell".into(),
                "--agent-arg=".into(),
            ];
            let matches = devtabs::launch::Options::command()
                .try_get_matches_from(args)
                .unwrap();
            let options = devtabs::cli::resolve(
                devtabs::launch::Options::from_arg_matches(&matches).unwrap(),
                &matches,
            )
            .unwrap();
            assert_eq!(options.agent, devtabs::launch::Agent::Claude);
            assert_eq!(options.layout, devtabs::launch::Layout::Float);
            assert!(!options.new_window);
            assert!(options.fresh && !options.continue_latest);
            assert_eq!(options.history, 42);
            assert_eq!(options.agent_args, ["--agent=shell", ""]);
            assert_eq!(options.debug_command, None);
            let bytes = fs::read(store.path()).unwrap();
            UserData::note(&devtabs::config::home(), &project.0, "", "中文便签").unwrap();
            UiState::update(
                &devtabs::config::home(),
                UiChange::Opened(project.0.clone(), 200),
            )
            .unwrap();
            let config = devtabs::config::Config::load().unwrap();
            assert_eq!(
                config.projects[&devtabs::config::key(&project.0)].note,
                "中文便签"
            );
            assert_eq!(
                config.projects[&devtabs::config::key(&project.0)].last_opened,
                200
            );
            assert_eq!(fs::read(store.path()).unwrap(), bytes);
            let spec =
                devtabs::overview::model::LaunchSpec::from_config(&project.0, &config).options();
            assert_eq!(spec.history, 42);
            assert!(spec.new_window);
            assert!(spec.fresh);
            assert!(spec.agent_args.is_empty());
        },
    );
}
#[test]
fn bad_check_is_isolated_while_prepare_and_launch_remain_usable() {
    let f = Fixture::new();
    let project = f.0.join("project.v2");
    let mut p = seed(&project);
    p.project[0].checks.prepare = vec![devtabs::checks::Spec {
        argv: vec!["pwsh".into()],
        cwd: ".".into(),
        env: Default::default(),
        timeout_ms: 1,
    }];
    let mut tree = toml::Value::try_from(p).unwrap();
    tree["project"].as_array_mut().unwrap()[0]["checks"]
        .as_table_mut()
        .unwrap()
        .insert("check".into(), toml::Value::String("bad".into()));
    fs::write(f.0.join("config.toml"), toml::to_string(&tree).unwrap()).unwrap();
    let store = SettingsStore::new(&f.0);
    let base = store.read().unwrap();
    let scope = m::project_scope(&project);
    assert!(base.require(&format!("{scope}.checks.check")).is_err());
    base.require(&format!("{scope}.checks.prepare")).unwrap();
    base.require(&format!("{scope}.launch")).unwrap();
    assert_eq!(
        base.effective
            .project(&project)
            .unwrap()
            .checks
            .prepare
            .len(),
        1
    );
    let mut draft = base.tree.clone();
    devtabs::settings::projection::set(&mut draft, &["appearance", "theme"], Some("light".into()));
    let saved = store.save(&base, &draft).unwrap();
    assert_eq!(
        saved.tree["project"][0]["checks"]["check"].as_str(),
        Some("bad")
    );
    assert_eq!(
        saved.effective.appearance.theme,
        Some(devtabs::appearance::Theme::Light)
    );
}
#[test]
fn plans_freeze_prepare_and_new_plans_capture_current_definitions() {
    isolated(
        "plans_freeze_prepare_and_new_plans_capture_current_definitions",
        || {
            let repo = Fixture::new();
            for args in [
                vec!["init", "--quiet"],
                vec![
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--allow-empty",
                    "--quiet",
                    "-m",
                    "baseline",
                ],
            ] {
                let result = Command::new("git")
                    .current_dir(&repo.0)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(result.status.success(), "{:?}", result);
            }
            let store = SettingsStore::current();
            let base = store.read().unwrap();
            let mut prefs = seed(&repo.0);
            let original = devtabs::checks::Spec {
                argv: vec!["original".into()],
                cwd: ".".into(),
                env: Default::default(),
                timeout_ms: 1,
            };
            prefs.project[0].checks.prepare = vec![original.clone()];
            let first = store
                .save(&base, &toml::Value::try_from(&prefs).unwrap())
                .unwrap();
            let mut options = devtabs::launch::Options::parse_for_view();
            options.project = repo.0.clone();
            let mut catalog = devtabs::tasks::Catalog::default();
            let main = catalog.main_task(&options).unwrap();
            let plan = catalog.plan(&main.id, "first", "HEAD", None).unwrap();
            prefs.project[0].checks.prepare[0].argv = vec!["latest".into()];
            store
                .save(&first, &toml::Value::try_from(&prefs).unwrap())
                .unwrap();
            assert_eq!(main.prepare, [original.clone()]);
            assert_eq!(plan.task.prepare, [original]);
            assert_eq!(
                catalog
                    .plan(&main.id, "second", "HEAD", None)
                    .unwrap()
                    .task
                    .prepare[0]
                    .argv,
                ["latest"]
            );
            let mut serialized = serde_json::to_value(&plan).unwrap();
            serialized["task"]
                .as_object_mut()
                .unwrap()
                .remove("prepare");
            assert!(serde_json::from_value::<devtabs::tasks::Plan>(serialized).is_err());
            assert!(
                repo.0
                    .read_dir()
                    .unwrap()
                    .all(|e| e.unwrap().file_name() != "original")
            );
        },
    );
}
