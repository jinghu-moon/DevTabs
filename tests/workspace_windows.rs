#![cfg(windows)]
use anyhow::Result;
use devtabs::{
    launch::Layout,
    protocol::{Request, Response, WorkspaceInfo},
    session::CommandStatus,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Harness {
    root: PathBuf,
    exe: PathBuf,
}
impl Harness {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("DevTabs v2 测试 {}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("devtabs.exe");
        fs::copy(env!("CARGO_BIN_EXE_devtabs"), &exe).unwrap();
        // Trap accidental native-window launches. DevTabs rejects wt's -w flag;
        // the isolated frontend must never resolve the installed wt.exe.
        fs::copy(&exe, root.join("wt.exe")).unwrap();
        Self { root, exe }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.env("DEVTABS_HOME", self.root.join("data"));
        cmd
    }
    fn open(&self, extra: &[&str]) -> WorkspaceInfo {
        let out = self
            .command()
            .args([
                "--headless",
                "--agent",
                "shell",
                "--no-profile",
                "--layout",
                "split",
                "--project",
            ])
            .arg(&self.root)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn rpc(&self, request: Request) -> Result<Response> {
        let mut child = self
            .command()
            .arg("__rpc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&request)?)?;
        let result = child.wait_with_output()?;
        anyhow::ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(serde_json::from_slice(&result.stdout)?)
    }
    fn info(&self, id: &str) -> WorkspaceInfo {
        let Response::Workspaces(work) = self.rpc(Request::List).unwrap() else {
            panic!()
        };
        work.into_iter().find(|w| w.id == id).unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Ok(Response::Workspaces(work)) = self.rpc(Request::List) {
            for w in work {
                let _ = self.rpc(Request::Close { workspace: w.id });
            }
            let _ = self.rpc(Request::Shutdown);
        }
        thread::sleep(Duration::from_millis(100));
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn wait(label: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "Timed out: {label}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}
#[test]
fn cleanup_rejects_live_task_terminals_without_hanging_or_stopping_the_other_task() {
    let h = Harness::new();
    fs::create_dir_all(h.root.join("data")).unwrap();
    struct OwnedServer(std::process::Child, Option<PathBuf>);
    impl Drop for OwnedServer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
            if let Some(p) = &self.1 {
                let _ = fs::remove_dir_all(p);
            }
        }
    }
    let mut server = OwnedServer(
        h.command()
            .arg("__serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        None,
    );
    wait("isolated host", || h.rpc(Request::Ping).is_ok());
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&h.root)
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
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "fixture"]);
    git(&["config", "user.email", "fixture@localhost"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.autocrlf", "false"]);
    fs::write(h.root.join("a"), "base").unwrap();
    git(&["add", "a"]);
    git(&["commit", "-m", "base"]);
    let w = h.open(&["--composition", "agent-only"]);
    wait("A ready", || h.info(&w.id).sessions[0].shell_pid.is_some());
    let before = h.info(&w.id).sessions[0].shell_pid;
    let target = h
        .root
        .with_file_name(format!("DevTabs-cleanup-{}", uuid::Uuid::new_v4()));
    let Response::TaskPlan(p) = h
        .rpc(Request::PlanTask {
            workspace: w.id.clone(),
            source: w.tasks[0].id.clone(),
            name: "cleanup fixture B".into(),
            base: "HEAD".into(),
            path: Some(target.clone()),
        })
        .unwrap()
    else {
        panic!()
    };
    server.1 = Some(target);
    h.rpc(Request::AddTask {
        workspace: w.id.clone(),
        operation: p.operation.clone(),
    })
    .unwrap();
    wait("B ready", || {
        h.info(&w.id).tasks.len() == 2
            && h.info(&w.id).sessions.iter().all(|s| s.shell_pid.is_some())
    });
    let Response::Lifecycle(op) = h
        .rpc(Request::Lifecycle {
            action: devtabs::lifecycle::Action::Plan {
                source: p.task.id.clone(),
                target: w.tasks[0].id.clone(),
                cleanup: true,
                require_check: false,
            },
        })
        .unwrap()
    else {
        panic!()
    };
    let at = Instant::now();
    let error = h
        .rpc(Request::Lifecycle {
            action: devtabs::lifecycle::Action::Apply {
                operation: op.id.clone(),
                confirm: op.id.clone(),
            },
        })
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("Close all source task terminals"),
        "{error:#}"
    );
    assert!(at.elapsed() < Duration::from_secs(3));
    assert_eq!(h.info(&w.id).sessions[0].shell_pid, before);
    assert!(op.source.checkout.is_dir());
    h.rpc(Request::Lifecycle {
        action: devtabs::lifecycle::Action::Close {
            task: op.source.id.clone(),
            confirm: op.source.id.clone(),
        },
    })
    .unwrap();
    wait("B exits", || {
        h.info(&w.id)
            .sessions
            .iter()
            .filter(|s| s.task.task_id == op.source.id)
            .all(|s| s.status != devtabs::session::Status::Running)
    });
    let Response::Lifecycle(done) = h
        .rpc(Request::Lifecycle {
            action: devtabs::lifecycle::Action::Apply {
                operation: op.id.clone(),
                confirm: op.id.clone(),
            },
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(done.phase, "Complete");
    assert!(!op.source.checkout.exists());
    assert_eq!(h.info(&w.id).sessions[0].shell_pid, before);
}
#[test]
fn managed_hook_events_are_bound_to_run_deduplicated_and_old_ack_cannot_clear_new_result() {
    let h = Harness::new();
    let w = h.open(&["--composition", "agent-only"]);
    let session = w.sessions[0].id.clone();
    let task = w.tasks[0].id.clone();
    h.rpc(Request::Attach {
        workspace: w.id.clone(),
        session: session.clone(),
        client: "fixture".into(),
        takeover: false,
        floating: None,
        rows: 24,
        cols: 100,
    })
    .unwrap();
    let send = |kind: &str| {
        let payload = serde_json::json!({"hook_event_name":kind,"session_id":"fixture-conversation","turn_id":"turn-A","cwd":h.root});
        let marker = h.root.join("hook-finished");
        let _ = fs::remove_file(&marker);
        let source = format!(
            "'{}' | & {} __hook shell; [IO.File]::WriteAllText({},'done')\r",
            payload.to_string().replace('\'', "''"),
            quoted(&h.exe),
            quoted(&marker)
        );
        h.rpc(Request::Input {
            workspace: w.id.clone(),
            session: session.clone(),
            client: "fixture".into(),
            bytes: source.into_bytes(),
        })
        .unwrap();
        wait("helper shell returns", || marker.exists());
    };
    send("UserPromptSubmit");
    wait("managed prompt event", || {
        h.info(&w.id).sessions[0].activity.phase == devtabs::activity::Phase::Working
    });
    send("Stop");
    {
        let until = Instant::now() + Duration::from_secs(5);
        while h.info(&w.id).sessions[0].activity.result != 1 {
            assert!(
                Instant::now() < until,
                "report event missing; helper diagnostic: {:?}; activity: {:?}",
                fs::read_to_string(h.root.join("data/hook-last-error.txt")),
                h.info(&w.id).sessions[0].activity
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    send("Stop");
    send("UserPromptSubmit");
    wait("continuation", || {
        h.info(&w.id).sessions[0].activity.phase == devtabs::activity::Phase::Working
    });
    send("Stop");
    wait("new result", || {
        h.info(&w.id).sessions[0].activity.result == 2
    });
    let mut target = devtabs::notifications::Target {
        workspace: w.id.clone(),
        task: task.clone(),
        session,
        run: w.sessions[0].task.run_id.clone(),
        result: 1,
    };
    h.rpc(Request::Notifications {
        action: devtabs::notifications::Action::Ack {
            target: target.clone(),
        },
    })
    .unwrap();
    assert!(h.info(&w.id).sessions[0].activity.pending());
    h.rpc(Request::Notifications {
        action: devtabs::notifications::Action::Navigate {
            target: target.clone(),
        },
    })
    .unwrap();
    let Response::Notifications(other) = h
        .rpc(Request::Notifications {
            action: devtabs::notifications::Action::Poll {
                workspace: w.id.clone(),
                clients: vec!["other-view".into()],
            },
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(other.navigation.is_none());
    let Response::Notifications(original) = h
        .rpc(Request::Notifications {
            action: devtabs::notifications::Action::Poll {
                workspace: w.id.clone(),
                clients: vec!["fixture".into()],
            },
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(original.navigation, Some(target.clone()));
    target.run = "expired".into();
    assert!(
        h.rpc(Request::Notifications {
            action: devtabs::notifications::Action::Navigate { target }
        })
        .is_err()
    );
    assert!(
        !h.root.join("data/notification-identity.json").exists(),
        "Fixture must not register or send real notifications"
    );
    h.rpc(Request::Close { workspace: w.id }).unwrap();
    let recorder = devtabs::rounds::Recorder::new(h.root.join("data/history"));
    let until = Instant::now() + Duration::from_secs(3);
    while recorder
        .list(&task)
        .unwrap()
        .iter()
        .any(|r| r.ended.is_none())
    {
        assert!(
            Instant::now() < until,
            "Closing the managed terminal must seal its active recording and release resource leases"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn running_dirty_task_can_add_an_isolated_task_without_restarting_or_crossing_input() {
    let h = Harness::new();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&h.root)
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
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@localhost"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.autocrlf", "false"]);
    fs::write(h.root.join("same.txt"), "base").unwrap();
    git(&["add", "same.txt"]);
    git(&["commit", "-m", "base"]);
    let w = h.open(&["--composition", "agent-only"]);
    wait("A starts", || h.info(&w.id).sessions[0].shell_pid.is_some());
    let before = h.info(&w.id);
    fs::write(h.root.join("same.txt"), "A dirty").unwrap();
    let target = h.root.with_extension("isolated-task");
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _target = Cleanup(target.clone());
    let Response::TaskPlan(plan) = h
        .rpc(Request::PlanTask {
            workspace: w.id.clone(),
            source: w.tasks[0].id.clone(),
            name: "Feature B".into(),
            base: "HEAD".into(),
            path: Some(target.clone()),
        })
        .unwrap()
    else {
        panic!("plan")
    };
    assert!(matches!(
        h.rpc(Request::AddTask {
            workspace: w.id.clone(),
            operation: plan.operation.clone()
        })
        .unwrap(),
        Response::TaskOperation(_)
    ));
    wait("B attached", || h.info(&w.id).tasks.len() == 2);
    let info = h.info(&w.id);
    assert_eq!(info.sessions[0].id, before.sessions[0].id);
    assert_eq!(info.sessions[0].shell_pid, before.sessions[0].shell_pid);
    assert_ne!(info.sessions[0].task.task_id, info.sessions[1].task.task_id);
    assert_eq!(info.sessions[1].task.role, devtabs::tasks::Role::Agent);
    assert_eq!(fs::read_to_string(target.join("same.txt")).unwrap(), "base");
    for (index, client, value) in [(0, "A-client", "A-only"), (1, "B-client", "B-only")] {
        h.rpc(Request::Attach {
            workspace: w.id.clone(),
            session: info.sessions[index].id.clone(),
            client: client.into(),
            takeover: false,
            floating: None,
            rows: 24,
            cols: 90,
        })
        .unwrap();
        h.rpc(Request::Input {
            workspace: w.id.clone(),
            session: info.sessions[index].id.clone(),
            client: client.into(),
            bytes: format!(
                "[IO.File]::WriteAllText((Join-Path (Get-Location) 'route.txt'), '{value}')\r"
            )
            .into_bytes(),
        })
        .unwrap();
    }
    wait("A input", || {
        fs::read_to_string(h.root.join("route.txt")).is_ok_and(|s| s == "A-only")
    });
    wait("B input", || {
        fs::read_to_string(target.join("route.txt")).is_ok_and(|s| s == "B-only")
    });
    assert_eq!(
        fs::read_to_string(h.root.join("same.txt")).unwrap(),
        "A dirty"
    );
    h.rpc(Request::Input {
        workspace: w.id.clone(),
        session: info.sessions[1].id.clone(),
        client: "B-client".into(),
        bytes: b"exit\r".to_vec(),
    })
    .unwrap();
    wait("B exits independently", || {
        matches!(
            h.info(&w.id).sessions[1].status,
            devtabs::session::Status::Exited(_)
        )
    });
    let restart = h
        .rpc(Request::StartRole {
            workspace: w.id.clone(),
            task: info.tasks[1].id.clone(),
            role: devtabs::tasks::Role::Agent,
            fresh: true,
            expected_run: Some(info.sessions[1].task.run_id.clone()),
        })
        .unwrap();
    assert!(
        matches!(restart, Response::TaskOperation(_)),
        "Independent task recovery must be available"
    );
    wait("B gets a new run", || {
        h.info(&w.id).sessions.iter().any(|s| {
            s.task.task_id == info.tasks[1].id && s.task.run_id != info.sessions[1].task.run_id
        })
    });
    let recovered = h.info(&w.id);
    assert_eq!(recovered.sessions[0].id, before.sessions[0].id);
    assert_eq!(
        recovered.sessions[0].shell_pid,
        before.sessions[0].shell_pid
    );
    assert_eq!(recovered.tasks[1].id, info.tasks[1].id);
    let Response::Host { pid } = h.rpc(Request::Ping).unwrap() else {
        panic!("host identity")
    };
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
        };
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        assert!(!process.is_null());
        assert_ne!(TerminateProcess(process, 77), 0);
        CloseHandle(process);
    }
    wait("fixture host exits", || h.rpc(Request::Ping).is_err());
    let result = h
        .command()
        .args(["--restore", "--headless"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let restored: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let next: WorkspaceInfo = serde_json::from_value(restored["workspaces"][0].clone()).unwrap();
    assert_eq!(
        next.tasks.iter().map(|t| &t.id).collect::<Vec<_>>(),
        recovered.tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    assert_eq!(next.sessions.len(), 2, "Do not automatically resume Debug");
    assert!(next.sessions.iter().all(|s| {
        !recovered
            .sessions
            .iter()
            .any(|old| old.id == s.id || old.task.run_id == s.task.run_id)
    }));
    assert_eq!(next.tasks[1].cwd, target);
    assert_eq!(
        fs::read_to_string(target.join("route.txt")).unwrap(),
        "B-only"
    );
    h.rpc(Request::Close { workspace: next.id }).unwrap();
}
#[test]
fn agent_only_creates_exactly_one_pty_and_roundtrips_recovery() {
    let h = Harness::new();
    let workspace = h.open(&["--composition", "agent-only", "--new-instance"]);
    assert_eq!(workspace.sessions.len(), 1);
    assert_eq!(workspace.sessions[0].name, "Agent");
    assert_eq!(
        workspace.options.composition,
        devtabs::launch::Composition::AgentOnly
    );
    wait("single Agent shell", || {
        h.info(&workspace.id).sessions[0].shell_pid.is_some()
    });
    let Response::Workspaces(work) = h.rpc(Request::List).unwrap() else {
        panic!()
    };
    assert_eq!(
        work.iter().map(|w| w.sessions.len()).sum::<usize>(),
        1,
        "there must be no hidden Debug process"
    );
    let journal = devtabs::recovery::Journal::load_at(&h.root.join("data")).unwrap();
    let restored = devtabs::recovery::restore_options(&journal.entries[&workspace.id]);
    assert_eq!(restored.terminal_count(), 1);
    assert_eq!(
        devtabs::overview::model::LaunchSpec::from_options(&restored)
            .options()
            .terminal_count(),
        1
    );
    let reused = h.open(&[]);
    assert_eq!(reused.id, workspace.id);
    assert_eq!(reused.sessions.len(), 1);
    h.rpc(Request::Close {
        workspace: workspace.id,
    })
    .unwrap();
}

#[test]
fn legacy_host_cannot_receive_new_build_requests_or_reject_new_sidebar_settings() {
    use interprocess::local_socket::{
        GenericNamespaced, ListenerNonblockingMode, ListenerOptions, prelude::*,
    };
    use std::{
        hash::{Hash, Hasher},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    let h = Harness::new();
    let home = h.root.join("data");
    fs::create_dir_all(&home).unwrap();
    let mut config = devtabs::config::Config::default();
    let project = config
        .projects
        .entry(devtabs::config::key(&h.root))
        .or_default();
    project.sidebar.width = 46;
    project.sidebar.agent_ratio = 6250;
    let path = home.join("config.toml");
    fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();

    // A real listener with the exact namespace used by existing v4 binaries.
    // It accepts Ping but rejects Open exactly where the old configuration parser failed.
    let mut hash = std::hash::DefaultHasher::new();
    devtabs::config::key(&home).hash(&mut hash);
    std::env::var_os("DEVTABS_INSTANCE").hash(&mut hash);
    let name = format!("DevTabs-v4-{:016x}", hash.finish());
    let listener = ListenerOptions::new()
        .name(name.to_ns_name::<GenericNamespaced>().unwrap())
        .nonblocking(ListenerNonblockingMode::Accept)
        .create_sync()
        .unwrap();
    struct LegacyHost {
        stop: Arc<AtomicBool>,
        requests: Arc<AtomicUsize>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl Drop for LegacyHost {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let legacy = LegacyHost {
        stop: stop.clone(),
        requests: requests.clone(),
        thread: Some(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok(mut stream) => {
                        requests.fetch_add(1, Ordering::Relaxed);
                        let response = match devtabs::protocol::receive::<Request>(&mut stream).unwrap() {
                        Request::Ping => Response::Host { pid: std::process::id() },
                        _ => Response::Error("unknown field `agent_ratio`, expected one of `width`, `follow`, `compact`".into()),
                    };
                        devtabs::protocol::send(&mut stream, &response).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("legacy listener: {error}"),
                }
            }
        })),
    };
    let workspace = h.open(&["--new-instance"]);
    assert_eq!(
        legacy.requests.load(Ordering::Relaxed),
        0,
        "a new build must not send any request to an old host"
    );
    let Response::Host { pid } = h.rpc(Request::Ping).unwrap() else {
        panic!("host identity")
    };
    assert_ne!(pid, std::process::id());
    let loaded: devtabs::config::Config =
        toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let saved = &loaded.projects[&devtabs::config::key(&h.root)].sidebar;
    assert_eq!(saved.width, 46);
    assert_eq!(saved.agent_ratio, 6250);
    assert!(loaded.projects[&devtabs::config::key(&h.root)].last_opened > 0);
    let copy = h.root.join("same-build.exe");
    fs::copy(&h.exe, &copy).unwrap();
    let out = Command::new(copy)
        .env("DEVTABS_HOME", &home)
        .arg("list")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed: Vec<WorkspaceInfo> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].id, workspace.id,
        "a copied identical build must reuse the same host"
    );
    h.rpc(Request::Close {
        workspace: workspace.id,
    })
    .unwrap();
}

#[test]
fn launch_journal_survives_host_crash_restore_is_idempotent_and_last_reuses_workspaces() {
    let h = Harness::new();
    let original = h.open(&["--title", "恢复测试", "--new-instance"]);
    wait("initial shell identity", || {
        h.info(&original.id)
            .sessions
            .iter()
            .all(|s| s.shell_pid.is_some())
    });
    let old_pid = h.info(&original.id).sessions[0].shell_pid.unwrap();
    let old_task = original.tasks[0].id.clone();
    let home = h.root.join("data");
    let journal = devtabs::recovery::Journal::load_at(&home).unwrap();
    assert!(journal.entries.contains_key(&original.id));
    let live_restore = h
        .command()
        .args(["--restore", "--headless"])
        .output()
        .unwrap();
    assert!(
        live_restore.status.success(),
        "{}",
        String::from_utf8_lossy(&live_restore.stderr)
    );
    let live: serde_json::Value = serde_json::from_slice(&live_restore.stdout).unwrap();
    assert!(live["workspaces"].as_array().unwrap().is_empty());
    assert_eq!(h.info(&original.id).sessions[0].shell_pid, Some(old_pid));
    let Response::Host { pid } = h.rpc(Request::Ping).unwrap() else {
        panic!("host identity")
    };
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
        };
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        assert!(!process.is_null());
        assert_ne!(TerminateProcess(process, 77), 0);
        CloseHandle(process);
    }
    wait("isolated host exits", || h.rpc(Request::Ping).is_err());
    assert!(
        devtabs::recovery::Journal::load_at(&home)
            .unwrap()
            .entries
            .contains_key(&original.id)
    );
    let result = h
        .command()
        .args(["--restore", "--headless"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let restored: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let work = restored["workspaces"].as_array().unwrap();
    assert_eq!(work.len(), 1);
    let id = work[0]["id"].as_str().unwrap().to_owned();
    assert_ne!(id, original.id);
    wait("restored shell identity", || {
        h.info(&id).sessions.iter().all(|s| s.shell_pid.is_some())
    });
    assert_eq!(h.info(&id).options.title.as_deref(), Some("恢复测试"));
    assert_eq!(
        h.info(&id).tasks[0].id,
        old_task,
        "Restoration must retain the authoritative task identity"
    );
    assert_ne!(h.info(&id).sessions[0].shell_pid, Some(old_pid));
    let journal = devtabs::recovery::Journal::load_at(&home).unwrap();
    assert_eq!(journal.entries.len(), 1);
    assert!(!journal.entries.contains_key(&original.id));
    assert!(journal.entries.contains_key(&id));
    let again = h
        .command()
        .args(["--restore", "--headless"])
        .output()
        .unwrap();
    assert!(again.status.success());
    let data: serde_json::Value = serde_json::from_slice(&again.stdout).unwrap();
    assert!(data["workspaces"].as_array().unwrap().is_empty());
    let last = h.command().args(["--last", "--headless"]).output().unwrap();
    assert!(
        last.status.success(),
        "{}",
        String::from_utf8_lossy(&last.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&last.stdout).unwrap();
    assert_eq!(data["workspaces"][0]["id"], id);
    h.rpc(Request::Close { workspace: id }).unwrap();
    assert!(
        devtabs::recovery::Journal::load_at(&home)
            .unwrap()
            .entries
            .is_empty()
    );
    let nothing = h
        .command()
        .args(["--restore", "--headless"])
        .output()
        .unwrap();
    assert!(nothing.status.success());
    let data: serde_json::Value = serde_json::from_slice(&nothing.stdout).unwrap();
    assert!(data["workspaces"].as_array().unwrap().is_empty());
}

#[test]
fn automatic_resume_uses_fixture_history_and_fresh_preserves_literal_agent_arguments() {
    let h = Harness::new();
    let bin = h.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let captured = h.root.join("arguments.json");
    // Bind the command explicitly inside the fixture shell: PATH precedence is
    // not sufficient isolation from PowerShell aliases, functions or app shims.
    let pwsh = Command::new("pwsh")
        .args(["-NoProfile", "-Command", "(Get-Process -Id $PID).Path"])
        .output()
        .unwrap();
    assert!(pwsh.status.success());
    let pwsh = String::from_utf8(pwsh.stdout).unwrap().trim().to_owned();
    let source = bin.join("fixture.rs");
    fs::write(&source, format!(r#"
fn main() {{
    if std::env::current_exe().unwrap().file_stem().unwrap() == "fixture-shell" {{
        let encoded = std::env::args().last().unwrap();
        let fixture = std::env::current_exe().unwrap().with_file_name("codex.exe");
        let script = format!("function global:codex {{{{ & '{{}}' @args }}}}; & ([scriptblock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{{}}'))))", fixture.display().to_string().replace('\'', "''"), encoded);
        let status = std::process::Command::new({pwsh:?}).args(["-NoLogo", "-NoExit", "-NoProfile", "-Command", &script]).status().unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }}
    let path = std::env::var_os("DEVTABS_FIXTURE_CAPTURE").unwrap();
    let args = std::env::args().skip(1).collect::<Vec<_>>().join("\0");
    std::fs::write(path, args).unwrap();
}}
"#)).unwrap();
    let compiled = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(bin.join("codex.exe"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let shell = bin.join("fixture-shell.exe");
    fs::copy(bin.join("codex.exe"), &shell).unwrap();
    let history = h.root.join("fake-codex/sessions");
    fs::create_dir_all(&history).unwrap();
    let line = serde_json::json!({"type":"session_meta","payload":{"cwd":h.root,"id":"fixture-session","timestamp":"2026-11-11T00:00:00Z"}});
    fs::write(history.join("fixture.jsonl"), format!("{line}\n")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let open = |flags: &[&str]| {
        let out = h
            .command()
            .env("PATH", &path)
            .arg("--shell")
            .arg(&shell)
            .env("DEVTABS_FIXTURE_CAPTURE", &captured)
            .env("CODEX_HOME", h.root.join("fake-codex"))
            .args([
                "--headless",
                "--agent",
                "codex",
                "--no-profile",
                "--project",
            ])
            .arg(&h.root)
            .args([
                "--agent-arg=--model",
                "--agent-arg=literal ' ; $(Write-Output injected)",
            ])
            .args(flags)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<WorkspaceInfo>(&out.stdout).unwrap()
    };
    let resumed = open(&["--continue-latest"]);
    assert_eq!(
        resumed.options.resume_id.as_deref(),
        Some("fixture-session")
    );
    h.rpc(Request::Attach {
        floating: None,
        takeover: false,
        workspace: resumed.id.clone(),
        session: resumed.sessions[0].id.clone(),
        client: "fixture".into(),
        rows: 32,
        cols: 160,
    })
    .unwrap();
    let resume_started = Instant::now();
    wait("fixture Agent captures resumed arguments", || {
        let current = h.info(&resumed.id);
        if resume_started.elapsed() > Duration::from_secs(5)
            || matches!(current.sessions[0].command,CommandStatus::Exited(c) if c!=0)
        {
            let output = h
                .rpc(Request::Poll {
                    workspace: resumed.id.clone(),
                    session: resumed.sessions[0].id.clone(),
                    client: "fixture".into(),
                    revision: None,
                    history: true,
                })
                .unwrap();
            if let Response::Update(Some(update)) = output {
                panic!(
                    "Fixture Agent failed: {}",
                    String::from_utf8_lossy(&update.screen)
                );
            }
        }

        fs::read(&captured).ok().is_some()
    });
    let args = fs::read_to_string(&captured)
        .unwrap()
        .split('\0')
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        args,
        vec![
            "resume",
            "fixture-session",
            "--model",
            "literal ' ; $(Write-Output injected)"
        ]
    );
    fs::remove_file(&captured).unwrap();
    let fresh = open(&["--fresh", "--new-instance"]);
    assert!(fresh.options.resume_id.is_none());
    wait("fixture Agent captures fresh arguments", || {
        fs::read(&captured).ok().is_some()
    });
    let args = fs::read_to_string(&captured)
        .unwrap()
        .split('\0')
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        args,
        vec!["--model", "literal ' ; $(Write-Output injected)"]
    );
}

#[test]
fn detached_output_reconnect_and_layouts_keep_same_processes() {
    let h = Harness::new();
    let source = "for($i=0;$i -lt 30;$i++){[Console]::WriteLine(('TICK-{0}' -f $i)); Start-Sleep -Milliseconds 50}";
    let w = h.open(&["--debug-command", source]);
    let sid = w.sessions[1].id.clone();
    wait("shell identity", || {
        h.info(&w.id).sessions[1].shell_pid.is_some()
    });
    let pid = h.info(&w.id).sessions[1].shell_pid;
    let attach = |client: &str| {
        h.rpc(Request::Attach {
            floating: None,
            takeover: false,
            workspace: w.id.clone(),
            session: sid.clone(),
            client: client.into(),
            rows: 24,
            cols: 100,
        })
    };
    attach("first").unwrap();
    h.rpc(Request::Detach {
        client: "first".into(),
    })
    .unwrap();
    wait("background command completed while detached", || {
        h.info(&w.id).sessions[1].command == devtabs::session::CommandStatus::Exited(0)
    });
    assert_eq!(h.info(&w.id).sessions[1].shell_pid, pid);
    attach("second").unwrap();
    let Response::Update(Some(update)) = h
        .rpc(Request::Poll {
            workspace: w.id.clone(),
            session: sid.clone(),
            client: "second".into(),
            revision: None,
            history: true,
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(String::from_utf8_lossy(&update.screen).contains("TICK-29"));
    assert!(
        attach("competing").is_err(),
        "two writers must not silently compete"
    );
    for layout in [Layout::Float, Layout::Tabs, Layout::Split] {
        h.rpc(Request::Layout {
            workspace: w.id.clone(),
            layout,
        })
        .unwrap();
        attach("second").unwrap();
        assert_eq!(h.info(&w.id).sessions[1].shell_pid, pid);
    }
    let reused = h.open(&[]);
    assert_eq!(reused.id, w.id);
    let fresh = h.open(&["--new-instance"]);
    assert_ne!(fresh.id, w.id);
    assert_ne!(fresh.sessions[0].id, w.sessions[0].id);
}

#[test]
fn concurrent_open_is_atomic_and_restart_preserves_shell() {
    let h = Harness::new();
    let mut children = Vec::new();
    for _ in 0..4 {
        children.push(
            h.command()
                .args([
                    "--headless",
                    "--agent",
                    "shell",
                    "--no-profile",
                    "--layout",
                    "split",
                    "--project",
                ])
                .arg(&h.root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let mut ids = Vec::new();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        ids.push(
            serde_json::from_slice::<WorkspaceInfo>(&out.stdout)
                .unwrap()
                .id,
        );
    }
    assert!(ids.iter().all(|id| id == &ids[0]));
    let workspace = &ids[0];
    wait("shell readiness", || {
        h.info(workspace).sessions[1].shell_pid.is_some()
    });
    let before = h.info(workspace).sessions[1].clone();
    let output = h.root.join("rerun-result");
    let source = format!(
        "[IO.File]::AppendAllText({},'done'); Write-Output 'complete'",
        quoted(&output)
    );
    h.rpc(Request::Run {
        workspace: workspace.clone(),
        session: before.id.clone(),
        source: Some(source),
    })
    .unwrap();
    wait("managed run", || {
        fs::read_to_string(&output).is_ok_and(|v| v == "done")
    });
    wait("command exit", || {
        h.info(workspace).sessions[1].command == CommandStatus::Exited(0)
    });
    h.rpc(Request::Run {
        workspace: workspace.clone(),
        session: before.id.clone(),
        source: None,
    })
    .unwrap();
    wait("managed rerun", || {
        fs::read_to_string(&output).is_ok_and(|v| v == "donedone")
    });
    assert_eq!(h.info(workspace).sessions[1].shell_pid, before.shell_pid);
    assert!(
        h.rpc(Request::Stop {
            workspace: workspace.clone(),
            session: before.id,
            force: true
        })
        .is_err(),
        "force requires previous normal stop"
    );
}

#[test]
fn installation_and_project_configuration_use_isolated_directories() {
    let h = Harness::new();
    let install = h.root.join("install with 空格");
    let out = h
        .command()
        .args(["install", "--no-path", "--directory"])
        .arg(&install)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(install.join("devtabs.exe").is_file());
    let version = Command::new(install.join("devtabs.exe"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    let repeated = Command::new(install.join("devtabs.exe"))
        .args(["install", "--no-path", "--directory"])
        .arg(&install)
        .output()
        .unwrap();
    assert!(
        repeated.status.success(),
        "install from installed copy should be idempotent"
    );
    let upgrade = h
        .command()
        .args(["install", "--no-path", "--directory"])
        .arg(&install)
        .output()
        .unwrap();
    assert!(upgrade.status.success());
    assert!(install.join("devtabs.old.exe").exists());
    let out = h
        .command()
        .args(["uninstall", "--no-path", "--directory"])
        .arg(&install)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!install.join("devtabs.exe").exists());
    for args in [["config", "agent", "shell"], ["config", "show", ""]] {
        let mut cmd = h.command();
        cmd.args(args.iter().filter(|s| !s.is_empty()));
        assert!(cmd.output().unwrap().status.success());
    }
    let root = h.root.join("projects");
    fs::create_dir_all(root.join("One")).unwrap();
    fs::create_dir_all(root.join("Two")).unwrap();
    assert!(
        h.command()
            .args(["config", "root"])
            .arg(&root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let c: devtabs::config::Config =
        toml::from_str(&fs::read_to_string(h.root.join("data/config.toml")).unwrap()).unwrap();
    assert_eq!(c.scan().len(), 2);
    assert!(
        h.command()
            .args(["config", "exclude"])
            .arg(root.join("One"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let c: devtabs::config::Config =
        toml::from_str(&fs::read_to_string(h.root.join("data/config.toml")).unwrap()).unwrap();
    assert_eq!(c.scan(), vec![root.join("Two")]);
}

#[test]
fn stop_force_and_native_exit_codes_leave_shell_available() {
    let h = Harness::new();
    let w = h.open(&[]);
    let session = w.sessions[1].id.clone();
    wait("shell", || h.info(&w.id).sessions[1].shell_pid.is_some());
    let pid = h.info(&w.id).sessions[1].shell_pid;
    h.rpc(Request::Run {
        workspace: w.id.clone(),
        session: session.clone(),
        source: Some("& cmd /c exit 23".into()),
    })
    .unwrap();
    wait("native exit code", || {
        h.info(&w.id).sessions[1].command == CommandStatus::Exited(23)
    });
    h.rpc(Request::Run {
        workspace: w.id.clone(),
        session: session.clone(),
        source: Some("& cmd /c 'ping -t 127.0.0.1'".into()),
    })
    .unwrap();
    wait("native running", || {
        h.info(&w.id).sessions[1].command == CommandStatus::Running
    });
    h.rpc(Request::Stop {
        workspace: w.id.clone(),
        session: session.clone(),
        force: false,
    })
    .unwrap();
    thread::sleep(Duration::from_millis(3200));
    h.rpc(Request::Stop {
        workspace: w.id.clone(),
        session: session.clone(),
        force: true,
    })
    .unwrap();
    wait("command stopped", || {
        matches!(h.info(&w.id).sessions[1].command, CommandStatus::Exited(_))
    });
    assert_eq!(h.info(&w.id).sessions[1].shell_pid, pid);
    h.rpc(Request::Run {
        workspace: w.id.clone(),
        session: session.clone(),
        source: Some("Write-Output 'shell-preserved'".into()),
    })
    .unwrap();
    wait("next command", || {
        h.info(&w.id).sessions[1].command == CommandStatus::Exited(0)
    });
}

#[test]
fn killed_frontend_reconnects_without_restarting_shells() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::Read;
    let h = Harness::new();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 36,
            cols: 140,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(&h.exe);
    command.args([
        "--agent",
        "shell",
        "--no-profile",
        "--layout",
        "split",
        "--project",
    ]);
    command.arg(&h.root);
    command.env("DEVTABS_HOME", h.root.join("data"));
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let drain = thread::spawn(move || {
        let mut terminal = devtabs::terminal::Terminal::new(36, 140, 100);
        let mut buffer = [0; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let replies = terminal.process(&buffer[..n]);
            if !replies.is_empty() {
                let _ = writer.write_all(&replies);
            }
        }
    });
    wait(
        "first workspace",
        || matches!(h.rpc(Request::List),Ok(Response::Workspaces(w)) if !w.is_empty()),
    );
    let Response::Workspaces(work) = h.rpc(Request::List).unwrap() else {
        panic!()
    };
    let id = work[0].id.clone();
    wait("shells", || {
        h.info(&id).sessions.iter().all(|s| s.shell_pid.is_some())
    });
    let before = h.info(&id);
    wait("frontend draft guard", || {
        before
            .tasks
            .iter()
            .all(|t| devtabs::lifecycle::lease(&h.root.join("data"), &t.id, true, true).is_err())
    });
    child.kill().unwrap();
    child.wait().unwrap();
    drop(pair.master);
    drain.join().unwrap();
    for task in &before.tasks {
        assert!(
            devtabs::lifecycle::lease(&h.root.join("data"), &task.id, true, true).is_ok(),
            "A killed frontend must release draft guards without a host restart"
        );
    }
    assert_eq!(
        h.info(&id).sessions[0].shell_pid,
        before.sessions[0].shell_pid
    );
    thread::sleep(Duration::from_millis(3200));
    h.rpc(Request::Attach {
        floating: None,
        takeover: false,
        workspace: id.clone(),
        session: before.sessions[0].id.clone(),
        client: "reconnected".into(),
        rows: 30,
        cols: 100,
    })
    .unwrap();
    assert_eq!(
        h.info(&id).sessions[0].shell_pid,
        before.sessions[0].shell_pid
    );
}

#[test]
fn managed_run_cancels_unsubmitted_edit_buffer_and_config_writers_merge() {
    let h = Harness::new();
    let w = h.open(&[]);
    let sid = w.sessions[1].id.clone();
    h.rpc(Request::Attach {
        floating: None,
        takeover: false,
        workspace: w.id.clone(),
        session: sid.clone(),
        client: "edit".into(),
        rows: 24,
        cols: 160,
    })
    .unwrap();
    wait("PSReadLine prompt", || {
        matches!(h.rpc(Request::Poll {
        workspace: w.id.clone(), session: sid.clone(), client: "edit".into(), revision: None, history: false
    }), Ok(Response::Update(Some(u))) if String::from_utf8_lossy(&u.screen).contains("PS "))
    });
    let unsafe_file = h.root.join("must-not-run");
    let safe_file = h.root.join("managed-ran");
    let pending = format!("[IO.File]::WriteAllText({},'unsafe')", quoted(&unsafe_file));
    let mut bytes = Vec::new();
    for c in pending.chars() {
        bytes.extend(devtabs::input::key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ),
            false,
            true,
        ));
    }
    h.rpc(Request::Input {
        workspace: w.id.clone(),
        session: sid.clone(),
        client: "edit".into(),
        bytes,
    })
    .unwrap();
    thread::sleep(Duration::from_millis(150));
    h.rpc(Request::Run {
        workspace: w.id.clone(),
        session: sid,
        source: Some(format!(
            "[IO.File]::WriteAllText({},'safe')",
            quoted(&safe_file)
        )),
    })
    .unwrap();
    wait("managed command", || safe_file.exists());
    assert!(
        !unsafe_file.exists(),
        "Run must never submit an unrelated edit buffer"
    );
    let mut writers = Vec::new();
    for i in 0..6 {
        let root = h.root.join(format!("root-{i}"));
        fs::create_dir(&root).unwrap();
        writers.push(
            h.command()
                .args(["config", "root"])
                .arg(root)
                .spawn()
                .unwrap(),
        );
    }
    for mut writer in writers {
        assert!(writer.wait().unwrap().success());
    }
    let c: devtabs::config::Config =
        toml::from_str(&fs::read_to_string(h.root.join("data/config.toml")).unwrap()).unwrap();
    assert_eq!(c.roots.len(), 6);
}

#[test]
fn named_sessions_keep_identity_and_relative_directory_across_reuse() {
    let h = Harness::new();
    let directory = h.root.join("docs");
    fs::create_dir(&directory).unwrap();
    let marker = h.root.join("extra-cwd");
    let mut config = devtabs::config::Config::default();
    config.projects.insert(
        devtabs::config::key(&h.root),
        devtabs::config::Project {
            sessions: vec![devtabs::config::SessionConfig {
                name: "Docs".into(),
                directory: Some("docs".into()),
                command: Some(format!(
                    "[IO.File]::WriteAllText({},(Get-Location).Path)",
                    quoted(&marker)
                )),
            }],
            ..Default::default()
        },
    );
    fs::create_dir(h.root.join("data")).unwrap();
    fs::write(
        h.root.join("data/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let w = h.open(&[]);
    assert_eq!(w.sessions.len(), 3);
    assert_eq!(w.sessions[2].name, "Docs");
    wait("extra session startup", || marker.exists());
    assert_eq!(
        devtabs::config::key(Path::new(&fs::read_to_string(&marker).unwrap())),
        devtabs::config::key(&directory)
    );
    let again = h.open(&[]);
    assert_eq!(again.sessions[2].id, w.sessions[2].id);
    let view = Frontend::start(&h, &["--workspace", &w.id]);
    wait("named session frontend", || {
        view.contents().contains("Ctrl+G")
    });
    view.writer
        .lock()
        .unwrap()
        .write_all(b"\x07n\x07n")
        .unwrap();
    wait("named session focus", || view.contents().contains("[Docs]"));
    assert_eq!(h.info(&w.id).sessions[2].id, w.sessions[2].id);
    assert!(!directory.join(".devtabs").exists());
}

struct Frontend {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    screen: std::sync::Arc<std::sync::Mutex<devtabs::terminal::Terminal>>,
    writer: std::sync::Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
    received: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
#[test]
fn parallel_task_form_adds_two_live_agents_to_one_frontend_without_stealing_input() {
    let h = Harness::new();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "core.autocrlf", "false"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@localhost"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&h.root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(h.root.join("base.txt"), "committed").unwrap();
    for args in [["add", "base.txt"], ["commit", "-m"]] {
        let mut c = Command::new("git");
        c.arg("-C").arg(&h.root).args(args);
        if args[0] == "commit" {
            c.arg("base");
        }
        assert!(c.status().unwrap().success());
    }
    let w = h.open(&["--composition", "agent-only"]);
    h.rpc(Request::Layout {
        workspace: w.id.clone(),
        layout: Layout::Float,
    })
    .unwrap();
    let front = Frontend::start(&h, &["--workspace", &w.id, "--language", "en-us"]);
    let keys = |bytes: &[u8]| front.writer.lock().unwrap().write_all(bytes).unwrap();
    wait("first Agent ready", || front.contents().contains("Ctrl+G"));
    let pid = h.info(&w.id).sessions[0].shell_pid;
    keys(b"\x07+");
    wait("new task form", || {
        front.contents().contains("Base commit/reference")
    });
    keys(b"Feature-B\r");
    wait("frozen creation summary", || {
        front.contents().contains("Frozen commit")
    });
    keys(b"\r");
    wait("second task attached", || h.info(&w.id).tasks.len() == 2);
    let both = h.info(&w.id);
    let checkout = both.tasks[1].cwd.clone();
    struct OwnedWorktree(PathBuf);
    impl Drop for OwnedWorktree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let expected = h.root.with_file_name(format!(
        "{}-worktrees",
        h.root.file_name().unwrap().to_string_lossy()
    ));
    let owned = dunce::canonicalize(&both.tasks[1].checkout).unwrap();
    assert!(owned.starts_with(dunce::canonicalize(expected).unwrap()));
    let _cleanup = OwnedWorktree(owned);
    wait("both Agents visible in the original frontend", || {
        let text = front.contents();
        text.contains("Feature-B") && text.contains("Main")
    });
    keys(b"[IO.File]::WriteAllText((Join-Path (Get-Location) 'ui-route.txt'),'A')\r");
    wait("creation preserved original input focus", || {
        h.root.join("ui-route.txt").exists()
    });
    assert!(!checkout.join("ui-route.txt").exists());
    keys(b"\x072");
    keys(b"[IO.File]::WriteAllText((Join-Path (Get-Location) 'ui-route.txt'),'B')\r");
    wait("explicit task switch routes input to B", || {
        checkout.join("ui-route.txt").exists()
    });
    assert_eq!(
        fs::read_to_string(h.root.join("ui-route.txt")).unwrap(),
        "A"
    );
    assert_eq!(
        fs::read_to_string(checkout.join("ui-route.txt")).unwrap(),
        "B"
    );
    assert_eq!(h.info(&w.id).sessions[0].shell_pid, pid);
    assert!(front.child.process_id().is_some());
    let before = h
        .info(&w.id)
        .sessions
        .iter()
        .map(|s| (s.id.clone(), s.shell_pid))
        .collect::<Vec<_>>();
    keys(b"\x07p");
    wait("overview reveals the original task", || {
        front.contents().contains("Feature-B") && front.contents().contains("Projects")
    });
    keys(b"\r");
    wait("reentered same workspace", || {
        front.contents().contains("[+ New task]")
    });
    keys(b"[IO.File]::WriteAllText((Join-Path (Get-Location) 'return-route.txt'),'B')\r");
    wait("return navigation preserved B", || {
        checkout.join("return-route.txt").exists()
    });
    assert!(!h.root.join("return-route.txt").exists());
    assert_eq!(
        h.info(&w.id)
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.shell_pid))
            .collect::<Vec<_>>(),
        before
    );
    drop(front);
    h.rpc(Request::Close { workspace: w.id }).unwrap();
}
impl Frontend {
    fn start(h: &Harness, args: &[&str]) -> Self {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::{
            io::Read,
            sync::{Arc, Mutex},
        };
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 36,
                cols: 140,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(&h.exe);
        command.args(args);
        command.env("DEVTABS_HOME", h.root.join("data"));
        let mut paths = vec![h.root.clone()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command.env("PATH", std::env::join_paths(paths).unwrap());
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let screen = Arc::new(Mutex::new(devtabs::terminal::Terminal::new(36, 140, 100)));
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let (screen_thread, writer_thread) = (screen.clone(), writer.clone());
        let mut reader = pair.master.try_clone_reader().unwrap();
        let received = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let received_thread = received.clone();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                received_thread.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                let replies = screen_thread.lock().unwrap().process(&buffer[..n]);
                if !replies.is_empty() {
                    let _ = writer_thread.lock().unwrap().write_all(&replies);
                }
            }
        });
        Self {
            child,
            _master: pair.master,
            screen,
            writer,
            received,
        }
    }
    fn contents(&self) -> String {
        self.screen.lock().unwrap().screen().contents()
    }
}
impl Drop for Frontend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn overview_recovers_when_its_empty_host_exits_without_creating_any_sessions() {
    let h = Harness::new();
    let projects = h.root.join("empty-projects");
    fs::create_dir_all(&projects).unwrap();
    assert!(
        h.command()
            .args(["config", "root"])
            .arg(&projects)
            .output()
            .unwrap()
            .status
            .success()
    );
    let view = Frontend::start(&h, &[]);
    let mut first = None;
    wait("overview starts an empty host", || {
        if let Ok(Response::Host { pid }) = h.rpc(Request::Ping) {
            first = Some(pid);
            true
        } else {
            false
        }
    });
    assert!(matches!(h.rpc(Request::List).unwrap(), Response::Workspaces(w) if w.is_empty()));
    // Same missing-pipe state as idle expiry or a crashed host, without a
    // 30-second sleep and without opening Windows Terminal or an Agent.
    h.rpc(Request::Shutdown).unwrap();
    let start = Instant::now();
    loop {
        if let Ok(Response::Host { pid }) = h.rpc(Request::Ping)
            && Some(pid) != first
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "overview never reconnects after the empty host exits"
        );
        thread::sleep(Duration::from_millis(50));
    }
    assert!(matches!(h.rpc(Request::List).unwrap(), Response::Workspaces(w) if w.is_empty()));
    wait("healthy status replaces stale connection error", || {
        !view.contents().contains("os error 2")
            && !view.contents().contains("Connect to DevTabs host")
    });
}
#[test]
fn workspace_overview_button_returns_without_agent_input_or_terminating_shells() {
    let h = Harness::new();
    let w = h.open(&["--composition", "agent-only"]);
    wait("shell starts", || {
        h.info(&w.id).sessions[0].shell_pid.is_some()
    });
    let pid = h.info(&w.id).sessions[0].shell_pid;
    assert!(
        h.command()
            .args(["config", "add"])
            .arg(&h.root)
            .output()
            .unwrap()
            .status
            .success()
    );
    let view = Frontend::start(&h, &["--workspace", &w.id]);
    wait("visible overview button", || view.contents().contains("[←"));
    view.writer
        .lock()
        .unwrap()
        .write_all(b"\x1b[<0;11;36M\x1b[<0;11;36m")
        .unwrap();
    wait("back in overview", || {
        view.contents().contains("DevTabs") && view.contents().contains("[ / ")
    });
    assert_eq!(h.info(&w.id).sessions[0].shell_pid, pid);
    assert!(matches!(h.rpc(Request::List).unwrap(),Response::Workspaces(rows)if rows.len()==1));
}
fn process_metrics(pids: &[u32]) -> (u64, u64, u64) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::{ProcessStatus::*, Threading::*},
    };
    let (mut cpu, mut working, mut private) = (0, 0, 0);
    for &pid in pids {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            assert!(!handle.is_null(), "open benchmark PID {pid}");
            let (mut creation, mut exit, mut kernel, mut user): (
                FILETIME,
                FILETIME,
                FILETIME,
                FILETIME,
            ) = std::mem::zeroed();
            assert_ne!(
                GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user),
                0
            );
            let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
            cpu += ticks(kernel) + ticks(user);
            let mut memory: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            memory.cb = std::mem::size_of_val(&memory) as u32;
            assert_ne!(
                GetProcessMemoryInfo(
                    handle,
                    (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                    memory.cb
                ),
                0
            );
            working += memory.WorkingSetSize as u64;
            private += memory.PrivateUsage as u64;
            CloseHandle(handle);
        }
    }
    (cpu, working, private)
}

#[test]
#[ignore = "release-only 100-project/8-live-session performance measurement"]
fn release_scale_100_projects_8_sessions() {
    fn frame_wait(label: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(20), "{label}");
            thread::sleep(Duration::from_millis(1));
        }
    }
    if cfg!(debug_assertions) {
        panic!("Run with cargo test --release");
    }
    let h = Harness::new();
    let projects = h.root.join("projects");
    for i in 0..100 {
        fs::create_dir_all(projects.join(format!("Project{i:03}"))).unwrap();
    }
    for args in [["config", "agent", "shell"], ["config", "show", ""]] {
        assert!(
            h.command()
                .args(args.into_iter().filter(|s| !s.is_empty()))
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        h.command()
            .args(["config", "root"])
            .arg(&projects)
            .output()
            .unwrap()
            .status
            .success()
    );
    let bench = h.command().arg("bench").output().unwrap();
    assert!(bench.status.success());
    let discovery: serde_json::Value = serde_json::from_slice(&bench.stdout).unwrap();
    assert_eq!(discovery["projects"], 100);
    let mut workspaces = Vec::new();
    for i in 0..4 {
        let project = projects.join(format!("Project{i:03}"));
        match i {
            0 => fs::write(project.join("Bench.rs"), "fn main() { println!(\"中文\"); }\n".repeat(1000)).unwrap(),
            1 => fs::write(project.join("Bench.md"), "# Markdown benchmark\n\n| Key | Value |\n| --- | --- |\n| A | 中文 |\n\n```rust\nfn main() {}\n```\n".repeat(100)).unwrap(),
            2 => fs::write(project.join("Bench.txt"), format!("{}\n", "x".repeat(126)).repeat(4000)).unwrap(),
            _ => for n in 0..10_000 { fs::write(project.join(format!("node{n:05}.txt")), "fixture").unwrap(); },
        }
        let out = h
            .command()
            .args([
                "--headless",
                "--agent",
                "shell",
                "--no-profile",
                "--layout",
                "split",
                "--project",
            ])
            .arg(projects.join(format!("Project{i:03}")))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        workspaces.push(serde_json::from_slice::<WorkspaceInfo>(&out.stdout).unwrap());
    }
    wait("eight shells", || {
        workspaces
            .iter()
            .all(|w| h.info(&w.id).sessions.iter().all(|s| s.shell_pid.is_some()))
    });
    let mut views = Vec::new();
    let mut warm_ms = Vec::new();
    let mut pids = Vec::new();
    for w in &workspaces {
        let start = Instant::now();
        let view = Frontend::start(&h, &["--workspace", &w.id]);
        frame_wait("warm frame", || view.contents().contains("Ctrl+G"));
        warm_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        pids.push(view.child.process_id().unwrap());
        views.push(view);
    }
    let start = Instant::now();
    let overview = Frontend::start(&h, &[]);
    frame_wait("100 project overview", || {
        overview.contents().contains("Project000")
    });
    let overview_ms = start.elapsed().as_secs_f64() * 1000.0;
    pids.push(overview.child.process_id().unwrap());
    let start = Instant::now();
    overview
        .writer
        .lock()
        .unwrap()
        .write_all(b"Project099")
        .unwrap();
    frame_wait("search frame", || {
        let s = overview.contents();
        s.contains("Project099") && !s.contains("Project000")
    });
    let search_frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    let Response::Host { pid } = h.rpc(Request::Ping).unwrap() else {
        panic!()
    };
    pids.push(pid);
    pids.sort();
    pids.dedup();
    thread::sleep(Duration::from_secs(2));
    let (before, _, _) = process_metrics(&pids);
    let sample = Instant::now();
    thread::sleep(Duration::from_secs(10));
    let (after, working, private) = process_metrics(&pids);
    let cpu_one_core =
        (after - before) as f64 / 10_000_000.0 / sample.elapsed().as_secs_f64() * 100.0;
    let cores = std::thread::available_parallelism().unwrap().get();
    let mut panel_ms = Vec::new();
    for (i, view) in views.iter().enumerate().take(3) {
        let name = ["Bench.rs", "Bench.md", "Bench.txt"][i];
        let begin = Instant::now();
        view.writer
            .lock()
            .unwrap()
            .write_all(format!("\x07f/{name}\r\r").as_bytes())
            .unwrap();
        frame_wait("document ready", || {
            view.contents().contains("read-only")
                && view.contents().contains(if i == 0 {
                    "fn main"
                } else if i == 1 {
                    "Markdown benchmark"
                } else {
                    "xxxxxxxx"
                })
        });
        panel_ms.push(begin.elapsed().as_secs_f64() * 1000.0);
    }
    views[2].writer.lock().unwrap().write_all(b"e").unwrap();
    frame_wait("near limit editor", || views[2].contents().contains("EDIT"));
    let input_start = Instant::now();
    views[2].writer.lock().unwrap().write_all(b"Z").unwrap();
    frame_wait("near limit edit visible", || {
        views[2].contents().contains("Zxxx")
    });
    let editor_input_ms = input_start.elapsed().as_secs_f64() * 1000.0;
    let undo_start = Instant::now();
    views[2].writer.lock().unwrap().write_all(b"\x1a").unwrap();
    frame_wait("undo visible", || !views[2].contents().contains("Zxxx"));
    let editor_undo_ms = undo_start.elapsed().as_secs_f64() * 1000.0;
    let search_start = Instant::now();
    views[2]
        .writer
        .lock()
        .unwrap()
        .write_all(b"\x06xxx")
        .unwrap();
    frame_wait("editor search visible", || {
        views[2].contents().contains("Find: xxx")
    });
    let editor_search_ms = search_start.elapsed().as_secs_f64() * 1000.0;
    thread::sleep(Duration::from_secs(2));
    let (_, panel_working, panel_private) = process_metrics(&pids);
    views[2].writer.lock().unwrap().write_all(b"\x1b").unwrap();
    frame_wait("close edit search", || {
        !views[2].contents().contains("Find: xxx")
    });
    views[2].writer.lock().unwrap().write_all(b"Z").unwrap();
    frame_wait("save candidate", || views[2].contents().contains("Zxxx"));
    let save_start = Instant::now();
    views[2].writer.lock().unwrap().write_all(b"\x13").unwrap();
    let mut save_peak = panel_working;
    frame_wait("asynchronous save", || {
        save_peak = save_peak.max(process_metrics(&pids).1);
        views[2].contents().contains("Saved")
            && fs::read(projects.join("Project002/Bench.txt")).is_ok_and(|b| b.starts_with(b"Zxxx"))
    });
    let save_ms = save_start.elapsed().as_secs_f64() * 1000.0;
    views[3].writer.lock().unwrap().write_all(b"\x07b").unwrap();
    frame_wait("sidebar collapsed", || {
        !views[3].contents().contains("Files [F2]")
    });
    let (_, closed_working, _) = process_metrics(&pids);
    let report = serde_json::json!({"projects":100,"live_sessions":8,"visible_session_frontends":4,"overview_frontends":1,
        "own_process_count":pids.len(),"pids":pids,"discovery":discovery,"warm_session_frame_ms":warm_ms,
        "overview_first_project_frame_ms":overview_ms,"search_frame_ms":search_frame_ms,
        "idle_cpu_single_core_percent":cpu_one_core,"idle_cpu_machine_percent":cpu_one_core/cores as f64,
        "logical_processors":cores,"working_set_sum_mib":working as f64/1048576.0,"private_bytes_sum_mib":private as f64/1048576.0,
        "document_open_ms":panel_ms,"editor_input_ms":editor_input_ms,"editor_undo_ms":editor_undo_ms,"editor_search_ms":editor_search_ms,
        "preview_editor_working_set_sum_mib":panel_working as f64/1048576.0,"preview_editor_private_bytes_sum_mib":panel_private as f64/1048576.0,"one_sidebar_closed_working_set_sum_mib":closed_working as f64/1048576.0,
        "save_sampled_peak_working_set_sum_mib":save_peak as f64/1048576.0,"save_visible_ms":save_ms,"edited_file_bytes":508000,
        "excludes":"PowerShell, Agents, Windows Terminal/OpenConsole, compilers and benchmark test runner"});
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts");
    fs::create_dir_all(&artifacts).unwrap();
    fs::write(
        artifacts.join("scale-release.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(
        warm_ms.iter().all(|&v| v <= 200.0),
        "warm frame exceeds 200ms"
    );
    assert!(overview_ms <= 200.0, "overview exceeds 200ms");
    assert!(search_frame_ms <= 50.0, "search exceeds 50ms");
    assert!(cpu_one_core / cores as f64 <= 1.0, "idle CPU exceeds 1%");
    assert!(working <= 100 * 1048576, "own working set exceeds 100MiB");
    assert!(
        panel_working <= 100 * 1048576,
        "preview/editor own working set exceeds 100MiB"
    );
    assert!(
        save_peak <= 100 * 1048576,
        "save working set exceeds 100MiB"
    );
    assert!(
        editor_input_ms <= 50.0 && editor_undo_ms <= 50.0 && editor_search_ms <= 50.0,
        "editor feedback exceeds 50ms"
    );
    drop(overview);
    drop(views);
}

#[test]
fn headless_host_validation_failure_returns_nonzero_and_equals_data_dir_is_honored() {
    let h = Harness::new();
    let alternate = h.root.join("explicit-data");
    let o = h
        .command()
        .arg(format!("--data-dir={}", alternate.display()))
        .args([
            "--headless",
            "--agent",
            "shell",
            "--shell",
            "missing-fixture-shell-0a167b.exe",
            "--project",
        ])
        .arg(&h.root)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("PowerShell not found"));
    assert!(alternate.join("launches.json").is_file());
    assert!(!h.root.join("data/launches.json").exists());
    let _ = h
        .command()
        .env("DEVTABS_HOME", alternate)
        .arg("shutdown")
        .output();
}

#[test]
fn literal_agent_argument_cannot_override_devtabs_data_directory() {
    let h = Harness::new();
    let decoy = h.root.join("decoy-data");
    let arg = format!("--data-dir={}", decoy.display());
    let workspace = h.open(&["--agent-arg", &arg]);
    assert_eq!(workspace.options.agent_args, vec![arg]);
    assert!(h.root.join("data/launches.json").is_file());
    assert!(!decoy.exists());
}

#[test]
fn floating_handoff_preserves_processes_and_rejects_stale_views_and_sizes() {
    use devtabs::presentation::{Placement, Target};
    let h = Harness::new();
    let mut config = devtabs::config::Config::default();
    config
        .projects
        .entry(devtabs::config::key(&h.root))
        .or_default()
        .sessions = vec![devtabs::config::SessionConfig {
        name: "Build".into(),
        command: None,
        directory: None,
    }];
    fs::create_dir_all(h.root.join("data")).unwrap();
    fs::write(
        h.root.join("data/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let work = h.open(&[
        "--debug-command",
        "for($i=0;$i -lt 40;$i++){Write-Output ('CONTINUE-'+$i);Start-Sleep -Milliseconds 50}",
    ]);
    wait("shell identities", || {
        h.info(&work.id)
            .sessions
            .iter()
            .all(|s| s.shell_pid.is_some())
    });
    let original = h.info(&work.id);
    let target = |i: usize| Target {
        workspace: work.id.clone(),
        session: work.sessions[i].id.clone(),
    };
    let attach = |i: usize, client: &str, floating| {
        h.rpc(Request::Attach {
            floating,
            takeover: true,
            workspace: work.id.clone(),
            session: work.sessions[i].id.clone(),
            client: client.into(),
            rows: 24,
            cols: 90,
        })
    };
    let poll = |i: usize, client: &str| {
        h.rpc(Request::Poll {
            workspace: work.id.clone(),
            session: work.sessions[i].id.clone(),
            client: client.into(),
            revision: None,
            history: true,
        })
    };
    let prepare = |i| {
        let Response::FloatPlan(plan) = h.rpc(Request::PrepareFloat { target: target(i) }).unwrap()
        else {
            panic!("plan")
        };
        plan
    };
    attach(0, "main", None).unwrap();
    let a = prepare(0);
    assert_eq!(h.info(&work.id).sessions[0].placement, Placement::Docked);
    assert!(
        poll(0, "main").is_ok(),
        "preparation must preserve the source view"
    );
    let invalid = h.rpc(Request::Attach {
        floating: Some(a.ticket.revision),
        takeover: true,
        workspace: work.id.clone(),
        session: work.sessions[0].id.clone(),
        client: "invalid-size".into(),
        rows: 0,
        cols: 0,
    });
    assert!(invalid.is_err());
    assert!(poll(0, "main").is_ok());
    attach(0, "float-a", Some(a.ticket.revision)).unwrap();
    assert_eq!(h.info(&work.id).sessions[0].placement, Placement::Floating);
    assert!(poll(0, "main").is_err());
    assert!(attach(0, "competing", None).is_err());
    assert!(attach(0, "competing-float", Some(a.ticket.revision)).is_err());
    let b = prepare(1);
    let c = prepare(2);
    h.rpc(Request::CancelFloat {
        revision: b.ticket.revision,
    })
    .unwrap();
    assert!(attach(1, "stale", Some(b.ticket.revision)).is_err());
    assert!(
        poll(0, "float-a").is_ok(),
        "pending switch keeps the old floating view until commit"
    );
    attach(2, "float-c", Some(c.ticket.revision)).unwrap();
    assert_eq!(h.info(&work.id).sessions[0].placement, Placement::Collapsed);
    assert!(poll(0, "float-a").is_err());
    assert!(
        h.rpc(Request::Resize {
            workspace: work.id.clone(),
            session: work.sessions[0].id.clone(),
            client: "main".into(),
            rows: 4,
            cols: 5
        })
        .is_err()
    );
    assert!(
        h.rpc(Request::Input {
            workspace: work.id.clone(),
            session: work.sessions[0].id.clone(),
            client: "float-a".into(),
            bytes: b"SHOULD-NOT-ARRIVE".to_vec()
        })
        .is_err()
    );
    for layout in [Layout::Float, Layout::Tabs, Layout::Split] {
        h.rpc(Request::Layout {
            workspace: work.id.clone(),
            layout,
        })
        .unwrap();
        assert!(
            poll(2, "float-c").is_ok(),
            "layout changes must preserve the floating lease"
        );
    }
    h.rpc(Request::Present {
        target: target(2),
        placement: Placement::Collapsed,
    })
    .unwrap();
    assert!(poll(2, "float-c").is_err());
    h.rpc(Request::Present {
        target: target(1),
        placement: Placement::Collapsed,
    })
    .unwrap();
    wait("hidden command completes", || {
        matches!(
            h.info(&work.id).sessions[1].command,
            CommandStatus::Exited(0)
        )
    });
    h.rpc(Request::Present {
        target: target(1),
        placement: Placement::Docked,
    })
    .unwrap();
    attach(1, "docked", None).unwrap();
    let Response::Update(Some(update)) = poll(1, "docked").unwrap() else {
        panic!("update")
    };
    assert!(String::from_utf8_lossy(&update.screen).contains("CONTINUE-39"));
    let again = h.info(&work.id);
    assert_eq!(
        again
            .sessions
            .iter()
            .map(|s| s.shell_pid)
            .collect::<Vec<_>>(),
        original
            .sessions
            .iter()
            .map(|s| s.shell_pid)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        again.sessions.iter().map(|s| &s.id).collect::<Vec<_>>(),
        original.sessions.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    let d = prepare(0);
    attach(0, "crashed-float", Some(d.ticket.revision)).unwrap();
    h.rpc(Request::Detach {
        client: "crashed-float".into(),
    })
    .unwrap();
    assert_eq!(h.info(&work.id).sessions[0].placement, Placement::Collapsed);
    assert!(matches!(
        h.rpc(Request::FloatingTarget).unwrap(),
        Response::Floating(None)
    ));
}

#[test]
fn terminal_panel_stays_in_one_frontend_and_routes_geometry_and_input() {
    use devtabs::presentation::Placement;
    let h = Harness::new();
    let work = h.open(&[]);
    let main = Frontend::start(&h, &["--workspace", &work.id, "--language", "en-us"]);
    let keys = |f: &Frontend, bytes: &[u8]| f.writer.lock().unwrap().write_all(bytes).unwrap();
    wait("main ready", || main.contents().contains("Ctrl+G"));
    let before = h
        .info(&work.id)
        .sessions
        .iter()
        .map(|s| (s.id.clone(), s.shell_pid))
        .collect::<Vec<_>>();
    keys(&main, b"\x07d\x07wf");
    wait("inline Debug", || {
        h.info(&work.id).sessions[1].placement == Placement::Floating
    });
    wait("panel drawn in the same frontend", || {
        main.contents().contains("[Dock]")
    });
    assert!(main.contents().contains("Agent"));
    keys(&main, b"Write-Output 'PANEL-INPUT-OK'\r");
    wait("panel command completed", || {
        main.contents().contains("PANEL-INPUT-OK")
    });
    // Enlarging Agent temporarily covers the floating Debug without detaching it.
    keys(&main, b"\x07a\x07z");
    wait("Agent enlarged", || !main.contents().contains("[Dock]"));
    keys(&main, b"\x07z\x07d");
    wait("Debug restored after Agent enlargement", || {
        main.contents().contains("[Dock]")
    });
    let restored = h.root.join("restored-float-input");
    keys(
        &main,
        format!("[IO.File]::WriteAllText({},'ok')\r", quoted(&restored)).as_bytes(),
    );
    wait("restored Debug still accepts input", || restored.exists());
    // Default 93 x 23 panel above the one-line footer: (23, 6).
    // SGR mouse coordinates are one based; drag title five cells right/two down.
    keys(&main, b"\x1b[<0;26;7M\x1b[<32;31;9M\x1b[<0;31;9m");
    let geometry = || {
        fs::read(h.root.join("data/terminal-panel.json"))
            .ok()
            .and_then(|v| serde_json::from_slice::<[u16; 4]>(&v).ok())
    };
    wait("drag saved cell coordinates", || {
        geometry().is_some_and(|r| r[0] == 28 && r[1] == 8)
    });
    let moved = geometry().unwrap();
    assert_eq!((moved[2], moved[3]), (93, 23));
    keys(&main, b"\x1b[<0;121;31M\x1b[<32;116;28M\x1b[<0;116;28m");
    wait("resize saved", || {
        geometry().is_some_and(|r| r[2] == 88 && r[3] == 20)
    });
    // Keyboard geometry mode consumes arrows instead of sending them to shell.
    keys(&main, b"\x07wm\x1b[D\r");
    wait("keyboard move saved", || {
        geometry().is_some_and(|r| r[0] == 27)
    });
    assert_eq!(
        before,
        h.info(&work.id)
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.shell_pid))
            .collect::<Vec<_>>()
    );
    keys(&main, b"\x07wc");
    wait("panel collapsed", || {
        h.info(&work.id).sessions[1].placement == Placement::Collapsed
    });
    wait("main stays visible", || main.contents().contains("Agent"));
    keys(&main, b"\x07d");
    wait("same panel restored", || main.contents().contains("[Dock]"));
    assert_eq!(geometry().unwrap()[0], 27);
    // A different project takes the shared target; the first frontend survives.
    let second = h.open(&["--new-instance"]);
    let other = Frontend::start(&h, &["--workspace", &second.id, "--language", "en-us"]);
    wait("second main", || other.contents().contains("Ctrl+G"));
    keys(&other, b"\x07d\x07wf");
    wait("shared panel moved project", || {
        h.info(&second.id).sessions[1].placement == Placement::Floating
            && h.info(&work.id).sessions[1].placement == Placement::Collapsed
    });
    keys(&main, b"\x07a");
    let marker = h.root.join("source-input-ok");
    keys(
        &main,
        format!("[IO.File]::WriteAllText({},'ok')\r", quoted(&marker)).as_bytes(),
    );
    wait("source frontend still interactive", || marker.exists());
    drop(other);
    wait("crashed main releases panel", || {
        h.info(&second.id).sessions[1].placement == Placement::Collapsed
    });
    // Dock, then collapse every managed terminal: workspace remains alive.
    keys(&main, b"\x07d\x07wd");
    wait("dock restored", || {
        h.info(&work.id).sessions[1].placement == Placement::Docked
    });
    keys(&main, b"\x07wc");
    wait("Debug hidden", || {
        h.info(&work.id).sessions[1].placement == Placement::Collapsed
    });
    keys(&main, b"\x07wc");
    wait("all hidden", || {
        h.info(&work.id)
            .sessions
            .iter()
            .all(|s| s.placement == Placement::Collapsed)
    });
    wait("empty workspace controls", || {
        main.contents().contains("All terminals are hidden")
    });
    assert_eq!(
        before,
        h.info(&work.id)
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.shell_pid))
            .collect::<Vec<_>>()
    );
    keys(&main, b"\x07wd");
    wait("menu usable after all hidden", || {
        h.info(&work.id).sessions[0].placement == Placement::Docked
    });
    keys(&main, b"\x07x");
}

#[test]
fn default_float_layout_uses_the_current_frontend() {
    use devtabs::presentation::Placement;
    let h = Harness::new();
    let work = h.open(&[]);
    h.rpc(Request::Layout {
        workspace: work.id.clone(),
        layout: Layout::Float,
    })
    .unwrap();
    let main = Frontend::start(&h, &["--workspace", &work.id, "--language", "en-us"]);
    wait("default inline Debug", || {
        h.info(&work.id).sessions[1].placement == Placement::Floating
    });
    wait("inline controls on main", || {
        main.contents().contains("[Dock]")
    });
    for layout in [Layout::Split, Layout::Tabs, Layout::Float] {
        main.writer.lock().unwrap().write_all(b"\x07l").unwrap();
        wait("inline layout switch", || {
            h.info(&work.id).options.layout == layout
        });
        assert_eq!(h.info(&work.id).sessions[1].placement, Placement::Floating);
    }
    main.writer
        .lock()
        .unwrap()
        .write_all(b"\x07d\x07wc")
        .unwrap();
    wait("collapse leaves main", || {
        h.info(&work.id).sessions[1].placement == Placement::Collapsed
    });
    drop(main);
    let reconnect = Frontend::start(&h, &["--workspace", &work.id, "--language", "en-us"]);
    wait("reconnect main", || reconnect.contents().contains("Ctrl+G"));
    assert_eq!(
        h.info(&work.id).sessions[1].placement,
        Placement::Collapsed,
        "reconnect must not undo intentional collapse"
    );
}

#[test]
#[ignore = "release-only 10-second expanded/collapsed terminal performance comparison"]
fn collapsed_terminal_output_workload() {
    use devtabs::presentation::{Placement, Target};
    use std::sync::atomic::Ordering;
    let h = Harness::new();
    let marker = h.root.join("producer-complete");
    let source = format!(
        "for($i=0;$i -lt 300;$i++){{Write-Output ('ACTIVE-'+$i);Start-Sleep -Milliseconds 100}};[IO.File]::WriteAllText({},'done')",
        quoted(&marker)
    );
    let work = h.open(&["--debug-command", &source]);
    let front = Frontend::start(&h, &["--workspace", &work.id, "--language", "en-us"]);
    wait("frontend active output", || {
        front.contents().contains("ACTIVE-")
    });
    let Response::Host { pid } = h.rpc(Request::Ping).unwrap() else {
        panic!("host")
    };
    let pids = [pid, front.child.process_id().unwrap()];
    let sample = || {
        let (cpu0, _, _) = process_metrics(&pids);
        let bytes0 = front.received.load(Ordering::Relaxed);
        let start = Instant::now();
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(100));
        }
        let (cpu, working, private) = process_metrics(&pids);
        serde_json::json!({"wall_ms":start.elapsed().as_millis(),"cpu_ms":(cpu-cpu0) as f64 /10000.0,"frontend_bytes":front.received.load(Ordering::Relaxed)-bytes0,"working_set_bytes":working,"private_bytes":private})
    };
    let visible = sample();
    h.rpc(Request::Present {
        target: Target {
            workspace: work.id.clone(),
            session: work.sessions[1].id.clone(),
        },
        placement: Placement::Collapsed,
    })
    .unwrap();
    wait("collapsed state delivered", || {
        front.contents().contains("Collapsed")
    });
    let hidden = sample();
    assert!(
        !marker.exists(),
        "producer must continue during both measurements"
    );
    assert!(matches!(
        h.info(&work.id).sessions[1].command,
        CommandStatus::Running
    ));
    assert!(
        hidden["frontend_bytes"].as_u64().unwrap() < visible["frontend_bytes"].as_u64().unwrap()
    );
    let report = serde_json::json!({"visible":visible,"collapsed":hidden,"producer_running":true,"memory_scope":"host and frontend only; excludes PowerShell, Agent and Windows Terminal","samples_seconds":10});
    fs::write(
        "artifacts/terminal-panel-output-benchmark.json",
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
}
