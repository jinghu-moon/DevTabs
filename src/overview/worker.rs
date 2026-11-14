//! Independent bounded workers for persistence/launch, discovery, host status and details.
use super::{
    history::{self, History},
    model::{GitSummary, LaunchSpec, LaunchState},
    stats::{self, Stats},
};
use crate::{
    config::{self, Config, Project},
    launch::Options,
    protocol::WorkspaceInfo,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
#[derive(Clone)]
pub enum Edit {
    Favorite(PathBuf),
    Note(PathBuf, String, String),
    Settings(PathBuf, Project),
    Root(PathBuf),
    Add(PathBuf),
    Exclude(PathBuf),
    Collapsed(u8, bool),
    Language(crate::i18n::Language),
    Theme(crate::appearance::Theme),
    Preset(String, Vec<LaunchSpec>),
    DeletePreset(String),
    RenamePreset(String, String),
    ExportProfile(PathBuf, Project),
    NewProject(PathBuf, String),
}
impl Edit {
    pub fn apply(self, c: &mut Config) -> anyhow::Result<()> {
        match self {
            Self::Favorite(p) => {
                let p = c.projects.entry(config::key(&p)).or_default();
                p.favorite = !p.favorite
            }
            Self::Note(p, _, n) => {
                anyhow::ensure!(
                    n.chars().count() <= 2048,
                    "{}",
                    crate::i18n::t("input_long")
                );
                c.projects.entry(config::key(&p)).or_default().note = n
            }
            Self::ExportProfile(path, p) => {
                crate::project_profile::Profile::from_project(&p).save(&path)?;
            }
            Self::NewProject(root, name) => {
                let path = crate::project_profile::create_project(&root, &name)?;
                c.added.push(path);
            }
            Self::DeletePreset(name) => {
                c.presets.remove(&name);
            }
            Self::RenamePreset(old, new) => {
                anyhow::ensure!(
                    !new.trim().is_empty() && new.len() <= 128,
                    "{}",
                    crate::i18n::t("invalid_name")
                );
                anyhow::ensure!(
                    new == old || !c.presets.contains_key(&new),
                    "Preset already exists"
                );
                let items = c
                    .presets
                    .remove(&old)
                    .ok_or_else(|| anyhow::anyhow!("Preset no longer exists"))?;
                c.presets.insert(new, items);
            }
            Self::Settings(p, value) => {
                super::model::validate_settings(&value)?;
                let p = c.projects.entry(config::key(&p)).or_default();
                p.title = value.title;
                p.color = value.color;
                p.agent_args = value.agent_args;
                p.resume_latest = value.resume_latest;
                p.agent = value.agent;
                p.debug_command = value.debug_command;
                p.layout = value.layout;
                p.composition = value.composition;
                p.sessions = value.sessions
            }
            Self::Root(p) => c.roots.push(directory(p)?),
            Self::Add(p) => c.added.push(directory(p)?),
            Self::Exclude(p) => c.excluded.push(directory(p)?),
            Self::Collapsed(group, value) => {
                if value {
                    c.overview.collapsed.insert(group);
                } else {
                    c.overview.collapsed.remove(&group);
                }
            }
            Self::Language(l) => c.language = l,
            Self::Theme(t) => c.theme = t,
            Self::Preset(name, items) => {
                anyhow::ensure!(
                    !name.trim().is_empty() && name.chars().count() <= 128,
                    "{}",
                    crate::i18n::t("invalid_name")
                );
                anyhow::ensure!(!items.is_empty(), "{}", crate::i18n::t("preset_empty"));
                c.presets.insert(name, items);
            }
        }
        Ok(())
    }
}
fn directory(path: PathBuf) -> anyhow::Result<PathBuf> {
    let p = dunce::canonicalize(path)?;
    anyhow::ensure!(p.is_dir(), "Not a directory");
    Ok(p)
}
pub enum Action {
    Edit(u64, Edit),
    Launch(u64, Vec<(usize, Options)>),
    Reload(u64),
}
pub enum Detail {
    Stats(PathBuf, u64),
    Source(PathBuf, u64),
    Context(PathBuf, crate::launch::Agent, u64),
    History(PathBuf, crate::launch::Agent, u64),
    Session(PathBuf, crate::launch::Agent, PathBuf, u64),
}
pub enum Reply {
    Config(u64, Result<Config, String>),
    Profile(u64, String, Result<Project, String>),
    Recovery(Result<crate::recovery::Journal, String>),
    Paths(u64, Vec<PathBuf>),
    Git(u64, String, Result<GitSummary, String>),
    Host(Result<Vec<WorkspaceInfo>, String>),
    Stats(String, u64, Stats),
    Source(String, u64, Stats),
    Context(String, crate::launch::Agent, u64, History),
    History(String, u64, History),
    Session(
        String,
        crate::launch::Agent,
        u64,
        Result<Option<history::Conversation>, String>,
    ),
    Launch(u64, usize, LaunchState),
    Attached(u64, usize, String),
    Ranked(u64, String, Stats),
    Usage(u64, BTreeMap<String, u64>),
}
type DiscoverySlot = Arc<(Mutex<Option<(u64, Config)>>, Condvar)>;
pub struct Workers {
    pub actions: SyncSender<Action>,
    scan: DiscoverySlot,
    launches: SyncSender<Action>,
    pub detail: SyncSender<Detail>,
    pub history: SyncSender<Detail>,
    pub source: SyncSender<Detail>,
    pub history_generation: Arc<AtomicU64>,
    pub source_generation: Arc<AtomicU64>,
    pub replies: Receiver<Reply>,
    pub generation: Arc<AtomicU64>,
    pub scan_generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}
impl Default for Workers {
    fn default() -> Self {
        Self::new()
    }
}
impl Workers {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        let (actions, action_rx) = mpsc::sync_channel(32);
        let scan = Arc::new((Mutex::new(None::<(u64, Config)>), Condvar::new()));
        let (launches, launch_rx) = mpsc::sync_channel::<Action>(2);
        let (detail, detail_rx) = mpsc::sync_channel(2);
        let (history, history_rx) = mpsc::sync_channel(2);
        let (source, source_rx) = mpsc::sync_channel(2);
        let history_generation = Arc::new(AtomicU64::new(0));
        let source_generation = Arc::new(AtomicU64::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let scan_generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let tx = tx.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while let Ok(action) = action_rx.recv() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match action {
                        Action::Edit(id, edit) => {
                            let result = match edit {
                                Edit::Note(path, base, draft) => {
                                    crate::settings::data::UserData::note(
                                        &config::home(),
                                        &path,
                                        &base,
                                        &draft,
                                    )
                                    .and_then(|_| Config::load())
                                }
                                edit => Config::update(|c| edit.apply(c)),
                            }
                            .map_err(|e| format!("{e:#}"));
                            if tx.send(Reply::Config(id, result)).is_err() {
                                break;
                            }
                        }
                        Action::Reload(id) => {
                            let invalidated = crate::config::atomic_write(
                                &crate::config::cache().join("stats-refresh"),
                                uuid::Uuid::new_v4().to_string().as_bytes(),
                            );
                            if tx
                                .send(Reply::Config(
                                    id,
                                    invalidated
                                        .and_then(|_| Config::load())
                                        .map_err(|e| format!("{e:#}")),
                                ))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Action::Launch(..) => unreachable!("launch channel"),
                    }
                }
            });
        }
        {
            let tx = tx.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while let Ok(action) = launch_rx.recv() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    match action {
                        Action::Launch(generation, items) => {
                            if let Err(e) = crate::recovery::remember(
                                &items.iter().map(|(_, o)| o.clone()).collect::<Vec<_>>(),
                            ) {
                                for (index, _) in items {
                                    let _ = tx.send(Reply::Launch(
                                        generation,
                                        index,
                                        LaunchState::Failed(format!("{e:#}")),
                                    ));
                                }
                                continue;
                            }
                            let window_group = uuid::Uuid::new_v4().to_string();
                            for (index, mut options) in items {
                                if options.new_window {
                                    options.window_group = Some(window_group.clone());
                                }
                                if stop.load(Ordering::Relaxed) {
                                    break;
                                }
                                if tx
                                    .send(Reply::Launch(generation, index, LaunchState::Opening))
                                    .is_err()
                                {
                                    return;
                                }
                                let _perf = crate::perf::span(crate::perf::Metric::OverviewLaunch);
                                let result = super::open_workspace(&options)
                                    .and_then(|info| {
                                        let _ = tx.send(Reply::Attached(
                                            generation,
                                            index,
                                            info.id.clone(),
                                        ));
                                        super::present_workspace(&info.id, &options)?;
                                        Ok(info.id)
                                    })
                                    .map(LaunchState::Opened)
                                    .unwrap_or_else(|e| LaunchState::Failed(format!("{e:#}")));
                                if tx.send(Reply::Launch(generation, index, result)).is_err() {
                                    return;
                                }
                            }
                        }
                        _ => unreachable!("persistence channel"),
                    }
                }
            });
        }
        {
            let tx = tx.clone();
            let cancel = scan_generation.clone();
            let scan = scan.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                loop {
                    let (mutex, wake) = &*scan;
                    let mut slot = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    while slot.is_none() && !stop.load(Ordering::Relaxed) {
                        slot = wake.wait(slot).unwrap_or_else(|e| e.into_inner());
                    }
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let job = slot.take().expect("discovery job");
                    drop(slot);
                    let (ticket, c) = job;
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut paths = c.scan();
                    if let Ok(j) = crate::recovery::load() {
                        paths.extend(
                            j.entries
                                .values()
                                .map(|r| r.options.project.clone())
                                .filter(|p| p.is_dir()),
                        );
                    }
                    paths.sort_by_key(|p| config::key(p));
                    paths.dedup_by_key(|p| config::key(p));
                    // Publish configuration before exposing newly discovered projects.
                    // Git/statistics can be slow; launch must not race their queue.
                    let mut agents = BTreeMap::new();
                    for p in &paths {
                        if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) != ticket
                        {
                            break;
                        }
                        let profile =
                            crate::project_profile::effective(&c, p).map_err(|e| format!("{e:#}"));
                        if let Ok(prefs) = &profile {
                            agents.insert(
                                config::key(p),
                                prefs
                                    .agent
                                    .or(c.default_agent)
                                    .unwrap_or(crate::launch::Agent::Codex),
                            );
                        }
                        if tx
                            .send(Reply::Profile(ticket, config::key(p), profile))
                            .is_err()
                        {
                            return;
                        }
                    }
                    if cancel.load(Ordering::Relaxed) != ticket {
                        continue;
                    }
                    if tx.send(Reply::Paths(ticket, paths.clone())).is_err() {
                        break;
                    }
                    // Recency determines project groups; do not queue it behind Git.
                    let mut usage = BTreeMap::new();
                    for agent in [crate::launch::Agent::Codex, crate::launch::Agent::Claude] {
                        let index =
                            history::usage_index(&history::root(agent), agent, &cancel, ticket);
                        for p in &paths {
                            if agents.get(&config::key(p)) == Some(&agent) {
                                let key = config::key(dunce::simplified(p)).replace('/', "\\");
                                if let Some(time) = index.get(&key) {
                                    usage.insert(config::key(p), *time);
                                }
                            }
                        }
                    }
                    if cancel.load(Ordering::Relaxed) == ticket {
                        if tx.send(Reply::Usage(ticket, usage)).is_err() {
                            return;
                        }
                    }
                    // Bound concurrent Git processes while publishing each result
                    // immediately. A slow repository must not hold up every row.
                    let next = std::sync::atomic::AtomicUsize::new(0);
                    std::thread::scope(|scope| {
                        let next = &next;
                        for _ in 0..paths.len().min(4) {
                            let paths = &paths;
                            let tx = &tx;
                            let stop = &stop;
                            let cancel = &cancel;
                            let c = &c;
                            scope.spawn(move || {
                                loop {
                                    if stop.load(Ordering::Relaxed)
                                        || cancel.load(Ordering::Relaxed) != ticket
                                    {
                                        break;
                                    }
                                    let Some(p) = paths.get(next.fetch_add(1, Ordering::Relaxed))
                                    else {
                                        break;
                                    };
                                    let result = stats::git(p).map_err(|e| format!("{e:#}"));
                                    if tx.send(Reply::Git(ticket, config::key(p), result)).is_err()
                                    {
                                        break;
                                    }
                                    if matches!(
                                        c.overview.sort,
                                        super::model::Sort::Size | super::model::Sort::Activity
                                    ) && !stop.load(Ordering::Relaxed)
                                        && cancel.load(Ordering::Relaxed) == ticket
                                    {
                                        let result = stats::cached(p, cancel, ticket);
                                        if tx
                                            .send(Reply::Ranked(ticket, config::key(p), result))
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                    });
                }
            });
        }
        for (detail_rx, cancel) in [
            (detail_rx, generation.clone()),
            (history_rx, history_generation.clone()),
            (source_rx, source_generation.clone()),
        ] {
            let tx = tx.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while let Ok(mut job) = detail_rx.recv() {
                    while let Ok(next) = detail_rx.try_recv() {
                        job = next
                    }
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = match job {
                        Detail::Session(p, agent, file, ticket) => Reply::Session(
                            config::key(&p),
                            agent,
                            ticket,
                            history::read(&file, &p, agent).map_err(|e| format!("{e:#}")),
                        ),
                        Detail::Source(p, ticket) => Reply::Source(
                            config::key(&p),
                            ticket,
                            stats::source(&p, &cancel, ticket),
                        ),
                        Detail::Context(p, agent, ticket) => {
                            let _perf = crate::perf::span(crate::perf::Metric::OverviewHistory);
                            Reply::Context(
                                config::key(&p),
                                agent,
                                ticket,
                                history::load(&history::root(agent), &p, agent, &cancel, ticket),
                            )
                        }

                        Detail::Stats(p, ticket) => {
                            let key = config::key(&p);
                            let result =
                                stats::cached_with_progress(&p, &cancel, ticket, &mut |s| {
                                    // Progress is replaceable; completion below is not.
                                    // Never block traversal on an occupied UI channel.
                                    if tx
                                        .try_send(Reply::Stats(key.clone(), ticket, s.clone()))
                                        .is_err()
                                    {
                                        crate::perf::add(
                                            crate::perf::Count::StatisticsProgressDropped,
                                            1,
                                        );
                                    }
                                });
                            Reply::Stats(key, ticket, result)
                        }
                        Detail::History(p, agent, ticket) => {
                            let _perf = crate::perf::span(crate::perf::Metric::OverviewHistory);
                            Reply::History(
                                config::key(&p),
                                ticket,
                                history::load(&history::root(agent), &p, agent, &cancel, ticket),
                            )
                        }
                    };
                    if tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        {
            let tx = tx.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let result = crate::host::workspaces().map_err(|e| format!("{e:#}"));
                    let host_ok = result.is_ok();
                    if tx.try_send(Reply::Host(result)).is_err() && stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if host_ok {
                        let _ = tx.try_send(Reply::Recovery(
                            crate::recovery::load().map_err(|e| format!("{e:#}")),
                        ));
                    }
                    for _ in 0..20 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
        }
        Self {
            actions,
            scan,
            launches,
            detail,
            history,
            source,
            history_generation,
            source_generation,
            replies: rx,
            generation,
            scan_generation,
            stop,
        }
    }
    pub fn history_ticket(&self) -> u64 {
        self.history_generation.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn source_ticket(&self) -> u64 {
        self.source_generation.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn ticket(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn discover(&self, c: Config) -> anyhow::Result<u64> {
        let ticket = self.scan_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let (mutex, wake) = &*self.scan;
        *mutex.lock().unwrap_or_else(|e| e.into_inner()) = Some((ticket, c));
        wake.notify_one();
        Ok(ticket)
    }
    pub fn send(&self, action: Action) -> anyhow::Result<()> {
        let sender = if matches!(action, Action::Launch(..)) {
            &self.launches
        } else {
            &self.actions
        };
        sender
            .try_send(action)
            .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("busy")))
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _slot = self.scan.0.lock().unwrap_or_else(|e| e.into_inner());
        self.scan.1.notify_all();
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.history_generation.fetch_add(1, Ordering::Relaxed);
        self.source_generation.fetch_add(1, Ordering::Relaxed);
        self.scan_generation.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn active(workspaces: &[WorkspaceInfo]) -> BTreeMap<String, Vec<WorkspaceInfo>> {
    let mut out = BTreeMap::<String, Vec<WorkspaceInfo>>::new();
    for w in workspaces {
        out.entry(config::key(&w.project))
            .or_default()
            .push(w.clone())
    }
    out
}

#[cfg(test)]
pub(super) struct TestWorkers {
    pub worker: Workers,
    pub edits: Receiver<Action>,
    pub launches: Receiver<Action>,
    pub details: Receiver<Detail>,
    pub histories: Receiver<Detail>,
    pub sources: Receiver<Detail>,
    pub replies: SyncSender<Reply>,
}
#[cfg(test)]
pub(super) fn isolated() -> TestWorkers {
    let (tx, rx) = mpsc::sync_channel(64);
    let (actions, edits) = mpsc::sync_channel(32);
    let (launches, launch_rx) = mpsc::sync_channel(2);
    let (detail, details) = mpsc::sync_channel(2);
    let (history, histories) = mpsc::sync_channel(2);
    let (source, sources) = mpsc::sync_channel(2);
    TestWorkers {
        worker: Workers {
            actions,
            launches,
            scan: Arc::new((Mutex::new(None), Condvar::new())),
            detail,
            history,
            source,
            history_generation: Arc::new(AtomicU64::new(0)),
            source_generation: Arc::new(AtomicU64::new(0)),
            replies: rx,
            generation: Arc::new(AtomicU64::new(0)),
            scan_generation: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
        },
        edits,
        launches: launch_rx,
        details,
        histories,
        sources,
        replies: tx,
    }
}
