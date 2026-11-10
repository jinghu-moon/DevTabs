use crate::{
    config,
    launch::Options,
    launch::encode_script,
    protocol::{self, Request, Response, SessionInfo, Update, WorkspaceInfo},
    session::{Session, Spawn},
};
use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerNonblockingMode, ListenerOptions, prelude::*,
};
use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

struct Managed {
    id: String,
    name: String,
    session: Session,
    owner: Option<(String, Instant)>,
    stop: Option<Instant>,
}
struct Workspace {
    id: String,
    options: Options,
    sessions: Vec<Managed>,
    finished: Option<Instant>,
}
impl Workspace {
    fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo {
            id: self.id.clone(),
            project: self.options.project.clone(),
            options: self.options.clone(),
            sessions: self.sessions.iter().map(Managed::info).collect(),
        }
    }
}
impl Managed {
    fn info(&self) -> SessionInfo {
        self.session.refresh_command();
        let s = self.session.state();
        SessionInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            shell_pid: s.shell_pid,
            status: s.status.clone(),
            command: s.command.clone(),
        }
    }
    fn own(&mut self, client: &str, takeover: bool) -> Result<()> {
        if let Some((id, at)) = &self.owner {
            anyhow::ensure!(
                takeover || id == client || at.elapsed() > Duration::from_secs(3),
                "Session is displayed in another view; close/detach that view first"
            );
        }
        self.owner = Some((client.into(), Instant::now()));
        Ok(())
    }
    fn check(&mut self, client: &str) -> Result<()> {
        anyhow::ensure!(
            self.owner.as_ref().is_some_and(|(id, _)| id == client),
            "This view no longer owns session input"
        );
        self.owner = Some((client.into(), Instant::now()));
        Ok(())
    }
}
#[derive(Default)]
struct Host {
    workspaces: HashMap<String, Workspace>,
    debug: Option<String>,
    shutdown: bool,
}

pub fn ensure() -> Result<()> {
    if protocol::rpc(Request::Ping).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(config::home())?;
    #[cfg(windows)]
    {
        // Do not inherit any client pipe/console handles. A real detached process
        // must also escape a caller's optional Job, not just hide its window.
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW},
        };
        let mut line: Vec<u16> = format!("\"{}\" __serve", std::env::current_exe()?.display())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let mut ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                0x00000008 | 0x00000200 | 0x01000000,
                std::ptr::null(),
                std::ptr::null(),
                &startup,
                &mut process,
            )
        };
        if ok == 0 && std::io::Error::last_os_error().raw_os_error() == Some(5) {
            // Some enclosing Windows hosts forbid job breakaway. Detached/no
            // inherited handles still separates the ordinary client lifetime.
            ok = unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    line.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    0x00000008 | 0x00000200,
                    std::ptr::null(),
                    std::ptr::null(),
                    &startup,
                    &mut process,
                )
            };
        }
        if ok == 0 {
            return Err(std::io::Error::last_os_error()).context("Create detached session host");
        }
        unsafe {
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
        }
    }
    #[cfg(not(windows))]
    std::process::Command::new(std::env::current_exe()?)
        .arg("__serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Start independent session host")?;
    let start = Instant::now();
    loop {
        if protocol::rpc(Request::Ping).is_ok() {
            return Ok(());
        }
        anyhow::ensure!(
            start.elapsed() < Duration::from_secs(10),
            "Session host did not start within 10 seconds"
        );
        thread::sleep(Duration::from_millis(50));
    }
}
pub fn serve() -> Result<()> {
    let name = protocol::endpoint();
    eprintln!("Host {} listening at {}", std::process::id(), name);
    // Default named-pipe DACL comes from this user's token. Application messages
    // are additionally local-only; no TCP listener or remote address is used.
    let options = ListenerOptions::new()
        .name(name.to_ns_name::<GenericNamespaced>()?)
        .nonblocking(ListenerNonblockingMode::Accept);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        let descriptor = widestring::U16CString::from_str(format!(
            "D:P(A;;GA;;;{})(A;;GA;;;SY)",
            crate::platform::user_sid()?
        ))?;
        options.security_descriptor(SecurityDescriptor::deserialize(&descriptor)?)
    };
    let listener = match options.create_sync() {
        Ok(v) => v,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let state = Arc::new(Mutex::new(Host::default()));
    let mut empty = Instant::now();
    let mut last_sweep = Instant::now();
    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    loop {
        handles.retain(|h| !h.is_finished());
        if handles.len() >= 64 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        match listener.accept() {
            Ok(mut stream) => {
                let state = state.clone();
                handles.push(thread::spawn(move || {
                    let result = (|| -> Result<Response> {
                        let request = protocol::receive(&mut stream)?;
                        let mut host = state
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Host state unavailable"))?;
                        host.handle(request)
                    })();
                    if let Err(error) = &result {
                        eprintln!("Host request failed: {error:#}");
                    }
                    let response = result.unwrap_or_else(|e| Response::Error(format!("{e:#}")));
                    let _ = protocol::send(&mut stream, &response);
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        handles.retain(|h| !h.is_finished());
        {
            let mut host = state.lock().unwrap_or_else(|p| p.into_inner());
            if last_sweep.elapsed() >= Duration::from_secs(1) {
                last_sweep = Instant::now();
                host.workspaces.retain(|_, w| {
                    if w.sessions.iter().any(|s| s.session.running()) {
                        w.finished = None;
                        true
                    } else {
                        w.finished.get_or_insert_with(Instant::now).elapsed()
                            < Duration::from_secs(3)
                    }
                });
                if host
                    .debug
                    .as_ref()
                    .is_some_and(|id| !host.workspaces.contains_key(id))
                {
                    host.debug = None;
                }
            }
            if host.shutdown {
                break;
            }
            if !host.workspaces.is_empty() {
                empty = Instant::now();
            } else if empty.elapsed() > Duration::from_secs(30) {
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
impl Host {
    fn managed(&mut self, workspace: &str, session: &str) -> Result<&mut Managed> {
        self.workspaces
            .get_mut(workspace)
            .context("Workspace no longer exists")?
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .context("Session no longer exists")
    }
    fn handle(&mut self, request: Request) -> Result<Response> {
        let _perf = crate::perf::span(crate::perf::Metric::HostRequest);
        match request {
            Request::Layout { workspace, layout } => {
                let work = self
                    .workspaces
                    .get_mut(&workspace)
                    .context("Unknown workspace")?;
                work.options.layout = layout;
                for session in &mut work.sessions {
                    session.owner = None;
                }
                Ok(Response::Ok)
            }
            Request::Ping => Ok(Response::Host {
                pid: std::process::id(),
            }),
            Request::List => Ok(Response::Workspaces(
                self.workspaces.values().map(Workspace::info).collect(),
            )),
            Request::Open { mut options } => {
                if let Some(id) = &options.workspace {
                    return Ok(Response::Workspace(
                        self.workspaces.get(id).context("Unknown workspace")?.info(),
                    ));
                }
                options.validate()?;
                config::Config::update(|c| {
                    c.projects
                        .entry(config::key(&options.project))
                        .or_default()
                        .last_opened = config::now();
                    Ok(())
                })?;
                if !options.new_instance
                    && let Some(work) = self
                        .workspaces
                        .values()
                        .filter(|w| {
                            config::key(&w.options.project) == config::key(&options.project)
                        })
                        .min_by_key(|w| (w.options.new_instance, w.id.clone()))
                {
                    return Ok(Response::Workspace(work.info()));
                }
                let id = uuid::Uuid::new_v4().to_string();
                let mut sessions = Vec::new();
                for index in 0..2 + options.extra_sessions.len() {
                    let sid = uuid::Uuid::new_v4().to_string();
                    let name = if index == 0 {
                        "Agent".into()
                    } else if index == 1 {
                        "Debug".into()
                    } else {
                        options.extra_sessions[index - 2].name.clone()
                    };
                    let directory = if index >= 2 {
                        options.extra_sessions[index - 2]
                            .directory
                            .clone()
                            .unwrap_or_else(|| options.project.clone())
                    } else {
                        options.project.clone()
                    };
                    let directory = if directory.is_relative() {
                        options.project.join(directory)
                    } else {
                        directory
                    };
                    let control = config::home().join("runtime").join(format!("{sid}.json"));
                    let args = managed_args(&options, index, &control)?;
                    let mut session = Session::spawn(Spawn {
                        id: index,
                        executable: &std::env::current_exe()?,
                        shell: &options.shell,
                        args: &args,
                        project: &directory,
                        rows: 32,
                        cols: 100,
                        history: usize::from(options.history),
                    })?;
                    session.control = Some(control);
                    sessions.push(Managed {
                        id: sid,
                        name,
                        session,
                        owner: None,
                        stop: None,
                    });
                }
                let workspace = Workspace {
                    id: id.clone(),
                    options,
                    sessions,
                    finished: None,
                };
                let info = workspace.info();
                self.workspaces.insert(id, workspace);
                Ok(Response::Workspace(info))
            }
            Request::Attach {
                takeover,
                workspace,
                session,
                client,
                rows,
                cols,
            } => {
                let m = self.managed(&workspace, &session)?;
                m.own(&client, takeover)?;
                m.session.resize(rows, cols)?;
                Ok(Response::Ok)
            }
            Request::Poll {
                workspace,
                session,
                client,
                revision,
                history,
            } => {
                let m = self.managed(&workspace, &session)?;
                m.check(&client)?;
                let info = m.info();
                let state = m.session.state();
                if revision == Some(state.revision) {
                    return Ok(Response::Update(None));
                }
                let _perf = crate::perf::span(crate::perf::Metric::HostSnapshot);
                let mut screen = state.terminal.screen().clone();
                screen.set_scrollback(0);
                let (rows, cols) = screen.size();
                let formatted = screen.state_formatted();
                let mut past = Vec::new();
                if history {
                    let count = screen.scrollback_len();
                    for offset in (1..=count).rev() {
                        screen.set_scrollback(offset);
                        past.push(
                            screen
                                .rows_formatted(0, cols.get())
                                .next()
                                .unwrap_or_default(),
                        );
                    }
                }
                Ok(Response::Update(Some(Update {
                    revision: state.revision,
                    rows: rows.get(),
                    cols: cols.get(),
                    screen: formatted.into_bytes(),
                    history: past.into_iter().map(String::into_bytes).collect(),
                    alternate_screen: state.terminal.screen().alternate_screen(),
                    focus_events: state.terminal.parser.callbacks().focus_events,
                    win32_input: state.terminal.parser.callbacks().win32_input,
                    info,
                    error: state.io_error.clone(),
                })))
            }
            Request::Input {
                workspace,
                session,
                client,
                bytes,
            } => {
                anyhow::ensure!(
                    bytes.len() <= crate::input::MAX_PASTE + 100000,
                    "Input too large"
                );
                let m = self.managed(&workspace, &session)?;
                m.check(&client)?;
                m.session.send(bytes)?;
                Ok(Response::Ok)
            }
            Request::Resize {
                workspace,
                session,
                client,
                rows,
                cols,
            } => {
                let m = self.managed(&workspace, &session)?;
                m.check(&client)?;
                m.session.resize(rows, cols)?;
                Ok(Response::Ok)
            }
            Request::Detach { client } => {
                for workspace in self.workspaces.values_mut() {
                    for session in &mut workspace.sessions {
                        if session.owner.as_ref().is_some_and(|(id, _)| id == &client) {
                            session.owner = None;
                        }
                    }
                }
                Ok(Response::Ok)
            }
            Request::Close { workspace } => {
                self.workspaces
                    .remove(&workspace)
                    .context("Unknown workspace")?;
                if self.debug.as_ref() == Some(&workspace) {
                    self.debug = None;
                }
                Ok(Response::Ok)
            }
            Request::Stop {
                workspace,
                session,
                force,
            } => {
                let m = self.managed(&workspace, &session)?;
                if force {
                    anyhow::ensure!(
                        m.stop
                            .is_some_and(|at| at.elapsed() >= Duration::from_secs(3)),
                        "Request normal stop and wait 3 seconds before force"
                    );
                    m.session.force_command()?;
                } else {
                    m.session.send(vec![3])?;
                    m.stop = Some(Instant::now());
                }
                Ok(Response::Ok)
            }
            Request::Run {
                workspace,
                session,
                source,
            } => {
                let m = self.managed(&workspace, &session)?;
                m.session.run_command(source)?;
                m.stop = None;
                m.session.send(crate::input::key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::F(12),
                        crossterm::event::KeyModifiers::CONTROL
                            | crossterm::event::KeyModifiers::ALT,
                    ),
                    false,
                    true,
                ))?;
                Ok(Response::Ok)
            }
            Request::DebugTarget { workspace } => {
                if let Some(id) = workspace {
                    anyhow::ensure!(self.workspaces.contains_key(&id), "Unknown workspace");
                    self.debug = Some(id);
                }
                Ok(Response::Target(self.debug.clone()))
            }
            Request::Shutdown => {
                anyhow::ensure!(
                    self.workspaces.is_empty(),
                    "Close all workspaces before host shutdown"
                );
                self.shutdown = true;
                Ok(Response::Ok)
            }
        }
    }
}

fn managed_args(options: &Options, index: usize, control: &std::path::Path) -> Result<Vec<String>> {
    fs::create_dir_all(control.parent().unwrap())?;
    let path = control.to_string_lossy().replace('\'', "''");
    let source = options.source(index).unwrap_or_default();
    let source = serde_json::to_string(&source)?;
    let source = source.replace('\'', "''");
    // A small shell integration records facts about managed commands only.
    // Existing Profile and prompt are retained, with one wrapper around prompt.
    let script = include_str!("shell.ps1")
        .replace("__CONTROL__", &path)
        .replace("__SOURCE_JSON__", &source);
    let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
    if options.no_profile {
        args.push("-NoProfile".into());
    }
    args.extend(["-EncodedCommand".into(), encode_script(&script)]);
    Ok(args)
}
