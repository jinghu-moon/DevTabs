use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn directory(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("Programs")
            .join("DevTabs")
    })
}
pub fn path_value(existing: &str, directory: &str, add: bool) -> String {
    let normalized = directory
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase();
    let mut items = existing
        .split(';')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            p.trim_matches('"')
                .trim_end_matches(['\\', '/'])
                .to_lowercase()
                != normalized
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if add {
        items.push(directory.into());
    }
    items.join(";")
}
fn modify_path(path: &Path, add: bool) -> Result<()> {
    modify_path_at(path, add, "Environment")?;
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text = wide("Environment");
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            text.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1000,
            std::ptr::null_mut(),
        );
    }
    Ok(())
}
#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
#[cfg(windows)]
fn modify_path_at(path: &Path, add: bool, subkey: &str) -> Result<()> {
    use windows_sys::Win32::System::Registry::*;
    let (key, name) = (wide(subkey), wide("Path"));
    let (mut kind, mut count) = (REG_EXPAND_SZ, 0);
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            &mut kind,
            std::ptr::null_mut(),
            &mut count,
        )
    };
    anyhow::ensure!(
        result == 0 || result == 2,
        "Read current-user PATH: {result}"
    );
    let mut buffer = vec![0u16; count as usize / 2 + 1];
    if result == 0 {
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut count,
            )
        };
        anyhow::ensure!(result == 0, "Read current-user PATH: {result}");
    }
    let end = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
    let existing = String::from_utf16(&buffer[..end])?;
    let next = wide(&path_value(&existing, &path.to_string_lossy(), add));
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            kind,
            next.as_ptr().cast(),
            (next.len() * 2) as u32,
        )
    };
    anyhow::ensure!(result == 0, "Write current-user PATH: {result}");
    Ok(())
}
#[cfg(not(windows))]
fn modify_path_at(_: &Path, _: bool, _: &str) -> Result<()> {
    anyhow::bail!("Windows required")
}
pub fn install(path: Option<PathBuf>, no_path: bool) -> Result<()> {
    let _lock = crate::config::ConfigLock::acquire()?;
    let path = directory(path);
    fs::create_dir_all(&path)?;
    let target = path.join("devtabs.exe");
    let source = std::env::current_exe()?;
    if dunce::canonicalize(&source).ok() == dunce::canonicalize(&target).ok() {
        if !no_path {
            modify_path(&path, true)?;
        }
        println!("Already installed at {}", target.display());
        return Ok(());
    }
    let staging = path.join("devtabs.new.exe");
    fs::copy(&source, &staging).context("Copy DevTabs executable")?;
    if target.exists() {
        let old = path.join("devtabs.old.exe");
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)
            .context("Close installed DevTabs processes before replacing it")?;
    }
    if let Err(error) = fs::rename(&staging, &target) {
        let _ = fs::rename(path.join("devtabs.old.exe"), &target);
        return Err(error).context("Activate installed executable");
    }
    if !no_path {
        modify_path(&path, true)?;
    }
    println!(
        "Installed {}. Open a new terminal to refresh PATH. Existing shells keep their environment.",
        target.display()
    );
    Ok(())
}
pub fn uninstall(path: Option<PathBuf>, no_path: bool) -> Result<()> {
    let _lock = crate::config::ConfigLock::acquire()?;
    let path = directory(path);
    if !no_path {
        modify_path(&path, false)?;
    }
    for name in ["devtabs.exe", "devtabs.old.exe", "devtabs.new.exe"] {
        let file = path.join(name);
        if file.exists() {
            fs::remove_file(&file).with_context(|| {
                format!(
                    "Close DevTabs and run uninstall from another copy: {}",
                    file.display()
                )
            })?;
        }
    }
    println!("Removed DevTabs executable and its PATH entry; user configuration retained.");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_changes_preserve_others_and_are_idempotent() {
        let p = "C:\\Windows;D:\\Tools;C:\\DevTabs\\";
        assert_eq!(
            path_value(p, "c:\\devtabs", true),
            "C:\\Windows;D:\\Tools;c:\\devtabs"
        );
        assert_eq!(path_value(p, "c:\\devtabs", false), "C:\\Windows;D:\\Tools");
    }
    #[test]
    #[cfg(windows)]
    fn real_registry_path_updates_preserve_expansions_and_other_entries() {
        use windows_sys::Win32::System::Registry::*;
        let key = format!("Software\\DevTabs-Test-{}", uuid::Uuid::new_v4());
        let (key_wide, name, initial) = (
            wide(&key),
            wide("Path"),
            wide("%USERPROFILE%\\bin;C:\\Windows"),
        );
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key_wide.as_ptr(),
                name.as_ptr(),
                REG_EXPAND_SZ,
                initial.as_ptr().cast(),
                (initial.len() * 2) as u32,
            )
        };
        assert_eq!(result, 0);
        let dir = Path::new("D:\\DevTabs 测试");
        modify_path_at(dir, true, &key).unwrap();
        modify_path_at(dir, true, &key).unwrap();
        modify_path_at(dir, false, &key).unwrap();
        let (mut buffer, mut count, mut kind) = ([0u16; 256], 512, 0);
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key_wide.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut count,
            )
        };
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, key_wide.as_ptr());
        }
        assert_eq!(result, 0);
        assert_eq!(kind, REG_EXPAND_SZ);
        assert_eq!(
            String::from_utf16(&buffer[..count as usize / 2 - 1]).unwrap(),
            "%USERPROFILE%\\bin;C:\\Windows"
        );
    }
}
