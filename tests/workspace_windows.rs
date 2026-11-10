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
    child.kill().unwrap();
    child.wait().unwrap();
    drop(pair.master);
    drain.join().unwrap();
    assert_eq!(
        h.info(&id).sessions[0].shell_pid,
        before.sessions[0].shell_pid
    );
    thread::sleep(Duration::from_millis(3200));
    h.rpc(Request::Attach {
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
    wait("named session focus", || {
        view.contents().contains("Docs [focus]")
    });
    assert_eq!(h.info(&w.id).sessions[2].id, w.sessions[2].id);
    assert!(!directory.join(".devtabs").exists());
}

struct Frontend {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    screen: std::sync::Arc<std::sync::Mutex<devtabs::terminal::Terminal>>,
    writer: std::sync::Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
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
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let screen = Arc::new(Mutex::new(devtabs::terminal::Terminal::new(36, 140, 100)));
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let (screen_thread, writer_thread) = (screen.clone(), writer.clone());
        let mut reader = pair.master.try_clone_reader().unwrap();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
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
#[ignore = "opens isolated real Windows Terminal windows to verify layouts and topmost state"]
fn windows_terminal_layouts_preserve_shells_and_debug_window_state() {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::*,
    };
    fn find(title: &str) -> HWND {
        struct Find {
            title: String,
            found: HWND,
        }
        unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
            let find = unsafe { &mut *(param as *mut Find) };
            let mut text = [0u16; 1024];
            let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
            if String::from_utf16_lossy(&text[..len.max(0) as usize]).contains(&find.title) {
                find.found = hwnd;
                return 0;
            }
            1
        }
        let mut find = Find {
            title: title.into(),
            found: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(Some(visit), &mut find as *mut Find as isize);
        }
        find.found
    }
    let h = Harness::new();
    let w = h.open(&[]);
    wait("shells", || {
        h.info(&w.id).sessions.iter().all(|s| s.shell_pid.is_some())
    });
    let before = h
        .info(&w.id)
        .sessions
        .iter()
        .map(|s| s.shell_pid)
        .collect::<Vec<_>>();
    let second = h.open(&["--new-instance"]);
    h.rpc(Request::Layout {
        workspace: second.id.clone(),
        layout: Layout::Float,
    })
    .unwrap();
    let frontend = Frontend::start(&h, &["--workspace", &w.id]);
    wait("split frontend", || frontend.contents().contains("Ctrl+G"));
    let title = format!(
        "DevTabs Debug · {}",
        h.root.file_name().unwrap().to_string_lossy()
    );
    h.rpc(Request::Layout {
        workspace: w.id.clone(),
        layout: Layout::Float,
    })
    .unwrap();
    wait("floating Debug window", || !find(&title).is_null());
    let hwnd = find(&title);
    assert_eq!(
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize,
        0
    );
    frontend.writer.lock().unwrap().write_all(b"\x07t").unwrap();
    wait(
        "topmost Debug",
        || unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize != 0,
    );
    frontend.writer.lock().unwrap().write_all(b"\x07d").unwrap();
    thread::sleep(Duration::from_millis(200));
    assert_ne!(
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize,
        0,
        "restoring Debug preserves topmost"
    );
    h.rpc(Request::DebugTarget {
        workspace: Some(second.id.clone()),
    })
    .unwrap();
    thread::sleep(Duration::from_millis(650));
    h.rpc(Request::Attach {
        workspace: w.id.clone(),
        session: w.sessions[1].id.clone(),
        client: "shared-released-original".into(),
        rows: 24,
        cols: 100,
        takeover: false,
    })
    .unwrap();
    assert!(
        h.rpc(Request::Attach {
            workspace: second.id.clone(),
            session: second.sessions[1].id.clone(),
            client: "competing-shared".into(),
            rows: 24,
            cols: 100,
            takeover: false
        })
        .is_err(),
        "the shared view must now own the selected project's Debug"
    );
    assert_eq!(
        find(&title),
        hwnd,
        "switching projects reuses the same Debug window"
    );
    h.rpc(Request::Detach {
        client: "shared-released-original".into(),
    })
    .unwrap();
    h.rpc(Request::DebugTarget {
        workspace: Some(w.id.clone()),
    })
    .unwrap();
    thread::sleep(Duration::from_millis(650));
    h.rpc(Request::Layout {
        workspace: w.id.clone(),
        layout: Layout::Tabs,
    })
    .unwrap();
    wait("floating view retired", || find(&title).is_null());
    wait("native tabs", || !find(&w.id).is_null());
    h.rpc(Request::Layout {
        workspace: w.id.clone(),
        layout: Layout::Split,
    })
    .unwrap();
    thread::sleep(Duration::from_millis(800));
    h.rpc(Request::Layout {
        workspace: w.id.clone(),
        layout: Layout::Float,
    })
    .unwrap();
    let start = Instant::now();
    while find(&title).is_null() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "Debug restore: {:?}; last error: {:?}",
            h.info(&w.id),
            fs::read_to_string(h.root.join("data/last-error.txt"))
        );
        thread::sleep(Duration::from_millis(50));
    }
    let after = h
        .info(&w.id)
        .sessions
        .iter()
        .map(|s| s.shell_pid)
        .collect::<Vec<_>>();
    assert_eq!(before, after);
    // Closing the independent frontend collapses Debug, leaving its shell alive.
    let debug = find(&title);
    unsafe {
        PostMessageW(debug, WM_CLOSE, 0, 0);
    }
    wait("Debug collapsed", || find(&title).is_null());
    assert_eq!(
        before,
        h.info(&w.id)
            .sessions
            .iter()
            .map(|s| s.shell_pid)
            .collect::<Vec<_>>()
    );
    let main = Frontend::start(&h, &["--workspace", &w.id]);
    wait("collapsed Debug restored", || !find(&title).is_null());
    h.rpc(Request::Close {
        workspace: w.id.clone(),
    })
    .unwrap();
    wait("owned windows closed", || {
        find(&title).is_null() && find(&w.id).is_null()
    });
    drop(main);
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
