use crate::presentation::{Floating, Placement, Target};
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
    placement: Placement,
}
struct Workspace {
    id: String,
    options: Options,
    sessions: Vec<Managed>,
    finished: Option<Instant>,
    view_ready: bool,
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
            placement: self.placement,
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
    floating: Floating,
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
/// Read-only status polling can outlive the host's empty-workspace timeout.
/// Reconnect once per poll, in the background worker; never launch a workspace.
pub fn workspaces() -> Result<Vec<WorkspaceInfo>> {
    let response = match protocol::rpc(Request::List) {
        Err(error) if protocol::disconnected(&error) => {
            ensure().context("Reconnect DevTabs host")?;
            protocol::rpc(Request::List)?
        }
        result => result?,
    };
    match response {
        Response::Workspaces(workspaces) => Ok(workspaces),
        _ => anyhow::bail!("Invalid host response"),
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
                // A visible overview polls even with no workspaces. Idle means
                // no clients, so retain the empty host while it is being used.
                empty = Instant::now();
                let state = state.clone();
                handles.push(thread::spawn(move || {
                    let result = (|| -> Result<Response> {
                        let mut request = protocol::receive(&mut stream)?;
                        if let Request::Open { options } = &mut request {
                            // History I/O must never hold the host's session/input mutex.
                            options.resolve_latest();
                        }
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
                host.floating.expire();
                let mut expired = vec![];
                for work in host.workspaces.values_mut() {
                    for m in &mut work.sessions {
                        if m.placement == Placement::Floating
                            && m.owner
                                .as_ref()
                                .is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(3))
                        {
                            m.placement = Placement::Collapsed;
                            m.owner = None;
                            expired.push(Target {
                                workspace: work.id.clone(),
                                session: m.id.clone(),
                            });
                        }
                    }
                }
                for target in expired {
                    host.floating.remove(&target);
                }

                host.workspaces.retain(|_, w| {
                    if w.sessions.iter().any(|s| s.session.running()) {
                        w.finished = None;
                        true
                    } else {
                        if w.finished.get_or_insert_with(Instant::now).elapsed()
                            < Duration::from_secs(3)
                        {
                            true
                        } else if let Err(e) = crate::recovery::remove(&w.id) {
                            eprintln!("Remove completed launch record: {e:#}");
                            true
                        } else {
                            false
                        }
                    }
                });
                if let Some(ticket) = host.floating.target()
                    && !host.workspaces.contains_key(&ticket.target.workspace)
                {
                    host.floating.remove(&ticket.target);
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
                crate::recovery::update_layout(&workspace, layout)?;
                work.options.layout = layout;
                for session in &mut work.sessions {
                    if session.placement == Placement::Docked {
                        session.owner = None;
                    }
                }
                Ok(Response::Ok)
            }
            Request::Ping => Ok(Response::Host {
                pid: std::process::id(),
            }),
            Request::List => Ok(Response::Workspaces(
                self.workspaces.values().map(Workspace::info).collect(),
            )),
            Request::Open { options } => {
                let mut options = *options;
                if let Some(id) = &options.workspace {
                    return Ok(Response::Workspace(Box::new(
                        self.workspaces.get(id).context("Unknown workspace")?.info(),
                    )));
                }
                if let Some(old) = &options.recovery_id
                    && let Some(work) = self
                        .workspaces
                        .values()
                        .find(|w| w.options.recovery_id.as_ref() == Some(old))
                {
                    return Ok(Response::Workspace(Box::new(work.info())));
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
                    return Ok(Response::Workspace(Box::new(work.info())));
                }
                let id = uuid::Uuid::new_v4().to_string();
                let mut sessions = Vec::new();
                for index in 0..options.terminal_count() {
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
                        placement: Placement::Docked,
                    });
                }
                let workspace = Workspace {
                    id: id.clone(),
                    options,
                    sessions,
                    finished: None,
                    view_ready: false,
                };
                crate::recovery::register(&id, &workspace.options)?;
                let info = workspace.info();
                self.workspaces.insert(id, workspace);
                Ok(Response::Workspace(Box::new(info)))
            }
            Request::ViewReady { workspace } => {
                let work = self
                    .workspaces
                    .get_mut(&workspace)
                    .context("Unknown workspace")?;
                let first = !work.view_ready;
                work.view_ready = true;
                Ok(Response::FirstView(first))
            }
            Request::PrepareFloat { target } => {
                self.managed(&target.workspace, &target.session)?;
                let plan = self.floating.prepare(target);
                Ok(Response::FloatPlan(plan))
            }
            Request::FloatingTarget => Ok(Response::Floating(self.floating.target())),
            Request::CancelFloat { revision } => {
                self.floating.cancel(revision);
                Ok(Response::Ok)
            }
            Request::Present { target, placement } => {
                anyhow::ensure!(
                    placement != Placement::Floating,
                    "Use PrepareFloat before attaching a floating view"
                );
                let m = self.managed(&target.workspace, &target.session)?;
                m.placement = placement;
                m.owner = None;
                self.floating.remove(&target);
                Ok(Response::Ok)
            }
            Request::Attach {
                floating,
                takeover,
                workspace,
                session,
                client,
                rows,
                cols,
            } => {
                crate::perf::add(crate::perf::Count::TerminalAttaches, 1);
                let _attach_perf =
                    floating.map(|_| crate::perf::span(crate::perf::Metric::FloatingAttach));
                let target = Target {
                    workspace: workspace.clone(),
                    session: session.clone(),
                };
                if let Some(revision) = floating {
                    self.floating.validate(&target, revision)?;
                    let moving = self.floating.is_pending(revision);
                    if !moving {
                        self.managed(&workspace, &session)?.own(&client, false)?;
                    }
                    // Validate dimensions before taking the source view's lease.
                    self.managed(&workspace, &session)?
                        .session
                        .resize(rows, cols)?;
                    if let Some(old) = self.floating.commit()
                        && old.target != target
                        && let Ok(m) = self.managed(&old.target.workspace, &old.target.session)
                    {
                        m.placement = Placement::Collapsed;
                        m.owner = None;
                    }
                    let m = self.managed(&workspace, &session)?;
                    m.placement = Placement::Floating;
                    m.own(&client, moving)?;
                } else {
                    let m = self.managed(&workspace, &session)?;
                    anyhow::ensure!(
                        m.placement == Placement::Docked,
                        "Session is collapsed or displayed in a floating view"
                    );
                    m.own(&client, takeover)?;
                }
                let m = self.managed(&workspace, &session)?;
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
                crate::perf::add(
                    crate::perf::Count::TerminalFrameBytes,
                    formatted.len() as u64,
                );
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
                crate::perf::add(crate::perf::Count::TerminalResizes, 1);
                m.session.resize(rows, cols)?;
                Ok(Response::Ok)
            }
            Request::Detach { client } => {
                crate::perf::add(crate::perf::Count::TerminalDetaches, 1);
                let mut released = vec![];
                for workspace in self.workspaces.values_mut() {
                    for session in &mut workspace.sessions {
                        if session.owner.as_ref().is_some_and(|(id, _)| id == &client) {
                            session.owner = None;
                            if session.placement == Placement::Floating {
                                session.placement = Placement::Collapsed;
                                released.push(Target {
                                    workspace: workspace.id.clone(),
                                    session: session.id.clone(),
                                });
                            }
                        }
                    }
                }
                for target in released {
                    self.floating.remove(&target);
                }
                Ok(Response::Ok)
            }
            Request::Close { workspace } => {
                anyhow::ensure!(
                    self.workspaces.contains_key(&workspace),
                    "Unknown workspace"
                );
                crate::recovery::remove(&workspace)?;
                let removed = self
                    .workspaces
                    .remove(&workspace)
                    .context("Unknown workspace")?;
                for m in removed.sessions {
                    self.floating.remove(&Target {
                        workspace: workspace.clone(),
                        session: m.id.clone(),
                    });
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
