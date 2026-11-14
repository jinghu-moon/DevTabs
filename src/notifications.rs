//! Host-owned notification policy; delivery success is never treated as read.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Policy {
    pub muted: bool,
    pub once: bool,
    pub snooze: Option<(u64, u64, u64)>,
    pub generation: u64,
}
impl Policy {
    pub fn allow(&mut self, global: bool, ending: bool) -> bool {
        if !global {
            return false;
        }
        if ending && self.once {
            self.once = false;
            return true;
        }
        !self.muted
    }
    pub fn due(&mut self, now: u64, result: u64, unread: bool, generation: u64) -> bool {
        let Some((at, version, original)) = self.snooze else {
            return false;
        };
        if now < at {
            return false;
        }
        self.snooze = None;
        !self.muted && unread && version == result && original == generation
    }
}
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub workspace: String,
    pub task: String,
    pub session: String,
    pub run: String,
    pub result: u64,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Ledger(pub BTreeMap<String, (Target, crate::activity::State)>);
impl Ledger {
    pub fn load(home: &std::path::Path) -> Result<Self> {
        let p = home.join("activity.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let b = std::fs::read(p)?;
        ensure!(
            b.len() <= 8 * 1024 * 1024,
            "Activity history exceeds budget"
        );
        Ok(serde_json::from_slice(&b)?)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let b = serde_json::to_vec(self)?;
        ensure!(
            b.len() <= 8 * 1024 * 1024,
            "Activity history exceeds budget"
        );
        Ok(b)
    }
    pub fn insert(&mut self, t: Target, s: crate::activity::State) -> Result<()> {
        if !self.0.contains_key(&t.run) && self.0.len() >= 1024 {
            let key = self
                .0
                .iter()
                .find(|(_, (_, s))| !s.pending())
                .map(|(k, _)| k.clone())
                .context("Activity history full; unread results retained")?;
            self.0.remove(&key);
        }
        self.0.insert(t.run.clone(), (t, s));
        Ok(())
    }
    pub fn pending(&self) -> Vec<(Target, crate::activity::State)> {
        let mut rows = vec![];
        for (t, s) in self.0.values() {
            rows.extend(pending(t, s));
        }
        sort_pending(&mut rows);
        rows
    }
}
pub fn pending(t: &Target, s: &crate::activity::State) -> Vec<(Target, crate::activity::State)> {
    let mut rows = vec![];
    for result in s.results.iter().filter(|r| !r.read) {
        let mut t = t.clone();
        t.result = result.version;
        let mut s = s.clone();
        s.result = result.version;
        s.phase = result.phase.clone();
        s.ended = Some(result.at);
        rows.push((t, s));
    }
    if matches!(
        s.phase,
        crate::activity::Phase::Permission | crate::activity::Phase::Input
    ) {
        rows.push((t.clone(), s.clone()));
    }
    rows
}
pub fn sort_pending(rows: &mut [(Target, crate::activity::State)]) {
    let rank = |p: &crate::activity::Phase| match p {
        crate::activity::Phase::Permission | crate::activity::Phase::Input => 0,
        crate::activity::Phase::Failed => 1,
        _ => 2,
    };
    rows.sort_by(|a, b| {
        (rank(&a.1.phase), a.1.ended, &a.0.task).cmp(&(rank(&b.1.phase), b.1.ended, &b.0.task))
    });
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    List,
    Poll {
        workspace: String,
        clients: Vec<String>,
    },
    Ack {
        target: Target,
    },
    Mute {
        task: String,
        muted: bool,
    },
    Once {
        task: String,
    },
    Snooze {
        target: Target,
        seconds: u64,
    },
    Global {
        enabled: bool,
    },
    Test,
    Diagnostics,
    AfterCheck {
        task: String,
        enabled: bool,
    },
    Category {
        task: Option<String>,
        category: String,
        enabled: bool,
    },
    Sound {
        enabled: bool,
    },
    Navigate {
        target: Target,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct View {
    pub pending: Vec<(Target, crate::activity::State)>,
    pub diagnostic: Vec<String>,
    pub navigation: Option<Target>,
    pub enabled: bool,
    pub policies: BTreeMap<String, Policy>,
    pub configuration: Adoption,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Adoption {
    pub adopted: Option<String>,
    pub checked: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct Notice {
    pub target: Target,
    pub title: String,
    pub body: String,
    pub test: bool,
    pub ending: bool,
    pub urgent: bool,
    pub silent: bool,
    pub category: String,
}
pub fn delivery_policy(
    notice: &mut Notice,
    preferences: &crate::settings::model::Notifications,
) -> bool {
    if notice.test {
        return true;
    }
    notice.silent = !preferences.sound.unwrap_or(true);
    preferences.enabled.unwrap_or(true)
        && preferences
            .categories
            .get(&notice.category)
            .copied()
            .unwrap_or(true)
}
pub struct Worker {
    pub tx: Sender,
    pub clicks: mpsc::Receiver<Target>,
    pub diagnostics: mpsc::Receiver<String>,
}
#[derive(Default)]
pub struct Queue {
    entries: std::collections::VecDeque<(u64, Notice)>,
}
impl Queue {
    pub fn push(&mut self, now: u64, notice: Notice) -> Result<()> {
        if let Some((_, old)) = self.entries.iter_mut().find(|(_, n)| {
            n.target.task == notice.target.task
                && n.target.run == notice.target.run
                && n.urgent == notice.urgent
                && n.test == notice.test
        }) {
            *old = notice;
            return Ok(());
        }
        if self.entries.len() >= 32 {
            if notice.urgent
                && let Some(i) = self.entries.iter().position(|(_, n)| !n.urgent)
            {
                self.entries.remove(i);
            } else {
                anyhow::bail!("Notification queue full; result remains in the attention inbox");
            }
        }
        self.entries.push_back((now, notice));
        Ok(())
    }
    pub fn pop(&mut self, now: u64) -> Option<Notice> {
        let index = self
            .entries
            .iter()
            .position(|(_, n)| n.urgent)
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|(at, n)| n.test || now.saturating_sub(*at) >= 1000)
            })?;
        self.entries.remove(index).map(|(_, n)| n)
    }
}
#[derive(Clone)]
pub struct Sender(std::sync::Arc<std::sync::Mutex<Queue>>);
impl Sender {
    pub fn try_send(&self, n: Notice) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("Notification queue unavailable"))?
            .push(queue_clock(), n)
    }
}
fn queue_clock() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}
impl Worker {
    pub fn spawn() -> Result<Self> {
        let queue = std::sync::Arc::new(std::sync::Mutex::new(Queue::default()));
        let tx = Sender(queue.clone());
        let (click_tx, clicks) = mpsc::sync_channel(64);
        let (diag_tx, diagnostics) = mpsc::sync_channel(64);
        std::thread::Builder::new()
            .name("devtabs-notifications".into())
            .spawn(move || {
                #[cfg(windows)]
                let mut backend = match native::Backend::new(click_tx) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        let _ =
                            diag_tx.try_send(format!("Notification backend unavailable: {e:#}"));
                        None
                    }
                };
                #[cfg(not(windows))]
                let _ = click_tx;
                let mut preferences = crate::settings::model::Notifications::default();
                loop {
                    let next = queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .pop(queue_clock());
                    if let Some(mut n) = next {
                        if !n.test {
                            if let Ok(snapshot) =
                                crate::settings::store::SettingsStore::current().read()
                                && snapshot.require("notifications").is_ok()
                            {
                                preferences = snapshot.effective.notifications;
                            }
                            if !delivery_policy(&mut n, &preferences) {
                                continue;
                            }
                        }
                        #[cfg(windows)]
                        let result = backend
                            .as_mut()
                            .context("Notification backend unavailable")
                            .and_then(|b| b.show(n));
                        #[cfg(not(windows))]
                        let result: Result<()> = {
                            let _ = n;
                            anyhow::bail!("Windows notifications are unavailable")
                        };
                        let _ = diag_tx.try_send(match result {
                            Ok(()) => {
                                "Sent; display and activation require user verification".into()
                            }
                            Err(e) => format!("Notification send failed: {e:#}"),
                        });
                    }
                    #[cfg(windows)]
                    if let Some(b) = &mut backend {
                        b.expire();
                    }
                    if std::sync::Arc::strong_count(&queue) == 1 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            })?;
        Ok(Self {
            tx,
            clicks,
            diagnostics,
        })
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(skip)]
    pub disabled: bool,
    pub muted: BTreeMap<String, bool>,
    pub after_check: BTreeMap<String, bool>,
    #[serde(skip)]
    pub categories: BTreeMap<String, bool>,
    pub task_categories: BTreeMap<String, BTreeMap<String, bool>>,
    #[serde(skip)]
    pub silent: bool,
}
impl Settings {
    pub fn load() -> Result<Self> {
        let p = crate::config::home()
            .join("runtime")
            .join("notification-tasks.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let b = std::fs::read(p)?;
        ensure!(b.len() < 1024 * 1024, "Notification settings exceed budget");
        Ok(serde_json::from_slice(&b)?)
    }
    pub fn save(&self) -> Result<()> {
        crate::config::atomic_write(
            &crate::config::home()
                .join("runtime")
                .join("notification-tasks.json"),
            &serde_json::to_vec(self)?,
        )
    }
}
pub fn register(remove: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let key = "HKCU\\Software\\Classes\\AppUserModelId\\DevTabs.Workspace";
        let mut c = std::process::Command::new("reg.exe");
        if remove {
            c.args(["delete", key, "/f"]);
        } else {
            c.args([
                "add",
                key,
                "/v",
                "DisplayName",
                "/t",
                "REG_SZ",
                "/d",
                "DevTabs",
                "/f",
            ]);
        }
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
        ensure!(
            c.status()?.success(),
            "DevTabs notification identity registration failed"
        );
        let path = crate::config::runtime().join("notification-identity.json");
        if remove {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        } else {
            crate::config::atomic_write(&path, b"{\"aumid\":\"DevTabs.Workspace\"}")?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = remove;
        anyhow::bail!("Windows only")
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        core::HSTRING,
    };
    struct Held {
        toast: ToastNotification,
        token: i64,
        at: Instant,
    }
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.toast.RemoveActivated(self.token);
        }
    }
    pub struct Backend {
        held: Vec<Held>,
        click: mpsc::SyncSender<Target>,
    }
    impl Backend {
        pub fn new(click: mpsc::SyncSender<Target>) -> Result<Self> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            }
            Ok(Self {
                held: vec![],
                click,
            })
        }
        pub fn expire(&mut self) {
            self.held
                .retain(|h| h.at.elapsed() < Duration::from_secs(600));
        }
        pub fn show(&mut self, n: Notice) -> Result<()> {
            ensure!(
                crate::config::runtime()
                    .join("notification-identity.json")
                    .exists(),
                "Register the DevTabs notification identity explicitly before sending"
            );
            self.expire();
            ensure!(self.held.len() < 64, "Notification lifetime budget reached");
            let escape = |s: &str| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect::<String>()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
            };
            let xml = XmlDocument::new()?;
            xml.LoadXml(&HSTRING::from(format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",escape(&n.title),escape(&n.body))))?;
            if n.silent {
                let nodes = xml.GetElementsByTagName(&HSTRING::from("toast"))?;
                let root = nodes.Item(0)?;
                let audio = xml.CreateElement(&HSTRING::from("audio"))?;
                audio.SetAttribute(&HSTRING::from("silent"), &HSTRING::from("true"))?;
                root.AppendChild(&audio)?;
            }
            let toast = ToastNotification::CreateToastNotification(&xml)?;
            let target = n.target;
            let tx = self.click.clone();
            let test = n.test;
            let token = toast.Activated(&TypedEventHandler::new(move |_, _| {
                if !test {
                    let _ = tx.try_send(target.clone());
                }
                Ok(())
            }))?;
            let held = Held {
                toast,
                token,
                at: Instant::now(),
            };
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
                "DevTabs.Workspace",
            ))?
            .Show(&held.toast)?;
            self.held.push(held);
            Ok(())
        }
    }
    impl Drop for Backend {
        fn drop(&mut self) {
            self.held.clear();
            unsafe {
                CoUninitialize();
            }
        }
    }
}
