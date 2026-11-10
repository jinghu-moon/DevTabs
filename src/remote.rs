use crate::{
    protocol::{self, Request, Response, SessionInfo, Update},
    session::{State, Status},
    terminal::Terminal,
};
use anyhow::{Context, Result};
use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Remote {
    pub info: SessionInfo,
    pub workspace: String,
    pub client: String,
    shared: Arc<Mutex<State>>,
    queue: SyncSender<Pending>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    refresh: Arc<std::sync::atomic::AtomicBool>,
}
struct Pending {
    request: Request,
    receipt: Option<SyncSender<Result<()>>>,
}
fn dispatch(pending: Pending, state: &Arc<Mutex<State>>) {
    let result = protocol::rpc(pending.request).map(|_| ());
    if let Err(e) = &result {
        let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
        state.io_error = Some(format!("{e:#}"));
        state.revision += 1;
    }
    if let Some(receipt) = pending.receipt {
        let _ = receipt.try_send(result);
    }
}
impl Remote {
    pub fn attach(
        workspace: &str,
        info: SessionInfo,
        rows: u16,
        cols: u16,
        takeover: bool,
    ) -> Result<Self> {
        let client = uuid::Uuid::new_v4().to_string();
        let start = Instant::now();
        loop {
            match protocol::rpc(Request::Attach {
                takeover,
                workspace: workspace.into(),
                session: info.id.clone(),
                client: client.clone(),
                rows,
                cols,
            }) {
                Ok(_) => break,
                Err(error)
                    if error.to_string().contains("displayed in another view")
                        && start.elapsed() < Duration::from_secs(4) =>
                {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error),
            }
        }
        let shared = Arc::new(Mutex::new(State {
            terminal: Terminal::new(rows, cols, 5000),
            status: info.status.clone(),
            io_error: None,
            revision: 0,
            shell_pid: info.shell_pid,
            command: info.command.clone(),
        }));
        let (queue, receiver) = mpsc::sync_channel::<Pending>(1024);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let refresh = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_refresh = refresh.clone();
        let (thread_state, thread_stop) = (shared.clone(), stop.clone());
        let (workspace_id, session_id, client_id) =
            (workspace.to_string(), info.id.clone(), client.clone());
        thread::spawn(move || {
            let mut revision = None;
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut first = true;
            while !thread_stop.load(std::sync::atomic::Ordering::Relaxed) {
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(mut pending) => {
                        if pending.receipt.is_none()
                            && let Request::Input { bytes, .. } = &mut pending.request
                        {
                            // Coalesce consecutive key records into one ordered write.
                            // A paste is still a single message and never interleaves.
                            while bytes.len() < 1024 * 1024 {
                                match receiver.try_recv() {
                                    Ok(Pending {
                                        request: Request::Input { bytes: more, .. },
                                        receipt: None,
                                    }) => bytes.extend(more),
                                    Ok(other) => {
                                        dispatch(pending, &thread_state);
                                        pending = other;
                                        break;
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                        dispatch(pending, &thread_state);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(_) => {}
                }
                if last.elapsed() < Duration::from_millis(100) {
                    continue;
                }
                last = Instant::now();
                if thread_refresh.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    revision = None;
                }
                match protocol::rpc(Request::Poll {
                    workspace: workspace_id.clone(),
                    session: session_id.clone(),
                    client: client_id.clone(),
                    revision,
                    history: first,
                }) {
                    Ok(Response::Update(Some(update))) => {
                        revision = Some(update.revision);
                        apply(&thread_state, update);
                        first = false;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let mut state = thread_state.lock().unwrap_or_else(|p| p.into_inner());
                        state.io_error = Some(format!("Host: {e:#}"));
                        state.revision += 1;
                        break;
                    }
                }
            }
            let _ = protocol::rpc(Request::Detach { client: client_id });
        });
        Ok(Self {
            info,
            workspace: workspace.into(),
            client,
            shared,
            queue,
            stop,
            refresh,
        })
    }
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn shared_clone(&self) -> Arc<Mutex<State>> {
        self.shared.clone()
    }
    pub fn running(&self) -> bool {
        self.state().status == Status::Running
    }
    fn enqueue(&self, request: Request) -> Result<()> {
        self.queue
            .try_send(Pending {
                request,
                receipt: None,
            })
            .context("Session input queue is full; this input was not sent")
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        self.enqueue(Request::Input {
            workspace: self.workspace.clone(),
            session: self.info.id.clone(),
            client: self.client.clone(),
            bytes,
        })
    }
    pub fn send_checked(&self, bytes: Vec<u8>) -> Result<mpsc::Receiver<Result<()>>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.queue
            .try_send(Pending {
                request: Request::Input {
                    workspace: self.workspace.clone(),
                    session: self.info.id.clone(),
                    client: self.client.clone(),
                    bytes,
                },
                receipt: Some(tx),
            })
            .map_err(|_| anyhow::anyhow!("Input queue full; path was not sent"))?;
        Ok(rx)
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.enqueue(Request::Resize {
            workspace: self.workspace.clone(),
            session: self.info.id.clone(),
            client: self.client.clone(),
            rows,
            cols,
        })
    }
    pub fn terminate(&self) -> Result<()> {
        protocol::rpc(Request::Close {
            workspace: self.workspace.clone(),
        })?;
        Ok(())
    }
    pub fn refresh_history(&self) -> Result<()> {
        if let Response::Update(Some(update)) = protocol::rpc(Request::Poll {
            workspace: self.workspace.clone(),
            session: self.info.id.clone(),
            client: self.client.clone(),
            revision: None,
            history: true,
        })? {
            apply(&self.shared, update);
        }
        Ok(())
    }
    pub fn scroll(&self, delta: isize) {
        if self.refresh_history().is_ok() {
            let mut state = self.state();
            state.terminal.scroll(delta);
            if state.terminal.screen().scrollback() == 0 {
                self.refresh
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            state.revision += 1;
        }
    }
    pub fn live(&self) {
        let mut state = self.state();
        if state.terminal.screen().scrollback() > 0 {
            self.refresh
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        state.terminal.live();
        state.revision += 1;
    }
    pub fn action(&self, stop: bool, force: bool) -> Result<()> {
        let request = if stop {
            Request::Stop {
                workspace: self.workspace.clone(),
                session: self.info.id.clone(),
                force,
            }
        } else {
            Request::Run {
                workspace: self.workspace.clone(),
                session: self.info.id.clone(),
                source: None,
            }
        };
        protocol::rpc(request)?;
        Ok(())
    }
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = protocol::rpc(Request::Detach {
            client: self.client.clone(),
        });
    }
}
fn apply(shared: &Arc<Mutex<State>>, update: Update) {
    let _perf = crate::perf::span(crate::perf::Metric::RemoteApply);
    let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
    let offset = state.terminal.screen().scrollback();
    // Scrolling is a frozen local viewport. Output continues in the host;
    // returning to live output consumes the next complete screen snapshot.
    if offset > 0 {
        state.status = update.info.status;
        state.command = update.info.command;
        state.io_error = update.error;
        state.revision += 1;
        return;
    }
    let mut terminal = Terminal::new(update.rows, update.cols, 5000);
    // Recreate scrollback without leaving it in the visible viewport. The host
    // snapshot clears/repositions the visible screen after the history prefix.
    for row in update.history {
        terminal.process(&row);
        terminal.process(b"\r\n");
    }
    for _ in 1..update.rows {
        terminal.process(b"\r\n");
    }
    if update.alternate_screen {
        terminal.process(b"\x1b[?1049h");
    }
    terminal.process(&update.screen);
    terminal.parser.callbacks_mut().focus_events = update.focus_events;
    terminal.parser.callbacks_mut().win32_input = update.win32_input;
    terminal.scroll(offset as isize);
    state.terminal = terminal;
    state.status = update.info.status;
    state.command = update.info.command;
    state.shell_pid = update.info.shell_pid;
    state.io_error = update.error;
    state.revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::CommandStatus;
    fn update(text: &[u8], alternate: bool) -> Update {
        Update {
            revision: 1,
            rows: 4,
            cols: 20,
            screen: text.into(),
            history: (0..12)
                .map(|i| format!("history-{i}").into_bytes())
                .collect(),
            alternate_screen: alternate,
            focus_events: true,
            win32_input: true,
            info: SessionInfo {
                id: "s".into(),
                name: "Debug".into(),
                shell_pid: Some(123),
                status: Status::Running,
                command: CommandStatus::Idle,
            },
            error: None,
        }
    }
    #[test]
    fn snapshots_keep_all_history_and_freeze_scrolled_output() {
        let state = Arc::new(Mutex::new(State {
            terminal: Terminal::new(4, 20, 100),
            status: Status::Running,
            command: CommandStatus::Idle,
            shell_pid: None,
            io_error: None,
            revision: 0,
        }));
        apply(&state, update(b"\x1b[H\x1b[2Jlive", false));
        {
            let mut state = state.lock().unwrap();
            assert_eq!(state.terminal.screen().scrollback_len(), 12);
            state.terminal.scroll(12);
            assert!(state.terminal.screen().contents().contains("history-0"));
        }
        apply(&state, update(b"\x1b[H\x1b[2Jnext", false));
        assert!(
            state
                .lock()
                .unwrap()
                .terminal
                .screen()
                .contents()
                .contains("history-0")
        );
        state.lock().unwrap().terminal.live();
        apply(&state, update(b"\x1b[H\x1b[2Jfull-screen", true));
        let state = state.lock().unwrap();
        assert!(state.terminal.screen().alternate_screen());
        assert!(state.terminal.screen().contents().contains("full-screen"));
    }
}
