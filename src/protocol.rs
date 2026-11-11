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
pub const BUILD_ID: &str = env!("DEVTABS_BUILD_ID");
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
    #[serde(default)]
    pub placement: crate::presentation::Placement,
}
#[derive(Serialize, Deserialize)]
pub enum Request {
    Layout {
        workspace: String,
        layout: crate::launch::Layout,
    },
    Ping,
    Open {
        options: Box<Options>,
    },
    List,
    PrepareFloat {
        target: crate::presentation::Target,
    },
    FloatingTarget,
    ViewReady {
        workspace: String,
    },
    CancelFloat {
        revision: u64,
    },
    Present {
        target: crate::presentation::Target,
        placement: crate::presentation::Placement,
    },
    Attach {
        #[serde(default)]
        floating: Option<u64>,
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
    Workspace(Box<WorkspaceInfo>),
    Workspaces(Vec<WorkspaceInfo>),
    Update(Option<Update>),
    FloatPlan(crate::presentation::Plan),
    FirstView(bool),
    Floating(Option<crate::presentation::Ticket>),
}

pub fn endpoint() -> String {
    endpoint_for(
        &crate::config::home(),
        std::env::var_os("DEVTABS_INSTANCE").as_deref(),
        BUILD_ID,
    )
}
fn endpoint_for(home: &std::path::Path, instance: Option<&std::ffi::OsStr>, build: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    crate::config::key(home).hash(&mut hash);
    instance.hash(&mut hash);
    // Matching package versions do not imply matching development builds.
    format!("DevTabs-v5-{build}-{:016x}", hash.finish())
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
            Ok(0) => {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
                    .context("Session host disconnected");
            }
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

/// Only transport loss permits restarting a host. Invalid responses, access
/// denial and host-reported application errors must remain visible to users.
pub(crate) fn disconnected(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            std::io::ErrorKind::NotFound
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::UnexpectedEof
        )
    })
}

#[cfg(test)]
mod build_tests {
    use super::*;
    #[test]
    fn reconnect_only_recognizes_transport_loss_through_context() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let error =
                anyhow::Error::from(std::io::Error::from(kind)).context("Connect to DevTabs host");
            assert!(disconnected(&error));
        }
        assert!(!disconnected(&anyhow::anyhow!("Invalid host response")));
        assert!(!disconnected(&anyhow::Error::from(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        ))));
    }
    #[test]
    fn host_scope_separates_builds_and_instances_but_reuses_identical_builds() {
        let home = std::path::Path::new("D:/DevTabs/data");
        let a = endpoint_for(home, None, "build-a");
        assert_eq!(a, endpoint_for(home, None, "build-a"));
        assert_ne!(a, endpoint_for(home, None, "build-b"));
        assert_ne!(a, endpoint_for(home, Some("isolated".as_ref()), "build-a"));
        assert_ne!(
            a,
            endpoint_for(std::path::Path::new("D:/other"), None, "build-a")
        );
        assert_eq!(BUILD_ID.len(), 16);
        assert!(BUILD_ID.bytes().all(|c| c.is_ascii_hexdigit()));
    }
}
