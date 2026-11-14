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
    resource_lease: Option<std::fs::File>,
    round_at: Option<Instant>,
    token: String,
    activity: crate::activity::State,
    id: String,
    task: crate::tasks::Binding,
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
    tasks: Vec<crate::tasks::Task>,
    finished: Option<Instant>,
    view_ready: bool,
}
impl Workspace {
    fn project_name(&self) -> String {
        self.options
            .project
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
    fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo {
            id: self.id.clone(),
            project: self.options.project.clone(),
            options: self.options.clone(),
            sessions: self.sessions.iter().map(Managed::info).collect(),
            tasks: self.tasks.clone(),
        }
    }
}
impl Managed {
    fn retire(&mut self, records: Option<&crate::rounds::service::Worker>) -> Result<()> {
        if !self.session.running() && self.resource_lease.is_some() {
            if self.task.role == crate::tasks::Role::Agent
                && let Some(records) = records
            {
                records.seal(
                    self.task.task_id.clone(),
                    self.task.run_id.clone(),
                    self.task.initial_cwd.clone(),
                )?;
            }
            self.resource_lease.take();
        }
        Ok(())
    }
    fn info(&self) -> SessionInfo {
        self.session.refresh_command();
        let s = self.session.state();
        SessionInfo {
            id: self.id.clone(),
            task: Box::new(self.task.clone()),
            name: self.name.clone(),
            shell_pid: s.shell_pid,
            status: s.status.clone(),
            command: s.command.clone(),
            placement: self.placement,
            activity: Box::new(self.activity.clone()),
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
    maintenance: std::collections::HashSet<String>,
    ledger: crate::notifications::Ledger,
    checking: std::collections::HashSet<String>,
    notifications: Option<crate::notifications::Worker>,
    notification_settings: crate::notifications::Settings,
    notification_configuration: crate::notifications::Adoption,
    policies: HashMap<String, crate::notifications::Policy>,
    diagnostic: std::collections::VecDeque<String>,
    navigation: HashMap<String, crate::notifications::Target>,
    notification_routes: HashMap<String, String>,
    records: Option<crate::rounds::service::Worker>,
    workspaces: HashMap<String, Workspace>,
    floating: Floating,
    shutdown: bool,
    starting: HashMap<String, String>,
}

pub fn ensure() -> Result<()> {
    if protocol::rpc(Request::Ping).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(config::runtime())?;
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
    let _ = host_clock();
    let hook_events = crate::hooks::listen(&format!("{}-hooks", protocol::endpoint()))?;
    let state = Arc::new(Mutex::new(Host {
        notifications: Some(crate::notifications::Worker::spawn()?),
        ledger: crate::notifications::Ledger::load(&config::runtime())?,
        notification_settings: crate::notifications::Settings::load()?,
        records: Some(crate::rounds::service::Worker::spawn(
            config::runtime().join("history"),
        )?),
        ..Host::default()
    }));
    let mut empty = Instant::now();
    let mut last_sweep = Instant::now();
    let mut notification_sweep = Instant::now();
    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    loop {
        handles.retain(|h| !h.is_finished());
        if handles.len() >= 64 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        for envelope in hook_events.try_iter().take(64) {
            refresh_notifications(&state);
            let mut host = state.lock().unwrap_or_else(|p| p.into_inner());
            let records = host.records.clone();
            let mut notices = vec![];
            let mut bindings = vec![];
            let mut activity_updates = vec![];
            for work in host.workspaces.values_mut() {
                let project_name = work.project_name();
                if let Some(m) = work.sessions.iter_mut().find(|m| {
                    m.id == envelope.context.session
                        && m.task.task_id == envelope.context.task
                        && m.task.run_id == envelope.context.run
                        && m.token == envelope.context.token
                }) && envelope.context.provider == envelope.event.provider
                    && !envelope.event.child
                    && dunce::canonicalize(&envelope.event.cwd)
                        .ok()
                        .is_some_and(|p| p.starts_with(&m.task.initial_cwd))
                {
                    let generation = m.activity.generation;
                    if work
                        .tasks
                        .iter()
                        .find(|t| t.id == m.task.task_id)
                        .is_none_or(|t| {
                            format!("{:?}", t.options.agent).to_lowercase()
                                != envelope.event.provider
                        })
                    {
                        continue;
                    }
                    if m.activity.apply(&envelope.event, config::now()) {
                        if let Some(task) = work.tasks.iter_mut().find(|t| t.id == m.task.task_id)
                            && task.options.resume_id.as_deref()
                                != Some(&envelope.event.conversation)
                            && matches!(
                                task.options.agent,
                                crate::launch::Agent::Codex | crate::launch::Agent::Claude
                            )
                        {
                            task.options.resume_id = Some(envelope.event.conversation.clone());
                            bindings.push(task.clone());
                        }
                        use crate::activity::Phase;
                        if m.activity.phase == Phase::Working && generation != m.activity.generation
                        {
                            m.round_at = Some(Instant::now());
                            m.activity.duration_ms = None;
                        }
                        if matches!(
                            m.activity.phase,
                            Phase::Reported | Phase::Failed | Phase::Interrupted | Phase::Ended
                        ) {
                            m.activity.duration_ms =
                                m.round_at.map(|t| t.elapsed().as_millis() as u64);
                        }
                        activity_updates.push((
                            crate::notifications::Target {
                                workspace: work.id.clone(),
                                task: m.task.task_id.clone(),
                                session: m.id.clone(),
                                run: m.task.run_id.clone(),
                                result: m.activity.result,
                            },
                            m.activity.clone(),
                        ));
                        let phase = &m.activity.phase;
                        if matches!(
                            phase,
                            Phase::Working
                                | Phase::Reported
                                | Phase::Failed
                                | Phase::Interrupted
                                | Phase::Ended
                        ) && let Some(records) = &records
                        {
                            let _ = records.event(
                                m.task.task_id.clone(),
                                m.task.run_id.clone(),
                                m.task.initial_cwd.clone(),
                                *phase == Phase::Working,
                                generation != m.activity.generation,
                                m.activity.result,
                            );
                        }
                        if matches!(
                            phase,
                            Phase::Reported | Phase::Failed | Phase::Permission | Phase::Input
                        ) {
                            notices.push(crate::notifications::Notice {
                                target: crate::notifications::Target {
                                    workspace: work.id.clone(),
                                    task: m.task.task_id.clone(),
                                    session: m.id.clone(),
                                    run: m.task.run_id.clone(),
                                    result: m.activity.result,
                                },
                                title: format!("DevTabs · {}", project_name),
                                body: crate::i18n::t(phase.label()).into_owned(),
                                test: false,
                                category: String::new(),
                                ending: matches!(phase, Phase::Reported | Phase::Failed),
                                urgent: *phase != Phase::Reported,
                                silent: false,
                            });
                        }
                    }
                }
            }
            for n in notices {
                host.notify(n);
            }
            let changed = !activity_updates.is_empty();
            for (t, s) in activity_updates {
                if let Err(e) = host.ledger.insert(t, s) {
                    host.diagnostic.push_back(e.to_string());
                }
            }
            drop(host);
            if changed {
                persist_notification_state(&state, true, false)?;
            }
            if !bindings.is_empty() {
                let _lock = config::ConfigLock::acquire()?;
                let mut catalog = crate::tasks::Catalog::load(&config::runtime())?;
                for t in bindings {
                    catalog.tasks.insert(t.id.clone(), t);
                }
                catalog.save(&config::runtime())?;
            }
        }
        if notification_sweep.elapsed() >= Duration::from_millis(100) {
            notification_sweep = Instant::now();
            refresh_notifications(&state);
            let mut host = state.lock().unwrap_or_else(|p| p.into_inner());
            let clicks = host
                .notifications
                .as_ref()
                .map(|n| n.clicks.try_iter().take(64).collect::<Vec<_>>())
                .unwrap_or_default();
            let diagnostics = host
                .notifications
                .as_ref()
                .map(|n| n.diagnostics.try_iter().take(64).collect::<Vec<_>>())
                .unwrap_or_default();
            for t in clicks {
                if host.valid_target(&t) {
                    let key = format!("{}:{}", t.run, t.result);
                    if let Some(client) = host.notification_routes.get(&key).cloned() {
                        host.navigation.insert(client, t);
                    } else {
                        host.diagnostic.push_back(
                            "Original notification view is unavailable; use the attention inbox"
                                .into(),
                        );
                    }
                }
            }
            for d in diagnostics {
                host.diagnostic.push_back(d);
            }
            host.diagnostic.truncate(64);
            let pending = host
                .workspaces
                .values()
                .flat_map(|w| {
                    w.sessions.iter().map(|m| {
                        (
                            crate::notifications::Target {
                                workspace: w.id.clone(),
                                task: m.task.task_id.clone(),
                                session: m.id.clone(),
                                run: m.task.run_id.clone(),
                                result: m.activity.result,
                            },
                            m.activity.clone(),
                        )
                    })
                })
                .collect::<Vec<_>>();
            let enabled = !host.notification_settings.disabled;
            for (target, activity) in pending {
                if host.policies.entry(target.task.clone()).or_default().due(
                    host_clock(),
                    activity.result,
                    activity.read < activity.result,
                    activity.generation,
                ) && enabled
                    && let Some(worker) = &host.notifications
                {
                    let _ = worker.tx.try_send(crate::notifications::Notice {
                        target,
                        title: "DevTabs".into(),
                        body: crate::i18n::t(activity.phase.label()).into_owned(),
                        test: false,
                        category: match activity.phase {
                            crate::activity::Phase::Failed => "failure",
                            crate::activity::Phase::Permission => "permission",
                            crate::activity::Phase::Input => "input",
                            _ => "end",
                        }
                        .into(),
                        ending: true,
                        urgent: false,
                        silent: false,
                    });
                }
            }
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
                        if let Some(response) = task_request(&state, &request)? {
                            return Ok(response);
                        }
                        let prepare = if let Request::Open { options } = &mut request {
                            if options.workspace.is_none() {
                                options.validate()?;
                                crate::settings::data::UiState::update(
                                    &config::home(),
                                    crate::settings::data::UiChange::Opened(
                                        options.project.clone(),
                                        config::now(),
                                    ),
                                )?;
                                if options.recovery_id.is_none() {
                                    Some(crate::checks::prepare_definition(&options.project)?)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        let mut host = state
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Host state unavailable"))?;
                        host.handle(request, prepare)
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
                let records = host.records.clone();
                for work in host.workspaces.values_mut() {
                    for m in &mut work.sessions {
                        if let Err(e) = m.retire(records.as_ref()) {
                            eprintln!("Recording finalization pending: {e:#}");
                        }
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
    fn guard_session(&self, workspace: &str, session: &str) -> Result<()> {
        let m = self
            .workspaces
            .get(workspace)
            .and_then(|w| w.sessions.iter().find(|m| m.id == session))
            .context("Session missing")?;
        anyhow::ensure!(
            !self.maintenance.contains(&m.task.task_id),
            "Task resource operation is in progress"
        );
        Ok(())
    }
    fn lifecycle_guard(&self, op: &crate::lifecycle::Operation) -> Result<()> {
        for id in [&op.source.id, &op.target.id] {
            anyhow::ensure!(
                !self.maintenance.contains(id)
                    && !self.checking.contains(id)
                    && !self.starting.keys().any(|k| k.contains(id)),
                "Task has a managed operation in progress"
            );
        }
        for w in self.workspaces.values() {
            for m in &w.sessions {
                if m.task.task_id == op.source.id
                    || matches!(op.kind, crate::lifecycle::Kind::FastForward)
                        && m.task.task_id == op.target.id
                {
                    m.session.refresh_command();
                    let s = m.session.state();
                    anyhow::ensure!(
                        !matches!(
                            s.command,
                            crate::session::CommandStatus::Starting
                                | crate::session::CommandStatus::Running
                        ),
                        "Stop related managed commands before applying"
                    );
                    if matches!(op.kind, crate::lifecycle::Kind::Cleanup)
                        && m.task.task_id == op.source.id
                    {
                        anyhow::ensure!(
                            s.status != crate::session::Status::Running,
                            "Close all source task terminals before cleanup"
                        );
                    }
                }
            }
        }
        Ok(())
    }
    fn managed(&mut self, workspace: &str, session: &str) -> Result<&mut Managed> {
        self.workspaces
            .get_mut(workspace)
            .context("Workspace no longer exists")?
            .sessions
            .iter_mut()
            .find(|s| s.id == session)
            .context("Session no longer exists")
    }
    fn handle(
        &mut self,
        request: Request,
        prepare: Option<Vec<crate::checks::Spec>>,
    ) -> Result<Response> {
        let _perf = crate::perf::span(crate::perf::Metric::HostRequest);
        match request {
            Request::Lifecycle { .. }
            | Request::Check { .. }
            | Request::CheckStatus { .. }
            | Request::Notifications { .. }
            | Request::Review { .. }
            | Request::StartRole { .. }
            | Request::PlanTask { .. }
            | Request::AddTask { .. }
            | Request::TaskOperation { .. } => {
                anyhow::bail!("Task operation must run outside host input lock")
            }
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
                let options = *options;
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
                let tasks = {
                    let _lock = config::ConfigLock::acquire()?;
                    let mut catalog = crate::tasks::Catalog::load(&config::runtime())?;
                    let tasks = if let Some(old) = &options.recovery_id {
                        let journal = crate::recovery::load()?;
                        let record = journal
                            .entries
                            .get(old)
                            .context("Recovery record missing; resources retained")?;
                        anyhow::ensure!(
                            !record.task_ids.is_empty(),
                            "Recovery has no authoritative task identities"
                        );
                        let mut tasks = vec![];
                        for id in &record.task_ids {
                            let task = catalog
                                .tasks
                                .get(id)
                                .context("Recovery task missing from catalog")?
                                .clone();
                            if task.archived {
                                continue;
                            }
                            anyhow::ensure!(
                                task.cwd.is_dir(),
                                "Task archived or directory missing; resources retained"
                            );
                            if task.owned {
                                let repo = crate::tasks::Repository::discover(&task.cwd)?;
                                anyhow::ensure!(
                                    crate::tasks::text(&repo.root, &["branch", "--show-current"])?
                                        == task.branch,
                                    "Task branch changed; explicit review required"
                                );
                            }
                            tasks.push(task);
                        }
                        anyhow::ensure!(!tasks.is_empty(), "All recovery tasks are archived");
                        tasks
                    } else {
                        vec![catalog.main_task_with_prepare(
                            &options,
                            prepare.context("Preparation snapshot missing")?,
                        )?]
                    };
                    catalog.save(&config::runtime())?;
                    tasks
                };
                let mut sessions = vec![];
                for task in &tasks {
                    if options.recovery_id.is_some() {
                        let mut restored = task.clone();
                        restored.options.continue_latest = false;
                        restored.options.resume = false;
                        restored.options.fresh = restored.options.resume_id.is_none();
                        sessions.extend(spawn_task_filtered(
                            &restored,
                            Some(crate::tasks::Role::Agent),
                        )?);
                    } else {
                        sessions.extend(spawn_task(task)?);
                    }
                }
                let workspace = Workspace {
                    id: id.clone(),
                    options,
                    sessions,
                    tasks,
                    finished: None,
                    view_ready: false,
                };
                crate::recovery::register_tasks(&id, &workspace.options, &workspace.tasks)?;
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
                self.guard_session(&workspace, &session)?;
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
                self.guard_session(&workspace, &session)?;
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
                    let binding = m.task.clone();
                    drop(m);
                    if binding.role == crate::tasks::Role::Agent
                        && let Some(records) = &self.records
                        && let Err(e) =
                            records.seal(binding.task_id, binding.run_id, binding.initial_cwd)
                    {
                        self.diagnostic
                            .push_back(format!("Recording finalization failed: {e:#}"));
                    }
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
                self.guard_session(&workspace, &session)?;
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

fn spawn_task(task: &crate::tasks::Task) -> Result<Vec<Managed>> {
    spawn_task_filtered(task, None)
}
fn spawn_task_filtered(
    task: &crate::tasks::Task,
    only: Option<crate::tasks::Role>,
) -> Result<Vec<Managed>> {
    use crate::tasks::{Binding, Role};
    let options = &task.options;
    let mut sessions = Vec::new();
    for index in 0..options.terminal_count() {
        let id = uuid::Uuid::new_v4().to_string();
        let (role, name, directory) = match index {
            0 => (Role::Agent, "Agent".into(), task.cwd.clone()),
            1 => (Role::Debug, "Debug".into(), task.cwd.clone()),
            i => {
                let extra = &options.extra_sessions[i - 2];
                (
                    Role::Extra,
                    extra.name.clone(),
                    extra.directory.clone().unwrap_or_else(|| task.cwd.clone()),
                )
            }
        };
        if only.is_some_and(|wanted| wanted != role) {
            continue;
        }
        let directory = if directory.is_relative() {
            task.cwd.join(directory)
        } else {
            directory
        };
        let directory = dunce::canonicalize(&directory)?;
        anyhow::ensure!(
            directory.starts_with(dunce::canonicalize(&task.checkout)?),
            "Terminal cwd leaves task checkout"
        );
        let control = config::runtime().join(format!("{id}.json"));
        let run_id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let hook = crate::hooks::HookContext {
            task: task.id.clone(),
            session: id.clone(),
            run: run_id.clone(),
            token: token.clone(),
            endpoint: format!("{}-hooks", protocol::endpoint()),
            provider: format!("{:?}", options.agent).to_lowercase(),
        };
        let args = managed_args_with_hook(
            options,
            index,
            &control,
            if role == Role::Agent {
                Some(&hook)
            } else {
                None
            },
        )?;
        let resource_lease = crate::lifecycle::lease(&config::runtime(), &task.id, false, false)?;
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
            resource_lease: Some(resource_lease),
            round_at: None,
            token,
            activity: Default::default(),
            id,
            task: Binding {
                task_id: task.id.clone(),
                run_id,
                role,
                initial_cwd: directory,
            },
            name,
            session,
            owner: None,
            stop: None,
            placement: Placement::Docked,
        });
    }
    Ok(sessions)
}

fn task_request(state: &Arc<Mutex<Host>>, request: &Request) -> Result<Option<Response>> {
    use crate::tasks::{Catalog, Operation};
    match request {
        Request::Lifecycle { action } => {
            use crate::lifecycle::Action as L;
            let home = config::runtime();
            let response = match action {
                L::List => Response::TaskCatalog(Box::new(Catalog::load(&home)?)),
                L::Plan {
                    source,
                    target,
                    cleanup,
                    require_check,
                } => Response::Lifecycle(Box::new(crate::lifecycle::plan(
                    &home,
                    source,
                    target,
                    *cleanup,
                    *require_check,
                )?)),
                L::Query { operation } => {
                    Response::Lifecycle(Box::new(crate::lifecycle::load(&home, operation)?))
                }
                L::Close { task, confirm } => {
                    anyhow::ensure!(task == confirm, "Confirm the exact task ID");
                    let host = state
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                    anyhow::ensure!(
                        !host.maintenance.contains(task),
                        "Task resource operation is in progress"
                    );
                    let mut found = false;
                    for w in host.workspaces.values() {
                        for m in &w.sessions {
                            if m.task.task_id == *task {
                                m.session.terminate()?;
                                found = true;
                            }
                        }
                    }
                    anyhow::ensure!(found, "No live task terminals");
                    Response::Ok
                }
                L::Archive { task, confirm } => {
                    anyhow::ensure!(task == confirm, "Confirm the exact task ID");
                    let task = crate::lifecycle::archive(&home, task)?;
                    let mut host = state
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                    for w in host.workspaces.values_mut() {
                        if let Some(t) = w.tasks.iter_mut().find(|t| t.id == task.id) {
                            t.archived = true;
                        }
                    }
                    Response::Ok
                }
                L::Apply { operation, confirm } => {
                    anyhow::ensure!(
                        operation == confirm,
                        "Confirm the exact operation ID after reviewing its summary"
                    );
                    let op = crate::lifecycle::load(&home, operation)?;
                    if op.phase == "Complete" {
                        return Ok(Some(Response::Lifecycle(Box::new(op))));
                    }
                    let ids = if matches!(op.kind, crate::lifecycle::Kind::Cleanup) {
                        vec![op.source.id.clone()]
                    } else {
                        vec![op.source.id.clone(), op.target.id.clone()]
                    };
                    let records = {
                        let mut host = state
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                        let records = host.records.clone();
                        for w in host.workspaces.values_mut() {
                            for m in &mut w.sessions {
                                m.retire(records.as_ref())?;
                            }
                        }
                        host.lifecycle_guard(&op)?;
                        for id in &ids {
                            host.maintenance.insert(id.clone());
                        }
                        host.records.clone()
                    };
                    let result = (|| -> Result<_> {
                        if let Some(worker) = records {
                            for t in [&op.source, &op.target]
                                .into_iter()
                                .filter(|t| ids.contains(&t.id))
                            {
                                let v = worker.request(
                                    t.id.clone(),
                                    String::new(),
                                    t.checkout.clone(),
                                    crate::rounds::service::Action::List,
                                )?;
                                anyhow::ensure!(
                                    !v.busy,
                                    "End the active recording before applying resource operations"
                                );
                            }
                        }
                        crate::lifecycle::execute(&home, operation)
                    })();
                    let mut host = state
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                    for id in &ids {
                        host.maintenance.remove(id);
                    }
                    if result.is_ok() && matches!(op.kind, crate::lifecycle::Kind::Cleanup) {
                        for w in host.workspaces.values_mut() {
                            for t in &mut w.tasks {
                                if t.id == op.source.id {
                                    t.archived = true;
                                }
                            }
                        }
                    }
                    Response::Lifecycle(Box::new(result?))
                }
            };
            Ok(Some(response))
        }
        Request::CheckStatus { task } => {
            anyhow::ensure!(uuid::Uuid::parse_str(task).is_ok(), "Invalid task ID");
            let path = config::runtime()
                .join("checks")
                .join(format!("{task}.json"));
            let mut report = if path.exists() {
                Some(serde_json::from_slice::<crate::checks::Report>(&fs::read(
                    path,
                )?)?)
            } else {
                None
            };
            if let Some(r) = &mut report {
                let catalog = crate::tasks::Catalog::load(&config::runtime())?;
                if let Some(t) = catalog.tasks.get(task) {
                    r.stale |= crate::checks::fingerprint(&t.checkout).ok().as_deref()
                        != Some(&r.after)
                        || t.checkout != r.checkout;
                    let host = state
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                    r.stale |= host
                        .workspaces
                        .values()
                        .flat_map(|w| &w.sessions)
                        .filter(|m| {
                            m.task.task_id == *task && m.task.role == crate::tasks::Role::Agent
                        })
                        .any(|m| m.task.run_id != r.run);
                }
            }
            Ok(Some(Response::Check(Box::new(report))))
        }
        Request::Check {
            workspace,
            task,
            spec,
            initialize,
        } => {
            let (root, project, prepare, result, run) = {
                let mut host = state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                anyhow::ensure!(
                    host.checking.len() < 2
                        && !host.checking.contains(task)
                        && !host.maintenance.contains(task),
                    "Check worker busy"
                );
                let w = host
                    .workspaces
                    .get(workspace)
                    .context("Workspace missing")?;
                let t = w
                    .tasks
                    .iter()
                    .find(|t| &t.id == task)
                    .context("Task missing")?;
                let result = w
                    .sessions
                    .iter()
                    .find(|m| m.task.task_id == *task && m.task.role == crate::tasks::Role::Agent)
                    .map(|m| (m.activity.result, m.task.run_id.clone()))
                    .unwrap_or_default();
                let root = t.checkout.clone();
                let project = t.project.clone();
                let prepare = t.prepare.clone();
                host.checking.insert(task.clone());
                (root, project, prepare, result.0, result.1)
            };
            let shared = state.clone();
            let task = task.clone();
            let spec = spec.clone();
            let initialize = *initialize;
            let failed_task = task.clone();
            let check_workspace = workspace.clone();
            let worker = thread::Builder::new()
                .name("devtabs-check".into())
                .spawn(move || {
                    let mut check_report = None;
                    let outcome = (|| -> Result<()> {
                        if initialize {
                            crate::checks::initialize(&task, &root, &config::runtime(), &prepare)?;
                        } else {
                            let spec = match spec {
                                Some(s) => s,
                                None => crate::checks::check_definition(&project)?,
                            };
                            let mut report = crate::checks::run(&task, result, &root, &spec)?;
                            report.run = run.clone();
                            config::atomic_write(
                                &config::runtime()
                                    .join("checks")
                                    .join(format!("{task}.json")),
                                &serde_json::to_vec(&report)?,
                            )?;
                            check_report = Some(report);
                        }
                        Ok(())
                    })();
                    let mut host = shared.lock().unwrap_or_else(|p| p.into_inner());
                    host.checking.remove(&task);
                    if host.notification_settings.after_check.get(&task) == Some(&true)
                        && let Some(report) = &check_report
                        && !report.stale
                        && let Some(m) = host.workspaces.get(&check_workspace).and_then(|w| {
                            w.sessions.iter().find(|m| {
                                m.task.task_id == task
                                    && m.task.role == crate::tasks::Role::Agent
                                    && m.activity.result == result
                                    && m.task.run_id == report.run
                            })
                        })
                    {
                        let notice = crate::notifications::Notice {
                            target: crate::notifications::Target {
                                workspace: check_workspace.clone(),
                                task: task.clone(),
                                session: m.id.clone(),
                                run: m.task.run_id.clone(),
                                result,
                            },
                            title: "DevTabs".into(),
                            body: crate::i18n::t(if report.exit == Some(0) && !report.timeout {
                                "review.check_passed"
                            } else {
                                "review.check_failed"
                            })
                            .into_owned(),
                            test: false,
                            category: String::new(),
                            ending: true,
                            urgent: report.exit != Some(0),
                            silent: host.notification_settings.silent,
                        };
                        host.notify_checked(notice);
                    }
                    host.diagnostic.push_back(match outcome {
                        Ok(()) => "Managed check ended; view its bound revision".into(),
                        Err(e) => format!("Managed check failed: {e:#}"),
                    });
                });
            if let Err(e) = worker {
                state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .checking
                    .remove(&failed_task);
                return Err(e.into());
            }
            Ok(Some(Response::Ok))
        }
        Request::Notifications { action } => {
            use crate::notifications::Action;
            let change = match action {
                Action::Global { enabled } => Some((vec!["notifications", "enabled"], *enabled)),
                Action::Sound { enabled } => Some((vec!["notifications", "sound"], *enabled)),
                Action::Category {
                    task: None,
                    category,
                    enabled,
                } => Some((
                    vec!["notifications", "categories", category.as_str()],
                    *enabled,
                )),
                _ => None,
            };
            if let Some((path, value)) = change {
                let store = crate::settings::store::SettingsStore::current();
                let base = store.read()?;
                let mut draft = base.tree.clone();
                crate::settings::projection::set(&mut draft, &path, Some(value.into()));
                store.save(&base, &draft)?;
            }
            refresh_notifications(state);
            let mut host = state
                .lock()
                .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
            let mut save = false;
            let mut navigation = None;
            match action {
                Action::List | Action::Diagnostics => {}
                Action::AfterCheck { task, enabled } => {
                    host.notification_settings
                        .after_check
                        .insert(task.clone(), *enabled);
                    save = true;
                }
                Action::Sound { enabled } => {
                    let _ = enabled;
                }
                Action::Category {
                    task,
                    category,
                    enabled,
                } => {
                    anyhow::ensure!(
                        ["end", "failure", "permission", "input"].contains(&category.as_str()),
                        "Unknown notification category"
                    );
                    if let Some(task) = task {
                        host.notification_settings
                            .task_categories
                            .entry(task.clone())
                            .or_default()
                            .insert(category.clone(), *enabled);
                    }
                    save = task.is_some();
                }
                Action::Poll { workspace, clients } => {
                    for client in clients.iter().take(64) {
                        if host
                            .navigation
                            .get(client)
                            .is_some_and(|t| &t.workspace == workspace)
                        {
                            navigation = host.navigation.remove(client);
                            break;
                        }
                    }
                }
                Action::Ack { target } => {
                    anyhow::ensure!(host.valid_target(target), "Result target expired");
                    if let Ok(m) = host.managed(&target.workspace, &target.session) {
                        m.activity.ack(target.result);
                    }
                    if let Some((_, s)) = host.ledger.0.get_mut(&target.run) {
                        s.ack(target.result);
                    }
                }
                Action::Navigate { target } => {
                    anyhow::ensure!(host.valid_target(target), "Result target expired");
                    let client = host
                        .managed(&target.workspace, &target.session)?
                        .owner
                        .as_ref()
                        .map(|(id, _)| id.clone())
                        .context(
                            "Original view unavailable; attach this existing task explicitly",
                        )?;
                    host.navigation.insert(client, target.clone());
                }
                Action::Mute { task, muted } => {
                    host.policies.entry(task.clone()).or_default().muted = *muted;
                    host.notification_settings
                        .muted
                        .insert(task.clone(), *muted);
                    save = true;
                }
                Action::Once { task } => {
                    host.policies.entry(task.clone()).or_default().once = true;
                }
                Action::Global { enabled } => {
                    let _ = enabled;
                }
                Action::Snooze { target, seconds } => {
                    anyhow::ensure!(host.valid_target(target), "Result target expired");
                    anyhow::ensure!(
                        *seconds == 300 || *seconds == 1800 || *seconds == 0,
                        "Choose 5 or 30 minutes, or cancel"
                    );
                    let generation = host
                        .managed(&target.workspace, &target.session)?
                        .activity
                        .generation;
                    let policy = host.policies.entry(target.task.clone()).or_default();
                    policy.snooze = if *seconds == 0 {
                        None
                    } else {
                        Some((host_clock() + seconds, target.result, generation))
                    };
                }
                Action::Test => {
                    let n = crate::notifications::Notice {
                        target: crate::notifications::Target {
                            workspace: String::new(),
                            task: String::new(),
                            session: String::new(),
                            run: String::new(),
                            result: 0,
                        },
                        title: "DevTabs · TEST".into(),
                        body: crate::i18n::t("notify.test").into_owned(),
                        test: true,
                        category: String::new(),
                        ending: false,
                        urgent: false,
                        silent: false,
                    };
                    host.notifications
                        .as_ref()
                        .context("Notifications unavailable")?
                        .tx
                        .try_send(n)
                        .map_err(|_| anyhow::anyhow!("Notification queue full"))?;
                }
            }
            let mut view = host.notification_view();
            view.navigation = navigation;
            drop(host);
            if save || matches!(action, Action::Ack { .. }) {
                persist_notification_state(state, matches!(action, Action::Ack { .. }), save)?;
            }
            if matches!(action, Action::Diagnostics) {
                for provider in ["codex", "claude"] {
                    if let Ok(path) = crate::hooks::config_path(provider) {
                        view.diagnostic
                            .push(crate::hooks::diagnostic(provider, &path));
                    }
                }
            }
            Ok(Some(Response::Notifications(Box::new(view))))
        }
        Request::Review {
            workspace,
            task,
            action,
        } => {
            let (records, run, root) = {
                let host = state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                anyhow::ensure!(
                    !host.maintenance.contains(task),
                    "Task resource operation is in progress"
                );
                let w = host
                    .workspaces
                    .get(workspace)
                    .context("Workspace missing")?;
                let t = w
                    .tasks
                    .iter()
                    .find(|t| &t.id == task)
                    .context("Task missing")?;
                let m = w
                    .sessions
                    .iter()
                    .find(|s| s.task.task_id == *task && s.task.role == crate::tasks::Role::Agent)
                    .context("Agent missing")?;
                (
                    host.records.clone().context("Records unavailable")?,
                    m.task.run_id.clone(),
                    t.checkout.clone(),
                )
            };
            Ok(Some(Response::Review(Box::new(records.request(
                task.clone(),
                run,
                root,
                action.clone(),
            )?))))
        }
        Request::StartRole {
            workspace,
            task,
            role,
            fresh,
            expected_run,
        } => start_role(
            state,
            workspace,
            task,
            *role,
            *fresh,
            expected_run.as_deref(),
        )
        .map(Some),
        Request::PlanTask {
            workspace,
            source,
            name,
            base,
            path,
        } => {
            {
                let host = state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                let work = host
                    .workspaces
                    .get(workspace)
                    .context("Unknown workspace")?;
                anyhow::ensure!(
                    work.tasks.iter().any(|t| &t.id == source),
                    "Source task not in workspace"
                );
            }
            let _lock = config::ConfigLock::acquire()?;
            let mut catalog = Catalog::load(&config::runtime())?;
            let mut plan = catalog.plan(source, name, base, path.clone())?;
            plan.workspace = Some(workspace.clone());
            catalog.operations.insert(
                plan.operation.clone(),
                Operation {
                    plan: plan.clone(),
                    phase: "Prepared".into(),
                    error: None,
                },
            );
            catalog.save(&config::runtime())?;
            Ok(Some(Response::TaskPlan(Box::new(plan))))
        }
        Request::TaskOperation { operation } => {
            let catalog = Catalog::load(&config::runtime())?;
            Ok(Some(Response::TaskOperation(Box::new(
                catalog
                    .operations
                    .get(operation)
                    .context("Unknown task operation")?
                    .clone(),
            ))))
        }
        Request::AddTask {
            workspace,
            operation,
        } => {
            let _lock = config::ConfigLock::acquire()?;
            let mut catalog = Catalog::load(&config::runtime())?;
            let pending = catalog
                .operations
                .values()
                .filter(|o| matches!(o.phase.as_str(), "Creating" | "Spawning"))
                .count();
            let op = catalog
                .operations
                .get_mut(operation)
                .context("Unknown prepared task")?;
            anyhow::ensure!(
                op.plan.workspace.as_ref() == Some(workspace),
                "Task operation belongs to another workspace"
            );
            if matches!(op.phase.as_str(), "Creating" | "Spawning" | "Attached") {
                return Ok(Some(Response::TaskOperation(Box::new(op.clone()))));
            }
            anyhow::ensure!(pending < 64, "Task operation queue full");
            anyhow::ensure!(
                op.phase == "Prepared",
                "Task failed; inspect operation before retry"
            );
            let plan = op.plan.clone();
            op.phase = "Creating".into();
            let response = op.clone();
            catalog.save(&config::runtime())?;
            let state = state.clone();
            let workspace = workspace.clone();
            let worker = thread::Builder::new()
                .name("devtabs-create-task".into())
                .spawn(move || {
                    let result = (|| -> Result<()> {
                        let task = {
                            let _lock = config::ConfigLock::acquire()?;
                            let mut c = Catalog::load(&config::runtime())?;
                            let task = c.create(&config::runtime(), &plan)?;
                            c.operations.get_mut(&plan.operation).unwrap().phase =
                                "Spawning".into();
                            c.save(&config::runtime())?;
                            task
                        };
                        crate::checks::initialize(
                            &task.id,
                            &task.checkout,
                            &config::runtime(),
                            &task.prepare,
                        )?;
                        let sessions = spawn_task(&task)?;
                        let mut host = state
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                        let work = host
                            .workspaces
                            .get_mut(&workspace)
                            .context("Workspace closed; task resources retained")?;
                        if !work.tasks.iter().any(|t| t.id == task.id) {
                            let task_id = task.id.clone();
                            work.tasks.push(task);
                            work.sessions.extend(sessions);
                            drop(host);
                            crate::recovery::append_task(&workspace, &task_id)?;
                        }
                        Ok(())
                    })();
                    if let Ok(_lock) = config::ConfigLock::acquire()
                        && let Ok(mut c) = Catalog::load(&config::runtime())
                        && let Some(op) = c.operations.get_mut(&plan.operation)
                    {
                        op.phase = if result.is_ok() { "Attached" } else { "Failed" }.into();
                        op.error = result.err().map(|e| format!("{e:#}"));
                        let _ = c.save(&config::runtime());
                    }
                });
            if let Err(error) = worker {
                let op = catalog.operations.get_mut(operation).unwrap();
                op.phase = "Failed".into();
                op.error = Some(format!("Cannot start task worker: {error}"));
                let op = op.clone();
                catalog.save(&config::runtime())?;
                return Ok(Some(Response::TaskOperation(Box::new(op))));
            }
            Ok(Some(Response::TaskOperation(Box::new(response))))
        }
        _ => Ok(None),
    }
}

fn start_role(
    state: &Arc<Mutex<Host>>,
    workspace: &str,
    task_id: &str,
    role: crate::tasks::Role,
    fresh: bool,
    expected: Option<&str>,
) -> Result<Response> {
    use crate::session::{CommandStatus, Status};
    use crate::tasks::{Catalog, Operation, Plan, Role};
    anyhow::ensure!(
        role != Role::Extra,
        "Choose an explicit configured Extra terminal"
    );
    let key = format!("{workspace}:{task_id}:{role:?}");
    let operation = uuid::Uuid::new_v4().to_string();
    let (mut task, old_run) = {
        let mut host = state
            .lock()
            .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
        anyhow::ensure!(
            !host.maintenance.contains(task_id),
            "Task resource operation is in progress"
        );
        if let Some(id) = host.starting.get(&key) {
            let id = id.clone();
            drop(host);
            let catalog = Catalog::load(&config::runtime())?;
            return Ok(Response::TaskOperation(Box::new(
                catalog
                    .operations
                    .get(&id)
                    .context("Task start preparing; query again")?
                    .clone(),
            )));
        }
        let work = host
            .workspaces
            .get(workspace)
            .context("Workspace missing")?;
        let task = work
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .context("Task not in workspace")?
            .clone();
        anyhow::ensure!(!task.archived, "Task archived");
        let old = work
            .sessions
            .iter()
            .find(|s| s.task.task_id == task_id && s.task.role == role);
        if let Some(old) = old {
            anyhow::ensure!(
                expected.is_none_or(|run| run == old.task.run_id),
                "Agent run changed; inspect before recovering"
            );
            let state = old.session.state();
            let active = state.status == Status::Running
                && (role != Role::Agent
                    || task.options.agent == crate::launch::Agent::Shell
                    || !matches!(state.command, CommandStatus::Exited(_)));
            if active {
                drop(state);
                return Ok(Response::Workspace(Box::new(work.info())));
            }
        }
        let old_run = old.map(|s| s.task.run_id.clone());
        host.starting.insert(key.clone(), operation.clone());
        (task, old_run)
    };
    let prepared = (|| -> Result<Operation> {
        if role == Role::Agent {
            if fresh {
                task.options.resume_id = None;
                task.options.fresh = true;
            } else {
                anyhow::ensure!(
                    task.options.resume_id.is_some(),
                    "Task has no verified conversation binding; choose a new conversation"
                );
                task.options.fresh = false;
            }
            task.options.resume = false;
            task.options.continue_latest = false;
        } else if task.options.composition == crate::launch::Composition::AgentOnly {
            task.options.composition = crate::launch::Composition::AgentDebug;
        }
        let op = Operation {
            plan: Plan {
                operation: operation.clone(),
                workspace: Some(workspace.into()),
                task: task.clone(),
                source: task.checkout.clone(),
            },
            phase: "Spawning".into(),
            error: None,
        };
        let _lock = config::ConfigLock::acquire()?;
        let mut catalog = Catalog::load(&config::runtime())?;
        anyhow::ensure!(
            catalog
                .operations
                .values()
                .filter(|o| o.phase == "Spawning")
                .count()
                < 64,
            "Task start queue full"
        );
        catalog.operations.insert(operation.clone(), op.clone());
        catalog.save(&config::runtime())?;
        Ok(op)
    })();
    let op = match prepared {
        Ok(op) => op,
        Err(e) => {
            state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .starting
                .remove(&key);
            return Err(e);
        }
    };
    let shared = state.clone();
    let workspace = workspace.to_owned();
    let task_id = task_id.to_owned();
    let worker_key = key.clone();
    let worker_op = operation.clone();
    let worker = thread::Builder::new()
        .name("devtabs-start-role".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let mut sessions = spawn_task_filtered(&task, Some(role))?;
                let next = sessions
                    .pop()
                    .context("Requested role has no configured terminal")?;
                let old = {
                    let mut host = shared
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
                    let work = host
                        .workspaces
                        .get_mut(&workspace)
                        .context("Workspace closed; task retained")?;
                    let previous = work
                        .sessions
                        .iter()
                        .position(|s| s.task.task_id == task_id && s.task.role == role);
                    anyhow::ensure!(
                        previous.map(|i| work.sessions[i].task.run_id.clone()) == old_run,
                        "Role changed while starting; new child discarded"
                    );
                    let old = if let Some(i) = previous {
                        let old = &work.sessions[i];
                        let state = old.session.state();
                        anyhow::ensure!(
                            state.status != Status::Running
                                || (role == Role::Agent
                                    && matches!(state.command, CommandStatus::Exited(_))),
                            "Original role became active; recovery canceled"
                        );
                        drop(state);
                        Some(std::mem::replace(&mut work.sessions[i], next))
                    } else {
                        work.sessions.push(next);
                        None
                    };
                    *work
                        .tasks
                        .iter_mut()
                        .find(|t| t.id == task_id)
                        .context("Task disappeared")? = task.clone();
                    old
                };
                // Closing the previous ConPTY and job never holds the host input lock.
                drop(old);
                let _lock = config::ConfigLock::acquire()?;
                let mut catalog = Catalog::load(&config::runtime())?;
                catalog.tasks.insert(task_id.clone(), task);
                catalog.save(&config::runtime())?;
                Ok(())
            })();
            shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .starting
                .remove(&worker_key);
            if let Ok(_lock) = config::ConfigLock::acquire()
                && let Ok(mut catalog) = Catalog::load(&config::runtime())
                && let Some(op) = catalog.operations.get_mut(&worker_op)
            {
                op.phase = if result.is_ok() { "Attached" } else { "Failed" }.into();
                op.error = result.err().map(|e| format!("{e:#}"));
                let _ = catalog.save(&config::runtime());
            }
        });
    if let Err(e) = worker {
        state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .starting
            .remove(&key);
        let _lock = config::ConfigLock::acquire()?;
        let mut catalog = Catalog::load(&config::runtime())?;
        let failed = catalog.operations.get_mut(&operation).unwrap();
        failed.phase = "Failed".into();
        failed.error = Some(e.to_string());
        let failed = failed.clone();
        catalog.save(&config::runtime())?;
        return Ok(Response::TaskOperation(Box::new(failed)));
    }
    Ok(Response::TaskOperation(Box::new(op)))
}

fn managed_args_with_hook(
    options: &Options,
    index: usize,
    control: &std::path::Path,
    hook: Option<&crate::hooks::HookContext>,
) -> Result<Vec<String>> {
    fs::create_dir_all(control.parent().unwrap())?;
    let path = control.to_string_lossy().replace('\'', "''");
    let source = options.source(index).unwrap_or_default();
    let source = serde_json::to_string(&source)?;
    let source = source.replace('\'', "''");
    // A small shell integration records facts about managed commands only.
    // Existing Profile and prompt are retained, with one wrapper around prompt.
    let environment = hook
        .map(serde_json::to_string)
        .transpose()?
        .unwrap_or_default()
        .replace('\'', "''");
    let script = format!(
        "$env:DEVTABS_HOOK_CONTEXT = '{environment}';\n{}",
        include_str!("shell.ps1")
    )
    .replace("__CONTROL__", &path)
    .replace("__SOURCE_JSON__", &source);
    let mut args = vec!["-NoLogo".into(), "-NoExit".into()];
    if options.no_profile {
        args.push("-NoProfile".into());
    }
    args.extend(["-EncodedCommand".into(), encode_script(&script)]);
    Ok(args)
}

fn host_clock() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
}
impl Host {
    fn valid_target(&self, t: &crate::notifications::Target) -> bool {
        self.workspaces.get(&t.workspace).is_some_and(|w| {
            w.sessions.iter().any(|m| {
                m.id == t.session
                    && m.task.task_id == t.task
                    && m.task.run_id == t.run
                    && (m.activity.results.iter().any(|r| r.version == t.result)
                        || t.result == m.activity.result)
            })
        }) || self.ledger.0.get(&t.run).is_some_and(|(target, state)| {
            target.task == t.task
                && target.session == t.session
                && target.workspace == t.workspace
                && state.results.iter().any(|r| r.version == t.result)
        })
    }
    fn notification_view(&self) -> crate::notifications::View {
        let pending = self.ledger.pending();
        crate::notifications::View {
            pending,
            diagnostic: self.diagnostic.iter().cloned().collect(),
            navigation: None,
            enabled: !self.notification_settings.disabled,
            policies: self
                .policies
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            configuration: self.notification_configuration.clone(),
        }
    }
    fn notify(&mut self, n: crate::notifications::Notice) {
        self.enqueue_notice(n, None);
    }
    fn notify_checked(&mut self, n: crate::notifications::Notice) {
        let category = if n.urgent { "failure" } else { "end" };
        self.enqueue_notice(n, Some(category));
    }
    fn enqueue_notice(
        &mut self,
        mut n: crate::notifications::Notice,
        check_category: Option<&str>,
    ) {
        if self.notification_routes.len() >= 1024 {
            self.notification_routes.retain(|_, client| {
                self.workspaces.values().any(|w| {
                    w.sessions
                        .iter()
                        .any(|m| m.owner.as_ref().is_some_and(|(id, _)| id == client))
                })
            });
        }
        if self.notification_routes.len() < 1024
            && let Some(client) = self
                .workspaces
                .get(&n.target.workspace)
                .and_then(|w| w.sessions.iter().find(|m| m.id == n.target.session))
                .and_then(|m| m.owner.as_ref())
                .map(|(id, _)| id.clone())
        {
            self.notification_routes
                .insert(format!("{}:{}", n.target.run, n.target.result), client);
        }
        let phase = self
            .workspaces
            .get(&n.target.workspace)
            .and_then(|w| w.sessions.iter().find(|m| m.id == n.target.session))
            .map(|m| m.activity.phase.clone());
        let category = check_category.unwrap_or(match phase {
            Some(crate::activity::Phase::Failed) => "failure",
            Some(crate::activity::Phase::Permission) => "permission",
            Some(crate::activity::Phase::Input) => "input",
            _ => "end",
        });
        let enabled = self
            .notification_settings
            .task_categories
            .get(&n.target.task)
            .and_then(|c| c.get(category))
            .or_else(|| self.notification_settings.categories.get(category))
            .copied()
            .unwrap_or(true);
        if !enabled
            || (category == "end"
                && self.notification_settings.after_check.get(&n.target.task) == Some(&true)
                && !n.test
                && check_category.is_none())
        {
            return;
        }
        n.silent = self.notification_settings.silent;
        n.category = category.into();
        let muted = self
            .notification_settings
            .muted
            .get(&n.target.task)
            .copied()
            .unwrap_or(false);
        let policy =
            self.policies
                .entry(n.target.task.clone())
                .or_insert(crate::notifications::Policy {
                    muted,
                    ..Default::default()
                });
        if policy.allow(!self.notification_settings.disabled, n.ending)
            && let Some(worker) = &self.notifications
            && worker.tx.try_send(n).is_err()
        {
            self.diagnostic
                .push_back("Notification queue full; result remains pending".into());
        }
    }
}

// Snapshot after serializing writers: an older ACK cannot overwrite a newer event on disk.
fn refresh_notifications(state: &Arc<Mutex<Host>>) {
    use sha2::{Digest, Sha256};
    // File I/O and validation happen before acquiring the session/input mutex.
    let read = crate::settings::store::SettingsStore::current().read();
    let (checked, policy, error) = match read {
        Ok(snapshot) => {
            let digest = Sha256::digest(snapshot.text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            match snapshot.require("notifications") {
                Ok(()) => (Some(digest), Some(snapshot.effective.notifications), None),
                Err(e) => (Some(digest), None, Some(format!("{e:#}"))),
            }
        }
        Err(e) => (None, None, Some(format!("{e:#}"))),
    };
    let mut host = state.lock().unwrap_or_else(|p| p.into_inner());
    host.notification_configuration.checked = checked.clone();
    host.notification_configuration.error = error;
    if let Some(policy) = policy {
        host.notification_settings.disabled = !policy.enabled.unwrap_or(true);
        host.notification_settings.silent = !policy.sound.unwrap_or(true);
        host.notification_settings.categories = policy.categories;
        host.notification_configuration.adopted = checked;
    }
}
fn persist_notification_state(
    state: &Arc<Mutex<Host>>,
    activity: bool,
    settings: bool,
) -> Result<()> {
    static WRITER: Mutex<()> = Mutex::new(());
    let _writer = WRITER.lock().unwrap_or_else(|p| p.into_inner());
    let host = state
        .lock()
        .map_err(|_| anyhow::anyhow!("Host unavailable"))?;
    let activity = activity.then(|| host.ledger.bytes()).transpose()?;
    let settings = settings
        .then(|| serde_json::to_vec(&host.notification_settings))
        .transpose()?;
    drop(host);
    if let Some(bytes) = activity {
        config::atomic_write(&config::runtime().join("activity.json"), &bytes)?;
    }
    if let Some(bytes) = settings {
        let _lock = config::ConfigLock::acquire()?;
        config::atomic_write(&config::runtime().join("notification-tasks.json"), &bytes)?;
    }
    Ok(())
}
