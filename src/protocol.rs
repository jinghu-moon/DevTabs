use crate::{
    launch::Options,
    session::{CommandStatus, Status},
};
use anyhow::{Context, Result, bail};
use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

pub const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub project: PathBuf,
    pub options: Options,
    pub sessions: Vec<SessionInfo>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub shell_pid: Option<u32>,
    pub status: Status,
    pub command: CommandStatus,
}
#[derive(Serialize, Deserialize)]
pub enum Request {
    Layout {
        workspace: String,
        layout: crate::launch::Layout,
    },
    Ping,
    Open {
        options: Options,
    },
    List,
    Attach {
        #[serde(default)]
        takeover: bool,
        workspace: String,
        session: String,
        client: String,
        rows: u16,
        cols: u16,
    },
    Poll {
        workspace: String,
        session: String,
        client: String,
        revision: Option<u64>,
        history: bool,
    },
    Input {
        workspace: String,
        session: String,
        client: String,
        bytes: Vec<u8>,
    },
    Resize {
        workspace: String,
        session: String,
        client: String,
        rows: u16,
        cols: u16,
    },
    Detach {
        client: String,
    },
    Close {
        workspace: String,
    },
    Stop {
        workspace: String,
        session: String,
        force: bool,
    },
    Run {
        workspace: String,
        session: String,
        source: Option<String>,
    },
    DebugTarget {
        workspace: Option<String>,
    },
    Shutdown,
}
#[derive(Serialize, Deserialize)]
pub struct Update {
    pub revision: u64,
    pub rows: u16,
    pub cols: u16,
    pub screen: Vec<u8>,
    pub history: Vec<Vec<u8>>,
    pub alternate_screen: bool,
    pub focus_events: bool,
    pub win32_input: bool,
    pub info: SessionInfo,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub enum Response {
    Host { pid: u32 },
    Ok,
    Error(String),
    Workspace(WorkspaceInfo),
    Workspaces(Vec<WorkspaceInfo>),
    Update(Option<Update>),
    Target(Option<String>),
}

pub fn endpoint() -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    crate::config::key(&crate::config::home()).hash(&mut hash);
    std::env::var_os("DEVTABS_INSTANCE").hash(&mut hash);
    format!("DevTabs-v2-{:016x}", hash.finish())
}
fn transfer(stream: &mut Stream, bytes: &mut [u8], write: bool) -> Result<()> {
    let start = Instant::now();
    let mut offset = 0;
    while offset < bytes.len() {
        let mut available = bytes.len() - offset;
        #[cfg(windows)]
        if !write {
            use std::os::windows::io::{AsHandle, AsRawHandle};
            let Stream::NamedPipe(pipe) = &*stream;
            let mut count = 0;
            let ok = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    pipe.inner().as_handle().as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut count,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error()).context("Read host pipe");
            }
            if count == 0 {
                anyhow::ensure!(
                    start.elapsed() < Duration::from_secs(15),
                    "Session host read timed out"
                );
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            available = available.min(count as usize);
        }
        let result = if write {
            stream.write(&bytes[offset..])
        } else {
            stream.read(&mut bytes[offset..offset + available])
        };
        match result {
            Ok(0) => bail!("Session host disconnected"),
            Ok(n) => offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                if start.elapsed() > Duration::from_secs(15) {
                    bail!("Session host timed out");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn send<T: Serialize>(stream: &mut Stream, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(bytes.len() <= MAX_FRAME, "IPC frame too large");
    let mut length = (bytes.len() as u32).to_le_bytes();
    transfer(stream, &mut length, true)?;
    transfer(stream, &mut bytes, true)
}
pub fn receive<T: DeserializeOwned>(stream: &mut Stream) -> Result<T> {
    let mut length = [0; 4];
    transfer(stream, &mut length, false)?;
    let count = u32::from_le_bytes(length) as usize;
    anyhow::ensure!(count <= MAX_FRAME, "IPC frame too large");
    let mut bytes = vec![0; count];
    transfer(stream, &mut bytes, false)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn rpc(request: Request) -> Result<Response> {
    let _perf = crate::perf::span(if matches!(request, Request::Poll { .. }) {
        crate::perf::Metric::RpcPoll
    } else {
        crate::perf::Metric::Rpc
    });
    let name = endpoint();
    let mut stream = Stream::connect(name.to_ns_name::<GenericNamespaced>()?)
        .context("Connect to DevTabs host")?;
    send(&mut stream, &request)?;
    let response = receive(&mut stream)?;
    if let Response::Error(error) = response {
        bail!("{error}");
    }
    Ok(response)
}
