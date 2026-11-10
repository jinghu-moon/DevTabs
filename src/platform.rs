//! A helper joins the session's job BEFORE creating PowerShell and its descendants.
//! The independent session host owns the Job. The startup helper exits after
//! transferring shell identity; the owner waits on the actual shell handle.

#[cfg(windows)]
mod windows {
    use anyhow::{Context, Result, bail};
    use std::{
        ffi::OsString,
        mem::size_of,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        process::Command,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
            JobObjects::*,
            SystemServices::JOB_OBJECT_ASSIGN_PROCESS,
            Threading::GetCurrentProcess,
        },
    };

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }
    pub fn user_sid() -> Result<String> {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY,
                TOKEN_USER, TokenUser,
            },
            System::Threading::OpenProcessToken,
        };
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut size = 0;
        unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut size,
            )
        };
        let mut bytes = vec![0u64; (size as usize).div_ceil(8)];
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let user = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() };
        let mut text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut len = 0;
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
        unsafe {
            LocalFree(text.cast());
        }
        Ok(sid)
    }

    pub struct Job {
        handle: OwnedHandle,
        pub name: String,
    }

    impl Job {
        pub fn new(id: usize) -> Result<Self> {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let name = format!("Local\\DevTabs-{}-{id}-{stamp}", std::process::id());
            // SAFETY: name is nul-terminated; the returned owning handle is checked.
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), wide(&name).as_ptr()) };
            if raw.is_null() {
                return Err(std::io::Error::last_os_error()).context("Create session job");
            }
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            let ok = unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error()).context("Configure session job");
            }
            Ok(Self { handle, name })
        }

        pub fn terminate(&self) -> Result<()> {
            if unsafe { TerminateJobObject(self.handle.as_raw_handle() as HANDLE, 1) } == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Terminate session process tree");
            }
            Ok(())
        }
        pub fn terminate_commands(&self, shell: u32) -> Result<()> {
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_TERMINATE, TerminateProcess,
            };
            // Query THIS job only; exclude the persistent shell and waiting host.
            let mut buffer = vec![0usize; 4096];
            let mut needed = 0;
            let ok = unsafe {
                QueryInformationJobObject(
                    self.handle.as_raw_handle() as HANDLE,
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    (buffer.len() * size_of::<usize>()) as u32,
                    &mut needed,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Query session-owned descendants");
            }
            let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let count = list.NumberOfProcessIdsInList as usize;
            anyhow::ensure!(count < buffer.len(), "Too many session processes");
            let pointer = list.ProcessIdList.as_ptr();
            for index in 0..count {
                let pid = unsafe { *pointer.add(index) } as u32;
                if pid == shell {
                    continue;
                }
                let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
                if !handle.is_null() {
                    unsafe {
                        TerminateProcess(handle, 1);
                        CloseHandle(handle);
                    }
                }
            }
            Ok(())
        }
    }

    unsafe extern "system" fn control(event: u32) -> i32 {
        i32::from(event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT)
    }

    impl Drop for Job {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }

    pub fn run_host(args: Vec<OsString>) -> Result<i32> {
        if args.len() < 3 {
            bail!("Internal host requires a job, handshake path and shell");
        }
        let name = args[0].to_str().context("Invalid job name")?;
        let job = unsafe { OpenJobObjectW(JOB_OBJECT_ASSIGN_PROCESS, 0, wide(name).as_ptr()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error()).context("Open session job");
        }
        let assigned = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
        let error = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(job);
        }
        if assigned == 0 {
            return Err(error).context("Join session job before shell startup");
        }
        // Consume Ctrl+C in the waiting helper only; PowerShell keeps its own handler.
        if unsafe { SetConsoleCtrlHandler(Some(control), 1) } == 0 {
            return Err(std::io::Error::last_os_error()).context("Install host control handler");
        }
        let child = Command::new(&args[2])
            .args(&args[3..])
            .spawn()
            .context("Start PowerShell")?;
        let path = std::path::Path::new(&args[1]);
        std::fs::write(path, child.id().to_string())?;
        let ack = path.with_extension("ack");
        let start = std::time::Instant::now();
        // Keep the child process handle alive until its new owner opens one,
        // including programs that exit immediately after launch.
        while !ack.exists() {
            anyhow::ensure!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "Shell handshake timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(0)
    }
    pub fn wait_shell(pid: u32, handshake: &std::path::Path) -> Result<u32> {
        use windows_sys::Win32::System::Threading::*;
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error()).context("Open shell process");
        }
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        std::fs::write(handshake.with_extension("ack"), b"ready")?;
        let _ = std::fs::remove_file(handshake);
        let result = unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
        anyhow::ensure!(result == 0, "Wait for shell: {result}");
        let mut code = 0;
        if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } == 0 {
            return Err(std::io::Error::last_os_error()).context("Read shell exit code");
        }
        Ok(code)
    }
}

#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub struct Job {
    pub name: String,
}
#[cfg(not(windows))]
impl Job {
    pub fn new(_: usize) -> anyhow::Result<Self> {
        anyhow::bail!("Windows required")
    }
    pub fn terminate(&self) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn terminate_commands(&self, _: u32) -> anyhow::Result<()> {
        Ok(())
    }
}
#[cfg(not(windows))]
pub fn run_host(_: Vec<std::ffi::OsString>) -> anyhow::Result<i32> {
    anyhow::bail!("Windows required")
}
