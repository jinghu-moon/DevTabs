use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard,
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
};

use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::{
    platform::Job,
    terminal::{Snapshot, Terminal},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Running,
    Exited(u32),
    Failed(String),
}
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Exited(code) => write!(f, "exited {code}"),
            Self::Failed(error) => write!(f, "failed: {error}"),
        }
    }
}

pub struct State {
    pub terminal: Terminal,
    pub status: Status,
    pub io_error: Option<String>,
    pub revision: u64,
    pub shell_pid: Option<u32>,
    pub command: CommandStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CommandStatus {
    #[default]
    Idle,
    Starting,
    Running,
    Exited(i32),
}

pub struct Session {
    pub id: usize,
    pub(crate) shared: Arc<Mutex<State>>,
    input: SyncSender<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    job: Job,
    size: (u16, u16),
    pub control: Option<std::path::PathBuf>,
}

pub struct Spawn<'a> {
    pub id: usize,
    pub executable: &'a Path,
    pub shell: &'a Path,
    pub args: &'a [String],
    pub project: &'a Path,
    pub rows: u16,
    pub cols: u16,
    pub history: usize,
}

pub struct Client {
    shared: Arc<Mutex<State>>,
    pub info: crate::protocol::SessionInfo,
    remote: Option<crate::remote::Remote>,
}
impl Client {
    pub fn hidden(info: crate::protocol::SessionInfo) -> Self {
        let state = State {
            terminal: Terminal::new(24, 80, 0),
            status: info.status.clone(),
            io_error: None,
            revision: 0,
            shell_pid: info.shell_pid,
            command: info.command.clone(),
        };
        Self {
            shared: Arc::new(Mutex::new(state)),
            info,
            remote: None,
        }
    }
    pub fn attach(&mut self, workspace: &str, rows: u16, cols: u16, takeover: bool) -> Result<()> {
        self.attach_view(workspace, rows, cols, takeover, None)
    }
    pub fn attach_view(
        &mut self,
        workspace: &str,
        rows: u16,
        cols: u16,
        takeover: bool,
        floating: Option<u64>,
    ) -> Result<()> {
        if self.remote.is_none() {
            let remote = crate::remote::Remote::attach_view(
                workspace,
                self.info.clone(),
                rows,
                cols,
                takeover,
                floating,
            )?;
            self.shared = remote.shared_clone();
            self.remote = Some(remote);
        } else {
            self.resize(rows, cols)?;
        }
        Ok(())
    }
    pub fn detach(&mut self) {
        self.remote = None;
    }
    pub fn attached(&self) -> bool {
        self.remote.is_some()
    }
    pub fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.shared)
    }
    pub fn running(&self) -> bool {
        self.state().status == Status::Running
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        self.remote
            .as_ref()
            .context("View is detached")?
            .send(bytes)
    }
    pub fn send_checked(&self, bytes: Vec<u8>) -> Result<mpsc::Receiver<Result<()>>> {
        self.remote
            .as_ref()
            .context("View is detached")?
            .send_checked(bytes)
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if let Some(remote) = &mut self.remote {
            remote.resize(rows, cols)?;
        }
        Ok(())
    }
    pub fn scroll(&self, delta: isize) {
        if let Some(remote) = &self.remote {
            remote.scroll(delta);
        }
    }
    pub fn live(&self) {
        if let Some(remote) = &self.remote {
            remote.live();
        }
    }
    pub fn history(&self) -> Result<()> {
        if let Some(remote) = &self.remote {
            remote.refresh_history()?;
        }
        Ok(())
    }
    pub fn action(&self, stop: bool, force: bool) -> Result<()> {
        self.remote
            .as_ref()
            .context("Select an attached session")?
            .action(stop, force)
    }
}

impl Session {
    pub fn spawn(config: Spawn<'_>) -> Result<Self> {
        let Spawn {
            id,
            executable,
            shell,
            args,
            project,
            rows,
            cols,
            history,
        } = config;
        anyhow::ensure!(
            rows >= 2 && cols >= 2,
            "ConPTY requires at least 2 rows and columns for interactive shells"
        );
        let rows = crate::terminal::nonzero(rows).get();
        let cols = crate::terminal::nonzero(cols).get();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Create ConPTY")?;
        // Created after the PTY, so startup errors kill the job BEFORE closing
        // ConPTY (which can otherwise block waiting for a live child's output).
        let job = Job::new(id)?;
        let mut cmd = CommandBuilder::new(executable);
        cmd.arg("__host");
        cmd.arg(&job.name);
        let handshake =
            std::env::temp_dir().join(format!("DevTabs-launch-{}.pid", uuid::Uuid::new_v4()));
        cmd.arg(&handshake);
        cmd.arg(shell);
        cmd.args(args);
        cmd.cwd(project);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "DevTabs");
        let mut child = pair.slave.spawn_command(cmd).context("Start PTY host")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let shared = Arc::new(Mutex::new(State {
            terminal: Terminal::new(rows, cols, history),
            status: Status::Running,
            io_error: None,
            revision: 0,
            shell_pid: None,
            command: CommandStatus::Idle,
        }));
        let (input, receiver) = mpsc::sync_channel::<Vec<u8>>(64);
        let write_state = shared.clone();
        thread::Builder::new()
            .name(format!("session-{id}-input"))
            .spawn(move || {
                for bytes in receiver {
                    if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                        let mut state = lock(&write_state);
                        state.io_error = Some(format!("Input: {error}"));
                        state.revision += 1;
                        break;
                    }
                }
            })?;
        let read_state = shared.clone();
        let response_writer = input.clone();
        thread::Builder::new().name(format!("session-{id}-output")).spawn(move || {
            let mut buffer = [0u8;8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let responses = {
                            let mut state = lock(&read_state);
                            let responses = state.terminal.process(&buffer[..count]);
                            state.revision += 1;
                            responses
                        };
                        if !responses.is_empty() && response_writer.try_send(responses).is_err() {
                            let mut state = lock(&read_state);
                            state.io_error = Some("Terminal response queue busy; protocol reply could not be sent".into());
                            state.revision += 1;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let mut state = lock(&read_state);
                        state.io_error = Some(format!("Output: {error}"));
                        state.revision += 1;
                        break;
                    }
                }
            }
        })?;
        let wait_state = shared.clone();
        thread::Builder::new()
            .name(format!("session-{id}-wait"))
            .spawn(move || {
                let result = (|| -> Result<u32> {
                    let start = std::time::Instant::now();
                    loop {
                        if let Ok(text) = std::fs::read_to_string(&handshake)
                            && let Ok(pid) = text.parse::<u32>()
                        {
                            lock(&wait_state).shell_pid = Some(pid);
                            #[cfg(windows)]
                            return crate::platform::wait_shell(pid, &handshake);
                        }
                        if let Some(status) = child.try_wait()? {
                            return Ok(status.exit_code());
                        }
                        anyhow::ensure!(
                            start.elapsed() < std::time::Duration::from_secs(10),
                            "Shell identity handshake timed out"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                })();
                let _ = child.wait();
                let _ = std::fs::remove_file(&handshake);
                let _ = std::fs::remove_file(handshake.with_extension("ack"));
                let mut state = lock(&wait_state);
                state.status = match result {
                    Ok(code) => Status::Exited(code),
                    Err(error) => Status::Failed(error.to_string()),
                };
                state.revision += 1;
            })?;
        Ok(Self {
            id,
            shared,
            input,
            master: pair.master,
            job,
            size: (rows, cols),
            control: None,
        })
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.shared)
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state().terminal.snapshot()
    }
    pub fn running(&self) -> bool {
        self.state().status == Status::Running
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.running() {
            bail!("This shell has exited; its last output is retained");
        }
        match self.input.try_send(bytes) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => bail!(
                "Input queue busy: this input was not sent; retry when the program is reading input"
            ),
            Err(TrySendError::Disconnected(_)) => bail!("Session input disconnected"),
        }
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(
            rows >= 2 && cols >= 2,
            "Refusing an unusable ConPTY size; keep the previous dimensions"
        );
        let rows = crate::terminal::nonzero(rows).get();
        let cols = crate::terminal::nonzero(cols).get();
        if self.size == (rows, cols) {
            return Ok(());
        }
        // Never hold the parser lock across a ConPTY call: resize may wait for
        // output to drain, which requires the reader to acquire that same lock.
        {
            let mut state = self.state();
            state.terminal.resize(rows, cols);
            state.revision += 1;
        }
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.size = (rows, cols);
        Ok(())
    }
    pub fn terminate(&self) -> Result<()> {
        self.job.terminate()
    }
    pub fn refresh_command(&self) {
        let Some(path) = &self.control else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        let pid = value["pid"].as_u64().map(|p| p as u32);
        let mut command = match value["state"].as_str() {
            Some("running") => CommandStatus::Running,
            Some("exited") => CommandStatus::Exited(value["code"].as_i64().unwrap_or(1) as i32),
            _ => CommandStatus::Idle,
        };
        if command != CommandStatus::Running && path.with_extension("request").exists() {
            command = CommandStatus::Starting;
        }
        let mut state = self.state();
        if state.command != command || state.shell_pid != pid {
            state.command = command;
            state.shell_pid = pid;
            state.revision += 1;
        }
    }
    pub fn run_command(&self, source: Option<String>) -> Result<()> {
        self.refresh_command();
        anyhow::ensure!(
            !matches!(
                self.state().command,
                CommandStatus::Running | CommandStatus::Starting
            ),
            "Managed command still running; stop it first"
        );
        let path = self
            .control
            .as_ref()
            .context("This session has no managed command integration")?
            .with_extension("request");
        crate::config::atomic_write(
            &path,
            &serde_json::to_vec(&serde_json::json!({"source":source}))?,
        )?;
        let mut state = self.state();
        state.command = CommandStatus::Starting;
        state.revision += 1;
        // The private PSReadLine binding consumes this request at a prompt.
        Ok(())
    }
    pub fn force_command(&self) -> Result<()> {
        self.refresh_command();
        let pid = self
            .state()
            .shell_pid
            .context("Shell identity is not available yet")?;
        self.job.terminate_commands(pid)
    }
    pub fn scroll(&self, delta: isize) {
        let mut state = self.state();
        state.terminal.scroll(delta);
        state.revision += 1;
    }
    pub fn live(&self) {
        let mut state = self.state();
        state.terminal.live();
        state.revision += 1;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Also terminates descendants left behind after the shell itself exits.
        let _ = self.job.terminate();
        if let Some(path) = &self.control {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("request"));
        }
    }
}

fn lock(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
