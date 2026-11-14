//! Explicit, owned hook configuration. Never alters Provider trust databases.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
};
#[derive(Serialize, Deserialize)]
struct Ownership {
    provider: String,
    groups: std::collections::BTreeMap<String, Value>,
    disabled: bool,
}
fn read(path: &Path) -> Result<Value> {
    match fs::File::open(path) {
        Ok(f) => {
            let mut b = vec![];
            f.take(1024 * 1024 + 1).read_to_end(&mut b)?;
            ensure!(b.len() <= 1024 * 1024, "Hook configuration exceeds 1 MiB");
            Ok(serde_json::from_slice(&b)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}
pub fn install(path: &Path, provider: &str, exe: &Path, remove: bool) -> Result<()> {
    ensure!(
        matches!(provider, "codex" | "claude"),
        "Unsupported Provider"
    );
    ensure!(exe.is_absolute(), "Hook executable must be absolute");
    fs::create_dir_all(path.parent().context("Missing config directory")?)?;
    let lock = path.with_extension("devtabs.lock");
    let _file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .context("Hook configuration is busy")?;
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _guard = Unlock(lock);
    let owner_path = path.with_extension("devtabs-owner.json");
    let old = if owner_path.exists() {
        Some(serde_json::from_value::<Ownership>(read(&owner_path)?)?)
    } else {
        None
    };
    let mut value = read(path)?;
    ensure!(value.is_object(), "Hook configuration must be an object");
    let mut owner = Ownership {
        provider: provider.into(),
        groups: Default::default(),
        disabled: remove,
    };
    if let Some(o) = &old {
        ensure!(o.provider == provider, "Configuration ownership mismatch");
    }
    if remove && old.is_none() {
        return Ok(());
    }
    let original = fs::read(path).ok();
    let hooks = value
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("hooks must be an object")?;
    let events = if provider == "codex" {
        vec![
            "UserPromptSubmit",
            "Stop",
            "PermissionRequest",
            "Interrupt",
            "SessionEnd",
        ]
    } else {
        vec![
            "UserPromptSubmit",
            "Stop",
            "StopFailure",
            "PermissionRequest",
            "Notification",
            "SessionEnd",
        ]
    };
    let quoted = exe.to_string_lossy().replace('\'', "''");
    let command = format!(
        "pwsh.exe -NoLogo -NoProfile -EncodedCommand {}",
        crate::launch::encode_script(&format!("& '{quoted}' __hook {provider}"))
    );
    for event in events {
        // Codex /hooks uses statusMessage as the list title (not a name field).
        let title = if provider == "codex" {
            "Codex"
        } else {
            "Claude"
        };
        let group = json!({"hooks":[{"type":"command","command":command,"timeout":2,
            "statusMessage":format!("DevTabs · {title} · {event}")}]});
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .context("Hook matcher groups must be an array")?;
        if let Some(previous) = old.as_ref().and_then(|o| o.groups.get(event)) {
            let matches = groups.iter().filter(|g| *g == previous).count();
            ensure!(
                matches == 1 || old.as_ref().is_some_and(|o| o.disabled),
                "Managed hook changed externally; backup retained, manual review required"
            );
            groups.retain(|g| g != previous);
        }
        if !remove {
            groups.push(group.clone());
            owner.groups.insert(event.into(), group);
        }
    }
    // Journal ownership intent before replacing external configuration; backup always retains original bytes.
    if let Some(bytes) = original {
        crate::config::atomic_write(
            &path.with_extension(format!("devtabs-backup-{}.json", uuid::Uuid::new_v4())),
            &bytes,
        )?;
    }
    crate::config::atomic_write(&owner_path, &serde_json::to_vec_pretty(&owner)?)?;
    crate::config::atomic_write(path, &serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
pub fn config_path(provider: &str) -> Result<PathBuf> {
    let (variable, folder, file) = match provider {
        "codex" => ("CODEX_HOME", ".codex", "hooks.json"),
        "claude" => ("CLAUDE_CONFIG_DIR", ".claude", "settings.json"),
        _ => anyhow::bail!("Unsupported Provider"),
    };
    let dir = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(folder)))
        .context("Provider config directory unknown; specify --config")?;
    Ok(dir.join(file))
}
pub fn diagnostic(provider: &str, path: &Path) -> String {
    let result = (|| -> Result<String> {
        let value = read(path)?;
        let owner_path = path.with_extension("devtabs-owner.json");
        if !owner_path.exists() {
            return Ok(format!(
                "{provider}: not installed; trust unknown; runtime version not probed"
            ));
        }
        let owner: Ownership = serde_json::from_value(read(&owner_path)?)?;
        if owner.disabled {
            return Ok(format!("{provider}: disabled; no automatic reinstall"));
        }
        let intact = owner.groups.iter().all(|(event, group)| {
            value["hooks"][event]
                .as_array()
                .is_some_and(|a| a.contains(group))
        });
        Ok(format!(
            "{provider}: {}; trust unknown; verify exact definitions in Provider; no Agent was launched",
            if intact {
                "configured, runtime capabilities require actual events"
            } else {
                "owned configuration changed externally"
            }
        ))
    })();
    result.unwrap_or_else(|e| {
        format!(
            "{provider}: configuration could not be read: {}",
            e.root_cause()
        )
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookContext {
    pub task: String,
    pub session: String,
    pub run: String,
    pub token: String,
    pub endpoint: String,
    pub provider: String,
}
#[derive(Serialize, Deserialize)]
pub struct Envelope {
    pub context: HookContext,
    pub event: crate::activity::Event,
}
/// Runs only in a short-lived helper process. The watchdog bounds even blocked stdin/connect calls.
pub fn helper(provider: &str) {
    let Some(raw) = std::env::var_os("DEVTABS_HOOK_CONTEXT") else {
        return;
    };
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(1800));
        std::process::exit(0);
    });
    let result = (|| -> Result<()> {
        let context: HookContext = serde_json::from_str(&raw.to_string_lossy())?;
        ensure!(context.provider == provider, "Provider mismatch");
        let mut bytes = vec![];
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "Hook payload over limit");
        let v: Value = serde_json::from_slice(&bytes)?;
        let kind = v["hook_event_name"].as_str().context("No event name")?;
        let child = v.get("agent_id").is_some_and(|v| !v.is_null())
            || v.get("agent_type").is_some_and(|v| !v.is_null())
            || kind.starts_with("Subagent");
        let kind = if kind == "Notification" {
            format!(
                "Notification:{}",
                v["notification_type"].as_str().unwrap_or("unknown")
            )
        } else {
            kind.into()
        };
        let event = crate::activity::Event {
            provider: provider.into(),
            conversation: v["session_id"]
                .as_str()
                .context("No session identity")?
                .into(),
            turn: v["turn_id"].as_str().map(str::to_owned),
            kind,
            child,
            cwd: v["cwd"].as_str().context("No cwd")?.into(),
            transcript: v["transcript_path"].as_str().map(PathBuf::from),
        };
        use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
        let mut stream = Stream::connect(
            context
                .endpoint
                .as_str()
                .to_ns_name::<GenericNamespaced>()?,
        )?;
        crate::protocol::send_short(&mut stream, &Envelope { context, event })?;
        let accepted: bool = crate::protocol::receive_short(&mut stream)?;
        ensure!(accepted, "Hook event queue full");
        Ok(())
    })();
    if let Err(e) = result {
        let text = format!("{e:#}");
        let _ = crate::config::atomic_write(
            &crate::config::runtime().join("hook-last-error.txt"),
            text.as_bytes(),
        );
    }
}
pub fn listen(endpoint: &str) -> Result<mpsc::Receiver<Envelope>> {
    use interprocess::local_socket::{
        GenericNamespaced, ListenerNonblockingMode, ListenerOptions, prelude::*,
    };
    let options = ListenerOptions::new()
        .name(endpoint.to_ns_name::<GenericNamespaced>()?)
        .nonblocking(ListenerNonblockingMode::Both);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        let s = widestring::U16CString::from_str(format!(
            "D:P(A;;GA;;;{})(A;;GA;;;SY)",
            crate::platform::user_sid()?
        ))?;
        options.security_descriptor(SecurityDescriptor::deserialize(&s)?)
    };
    let listener = options.create_sync()?;
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::Builder::new()
        .name("devtabs-hooks".into())
        .spawn(move || {
            loop {
                match listener.accept() {
                    Ok(mut stream) => {
                        if let Ok(e) = crate::protocol::receive_short(&mut stream) {
                            let accepted = tx.try_send(e).is_ok();
                            let _ = crate::protocol::send_short(&mut stream, &accepted);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(rx)
}
