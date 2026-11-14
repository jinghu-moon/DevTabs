use crate::launch::{Agent, Layout};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn home() -> PathBuf {
    let path = std::env::var_os("DEVTABS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("DevTabs")
        });
    if path.is_relative() {
        std::env::current_dir().unwrap_or_default().join(path)
    } else {
        path
    }
}
pub fn key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Project {
    pub note: String,
    pub title: Option<String>,
    pub color: Option<String>,
    pub agent_args: Vec<String>,
    pub resume_latest: Option<bool>,
    pub sidebar: crate::explorer::Preferences,
    pub agent: Option<Agent>,
    pub debug_command: Option<String>,
    pub layout: Option<Layout>,
    pub composition: Option<crate::launch::Composition>,
    pub sessions: Vec<SessionConfig>,
    pub favorite: bool,
    pub last_opened: u64,
}
impl Project {
    pub fn terminal_composition(&self) -> crate::launch::Composition {
        self.composition.unwrap_or(if self.sessions.is_empty() {
            crate::launch::Composition::AgentDebug
        } else {
            crate::launch::Composition::Custom
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    pub name: String,
    pub command: Option<String>,
    pub directory: Option<PathBuf>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: crate::i18n::Language,
    pub theme: crate::appearance::Theme,
    pub overview: crate::overview::model::Preferences,
    pub presets: BTreeMap<String, Vec<crate::overview::model::LaunchSpec>>,
    pub default_agent: Option<Agent>,
    pub roots: Vec<PathBuf>,
    pub added: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
    pub projects: BTreeMap<String, Project>,
}
impl Config {
    pub fn update(edit: impl FnOnce(&mut Self) -> Result<()>) -> Result<Self> {
        let _lock = ConfigLock::acquire()?;
        let mut value = Self::load()?;
        edit(&mut value)?;
        value.save()?;
        Ok(value)
    }
    pub fn load() -> Result<Self> {
        let path = home().join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("Read {}", path.display()))
    }
    pub fn save(&self) -> Result<()> {
        atomic_write(
            &home().join("config.toml"),
            toml::to_string_pretty(self)?.as_bytes(),
        )
    }
    pub fn scan(&self) -> Vec<PathBuf> {
        let mut paths = self.added.clone();
        for root in &self.roots {
            if let Ok(entries) = fs::read_dir(root) {
                paths.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
        }
        paths.retain(|p| p.is_dir() && !self.excluded.iter().any(|e| key(e) == key(p)));
        paths.sort_by_key(|p| key(p));
        paths.dedup_by_key(|p| key(p));
        paths.sort_by_key(|p| {
            let project = self.projects.get(&key(p)).cloned().unwrap_or_default();
            (
                !project.favorite,
                std::cmp::Reverse(project.last_opened),
                key(p),
            )
        });
        paths
    }
}
pub struct ConfigLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
fn lock_name(home: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    key(home).hash(&mut hash);
    // Builds and host instances share one configuration file and one lock.
    format!("Local\\DevTabs-config-{:016x}", hash.finish())
}
impl ConfigLock {
    pub fn acquire() -> Result<Self> {
        Self::acquire_at(&home())
    }
    pub fn acquire_at(root: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
            let name: Vec<u16> = lock_name(root).encode_utf16().chain(Some(0)).collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            anyhow::ensure!(!handle.is_null(), "Create configuration lock");
            let wait = unsafe { WaitForSingleObject(handle, 5000) };
            if wait != 0 && wait != 0x80 {
                unsafe {
                    CloseHandle(handle);
                }
                anyhow::bail!("Configuration is busy; retry after the other writer finishes");
            }
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        Ok(Self {})
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing parent directory")?)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, bytes)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        if unsafe {
            MoveFileExW(
                wide(&temporary).as_ptr(),
                wide(path).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            let _ = fs::remove_file(&temporary);
            return Err(std::io::Error::last_os_error()).context("Replace configuration");
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, path)?;
    Ok(())
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_config_lock_depends_only_on_data_directory() {
        let a = lock_name(Path::new("D:/DevTabs/data"));
        assert_eq!(a, lock_name(Path::new("d:/devtabs/data/")));
        assert_ne!(a, lock_name(Path::new("D:/DevTabs/other")));
        assert!(!a.contains(crate::protocol::BUILD_ID));
    }
    #[test]
    fn sidebar_ratio_roundtrips_and_missing_setting_keeps_default() {
        let old: Config =
            toml::from_str("[projects.demo.sidebar]\nwidth = 46\nfollow = true\ncompact = true\n")
                .unwrap();
        assert_eq!(old.projects["demo"].sidebar.agent_ratio, 5000);
        let mut new = old;
        new.projects.get_mut("demo").unwrap().sidebar.agent_ratio = 6250;
        let loaded: Config = toml::from_str(&toml::to_string_pretty(&new).unwrap()).unwrap();
        assert_eq!(loaded.projects["demo"].sidebar.width, 46);
        assert_eq!(loaded.projects["demo"].sidebar.agent_ratio, 6250);
        assert!(toml::from_str::<Config>("[projects.demo.sidebar]\nagent_raito = 6250\n").is_err());
    }
}
