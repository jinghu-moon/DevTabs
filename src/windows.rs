use crate::protocol::{self, Request};
use anyhow::{Context, Result};
use std::process::Command;

pub fn show_debug(workspace: &str) -> Result<()> {
    protocol::rpc(Request::DebugTarget {
        workspace: Some(workspace.into()),
    })?;
    if focus_debug().is_ok() {
        // A previous layout's view may still be releasing its lease. If it
        // closes during that transition, restore the requested floating view.
        let workspace = workspace.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(750));
            if let Ok(crate::protocol::Response::Workspaces(work)) = protocol::rpc(Request::List)
                && work
                    .iter()
                    .any(|w| w.id == workspace && w.options.layout == crate::launch::Layout::Float)
                && window(&format!("[{}]", protocol::endpoint()), false, false).is_err()
            {
                let _ = spawn_view(&workspace, "debug-shared", true);
            }
        });
        return Ok(());
    }
    spawn_view(workspace, "debug-shared", true)
}
pub fn spawn_view(workspace: &str, view: &str, new_window: bool) -> Result<()> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new("wt.exe");
    if new_window {
        command.args([
            "-w",
            &format!("DevTabs-Debug-{}", protocol::endpoint()),
            "--size",
            "100,28",
        ]);
    } else {
        command.args(["-w", &format!("DevTabs-{}", protocol::endpoint())]);
    }
    command.args([
        "new-tab",
        "--useApplicationTitle",
        "--title",
        if view == "debug-shared" {
            "DevTabs Debug"
        } else {
            "DevTabs Session"
        },
    ]);
    // suppressApplicationTitle would prevent role/project titles; do not rely on
    // tab indices for identity. The view always attaches by Workspace/SessionId.
    command
        .arg(executable)
        .args(["--workspace", workspace, "--view", view, "--data-dir"])
        .arg(crate::config::home());
    let status = command.status().context("Open Windows Terminal view")?;
    anyhow::ensure!(status.success(), "Windows Terminal rejected view launch");
    Ok(())
}
#[cfg(windows)]
pub struct WindowLease(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WindowLease {
    pub fn shared_debug() -> Result<Option<Self>> {
        use windows_sys::Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        };
        let name: Vec<u16> = format!("Local\\{}-debug-view", protocol::endpoint())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let raw = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(raw) };
            return Ok(None);
        }
        Ok(Some(Self(raw)))
    }
}
#[cfg(windows)]
impl Drop for WindowLease {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn window(title: &str, toggle_topmost: bool, activate: bool) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::*,
    };
    struct Find {
        title: String,
        found: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
        let find = unsafe { &mut *(param as *mut Find) };
        let mut text = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
        let mut class = [0u16; 128];
        let class_len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let class = String::from_utf16_lossy(&class[..class_len.max(0) as usize]);
        if len > 0
            && class == "CASCADIA_HOSTING_WINDOW_CLASS"
            && String::from_utf16_lossy(&text[..len as usize]).contains(&find.title)
        {
            find.found = hwnd;
            return 0;
        }
        1
    }
    let mut find = Find {
        title: title.into(),
        found: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(visit), &mut find as *mut Find as isize) };
    anyhow::ensure!(!find.found.is_null(), "Debug window is not open");
    if !activate {
        return Ok(());
    }
    unsafe {
        ShowWindow(find.found, SW_RESTORE);
        if toggle_topmost {
            let topmost = GetWindowLongPtrW(find.found, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
            SetWindowPos(
                find.found,
                if !topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
        }
        SetForegroundWindow(find.found);
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn window(_: &str, _: bool, _: bool) -> Result<()> {
    anyhow::bail!("Windows required")
}

pub fn debug_title(project: &str) -> String {
    format!("DevTabs Debug · {project} · [{}]", protocol::endpoint())
}
pub fn focus_debug() -> Result<()> {
    window(&format!("[{}]", protocol::endpoint()), false, true)
}
pub fn toggle_debug_topmost() -> Result<()> {
    window(&format!("[{}]", protocol::endpoint()), true, true)
}
pub fn focus_workspace(workspace: &str) -> Result<()> {
    window(workspace, false, true)
}
