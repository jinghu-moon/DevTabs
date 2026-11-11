use crate::protocol::{self, Request};
use anyhow::{Context, Result};
use std::process::Command;

pub fn spawn_view(workspace: &str, view: &str, new_window: bool) -> Result<()> {
    if view == "debug" {
        let protocol::Response::Workspaces(work) = protocol::rpc(Request::List)? else {
            anyhow::bail!("No workspace list");
        };
        if work
            .iter()
            .find(|w| w.id == workspace)
            .and_then(|w| w.sessions.get(1))
            .is_none_or(|s| s.placement != crate::presentation::Placement::Docked)
        {
            return Ok(());
        }
    }
    let mut options = crate::launch::Options::parse_for_view();
    options.new_window = new_window;
    spawn_workspace_view(workspace, view, &options)
}
pub fn spawn_workspace_view(
    workspace: &str,
    view: &str,
    options: &crate::launch::Options,
) -> Result<()> {
    let mut command = workspace_command(workspace, view, options, &std::env::current_exe()?);
    let status = command.status().context("Open Windows Terminal view")?;
    anyhow::ensure!(status.success(), "Windows Terminal rejected view launch");
    Ok(())
}
fn workspace_command(
    workspace: &str,
    view: &str,
    options: &crate::launch::Options,
    executable: &std::path::Path,
) -> Command {
    let mut command = Command::new("wt.exe");
    if options.new_window {
        command.args([
            "-w",
            &options
                .window_group
                .as_ref()
                .map_or_else(|| "new".into(), |id| format!("DevTabs-{id}")),
        ]);
    } else {
        command.args(["-w", "0"]);
    }
    command.args([
        "new-tab",
        "--useApplicationTitle",
        "--title",
        "DevTabs Session",
    ]);
    if let Some(color) = &options.color {
        command.args(["--tabColor", color]);
    }
    // suppressApplicationTitle would prevent role/project titles; do not rely on
    // tab indices for identity. The view always attaches by Workspace/SessionId.
    command
        .arg(executable)
        .args(["--workspace", workspace, "--view", view, "--data-dir"])
        .arg(crate::config::home());
    if let Some(language) = options.language {
        command.args([
            "--language",
            match language {
                crate::i18n::Language::Auto => "auto",
                crate::i18n::Language::EnUs => "en-us",
                crate::i18n::Language::ZhCn => "zh-cn",
            },
        ]);
    }
    command
}
#[cfg(windows)]
fn find_window(title: &str) -> Result<windows_sys::Win32::Foundation::HWND> {
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
    anyhow::ensure!(!find.found.is_null(), "Terminal window is not open");
    Ok(find.found)
}
#[cfg(windows)]
pub fn focus_workspace(workspace: &str) -> Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let hwnd = find_window(workspace)?;
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn focus_workspace(_: &str) -> Result<()> {
    anyhow::bail!("Windows required")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_target_group_color_and_child_identity_are_unambiguous() {
        let mut o = crate::launch::Options::parse_for_view();
        o.color = Some("#123ABC".into());
        o.language = Some(crate::i18n::Language::ZhCn);
        let exe = std::path::Path::new(r"D:\项目 with space\devtabs.exe");
        let args = |o: &crate::launch::Options, view| {
            workspace_command("workspace-id", view, o, exe)
                .get_args()
                .map(|s| s.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let current = args(&o, "main");
        assert_eq!(&current[..3], ["-w", "0", "new-tab"]);
        assert!(current.windows(2).any(|s| s == ["--tabColor", "#123ABC"]));
        assert!(
            current
                .windows(2)
                .any(|s| s == ["--workspace", "workspace-id"])
        );
        assert!(current.windows(2).any(|s| s == ["--language", "zh-cn"]));
        assert!(current.iter().any(|s| s == &exe.to_string_lossy()));
        assert!(!current.iter().any(|s| s == "--new-window"));
        o.new_window = true;
        assert_eq!(args(&o, "main")[1], "new");
        o.window_group = Some("batch-1".into());
        assert_eq!(args(&o, "main")[1], "DevTabs-batch-1");
        assert_eq!(args(&o, "debug")[1], "DevTabs-batch-1");
    }
}
