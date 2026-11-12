//! Atomic launch journal. A view detaching never deletes a workspace record.
use crate::{
    config::{self, ConfigLock},
    launch::Options,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub opened: u64,
    pub options: Options,
    // Older records have no authoritative task identities. Reading them must
    // not fail the whole journal or infer identities from paths/branches.
    #[serde(default)]
    pub task_ids: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub entries: BTreeMap<String, Record>,
    pub last: Vec<Options>,
}
impl Journal {
    pub fn load_at(home: &Path) -> Result<Self> {
        let path = home.join("launches.json");
        match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                anyhow::ensure!(
                    bytes.len() <= 4 * 1024 * 1024,
                    "Launch journal exceeds 4 MiB"
                );
                Ok(serde_json::from_slice(&bytes)
                    .with_context(|| format!("Read {}", path.display()))?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save_at(&self, home: &Path) -> Result<()> {
        config::atomic_write(
            &home.join("launches.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
    pub fn pending(&self, active: &std::collections::BTreeSet<String>) -> Vec<Record> {
        let mut records = self
            .entries
            .values()
            .filter(|r| !active.contains(&r.id))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|r| (r.opened, r.id.clone()));
        records
    }
    pub fn register(&mut self, id: &str, options: &Options) {
        if let Some(old) = &options.recovery_id {
            self.entries.remove(old);
        }
        self.entries.insert(
            id.into(),
            Record {
                id: id.into(),
                opened: config::now(),
                options: clean(options.clone()),
                task_ids: vec![],
            },
        );
    }
}
fn edit(f: impl FnOnce(&mut Journal)) -> Result<()> {
    let _lock = ConfigLock::acquire()?;
    let home = config::home();
    let mut j = Journal::load_at(&home)?;
    f(&mut j);
    j.save_at(&home)
}
pub fn load() -> Result<Journal> {
    Journal::load_at(&config::home())
}
pub fn register(id: &str, o: &Options) -> Result<()> {
    edit(|j| j.register(id, o))
}
pub fn register_tasks(id: &str, o: &Options, tasks: &[crate::tasks::Task]) -> Result<()> {
    if let Some(old) = &o.recovery_id {
        anyhow::ensure!(
            uuid::Uuid::parse_str(old).is_ok() && uuid::Uuid::parse_str(id).is_ok(),
            "Invalid recovery layout identity"
        );
        let source = config::home().join("layouts").join(format!("{old}.json"));
        match std::fs::File::open(source) {
            Ok(file) => {
                let mut bytes = vec![];
                file.take(65537).read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 65536, "Recovery layout exceeds 64 KiB");
                config::atomic_write(
                    &config::home().join("layouts").join(format!("{id}.json")),
                    &bytes,
                )?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    edit(|j| {
        j.register(id, o);
        j.entries.get_mut(id).unwrap().task_ids = tasks.iter().map(|t| t.id.clone()).collect();
    })
}
pub fn append_task(workspace: &str, task: &str) -> Result<()> {
    edit(|j| {
        if let Some(record) = j.entries.get_mut(workspace)
            && !record.task_ids.iter().any(|id| id == task)
        {
            record.task_ids.push(task.into());
        }
    })
}
pub fn remove(id: &str) -> Result<()> {
    edit(|j| {
        j.entries.remove(id);
    })
}
pub fn remember(group: &[Options]) -> Result<()> {
    edit(|j| j.last = group.iter().cloned().map(clean).collect())
}
pub fn update_layout(id: &str, layout: crate::launch::Layout) -> Result<()> {
    edit(|j| {
        if let Some(r) = j.entries.get_mut(id) {
            r.options.layout = layout;
        }
    })
}
pub fn clean(mut o: Options) -> Options {
    o.navigation = None;
    o.workspace = None;
    o.view = None;
    o.restore = false;
    o.last = false;
    o.headless = false;
    o.recovery_id = None;
    o.data_dir = None;
    o.window_group = None;
    o
}
pub fn restore_options(r: &Record) -> Options {
    let mut o = clean(r.options.clone());
    o.new_instance = true;
    o.recovery_id = Some(r.id.clone());
    if o.agent != crate::launch::Agent::Shell && o.resume_id.is_none() {
        o.continue_latest = false;
        o.fresh = true;
        o.resume = false;
    }
    o
}
pub fn launch_group(request: &Options) -> Result<()> {
    use crate::protocol::{self, Request, Response};
    crate::host::ensure()?;
    let active = match protocol::rpc(Request::List)? {
        Response::Workspaces(w) => w,
        Response::Error(e) => anyhow::bail!("{e}"),
        _ => anyhow::bail!("Unexpected host response"),
    };
    let journal = load()?;
    let mut group = if request.restore {
        journal
            .pending(&active.iter().map(|w| w.id.clone()).collect())
            .iter()
            .map(restore_options)
            .collect::<Vec<_>>()
    } else {
        journal.last.into_iter().map(clean).collect()
    };
    // An existing record with an active host is always reused by --last.
    if request.last {
        for o in &mut group {
            o.new_instance = false;
        }
    }
    if !group.is_empty() {
        remember(&group)?;
    }
    let mut failures = vec![];
    let mut workspaces = vec![];
    let window_group = uuid::Uuid::new_v4().to_string();
    for mut o in group {
        o.language = request.language.or(o.language);
        o.new_window |= request.new_window;
        o.window_group = Some(window_group.clone());
        let path = o.project.clone();
        let result = (|| -> Result<()> {
            match protocol::rpc(Request::Open {
                options: Box::new(o.clone()),
            })? {
                Response::Workspace(w) => {
                    if !request.headless {
                        crate::overview::present_workspace(&w.id, &o)?;
                    }
                    workspaces.push(w);
                }
                Response::Error(e) => anyhow::bail!("{e}"),
                _ => anyhow::bail!("No workspace returned"),
            }
            Ok(())
        })();
        if let Err(e) = result {
            failures.push(format!("{}: {e:#}", path.display()));
        }
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({"workspaces":workspaces,"failures":failures}))?
    );
    anyhow::ensure!(failures.is_empty(), "Some workspaces could not be reopened");
    Ok(())
}
