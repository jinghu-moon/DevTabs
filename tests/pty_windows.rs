#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devtabs::{
    input,
    launch::encode_script,
    session::{Session, Spawn, Status},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, STILL_ACTIVE},
    System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

struct Workspace(PathBuf);
impl Workspace {
    fn new(label: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "DevTabs 测试 {label} {} {stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_devtabs"), path.join("devtabs.exe")).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        if self.file("data").exists() {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };
            let rpc = |request: devtabs::protocol::Request| -> Option<devtabs::protocol::Response> {
                let mut child = Command::new(self.file("devtabs.exe"))
                    .arg("__rpc")
                    .env("DEVTABS_HOME", self.file("data"))
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                child
                    .stdin
                    .take()?
                    .write_all(&serde_json::to_vec(&request).ok()?)
                    .ok()?;
                let out = child.wait_with_output().ok()?;
                serde_json::from_slice(&out.stdout).ok()
            };
            if let Some(devtabs::protocol::Response::Workspaces(work)) =
                rpc(devtabs::protocol::Request::List)
            {
                for w in work {
                    let _ = rpc(devtabs::protocol::Request::Close { workspace: w.id });
                }
                let _ = rpc(devtabs::protocol::Request::Shutdown);
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

fn shell(work: &Workspace, id: usize, source: &str, interactive: bool) -> Session {
    let mut args = vec!["-NoLogo".into(), "-NoProfile".into()];
    if interactive {
        args.push("-NoExit".into());
    }
    args.extend(["-EncodedCommand".into(), encode_script(source)]);
    spawn(work, id, Path::new("pwsh.exe"), &args)
}

fn spawn(work: &Workspace, id: usize, program: &Path, args: &[String]) -> Session {
    let mut args = args.to_vec();
    let local = work.file("devtabs.exe");
    let program = if program.file_stem().is_some_and(|name| name == "devtabs") {
        local.as_path()
    } else {
        program
    };
    if program.file_stem().is_some_and(|name| name == "devtabs") {
        args.extend(["--data-dir".into(), work.file("data").display().to_string()]);
    }
    Session::spawn(Spawn {
        id,
        executable: &local,
        shell: program,
        args: &args,
        project: &work.0,
        rows: 36,
        cols: 140,
        history: 5000,
    })
    .unwrap()
}

fn until(label: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < timeout, "Timed out: {label}");
        thread::sleep(Duration::from_millis(25));
    }
}
fn wait_file(file: &Path) -> String {
    until(&file.display().to_string(), Duration::from_secs(20), || {
        fs::read_to_string(file).is_ok_and(|s| !s.trim().is_empty())
    });
    fs::read_to_string(file).unwrap()
}
fn command(session: &Session, text: &str) {
    // Encode through the exact keyboard path used by the application, including
    // ConPTY's win32-input-mode when PowerShell requests it.
    let win32 = session.state().terminal.parser.callbacks().win32_input;
    let mut bytes = Vec::new();
    for character in text.chars() {
        let event = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);
        bytes.extend(input::key(event, false, win32));
        if win32 {
            let mut release = event;
            release.kind = crossterm::event::KeyEventKind::Release;
            bytes.extend(input::key(release, false, true));
        }
    }
    bytes.extend(input::key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        false,
        win32,
    ));
    session.send(bytes).unwrap();
}
fn press(session: &Session, code: KeyCode, modifiers: KeyModifiers) {
    let win32 = session.state().terminal.parser.callbacks().win32_input;
    let event = KeyEvent::new(code, modifiers);
    let mut bytes = input::key(event, false, win32);
    if win32 {
        let mut release = event;
        release.kind = crossterm::event::KeyEventKind::Release;
        bytes.extend(input::key(release, false, true));
    }
    session.send(bytes).unwrap();
}
fn paste_console(session: &Session, text: &str) {
    // WT's Windows input mode delivers UTF-16 console records, including zero-
    // width codepoints. Raw VT UTF-8 through OpenConsole can discard these.
    let mut bytes = Vec::new();
    for ch in format!("\x1b[200~{text}\x1b[201~").chars() {
        bytes.extend(input::key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            false,
            true,
        ));
    }
    session.send(bytes).unwrap();
}

#[test]
fn nested_sidebar_editor_saves_once_and_guards_detach_without_terminal_leaks() {
    let work = Workspace::new("edit-ui");
    let path = work.file("EditCase.md");
    fs::write(&path, "# Original\n").unwrap();
    let args = vec![
        "--layout".into(),
        "split".into(),
        "--agent".into(),
        "shell".into(),
        "--no-profile".into(),
        "--project".into(),
        work.0.display().to_string(),
    ];
    let mut outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
    let screen = || outer.state().terminal.screen().contents();
    until("file tree and shells", Duration::from_secs(20), || {
        screen().contains("EditCase.md") && screen().matches("PS ").count() >= 2
    });
    outer.send(b"\x07f/EditCase.md".to_vec()).unwrap();
    until("tree query", Duration::from_secs(10), || {
        screen().contains("1 matches")
    });
    press(&outer, KeyCode::Enter, KeyModifiers::NONE);
    press(&outer, KeyCode::Enter, KeyModifiers::NONE);
    until("read only preview", Duration::from_secs(10), || {
        screen().contains("[More]") && screen().contains("Original") && !screen().contains("· EDIT")
    });
    outer.send(b"e".to_vec()).unwrap();
    until("explicit editor", Duration::from_secs(10), || {
        screen().contains("EDIT")
    });
    let inserted = "中文 M/O/I/E . / 👩‍💻 e\u{301}\n";
    // The same bracketed paste event twice must remain two edits.
    for _ in 0..2 {
        paste_console(&outer, inserted);
    }
    until("unsaved editor", Duration::from_secs(10), || {
        screen().contains("* · EDIT")
    });
    press(&outer, KeyCode::Char('s'), KeyModifiers::CONTROL);
    until("disk saved", Duration::from_secs(10), || {
        let disk = fs::read_to_string(&path).unwrap();
        if disk != "# Original\n" && disk != format!("{inserted}{inserted}# Original\n") {
            panic!("Saved unexpected bytes: {disk:?}\n{}", screen());
        }
        if screen().contains("Save failed") {
            panic!("{}\nDisk: {:?}", screen(), fs::read_to_string(&path));
        }
        fs::read_to_string(&path).is_ok_and(|s| s == format!("{inserted}{inserted}# Original\n"))
    });
    outer.send(b"\x07a".to_vec()).unwrap();
    command(
        &outer,
        &format!(
            "'NO-LEAK' | Set-Content -LiteralPath {}",
            quote(&work.file("agent-check"))
        ),
    );
    assert_eq!(wait_file(&work.file("agent-check")).trim(), "NO-LEAK");
    outer.send(b"\x07o".to_vec()).unwrap();
    outer.send(b"e".to_vec()).unwrap();
    until("return editor", Duration::from_secs(10), || {
        screen().contains("EDIT")
    });
    outer
        .send(b"\x1b[200~discard-me\x1b[201~".to_vec())
        .unwrap();
    outer.send(b"\x07x".to_vec()).unwrap();
    until("detach guard", Duration::from_secs(10), || {
        screen().contains("Unsaved changes")
    });
    press(&outer, KeyCode::Esc, KeyModifiers::NONE);
    until("cancel detach", Duration::from_secs(10), || {
        !screen().contains("Unsaved changes") && screen().contains("EDIT")
    });
    // Release closure borrow before resize.
    outer.resize(5, 40).unwrap();
    until("small editor window", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("enlarge")
    });
    outer.resize(36, 140).unwrap();
    until("editor restored", Duration::from_secs(10), || {
        outer.state().terminal.screen().contents().contains("EDIT")
    });
    outer.send(b"\x07x".to_vec()).unwrap();
    until("detach guard again", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Unsaved changes")
    });
    outer.send(b"d".to_vec()).unwrap();
    until("discard detaches frontend", Duration::from_secs(10), || {
        !outer.running()
    });
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        format!("{inserted}{inserted}# Original\n")
    );
}
fn alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

fn result_file(session: &Session, file: &Path) -> String {
    let start = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(file)
            && !text.trim().is_empty()
        {
            return text;
        }
        if start.elapsed() >= Duration::from_secs(15) {
            let state = session.state();
            panic!(
                "No result {}. win32={} status={:?} error={:?}\n{}",
                file.display(),
                state.terminal.parser.callbacks().win32_input,
                state.status,
                state.io_error,
                state.terminal.screen().contents()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn real_shells_isolate_input_resize_and_exit() {
    let work = Workspace::new("shells");
    let boot = |name: &str| {
        format!(
            "@($PID, $PWD.Path) | Set-Content -LiteralPath {}",
            quote(&work.file(name))
        )
    };
    let mut left = shell(&work, 0, &boot("left-boot"), true);
    let right = shell(&work, 1, &boot("right-boot"), true);
    let a = wait_file(&work.file("left-boot"));
    let b = wait_file(&work.file("right-boot"));
    until("left prompt", Duration::from_secs(10), || {
        left.state().terminal.screen().contents().contains("PS ")
    });
    until("right prompt", Duration::from_secs(10), || {
        right.state().terminal.screen().contents().contains("PS ")
    });
    assert_ne!(a.lines().next(), b.lines().next());
    assert!(a.contains(work.0.to_str().unwrap()));
    assert!(b.contains(work.0.to_str().unwrap()));
    let left_pid = left.state().shell_pid;
    for (rows, cols) in [(10, 60), (50, 180), (36, 140)] {
        left.resize(rows, cols).unwrap();
    }
    assert_eq!(left.state().shell_pid, left_pid);
    assert!(left.resize(0, 0).is_err());
    command(
        &left,
        &format!(
            "'左侧 ✓' | Set-Content -LiteralPath {}",
            quote(&work.file("left-result"))
        ),
    );
    command(
        &right,
        &format!(
            "'右侧 ✓' | Set-Content -LiteralPath {}",
            quote(&work.file("right-result"))
        ),
    );
    assert_eq!(
        result_file(&left, &work.file("left-result")).trim(),
        "左侧 ✓"
    );
    assert_eq!(wait_file(&work.file("right-result")).trim(), "右侧 ✓");
    command(&left, "exit 7");
    until("left exit", Duration::from_secs(10), || !left.running());
    assert_eq!(left.state().status, Status::Exited(7));
    assert!(right.running());
    command(
        &right,
        &format!(
            "'still-alive' | Set-Content -LiteralPath {}",
            quote(&work.file("right-alive"))
        ),
    );
    assert_eq!(wait_file(&work.file("right-alive")).trim(), "still-alive");
    command(&right, "exit");
    until("right exit", Duration::from_secs(10), || !right.running());
}

#[test]
fn job_drop_cleans_descendants_after_shell_exits() {
    let work = Workspace::new("cleanup");
    let child_pid_file = work.file("child-pid");
    let source = format!(
        "$child = Start-Process pwsh -ArgumentList '-NoProfile','-Command','Start-Sleep 120' -PassThru; $child.Id | Set-Content -LiteralPath {}; exit 0",
        quote(&child_pid_file)
    );
    let session = shell(&work, 0, &source, false);
    let pid = wait_file(&child_pid_file).trim().parse().unwrap();
    until("shell exit", Duration::from_secs(10), || !session.running());
    assert!(alive(pid), "fixture descendant should outlive its shell");
    drop(session);
    until("owned descendant cleanup", Duration::from_secs(10), || {
        !alive(pid)
    });
    assert!(alive(std::process::id()), "test runner must remain alive");
}

#[test]
fn nested_tui_routes_focus_and_survives_small_window() {
    let work = Workspace::new("nested");
    // These are two actual interactive PowerShell instances inside the actual
    // DevTabs executable, itself hosted by an outer ConPTY test harness.
    let args = vec![
        "--layout".into(),
        "split".into(),
        "--agent".into(),
        "shell".into(),
        "--no-profile".into(),
        "--project".into(),
        work.0.display().to_string(),
    ];
    let mut outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
    let start = Instant::now();
    while !outer
        .state()
        .terminal
        .screen()
        .contents()
        .contains("Ctrl+G")
    {
        if start.elapsed() > Duration::from_secs(20) {
            let state = outer.state();
            panic!(
                "TUI initial frame; status={:?} error={:?}\n{}",
                state.status,
                state.io_error,
                state.terminal.screen().contents()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    // Wait for both PSReadLine prompts before injecting input.
    until("both inner prompts", Duration::from_secs(20), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .matches("PS ")
            .count()
            >= 2
    });
    command(
        &outer,
        &format!(
            "$PID | Set-Content -LiteralPath {}",
            quote(&work.file("agent-pid"))
        ),
    );
    let agent_pid: u32 = result_file(&outer, &work.file("agent-pid"))
        .trim()
        .parse()
        .unwrap();
    outer.send(b"\x07d".to_vec()).unwrap();
    command(
        &outer,
        &format!(
            "$PID | Set-Content -LiteralPath {}",
            quote(&work.file("debug-pid"))
        ),
    );
    let debug_pid: u32 = wait_file(&work.file("debug-pid")).trim().parse().unwrap();
    assert_ne!(agent_pid, debug_pid);
    outer.resize(5, 40).unwrap();
    until("small-window message", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("enlarge")
    });
    thread::sleep(Duration::from_millis(700));
    assert!(outer.running(), "small terminals must not detach or exit");
    outer.resize(40, 160).unwrap();
    until("restored layout", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Ctrl+G")
    });
    command(
        &outer,
        &format!(
            "$PID | Set-Content -LiteralPath {}",
            quote(&work.file("debug-after"))
        ),
    );
    assert_eq!(
        wait_file(&work.file("debug-after"))
            .trim()
            .parse::<u32>()
            .unwrap(),
        debug_pid
    );
    outer.send(b"\x07q".to_vec()).unwrap();
    until("close confirmation", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Close DevTabs workspace")
    });
    outer
        .send(input::key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            false,
            true,
        ))
        .unwrap();
    until("cancel close", Duration::from_secs(10), || {
        !outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Close DevTabs workspace")
    });
    assert!(alive(agent_pid) && alive(debug_pid));
    outer.send(b"\x07a".to_vec()).unwrap();
    command(&outer, "exit");
    until("left exit shown", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("exited 0")
    });
    assert!(alive(debug_pid));
    outer.send(b"\x07d".to_vec()).unwrap();
    command(&outer, "exit");
    until("TUI clean exit", Duration::from_secs(10), || {
        !outer.running()
    });
    assert_eq!(outer.state().status, Status::Exited(0));
    until("both inner shells exited", Duration::from_secs(10), || {
        !alive(agent_pid) && !alive(debug_pid)
    });
}

#[test]
fn output_flood_does_not_block_other_shell_or_grow_history() {
    let work = Workspace::new("flood");
    let flooding = shell(
        &work,
        0,
        "for ($i=0;$i -lt 12000;$i++) { [Console]::WriteLine(('LOG-{0:D5} 中文' -f $i)) }; [Console]::WriteLine('FLOOD-COMPLETE')",
        true,
    );
    let interactive = shell(&work, 1, "[Console]::WriteLine('INPUT-READY')", true);
    until("input shell ready", Duration::from_secs(10), || {
        interactive
            .state()
            .terminal
            .screen()
            .contents()
            .contains("PS ")
    });
    let started = Instant::now();
    command(
        &interactive,
        &format!(
            "'responsive' | Set-Content -LiteralPath {}",
            quote(&work.file("responsive"))
        ),
    );
    assert_eq!(
        result_file(&interactive, &work.file("responsive")).trim(),
        "responsive"
    );
    println!(
        "Input completion during output flood: {} ms",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    loop {
        if flooding
            .state()
            .terminal
            .screen()
            .contents()
            .contains("FLOOD-COMPLETE")
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let state = flooding.state();
            panic!(
                "Flood stalled: {:?} {:?}\n{}",
                state.status,
                state.io_error,
                state.terminal.screen().contents()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(flooding.state().terminal.screen().scrollback_len(), 5000);
    assert!(flooding.state().io_error.is_none());
    command(&flooding, "exit");
    command(&interactive, "exit");
}

#[test]
#[ignore = "runs the current user's real PowerShell Profile; never edits it"]
fn personal_profile_debug_startup_and_force_close() {
    let work = Workspace::new("profile");
    let marker = work.file("profile-ready");
    let source = format!("$PID | Set-Content -LiteralPath {}", quote(&marker));
    let args = vec![
        "--layout".into(),
        "split".into(),
        "--agent".into(),
        "shell".into(),
        "--project".into(),
        work.0.display().to_string(),
        "--debug-command".into(),
        source,
    ];
    let outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
    let debug_pid: u32 = result_file(&outer, &marker).trim().parse().unwrap();
    // After the startup command, Debug must still be an interactive shell.
    until("profile prompts", Duration::from_secs(30), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Ctrl+G")
    });
    outer.send(b"\x07d".to_vec()).unwrap();
    command(
        &outer,
        &format!(
            "'after-command' | Set-Content -LiteralPath {}",
            quote(&work.file("after-command"))
        ),
    );
    assert_eq!(
        result_file(&outer, &work.file("after-command")).trim(),
        "after-command"
    );
    outer.send(b"\x07q".to_vec()).unwrap();
    until("force close dialog", Duration::from_secs(10), || {
        outer
            .state()
            .terminal
            .screen()
            .contents()
            .contains("Close DevTabs workspace")
    });
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(3200) {
        thread::sleep(Duration::from_millis(100));
    }
    command(&outer, "f");
    until("forced DevTabs exit", Duration::from_secs(10), || {
        !outer.running()
    });
    until("Debug process terminated", Duration::from_secs(10), || {
        !alive(debug_pid)
    });
    assert_eq!(outer.state().status, Status::Exited(0));
}

#[test]
#[ignore = "requires locally installed Codex/Claude; opens startup screens without submitting prompts"]
fn installed_agents_startup_smoke() {
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts");
    fs::create_dir_all(&artifacts).unwrap();
    for agent in ["codex", "claude"] {
        for resume in [false, true] {
            let work = Workspace::new(&format!("{agent}-{resume}"));
            let mut args = vec![
                "--layout".into(),
                "split".into(),
                "--agent".into(),
                agent.into(),
                "--no-profile".into(),
                "--project".into(),
                work.0.display().to_string(),
            ];
            if resume {
                args.push("--resume".into());
            }
            let outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
            let started = Instant::now();
            let mut screen = loop {
                let state = outer.state();
                let screen = state
                    .terminal
                    .screen()
                    .rows(0, 140)
                    .collect::<Vec<_>>()
                    .join("\n");
                drop(state);
                if started.elapsed() > Duration::from_secs(12) {
                    break screen;
                }
                thread::sleep(Duration::from_millis(100));
            };
            if agent == "codex" {
                outer.send("DevTabs验证".as_bytes().to_vec()).unwrap();
                until("Codex input/search", Duration::from_secs(10), || {
                    outer
                        .state()
                        .terminal
                        .screen()
                        .contents()
                        .contains("DevTabs验证")
                });
            } else {
                outer
                    .send(input::key(
                        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                        false,
                        true,
                    ))
                    .unwrap();
                until("Claude menu movement", Duration::from_secs(10), || {
                    outer.state().terminal.screen().contents().contains("❯ Yes")
                });
                outer
                    .send(input::key(
                        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                        false,
                        true,
                    ))
                    .unwrap();
                until("Claude menu return", Duration::from_secs(10), || {
                    outer.state().terminal.screen().contents().contains("❯ No")
                });
            }
            screen.clear();
            screen.push_str(
                &outer
                    .state()
                    .terminal
                    .screen()
                    .rows(0, 140)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            let name = format!("{agent}-{}", if resume { "resume" } else { "new" });
            fs::write(artifacts.join(format!("{name}.txt")), &screen).unwrap();
            println!("{name}:\n{screen}");
            assert!(outer.running(), "DevTabs unexpectedly exited during {name}");
            assert!(
                !screen.contains("not recognized") && !screen.contains("not digitally signed"),
                "Agent failed to launch: {name}"
            );
            assert!(
                [
                    "trust",
                    "Trust",
                    "Welcome",
                    "OpenAI Codex",
                    "Claude Code",
                    "No sessions",
                    "No conversations",
                    "Sign in",
                    "Select a theme",
                    "Choose the text style",
                    "No matching",
                    "Resume a previous session"
                ]
                .iter()
                .any(|text| screen.contains(text)),
                "No recognizable native Agent startup screen for {name}"
            );
            // No trust acceptance, model request, or user conversation is sent.
        }
    }
}

#[test]
#[ignore = "starts installed Codex in an isolated fixture; inserts a path without submitting a prompt"]
fn native_codex_tree_drag_inserts_once_at_cursor_without_enter() {
    let work = Workspace::new("drop");
    let path = work.file("DropCase.md");
    fs::write(&path, "# Drag fixture\n").unwrap();
    let args = vec![
        "--layout".into(),
        "split".into(),
        "--agent".into(),
        "codex".into(),
        "--no-profile".into(),
        "--project".into(),
        work.0.display().to_string(),
    ];
    let mut outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
    outer.resize(40, 400).unwrap();
    let screen = || {
        outer
            .state()
            .terminal
            .screen()
            .rows(0, 400)
            .collect::<Vec<_>>()
            .join("\n")
    };
    until("native Codex prompt", Duration::from_secs(25), || {
        screen().contains("OpenAI Codex") && screen().contains("DropCase.md")
    });
    outer.send(b"LEFTRIGHT".to_vec()).unwrap();
    until("existing prompt text", Duration::from_secs(10), || {
        screen().contains("LEFTRIGHT")
    });
    for _ in 0..5 {
        press(&outer, KeyCode::Left, KeyModifiers::NONE);
    }
    outer.send(b"\x07f/DropCase.md".to_vec()).unwrap();
    until("loaded name located", Duration::from_secs(10), || {
        screen().contains("1 matches")
    });
    press(&outer, KeyCode::Enter, KeyModifiers::NONE);
    press(&outer, KeyCode::Enter, KeyModifiers::NONE);
    until("preview visible", Duration::from_secs(10), || {
        screen().contains("Drag fixture")
    });
    let row = outer
        .state()
        .terminal
        .screen()
        .rows(0, 28)
        .position(|row| row.contains("DropCase.md"))
        .unwrap()
        + 1;
    outer
        .send(format!("\x1b[<0;9;{row}M\x1b[<32;40;15M").into_bytes())
        .unwrap();
    until("drag captured", Duration::from_secs(10), || {
        screen().contains("Drag to Agent content")
    });
    outer.send(b"\x1b[<0;40;15m".to_vec()).unwrap();
    let expected = format!("LEFT{}RIGHT", devtabs::files::path_input(&path).unwrap());
    until(
        "native path at existing cursor",
        Duration::from_secs(10),
        || screen().contains(&expected),
    );
    let captured = screen();
    assert_eq!(captured.matches(&expected).count(), 1);
    assert!(captured.contains("command Running"));
    assert!(!captured.contains("Working ("));
    fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/codex-file-drop.txt"),
        captured,
    )
    .unwrap();
}

#[test]
#[ignore = "starts installed Claude in this test's generated fixture only; no model prompt is submitted"]
fn native_claude_keyboard_insert_preserves_existing_prompt_without_enter() {
    let work = Workspace::new("claude-drop");
    let path = work.file("DropCase.md");
    fs::write(&path, "# This test generated this file\n").unwrap();
    let args = vec![
        "--layout".into(),
        "split".into(),
        "--agent".into(),
        "claude".into(),
        "--no-profile".into(),
        "--project".into(),
        work.0.display().to_string(),
    ];
    let mut outer = spawn(&work, 0, Path::new(env!("CARGO_BIN_EXE_devtabs")), &args);
    outer.resize(40, 400).unwrap();
    let screen = || {
        outer
            .state()
            .terminal
            .screen()
            .rows(0, 400)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let startup = Instant::now();
    until(
        "Claude test fixture startup",
        Duration::from_secs(25),
        || {
            startup.elapsed() > Duration::from_secs(3)
                && (screen().contains("Yes, I trust this folder")
                    || screen().contains("Claude Code v"))
        },
    );
    if screen().contains("Yes, I trust this folder") {
        // The path is an empty, test-owned temporary directory. Never accept a
        // trust dialog for a user's project or submit a model request here.
        assert!(screen().contains(work.0.to_str().unwrap()));
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/claude-before-trust.txt"),
            screen(),
        )
        .unwrap();
        if !screen().contains("❯ Yes, I trust this folder") {
            outer
                .send(input::key(
                    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                    false,
                    true,
                ))
                .unwrap();
        }
        until("test fixture trust option", Duration::from_secs(10), || {
            fs::write(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/claude-trust-option.txt"),
                screen(),
            )
            .unwrap();
            screen().contains("❯ Yes, I trust this folder")
        });
        outer.send(b"\r".to_vec()).unwrap();
    }
    until("Claude prompt", Duration::from_secs(25), || {
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/claude-after-trust.txt"),
            screen(),
        )
        .unwrap();
        screen().contains("Claude Code v")
            && screen().contains('❯')
            && !screen().contains("Yes, I trust this folder")
    });
    outer.send(b"LEFTRIGHT".to_vec()).unwrap();
    until(
        "Claude existing prompt text",
        Duration::from_secs(10),
        || screen().contains("LEFTRIGHT"),
    );
    for _ in 0..5 {
        press(&outer, KeyCode::Left, KeyModifiers::NONE);
    }
    outer.send(b"\x07f/DropCase.md".to_vec()).unwrap();
    until("Claude selected path", Duration::from_secs(10), || {
        screen().contains("1 matches")
    });
    press(&outer, KeyCode::Enter, KeyModifiers::NONE);
    outer.send(b"\x07i".to_vec()).unwrap();
    let expected = format!("LEFT{}RIGHT", devtabs::files::path_input(&path).unwrap());
    until("Claude native insertion", Duration::from_secs(15), || {
        screen().contains(&expected)
    });
    let captured = screen();
    assert_eq!(captured.matches(&expected).count(), 1);
    fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/claude-file-insert.txt"),
        captured,
    )
    .unwrap();
}
