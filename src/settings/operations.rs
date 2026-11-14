//! Explicit operations have no relationship to preference draft cancellation.
use anyhow::{Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenConfig,
    OpenHome,
    Diagnostics,
    ExportConfig,
    ExportShortcuts,
    ExportEffectiveShortcuts,
    BackupContent,
    InspectContent,
    RestoreContent,
    ClearStats,
    ResetMemory,
    HookDiagnostic(&'static str),
    HookInstall(&'static str),
    HookRemove(&'static str),
    Register,
    Unregister,
    TestNotification,
}
impl Action {
    pub fn destructive(&self) -> bool {
        matches!(
            self,
            Self::RestoreContent
                | Self::ClearStats
                | Self::ResetMemory
                | Self::HookInstall(_)
                | Self::HookRemove(_)
                | Self::Register
                | Self::Unregister
                | Self::TestNotification
                | Self::ExportConfig
                | Self::ExportShortcuts
                | Self::ExportEffectiveShortcuts
                | Self::BackupContent
        )
    }
    pub fn label(&self) -> (&'static str, &'static str) {
        match self {
            Self::OpenConfig => ("打开原配置", "Open configuration"),
            Self::OpenHome => ("打开数据目录", "Open data directory"),
            Self::Diagnostics => ("刷新宿主/通知诊断", "Refresh host/notification diagnostics"),
            Self::ExportConfig => ("导出已提交配置", "Export committed configuration"),
            Self::ExportShortcuts => ("导出已提交快捷键", "Export committed shortcuts"),
            Self::ExportEffectiveShortcuts => (
                "导出完整有效快捷键目录（JSON）",
                "Export effective shortcut catalog (JSON)",
            ),
            Self::BackupContent => ("备份用户内容", "Back up user content"),
            Self::InspectContent => ("预览恢复差异", "Inspect restore differences"),
            Self::RestoreContent => ("恢复无冲突用户内容", "Restore content without conflicts"),
            Self::ClearStats => ("清理统计缓存", "Clear statistics cache"),
            Self::ResetMemory => ("重置 UI 记忆", "Reset UI memory"),
            Self::HookDiagnostic("codex") => ("Codex Hooks 诊断", "Codex Hooks diagnostics"),
            Self::HookDiagnostic(_) => ("Claude Hooks 诊断", "Claude Hooks diagnostics"),
            Self::HookInstall("codex") => ("安装 Codex Hooks", "Install Codex Hooks"),
            Self::HookInstall(_) => ("安装 Claude Hooks", "Install Claude Hooks"),
            Self::HookRemove("codex") => ("移除 Codex Hooks", "Remove Codex Hooks"),
            Self::HookRemove(_) => ("移除 Claude Hooks", "Remove Claude Hooks"),
            Self::Register => ("注册 Windows 通知", "Register Windows notifications"),
            Self::Unregister => ("移除 Windows 通知注册", "Unregister Windows notifications"),
            Self::TestNotification => (
                "立即发送测试通知（可能有声音）",
                "Send test notification now (may play sound)",
            ),
        }
    }
}
pub fn actions() -> Vec<Action> {
    vec![
        Action::OpenConfig,
        Action::OpenHome,
        Action::Diagnostics,
        Action::HookDiagnostic("codex"),
        Action::HookInstall("codex"),
        Action::HookRemove("codex"),
        Action::HookDiagnostic("claude"),
        Action::HookInstall("claude"),
        Action::HookRemove("claude"),
        Action::Register,
        Action::Unregister,
        Action::TestNotification,
        Action::ExportConfig,
        Action::ExportShortcuts,
        Action::ExportEffectiveShortcuts,
        Action::BackupContent,
        Action::InspectContent,
        Action::RestoreContent,
        Action::ClearStats,
        Action::ResetMemory,
    ]
}
fn output(root: &Path, path: &str) -> Result<PathBuf> {
    let p = PathBuf::from(path);
    ensure!(p.is_absolute(), "Choose an absolute archive/output path");
    let key = super::model::project_key(&p);
    for name in [
        "config.toml",
        "config.previous.toml",
        "user-data.json",
        "ui-state.json",
        "runtime",
        "cache",
    ] {
        let protected = super::model::project_key(&root.join(name));
        ensure!(
            key != protected && !key.starts_with(&format!("{protected}\\")),
            "Archive must be separate from active data files/directories"
        );
    }
    Ok(p)
}
pub fn execute(action: &Action, archive: &str) -> Result<String> {
    execute_at(&crate::config::home(), action, archive)
}
pub fn execute_at(root: &Path, action: &Action, archive: &str) -> Result<String> {
    match action {
        Action::OpenConfig => {
            let p = root.join("config.toml");
            if p.exists() {
                crate::files::external(&p, false)?;
            } else {
                crate::files::copy(p.display().to_string())?;
            }
        }
        Action::OpenHome => crate::files::external(&root, false)?,
        Action::ExportConfig => {
            let source = super::store::SettingsStore::new(root).read()?;
            crate::config::atomic_write(
                &output(root, archive)?,
                if source.text.is_empty() {
                    b"schema = 1\n"
                } else {
                    source.text.as_bytes()
                },
            )?;
        }
        Action::ExportShortcuts => {
            let source = super::store::SettingsStore::new(root).read()?;
            let mut t = toml::Table::new();
            t.insert("schema".into(), 1.into());
            t.insert(
                "shortcuts".into(),
                source
                    .tree
                    .get("shortcuts")
                    .cloned()
                    .unwrap_or_else(|| toml::Value::Table(Default::default())),
            );
            crate::config::atomic_write(
                &output(root, archive)?,
                toml::to_string_pretty(&t)?.as_bytes(),
            )?;
        }
        Action::ExportEffectiveShortcuts => {
            let source = super::store::SettingsStore::new(root).read()?;
            let p = &source.effective.shortcuts;
            let index = crate::shortcuts::Index::build(p)?;
            let rows=index.bindings.iter().map(|b| {
                let c=index.commands.iter().find(|c|c.id==b.command).unwrap();
                let edit=p.bindings.iter().find(|e|e.command==b.command && e.context==b.context);
                serde_json::json!({"command":b.command,"context":b.context,"effective":if b.disabled {vec![]} else {b.keys.iter().map(|s|s.display()).collect::<Vec<_>>()},"preferred":index.preferred(&b.command,&b.context),"disabled":b.disabled,"defaults":c.defaults.iter().map(|s|s.display()).collect::<Vec<_>>(),"difference":edit})
            }).collect::<Vec<_>>();
            let report = serde_json::json!({"kind":"devtabs.effective-shortcut-catalog","description":"Read-only committed catalog, not importable user differences","prefix":index.prefix.to_string(),"terminal_owned":index.terminal_owned(),"diagnostics":index.diagnostics,"bindings":rows});
            crate::config::atomic_write(
                &output(root, archive)?,
                &serde_json::to_vec_pretty(&report)?,
            )?;
        }
        Action::BackupContent => {
            let d = super::data::UserData::read(&root.join("user-data.json"))?;
            d.save(&output(root, archive)?)?;
        }
        Action::InspectContent | Action::RestoreContent => {
            return restore(
                &root,
                &output(root, archive)?,
                matches!(action, Action::RestoreContent),
            );
        }
        Action::ClearStats => {
            let cache = root.join("cache/stats");
            if cache.exists() {
                for entry in fs::read_dir(&cache)?.flatten() {
                    let p = entry.path();
                    if p.is_file() && p.extension().is_some_and(|e| e == "json") {
                        fs::remove_file(p)?;
                    }
                }
            }
        }
        Action::ResetMemory => {
            let _lock = crate::config::ConfigLock::acquire_at(&root)?;
            crate::config::atomic_write(&root.join("ui-state.json"), b"{}")?;
        }
        Action::HookDiagnostic(provider) => {
            let p = crate::hooks::config_path(provider)?;
            return Ok(format!(
                "{}\n{}",
                p.display(),
                crate::hooks::diagnostic(provider, &p)
            ));
        }
        Action::HookInstall(provider) | Action::HookRemove(provider) => {
            let p = crate::hooks::config_path(provider)?;
            crate::hooks::install(
                &p,
                provider,
                &std::env::current_exe()?,
                matches!(action, Action::HookRemove(_)),
            )?;
            return Ok(crate::hooks::diagnostic(provider, &p));
        }
        Action::Register | Action::Unregister => {
            crate::notifications::register(matches!(action, Action::Unregister))?
        }
        Action::Diagnostics | Action::TestNotification => {
            let response = crate::protocol::rpc(crate::protocol::Request::Notifications {
                action: if matches!(action, Action::TestNotification) {
                    crate::notifications::Action::Test
                } else {
                    crate::notifications::Action::Diagnostics
                },
            })?;
            return Ok(serde_json::to_string_pretty(&response)?);
        }
    }
    Ok(format!("Completed: {action:?}. {}", archive))
}
#[derive(Clone, Debug)]
pub struct RestorePlan {
    pub base: super::data::UserData,
    pub incoming: super::data::UserData,
    pub conflicts: Vec<(String, String, String)>,
    pub use_archive: std::collections::BTreeSet<String>,
    pub added: usize,
    pub same: usize,
}
impl RestorePlan {
    pub fn summary(&self) -> String {
        format!(
            "New / 新增: {}; same / 同值: {}; conflicts / 冲突: {}. Selected archive values / 采用归档: {}",
            self.added,
            self.same,
            self.conflicts.len(),
            self.use_archive.len()
        )
    }
}
pub fn inspect_restore(root: &Path, archive: &str) -> Result<RestorePlan> {
    let archive = output(root, archive)?;
    ensure!(archive.is_file(), "Archive does not exist");
    let incoming = super::data::UserData::read(&archive)?;
    incoming.validate()?;
    let base = super::data::UserData::read(&root.join("user-data.json"))?;
    let mut plan = RestorePlan {
        base,
        incoming,
        conflicts: vec![],
        use_archive: Default::default(),
        added: 0,
        same: 0,
    };
    for (key, value) in &plan.incoming.notes {
        match plan.base.notes.get(key) {
            None => plan.added += 1,
            Some(old) if old == value => plan.same += 1,
            Some(old) => plan
                .conflicts
                .push((format!("note:{key}"), old.clone(), value.clone())),
        }
    }
    for entry in &plan.incoming.bookmarks {
        match plan.base.bookmarks.iter().find(|e| e.id == entry.id) {
            None => plan.added += 1,
            Some(old) if old == entry => plan.same += 1,
            Some(old) => plan.conflicts.push((
                format!("bookmark:{}", entry.id),
                serde_json::to_string_pretty(old)?,
                serde_json::to_string_pretty(entry)?,
            )),
        }
    }
    Ok(plan)
}
pub fn commit_restore(root: &Path, plan: &RestorePlan) -> Result<String> {
    let _lock = crate::config::ConfigLock::acquire_at(root)?;
    let path = root.join("user-data.json");
    let mut latest = super::data::UserData::read(&path)?;
    for (key, value) in &plan.incoming.notes {
        let conflict = format!("note:{key}");
        if plan.conflicts.iter().any(|(id, _, _)| *id == conflict)
            && !plan.use_archive.contains(&conflict)
        {
            continue;
        }
        let current = latest.notes.get(key);
        let base = plan.base.notes.get(key);
        ensure!(
            current == base || current == Some(value),
            "{conflict} changed after preview; inspect again. Original content preserved"
        );
        latest.notes.insert(key.clone(), value.clone());
    }
    for entry in &plan.incoming.bookmarks {
        let conflict = format!("bookmark:{}", entry.id);
        if plan.conflicts.iter().any(|(id, _, _)| *id == conflict)
            && !plan.use_archive.contains(&conflict)
        {
            continue;
        }
        let i = latest.bookmarks.iter().position(|e| e.id == entry.id);
        let current = i.map(|i| &latest.bookmarks[i]);
        let base = plan.base.bookmarks.iter().find(|e| e.id == entry.id);
        ensure!(
            current == base || current == Some(entry),
            "{conflict} changed after preview; inspect again. Original content preserved"
        );
        if let Some(i) = i {
            latest.bookmarks[i] = entry.clone();
        } else {
            latest.bookmarks.push(entry.clone());
        }
    }
    latest.save(&path)?;
    Ok(format!("Restored user content. {}", plan.summary()))
}
fn restore(root: &Path, archive: &Path, apply: bool) -> Result<String> {
    let plan = inspect_restore(root, &archive.to_string_lossy())?;
    if apply {
        ensure!(
            plan.conflicts.is_empty(),
            "Inspect restore conflicts before applying. {}",
            plan.summary()
        );
        commit_restore(root, &plan)
    } else {
        Ok(plan.summary())
    }
}

#[cfg(test)]
mod tests;
