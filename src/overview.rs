//! Project launcher: UI state stays on the foreground; all disk/IPC work is bounded and asynchronous.
mod dashboard;
pub mod history;
pub mod model;
pub mod stats;
mod view;
pub mod worker;
use crate::{
    config::{self, Config, Project},
    events::Events,
    i18n::{format as tf, t},
    image_output::Terminal as DefaultTerminal,
    launch::{Agent, Layout, Options},
};
use anyhow::Result;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use model::{Batch, LaunchSpec, LaunchState, Row};
use ratatui::{layout::Rect, widgets::ListState};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use worker::{Action, Detail, Edit, Reply, Workers};
#[derive(Clone)]
enum Command {
    Attention,
    Open,
    Diagnostics,
    CopyPath,
    ResetDraft,
    DraftAgent(Agent),
    DraftComposition(crate::launch::Composition),
    NewSession,
    LatestSession,
    NativePicker,
    Options,
    Favorite,
    Note,
    Settings,
    History,
    Stats,
    Details,
    Filter,
    Sort,
    Group,
    Launch,
    SavePreset,
    Presets,
    SelectAll,
    NewProject,
    ExportProfile,
    Restore,
    Last,
    Help,
    SetFilter(model::Filter),
    Retry,
    Refresh,
    Root,
    Add,
    Exclude,
    Folder,
    Editor,
    Config,
    Language(crate::i18n::Language),
    Theme(crate::appearance::Theme),
    Project(PathBuf),
    Preset(String),
    Quit,
}
struct Entry {
    label: String,
    keywords: String,
    hint: &'static str,
    command: Command,
    enabled: bool,
}
#[derive(Clone)]
enum Input {
    Root,
    Add,
    Exclude,
    Note(PathBuf),
    Preset,
    NewProject(PathBuf),
    RenamePreset(String),
}
#[derive(Clone)]
enum Dialog {
    Attention {
        selected: usize,
    },
    Input(Input, String),
    Settings {
        path: PathBuf,
        project: Project,
        field: usize,
        debug: String,
        arguments: String,
    },
    Options {
        options: Options,
        new_tab: bool,
        editing: Option<(usize, String)>,
    },
    Palette {
        query: String,
        selected: usize,
        original: crate::appearance::Theme,
    },
    History {
        path: PathBuf,
        agent: Agent,
        selected: usize,
        scroll: u16,
    },
    Batch,
    Presets {
        selected: usize,
    },
    Help,
    Queue {
        selected: usize,
    },
    Instances {
        path: PathBuf,
        selected: usize,
    },
    Diagnostics,
}
pub struct Overview {
    config: Config,
    dashboard: dashboard::Dashboard,
    filter_counts: BTreeMap<model::Filter, usize>,
    group_counts: BTreeMap<u8, usize>,
    profiles: BTreeMap<String, Project>,
    profile_errors: BTreeMap<String, String>,
    staged: BTreeMap<String, Options>,
    changed_defaults: BTreeSet<String>,
    recovery: crate::recovery::Journal,
    filter_hits: Vec<(Rect, model::Filter)>,
    projects: Vec<PathBuf>,
    rows: Vec<Row>,
    selected: usize,
    list: ListState,
    query: String,
    marked: model::Queue,
    git: BTreeMap<String, Result<model::GitSummary, String>>,
    workspaces: BTreeMap<String, Vec<crate::protocol::WorkspaceInfo>>,
    host_error: String,
    stats: BTreeMap<String, stats::Stats>,
    sizes: BTreeMap<String, u64>,
    activity: BTreeMap<String, u64>,
    usage: BTreeMap<String, u64>,
    history: Option<history::History>,
    detail_ticket: u64,
    detail_key: Option<String>,
    detail_pending: bool,
    refresh_pending: Option<u64>,
    show_stats: bool,
    show_detail: bool,
    detail_scroll: u16,
    dialog: Option<Dialog>,
    batch: Batch,
    message: String,
    scanning: bool,
    scan_ticket: u64,
    list_area: Rect,
    detail_areas: Vec<Rect>,
    dialog_list: Option<(Rect, usize)>,
    dialog_preview: Rect,
    last_click: Option<(usize, Instant)>,
    focus_after_scan: Option<PathBuf>,
    return_target: Option<crate::tasks::Navigation>,
    return_applied: bool,
    dirty: bool,
    pending_edits: usize,
    edit_serial: u64,
    pending_forms: BTreeMap<u64, Dialog>,
    failed_forms: Vec<Dialog>,
    batch_scroll: u16,
}
enum Outcome {
    Stay,
    Quit,
    Current(Box<Options>),
}
impl Overview {
    pub fn new(config: Config) -> Self {
        let show_detail = config.overview.detail.unwrap_or(true);
        let show_stats = config.overview.statistics.unwrap_or(true);
        Self {
            return_target: None,
            return_applied: false,
            dashboard: dashboard::Dashboard::default(),
            filter_counts: BTreeMap::new(),
            group_counts: BTreeMap::new(),
            config,
            profiles: BTreeMap::new(),
            profile_errors: BTreeMap::new(),
            staged: BTreeMap::new(),
            changed_defaults: BTreeSet::new(),
            recovery: Default::default(),
            filter_hits: vec![],
            projects: vec![],
            rows: vec![],
            selected: 0,
            list: ListState::default(),
            query: String::new(),
            marked: model::Queue::default(),
            git: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            host_error: String::new(),
            stats: BTreeMap::new(),
            sizes: BTreeMap::new(),
            activity: BTreeMap::new(),
            usage: BTreeMap::new(),
            history: None,
            detail_ticket: 0,
            detail_key: None,
            detail_pending: false,
            refresh_pending: None,
            show_stats,
            show_detail,
            detail_scroll: 0,
            dialog: None,
            batch: Batch::default(),
            message: String::new(),
            scanning: true,
            scan_ticket: 0,
            list_area: Rect::default(),
            detail_areas: vec![],
            dialog_list: None,
            dialog_preview: Rect::default(),
            last_click: None,
            focus_after_scan: None,
            dirty: true,
            pending_edits: 0,
            edit_serial: 0,
            pending_forms: BTreeMap::new(),
            failed_forms: vec![],
            batch_scroll: 0,
        }
    }
    fn path(&self) -> Option<PathBuf> {
        self.rows
            .get(self.selected)
            .and_then(Row::path)
            .map(PathBuf::from)
    }
    fn rebuild(&mut self) {
        let old_group = self.rows.get(self.selected).and_then(|row| {
            if let Row::Group(group) = row {
                Some(*group)
            } else {
                None
            }
        });
        let old = self.path();
        let old_task = self.rows.get(self.selected).and_then(|r| {
            if let Row::Task(_, w, t) = r {
                Some((w.clone(), t.clone()))
            } else {
                None
            }
        });
        let old_session = self.rows.get(self.selected).and_then(|r| {
            if let Row::Session(_, a, id) = r {
                Some((*a, id.clone()))
            } else {
                None
            }
        });
        let mut effective = self.config.clone();
        for (key, profile) in &self.profiles {
            crate::project_profile::Profile::from_project(profile)
                .apply(effective.projects.entry(key.clone()).or_default());
        }
        for (key, time) in &self.usage {
            let p = effective.projects.entry(key.clone()).or_default();
            p.last_opened = p.last_opened.max(*time);
        }
        // Search labels are transient metadata; project settings remain untouched.
        for (key, workspaces) in &self.workspaces {
            let search = &mut effective.projects.entry(key.clone()).or_default().note;
            for task in workspaces.iter().flat_map(|w| &w.tasks) {
                search.push_str(&format!(
                    " {} {} {}",
                    task.name,
                    task.branch,
                    task.cwd.display()
                ));
            }
        }
        effective.overview.attention = self.profile_errors.keys().cloned().collect();
        effective.overview.attention.extend(
            self.batch
                .items
                .iter()
                .filter(|i| matches!(i.state, LaunchState::Failed(_)))
                .map(|i| config::key(&i.options.project)),
        );
        // Returning from a workspace must reveal its project even if an old
        // saved filter or folded group would otherwise hide it.
        if let Some(path) = &self.focus_after_scan
            && self
                .projects
                .iter()
                .any(|p| config::key(p) == config::key(path))
        {
            let group = model::project_group(
                path,
                &effective,
                config::now(),
                &self.workspaces.keys().cloned().collect(),
                &self.git,
            );
            self.query.clear();
            self.config.overview.filter = model::Filter::All;
            self.config.overview.collapsed.remove(&group);
            effective.overview.filter = model::Filter::All;
            effective.overview.collapsed.remove(&group);
        }
        self.filter_counts.clear();
        self.group_counts.clear();
        let mut counts = effective.clone();
        counts.overview.collapsed.clear();
        for filter in [
            model::Filter::All,
            model::Filter::Running,
            model::Filter::Attention,
            model::Filter::Dirty,
            model::Filter::Pending,
            model::Filter::Codex,
            model::Filter::Claude,
            model::Filter::Shell,
        ] {
            counts.overview.filter = filter;
            let rows = model::rows(
                &self.projects,
                &counts,
                &self.query,
                &self.workspaces.keys().cloned().collect(),
                &self.git,
                &self.sizes,
                &self.activity,
            );
            self.filter_counts
                .insert(filter, rows.iter().filter(|r| r.path().is_some()).count());
            if filter == effective.overview.filter {
                for row in rows {
                    if let Row::Project(p) = row {
                        *self
                            .group_counts
                            .entry(model::project_group(
                                &p,
                                &effective,
                                config::now(),
                                &self.workspaces.keys().cloned().collect(),
                                &self.git,
                            ))
                            .or_default() += 1;
                    }
                }
            }
        }
        self.rows = model::rows(
            &self.projects,
            &effective,
            &self.query,
            &self.workspaces.keys().cloned().collect::<BTreeSet<_>>(),
            &self.git,
            &self.sizes,
            &self.activity,
        );
        let mut expanded = Vec::new();
        for row in self.rows.drain(..) {
            let children = if let Row::Project(p) = &row {
                if self.dashboard.expanded_projects.contains(&config::key(p))
                    || !self.query.is_empty()
                {
                    let mut children = self
                        .workspaces
                        .get(&config::key(p))
                        .into_iter()
                        .flatten()
                        .flat_map(|w| {
                            w.tasks
                                .iter()
                                .filter(|t| !t.archived)
                                .map(|t| Row::Task(p.clone(), w.id.clone(), t.id.clone()))
                        })
                        .collect::<Vec<_>>();
                    let agent = self.staged.get(&config::key(p)).map_or_else(
                        || {
                            effective
                                .projects
                                .get(&config::key(p))
                                .and_then(|p| p.agent)
                                .or(effective.default_agent)
                                .unwrap_or(Agent::Codex)
                        },
                        |o| o.agent,
                    );
                    children.extend(
                        self.dashboard
                            .contexts
                            .get(&dashboard::context_key(p, agent))
                            .map(|h| {
                                h.sessions
                                    .iter()
                                    .map(|c| Row::Session(p.clone(), agent, c.id.clone()))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                    );
                    children
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            expanded.push(row);
            expanded.extend(children);
        }
        self.rows = expanded;
        self.selected = old
            .as_ref()
            .and_then(|path| {
                self.rows.iter().position(|r| {
                    r.path()
                        .is_some_and(|p| config::key(p) == config::key(path))
                        && if let Some((w, t)) = &old_task {
                            matches!(r,Row::Task(_,other_w,other_t) if w==other_w&&t==other_t)
                        } else {
                            match (&old_session, r) {
                                (Some((a, id)), Row::Session(_, b, other)) => a == b && id == other,
                                (None, Row::Project(_)) => true,
                                _ => false,
                            }
                        }
                })
            })
            .or_else(|| {
                old_group.and_then(|group| self.rows.iter().position(|r| *r == Row::Group(group)))
            })
            .or_else(|| self.rows.iter().position(|r| r.path().is_some()))
            .unwrap_or(0);
        self.last_click = None;
        self.reveal_return_target();
        self.dashboard.hover = None;
        self.dirty = true;
    }
    fn reveal_return_target(&mut self) {
        if !self.return_applied
            && let Some(target) = &self.return_target
            && let Some(index) = self
                .rows
                .iter()
                .position(|r| matches!(r,Row::Task(_,w,t) if *w==target.workspace&&*t==target.task))
        {
            self.selected = index;
            self.focus_after_scan = None;
            self.return_applied = true;
        }
    }
    fn selected_task(&self) -> Option<crate::tasks::Task> {
        let Row::Task(path, workspace, id) = self.rows.get(self.selected)? else {
            return None;
        };
        self.workspaces
            .get(&config::key(path))?
            .iter()
            .find(|w| w.id == *workspace)?
            .tasks
            .iter()
            .find(|t| t.id == *id)
            .cloned()
    }
    fn active_path(&self) -> Option<PathBuf> {
        self.selected_task().map(|t| t.cwd).or_else(|| self.path())
    }
    fn toggle_selected_queue(&mut self) {
        if let Some(p) = self.path() {
            if matches!(self.rows.get(self.selected), Some(Row::Task(..))) {
                self.marked.toggle_task(self.options_for(&p));
            } else {
                self.marked.toggle(p);
            }
        }
    }
    fn edit(&mut self, w: &Workers, edit: Edit) -> Result<()> {
        self.edit_serial += 1;
        w.send(Action::Edit(self.edit_serial, edit))?;
        self.pending_edits += 1;
        Ok(())
    }
    fn discover(&mut self, w: &Workers) -> Result<()> {
        self.scan_ticket = w.discover(self.config.clone())?;
        self.scanning = true;
        Ok(())
    }
    fn start(&mut self, w: &Workers, options: Vec<Options>) -> Result<()> {
        for o in &options {
            anyhow::ensure!(
                o.new_instance
                    || o.workspace.is_some()
                    || self
                        .workspaces
                        .get(&config::key(&o.project))
                        .is_none_or(|ws| ws.len() <= 1),
                "{}",
                t("overview.choose_instance")
            );
            if let Some(e) = self.profile_errors.get(&config::key(&o.project)) {
                anyhow::bail!("{e}");
            }
        }
        let jobs = self.batch.start(options)?;
        if jobs.is_empty() {
            anyhow::bail!("{}", t("preset_empty"))
        }
        if let Err(e) = w.send(Action::Launch(self.batch.generation, jobs)) {
            for item in &mut self.batch.items {
                item.state = LaunchState::Failed(e.to_string())
            }
            return Err(e);
        }
        self.batch_scroll = 0;
        self.dialog = Some(Dialog::Batch);
        Ok(())
    }
    fn retry(&mut self, w: &Workers) -> Result<()> {
        let items = self.batch.retry();
        if !items.is_empty()
            && let Err(e) = w.send(Action::Launch(self.batch.generation, items))
        {
            for item in &mut self.batch.items {
                if item.state == LaunchState::Queued {
                    item.state = LaunchState::Failed(e.to_string())
                }
            }
            return Err(e);
        }
        self.dialog = Some(Dialog::Batch);
        Ok(())
    }
    fn launch_spec(&self, path: &std::path::Path) -> LaunchSpec {
        let key = config::key(path);
        let p = self
            .profiles
            .get(&key)
            .or_else(|| self.config.projects.get(&key))
            .cloned()
            .unwrap_or_default();
        LaunchSpec::from_project(path, self.config.default_agent, p)
    }
    fn options_for(&self, path: &std::path::Path) -> Options {
        let mut options = self
            .staged
            .get(&config::key(path))
            .cloned()
            .unwrap_or_else(|| self.launch_spec(path).options());
        if let Some(Row::Session(p, agent, id)) = self.rows.get(self.selected)
            && config::key(p) == config::key(path)
        {
            options.agent = *agent;
            options.resume_id = Some(id.clone());
            options.resume = false;
            options.continue_latest = false;
            options.fresh = false;
            options.recovery_id = None;
            let existing = self.workspaces.get(&config::key(path)).and_then(|ws| {
                ws.iter()
                    .find(|w| w.options.agent == *agent && w.options.resume_id.as_ref() == Some(id))
            });
            options.new_instance = existing.is_none();
            options.workspace = existing.map(|w| w.id.clone());
        }
        if let Some(Row::Task(p, workspace, task)) = self.rows.get(self.selected)
            && config::key(p) == config::key(path)
            && let Some(work) = self
                .workspaces
                .get(&config::key(p))
                .into_iter()
                .flatten()
                .find(|w| w.id == *workspace)
        {
            options = work
                .tasks
                .iter()
                .find(|t| t.id == *task)
                .map_or_else(|| work.options.clone(), |t| t.options.clone());
            options.project = p.clone();
            options.workspace = Some(workspace.clone());
            if let Some(session) = work
                .sessions
                .iter()
                .find(|s| s.task.task_id == *task && s.task.role == crate::tasks::Role::Agent)
            {
                options.navigation = Some(Box::new(crate::tasks::Navigation {
                    project: p.clone(),
                    workspace: workspace.clone(),
                    task: task.clone(),
                    session: session.id.clone(),
                }));
                if let Some(target) = &self.return_target
                    && target.workspace == *workspace
                    && target.task == *task
                    && work
                        .sessions
                        .iter()
                        .any(|s| s.id == target.session && s.task.task_id == *task)
                {
                    options.navigation = Some(Box::new(target.clone()));
                }
            }
        }
        options
    }
    fn stage_resume(&mut self, path: &std::path::Path, agent: Agent, id: String) {
        let mut options = self.options_for(path);
        options.agent = agent;
        options.resume_id = Some(id.clone());
        options.continue_latest = false;
        options.resume = false;
        options.fresh = false;
        options.recovery_id = None;
        let existing = self.workspaces.get(&config::key(path)).and_then(|ws| {
            ws.iter()
                .find(|w| w.options.agent == agent && w.options.resume_id.as_ref() == Some(&id))
        });
        options.workspace = existing.map(|w| w.id.clone());
        options.new_instance = existing.is_none();
        self.selected=self.rows.iter().position(|r|matches!(r,Row::Session(p,a,other) if config::key(p)==config::key(path) && *a==agent && *other==id))
            .or_else(||self.rows.iter().position(|r|matches!(r,Row::Project(p) if config::key(p)==config::key(path)))).unwrap_or(self.selected);
        self.staged.insert(config::key(path), options);
        self.dashboard.summary_cache = None;
    }
    fn launch_description(&self, o: &Options) -> String {
        let action = if o.recovery_id.is_some() {
            t("parity.pending")
        } else if o.new_instance {
            t("overview.new_instance")
        } else if o.workspace.is_some() || self.workspaces.contains_key(&config::key(&o.project)) {
            t("overview.activate")
        } else if !self.host_error.is_empty() {
            t("overview.host_unknown")
        } else {
            t("overview.create")
        };
        format!("{} · {action}", view::opening(o))
    }
    fn specs(&self) -> Vec<LaunchSpec> {
        self.queue_options()
            .iter()
            .map(LaunchSpec::from_options)
            .collect()
    }
    fn queue_options(&self) -> Vec<Options> {
        self.marked
            .0
            .iter()
            .map(|e| {
                e.restore
                    .clone()
                    .or_else(|| e.connect.clone())
                    .unwrap_or_else(|| self.options_for(&e.path))
            })
            .collect()
    }
    fn open_history(&mut self, w: &Workers, path: PathBuf) -> Result<()> {
        self.dashboard.history_query.clear();
        self.dashboard.history_preview = None;
        let agent = self.options_for(&path).agent;
        anyhow::ensure!(agent != Agent::Shell, "{}", t("history_shell"));
        self.history = self
            .dashboard
            .contexts
            .get(&dashboard::context_key(&path, agent))
            .cloned();
        self.dashboard.history_ticket = w.history_ticket();
        self.dashboard.history_pending = true;
        self.dashboard.context_pending = None;
        w.history
            .try_send(Detail::History(
                path.clone(),
                agent,
                self.dashboard.history_ticket,
            ))
            .map_err(|_| anyhow::anyhow!("{}", t("busy")))?;
        self.dialog = Some(Dialog::History {
            path,
            agent,
            selected: 0,
            scroll: 0,
        });
        Ok(())
    }
    fn poll(&mut self, w: &Workers) -> Result<()> {
        let mut rebuild = false;
        // Bound per-tick delivery work so a large discovery never monopolizes input.
        let mut delivered = 0;
        while delivered < 32 {
            let Ok(reply) = w.replies.try_recv() else {
                break;
            };
            delivered += 1;
            let host_only = matches!(reply, Reply::Host(_));
            if !host_only {
                self.dirty = true;
            }
            match reply {
                Reply::Config(id, result) => {
                    let refreshed = self.refresh_pending == Some(id);
                    if refreshed {
                        self.refresh_pending = None;
                        self.stats.clear();
                        self.detail_pending = false;
                    }
                    let form = self.pending_forms.remove(&id);
                    self.pending_edits = self.pending_edits.saturating_sub(1);
                    match result {
                        Ok(c) => {
                            for key in self.staged.keys() {
                                if serde_json::to_value(c.projects.get(key)).ok()
                                    != serde_json::to_value(self.config.projects.get(key)).ok()
                                {
                                    self.changed_defaults.insert(key.clone());
                                }
                            }
                            let scan = c.roots != self.config.roots
                                || c.added != self.config.added
                                || c.excluded != self.config.excluded
                                || c.overview.sort != self.config.overview.sort;
                            let pending = self.config.overview.pending.clone();
                            self.config = c;
                            self.config.overview.pending = pending;
                            self.profiles.clear();
                            self.profile_errors.clear();
                            if !matches!(self.dialog, Some(Dialog::Palette { .. })) {
                                crate::appearance::set(self.config.theme)
                            }
                            crate::i18n::set(self.config.language);
                            if refreshed || scan || !self.projects.is_empty() {
                                self.discover(w)?
                            }
                            if let Some(Dialog::Input(Input::NewProject(root), name)) = &form {
                                let target = root.join(name.trim());
                                if !self.marked.contains(&target) {
                                    self.marked.push(target.clone());
                                }
                                self.focus_after_scan = Some(target);
                            }
                            self.message = t("saved").into();
                            rebuild = true
                        }
                        Err(e) => {
                            if let Some(form) = form {
                                self.failed_forms.push(form);
                            }
                            self.message = e;
                        }
                    }
                }
                Reply::Profile(ticket, key, result) if ticket == self.scan_ticket => {
                    match result {
                        Ok(p) => {
                            if self.staged.contains_key(&key)
                                && self.profiles.get(&key).is_some_and(|old| {
                                    serde_json::to_value(old).ok() != serde_json::to_value(&p).ok()
                                })
                            {
                                self.changed_defaults.insert(key.clone());
                            }
                            self.profile_errors.remove(&key);
                            self.profiles.insert(key, p);
                        }
                        Err(e) => {
                            self.message = e.clone();
                            self.profile_errors.insert(key, e);
                        }
                    }
                    rebuild = true;
                }
                Reply::Recovery(result) => match result {
                    Ok(j) => {
                        let active = self
                            .workspaces
                            .values()
                            .flatten()
                            .map(|w| w.id.clone())
                            .collect();
                        let pending = j
                            .pending(&active)
                            .iter()
                            .map(|r| config::key(&r.options.project))
                            .collect::<BTreeSet<_>>();
                        if pending != self.config.overview.pending {
                            self.config.overview.pending = pending;
                            rebuild = true;
                            self.dirty = true;
                        }
                        self.recovery = j;
                    }
                    Err(e) => {
                        self.message = e;
                        self.dirty = true;
                    }
                },
                Reply::Paths(ticket, paths) if ticket == self.scan_ticket => {
                    self.projects = paths;
                    self.scanning = false;
                    rebuild = true
                }
                Reply::Git(ticket, key, result) if ticket == self.scan_ticket => {
                    self.git.insert(key, result);
                    rebuild = true
                }
                Reply::Host(result) => {
                    let (next, error) = match result {
                        Ok(work) => (worker::active(&work), String::new()),
                        Err(e) => (BTreeMap::new(), e),
                    };
                    if workspace_signature(&next) != workspace_signature(&self.workspaces)
                        || error != self.host_error
                    {
                        // The host can reorder instances between polls. Keep the
                        // selected identity, never silently target the old index.
                        if let Some(Dialog::Instances { path, selected }) = &mut self.dialog {
                            let key = config::key(path);
                            let id = self
                                .workspaces
                                .get(&key)
                                .and_then(|work| work.get(*selected))
                                .map(|work| &work.id);
                            if let Some(index) = next
                                .get(&key)
                                .and_then(|work| work.iter().position(|work| Some(&work.id) == id))
                            {
                                *selected = index;
                            } else {
                                self.dialog = None;
                                self.message = t("overview.instance_gone").into_owned();
                            }
                        }
                        self.workspaces = next;
                        self.host_error = error;
                        rebuild = true;
                    }
                }
                Reply::Stats(key, ticket, result)
                    if ticket == self.detail_ticket && Some(&key) == self.detail_key.as_ref() =>
                {
                    self.detail_pending = result.disk_scanning;
                    self.sizes.insert(key.clone(), result.source_bytes);
                    self.activity.insert(key.clone(), result.activity);
                    if self.stats.len() >= 16
                        && let Some(k) = self.stats.keys().next().cloned()
                    {
                        self.stats.remove(&k);
                        self.dashboard.stats_at.remove(&k);
                    }
                    if !result.disk_scanning {
                        self.dashboard.stats_at.insert(key.clone(), Instant::now());
                    }
                    self.stats.insert(key, result);
                    rebuild = true
                }
                Reply::Session(key, agent, ticket, result)
                    if self
                        .dashboard
                        .session_pending
                        .as_ref()
                        .is_some_and(|(_, t)| *t == ticket) =>
                {
                    if let Some((id, _)) = self.dashboard.session_pending.take() {
                        self.dashboard.session_loaded.insert(id);
                    }
                    match result {
                        Ok(Some(c)) => {
                            if let Some(h) =
                                self.dashboard.contexts.get_mut(&format!("{key}|{agent:?}"))
                                && let Some(old) = h.sessions.iter_mut().find(|s| s.id == c.id)
                            {
                                old.turns = c.turns.clone();
                            }
                            if let Some(h) = &mut self.history
                                && let Some(old) = h.sessions.iter_mut().find(|s| s.id == c.id)
                            {
                                old.turns = c.turns;
                                self.dashboard.history_preview = None;
                            }
                            self.dashboard.summary_cache = None;
                        }
                        Err(e) => self.message = e,
                        _ => {}
                    }
                }
                Reply::History(key, ticket, result)
                    if ticket == self.dashboard.history_ticket
                        && matches!(&self.dialog, Some(Dialog::History {path,..}) if config::key(path)==key) =>
                {
                    self.dashboard.history_pending = false;
                    if let Some(Dialog::History { agent, .. }) = &self.dialog {
                        self.dashboard
                            .store_context(format!("{key}|{agent:?}"), result.clone());
                    }
                    self.dashboard.history_preview = None;
                    self.history = Some(result);
                }
                Reply::Context(key, agent, ticket, result)
                    if self.dashboard.context_pending == Some(ticket)
                        && self.dashboard.context_target.as_deref()
                            == Some(&format!("{key}|{agent:?}")) =>
                {
                    self.dashboard.context_pending = None;
                    self.dashboard
                        .store_context(format!("{key}|{agent:?}"), result);
                    rebuild = true;
                }
                Reply::Source(key, ticket, result)
                    if self.dashboard.source_pending == Some(ticket)
                        && self.detail_key.as_ref() == Some(&key) =>
                {
                    self.dashboard.source_pending = None;
                    self.sizes.insert(key.clone(), result.source_bytes);
                    self.activity.insert(key.clone(), result.activity);
                    if self.dashboard.sources.len() >= 16
                        && let Some(old) = self.dashboard.sources.keys().next().cloned()
                    {
                        self.dashboard.sources.remove(&old);
                        self.dashboard.sources_at.remove(&old);
                    }
                    self.dashboard
                        .sources_at
                        .insert(key.clone(), Instant::now());
                    self.dashboard.sources.insert(key, result);
                    rebuild = true;
                }
                Reply::Usage(ticket, usage) if ticket == self.scan_ticket => {
                    self.usage = usage;
                    rebuild = true;
                }
                Reply::Ranked(ticket, key, result) if ticket == self.scan_ticket => {
                    self.sizes.insert(key.clone(), result.source_bytes);
                    self.activity.insert(key, result.activity);
                    rebuild = true;
                }
                Reply::Attached(g, i, id) if g == self.batch.generation => {
                    if let Some(item) = self.batch.items.get_mut(i) {
                        item.options.workspace = Some(id);
                    }
                }
                Reply::Launch(g, i, state) => {
                    if g == self.batch.generation
                        && matches!(state, LaunchState::Opened(_))
                        && let Some(item) = self.batch.items.get(i)
                    {
                        self.config
                            .projects
                            .entry(config::key(&item.options.project))
                            .or_default()
                            .last_opened = config::now();
                        rebuild = true;
                    }
                    self.batch.update(g, i, state);
                    if g == self.batch.generation {
                        rebuild = true;
                    }
                }
                Reply::Stats(..)
                | Reply::Source(..)
                | Reply::Context(..)
                | Reply::History(..)
                | Reply::Session(..) => crate::perf::add(crate::perf::Count::OverviewStale, 1),
                _ => {}
            }
        }
        if self.dialog.is_none() {
            self.dialog = self.failed_forms.pop();
        }
        if rebuild {
            self.rebuild();
            if !self.return_applied
                && let Some(path) = &self.focus_after_scan
                && let Some(index) = self.rows.iter().position(|r| {
                    r.path()
                        .is_some_and(|p| config::key(p) == config::key(path))
                })
            {
                self.selected = index;
                self.focus_after_scan = None;
            }
        }
        Ok(())
    }
    fn request_stats(&mut self, w: &Workers) -> Result<()> {
        use dashboard::Focus;
        if self.refresh_pending.is_some() {
            return Ok(());
        }
        let Some(path) = self.active_path() else {
            return Ok(());
        };
        let key = config::key(&path);
        if self.detail_key.as_ref() != Some(&key) {
            w.ticket();
            w.source_ticket();
            self.detail_key = Some(key.clone());
            self.detail_pending = false;
            self.dashboard.source_pending = None;
            self.dashboard.selected_since = Some((key.clone(), Instant::now()));
            self.detail_scroll = 0;
            self.dashboard.stats_scroll = 0;
            self.dashboard.expanded = false;
            self.dashboard.file_selected = None;
        }
        let stats_visible = self.dashboard.columns.is_some_and(|c| c.stats.is_some())
            || self.dashboard.overlay == Some(Focus::Stats);
        let detail_visible = self.dashboard.columns.is_some_and(|c| c.detail.is_some())
            || self.dashboard.overlay == Some(Focus::Detail);
        if !stats_visible && self.detail_pending {
            self.detail_ticket = w.ticket();
            self.detail_pending = false;
            crate::perf::add(crate::perf::Count::OverviewCanceled, 1);
        }
        if !detail_visible && !stats_visible && self.dashboard.source_pending.take().is_some() {
            w.source_ticket();
            crate::perf::add(crate::perf::Count::OverviewCanceled, 1);
        }
        if !detail_visible
            && !matches!(self.dialog, Some(Dialog::History { .. }))
            && self.dashboard.context_pending.take().is_some()
        {
            w.history_ticket();
            crate::perf::add(crate::perf::Count::OverviewCanceled, 1);
        }
        let elapsed = self
            .dashboard
            .selected_since
            .as_ref()
            .map_or(Duration::ZERO, |(_, since)| since.elapsed());
        if stats_visible
            && (self.stats.get(&key).is_none_or(|s| !s.disk_ready)
                || self.dashboard.stats_at.get(&key).is_some_and(|t| {
                    t.elapsed()
                        >= Duration::from_secs(
                            self.stats.get(&key).map_or(600, stats::refresh_secs),
                        )
                }))
            && !self.detail_pending
            && (elapsed >= Duration::from_millis(500)
                || self.dashboard.overlay == Some(Focus::Stats))
        {
            self.detail_ticket = w.ticket();
            if w.detail
                .try_send(Detail::Stats(path.clone(), self.detail_ticket))
                .is_ok()
            {
                self.detail_pending = true;
                self.dirty = true;
            }
        }
        if (detail_visible || stats_visible)
            && !self.stats.contains_key(&key)
            && (!self.dashboard.sources.contains_key(&key)
                || self.dashboard.sources_at.get(&key).is_some_and(|t| {
                    t.elapsed()
                        >= Duration::from_secs(
                            self.dashboard
                                .sources
                                .get(&key)
                                .map_or(600, stats::refresh_secs),
                        )
                }))
            && self.dashboard.source_pending.is_none()
            && elapsed >= Duration::from_millis(200)
        {
            self.dashboard.source_ticket = w.source_ticket();
            if w.source
                .try_send(Detail::Source(path.clone(), self.dashboard.source_ticket))
                .is_ok()
            {
                self.dashboard.source_pending = Some(self.dashboard.source_ticket);
            }
        }
        let task = self.selected_task();
        let options = task
            .as_ref()
            .map_or_else(|| self.options_for(&path), |t| t.options.clone());
        if task.is_some() && options.resume_id.is_none() {
            if self.dashboard.context_pending.take().is_some() {
                w.history_ticket();
            }
            return Ok(());
        }
        let agent = options.agent;
        let target = dashboard::context_key(&path, agent);
        if self.dashboard.context_target.as_ref() != Some(&target) {
            if !matches!(self.dialog, Some(Dialog::History { .. })) {
                w.history_ticket();
            }
            if self.dashboard.contexts.contains_key(&target) {
                self.dashboard.context_order.retain(|k| *k != target);
                self.dashboard.context_order.push_back(target.clone());
                crate::perf::add(crate::perf::Count::OverviewContextHits, 1);
            }
            self.dashboard.context_target = Some(target.clone());
            self.dashboard.context_pending = None;
            self.dashboard.context_since = Some(Instant::now());
        }
        if detail_visible
            && agent != Agent::Shell
            && !matches!(self.dialog, Some(Dialog::History { .. }))
            && !self.dashboard.contexts.contains_key(&target)
            && self.dashboard.context_pending.is_none()
            && self
                .dashboard
                .context_since
                .is_some_and(|s| s.elapsed() >= Duration::from_millis(200))
        {
            let ticket = w.history_ticket();
            if w.history
                .try_send(Detail::Context(path.clone(), agent, ticket))
                .is_ok()
            {
                self.dashboard.context_pending = Some(ticket);
            }
        }
        let session = if let Some(Dialog::History { selected, .. }) = &self.dialog {
            self.history.as_ref().and_then(|h| {
                h.sessions
                    .iter()
                    .filter(|c| history::matches(c, &self.dashboard.history_query))
                    .nth(*selected)
            })
        } else {
            self.dashboard.contexts.get(&target).and_then(|h| {
                options
                    .resume_id
                    .as_ref()
                    .and_then(|id| h.sessions.iter().find(|c| c.id == *id))
                    .or_else(|| {
                        if options.resume_id.is_none() {
                            h.sessions.first()
                        } else {
                            None
                        }
                    })
            })
        };
        if let Some(c) = session.filter(|c| c.turns.is_empty()) {
            let identity = format!("{target}|{}", c.id);
            if !self.dashboard.session_loaded.contains(&identity)
                && self
                    .dashboard
                    .session_pending
                    .as_ref()
                    .is_none_or(|(id, _)| *id != identity)
            {
                let file = c.file.clone();
                let ticket = w.history_ticket();
                if w.history
                    .try_send(Detail::Session(path, agent, file, ticket))
                    .is_ok()
                {
                    self.dashboard.session_pending = Some((identity, ticket));
                }
            }
        }
        Ok(())
    }
    fn entries(&self) -> Vec<Entry> {
        let available = self.path().is_some();
        let mut out = vec![];
        let commands = [
            ("open", "Enter", Command::Open, available),
            ("options", "Alt+Enter", Command::Options, available),
            ("overview.diagnostics", "", Command::Diagnostics, available),
            ("review.inbox", "!", Command::Attention, true),
            ("overview.copy_path", "Ctrl+C", Command::CopyPath, available),
            ("overview.reset_draft", "", Command::ResetDraft, available),
            ("overview.new", "", Command::NewSession, available),
            ("overview.latest", "", Command::LatestSession, available),
            ("overview.native", "", Command::NativePicker, available),
            ("favorite", "F2", Command::Favorite, available),
            ("edit_note", "F3", Command::Note, available),
            ("settings", "F4", Command::Settings, available),
            (
                "history",
                "F9",
                Command::History,
                available
                    && self
                        .path()
                        .is_some_and(|p| self.options_for(&p).agent != Agent::Shell),
            ),
            ("statistics", "F8", Command::Stats, available),
            ("detail", "F10", Command::Details, available),
            ("filter", "F6", Command::Filter, true),
            ("sort", "F7", Command::Sort, true),
            ("group", "Left/Right", Command::Group, !self.rows.is_empty()),
            (
                "launch_selected",
                "Ctrl+Enter",
                Command::Launch,
                !self.marked.is_empty() && !self.batch.busy(),
            ),
            (
                "save_preset",
                "Ctrl+S",
                Command::SavePreset,
                !self.marked.is_empty(),
            ),
            (
                "retry",
                "R",
                Command::Retry,
                !self.batch.busy()
                    && self
                        .batch
                        .items
                        .iter()
                        .any(|i| matches!(i.state, LaunchState::Failed(_))),
            ),
            ("parity.help", "F1", Command::Help, true),
            (
                "parity.select_all",
                "Ctrl+Shift+A",
                Command::SelectAll,
                !self.rows.is_empty(),
            ),
            (
                "parity.new_project",
                "Ctrl+N",
                Command::NewProject,
                !self.config.roots.is_empty() || available,
            ),
            (
                "parity.presets",
                "Ctrl+B",
                Command::Presets,
                !self.config.presets.is_empty(),
            ),
            (
                "parity.export_profile",
                "",
                Command::ExportProfile,
                available,
            ),
            (
                "parity.restore",
                "",
                Command::Restore,
                !self.config.overview.pending.is_empty(),
            ),
            (
                "parity.last",
                "",
                Command::Last,
                !self.recovery.last.is_empty(),
            ),
            ("refresh", "F5", Command::Refresh, true),
            ("root", "Ctrl+R", Command::Root, true),
            ("add", "Ctrl+A", Command::Add, true),
            ("exclude", "Ctrl+E", Command::Exclude, available),
            ("folder", "", Command::Folder, available),
            ("editor", "", Command::Editor, available),
            ("config", "", Command::Config, true),
            ("quit", "Esc", Command::Quit, true),
        ];
        for (key, hint, command, enabled) in commands {
            out.push(Entry {
                label: t(key).into(),
                keywords: format!(
                    "{} {}",
                    crate::i18n::in_language(crate::i18n::Language::EnUs, key),
                    crate::i18n::in_language(crate::i18n::Language::ZhCn, key)
                ),
                hint,
                command,
                enabled,
            })
        }
        for agent in [Agent::Codex, Agent::Claude, Agent::Shell] {
            out.push(Entry {
                label: format!("{}: {agent:?}", t("agent")),
                keywords: format!("agent 工具 {agent:?}"),
                hint: "←/→",
                command: Command::DraftAgent(agent),
                enabled: available,
            });
        }
        for composition in [
            crate::launch::Composition::AgentOnly,
            crate::launch::Composition::AgentDebug,
            crate::launch::Composition::Custom,
        ] {
            out.push(Entry {
                label: format!("{}: {}", t("overview.composition"), composition.label()),
                keywords: format!("composition 终端 组合 {:?}", composition),
                hint: "",
                command: Command::DraftComposition(composition),
                enabled: available,
            });
        }
        for language in [
            crate::i18n::Language::Auto,
            crate::i18n::Language::ZhCn,
            crate::i18n::Language::EnUs,
        ] {
            let label = match language {
                crate::i18n::Language::Auto => t("auto"),
                crate::i18n::Language::ZhCn => "简体中文".into(),
                crate::i18n::Language::EnUs => "English".into(),
            };
            out.push(Entry {
                label: format!("{}: {label}", t("language")),
                keywords: "language 语言 locale 中文 English".into(),
                hint: "",
                command: Command::Language(language),
                enabled: true,
            })
        }
        for theme in [
            crate::appearance::Theme::Terminal,
            crate::appearance::Theme::Tokyo,
            crate::appearance::Theme::Catppuccin,
            crate::appearance::Theme::Light,
        ] {
            out.push(Entry {
                label: format!("{}: {}", t("theme"), theme.label()),
                keywords: format!("theme 主题 {:?}", theme),
                hint: "",
                command: Command::Theme(theme),
                enabled: true,
            })
        }
        for path in &self.projects {
            out.push(Entry {
                label: format!("{}: {}", t("projects"), path.display()),
                keywords: path.to_string_lossy().into_owned(),
                hint: "Enter",
                command: Command::Project(path.clone()),
                enabled: true,
            })
        }
        for name in self.config.presets.keys() {
            out.push(Entry {
                label: format!("{}: {name}", t("preset")),
                keywords: format!("preset 预设 {name}"),
                hint: "Enter",
                command: Command::Preset(name.clone()),
                enabled: !self.batch.busy(),
            })
        }
        out
    }
    fn palette_entries(&self, query: &str) -> Vec<Entry> {
        let query = query.to_lowercase();
        let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
        let mut buf = vec![];
        let needle = nucleo_matcher::Utf32Str::new(&query, &mut buf);
        let mut matches = self
            .entries()
            .into_iter()
            .filter_map(|e| {
                let label = format!("{} {}", e.label, e.keywords).to_lowercase();
                let mut buffer = vec![];
                let score = if query.is_empty() {
                    Some(0)
                } else {
                    matcher.fuzzy_match(nucleo_matcher::Utf32Str::new(&label, &mut buffer), needle)
                };
                score.map(|s| (s, e))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        matches.into_iter().map(|(_, e)| e).collect()
    }
    fn command(&mut self, w: &Workers, command: Command) -> Result<Outcome> {
        let path = self.path();
        match command {
            Command::Open | Command::Options => {
                if let Some(p) = path {
                    let options = self.options_for(&p);
                    if matches!(command, Command::Open)
                        && matches!(self.rows.get(self.selected), Some(Row::Task(..)))
                    {
                        anyhow::ensure!(
                            options.navigation.is_some(),
                            "Task Agent unavailable; select recovery explicitly"
                        );
                        return Ok(Outcome::Current(Box::new(options)));
                    }
                    if matches!(command, Command::Options) {
                        self.dialog = Some(Dialog::Options {
                            options,
                            new_tab: true,
                            editing: None,
                        })
                    } else {
                        if self
                            .workspaces
                            .get(&config::key(&p))
                            .is_some_and(|ws| ws.len() > 1)
                            && !options.new_instance
                            && options.workspace.is_none()
                        {
                            self.dialog = Some(Dialog::Instances {
                                path: p,
                                selected: 0,
                            });
                        } else {
                            self.start(w, vec![options])?;
                        }
                    }
                }
            }
            Command::Project(p) => self.start(w, vec![self.options_for(&p)])?,
            Command::DraftAgent(_)
            | Command::DraftComposition(_)
            | Command::NewSession
            | Command::LatestSession
            | Command::NativePicker => {
                if let Some(path) = path {
                    let mut o = self.options_for(&path);
                    match command {
                        Command::DraftAgent(agent) => {
                            if o.agent != agent {
                                o.resume_id = None;
                            }
                            o.agent = agent;
                            if agent == Agent::Shell {
                                o.resume = false;
                                o.continue_latest = false;
                                o.fresh = true;
                            }
                        }
                        Command::DraftComposition(composition) => o.composition = composition,
                        Command::NewSession => {
                            o.fresh = true;
                            o.continue_latest = false;
                            o.resume = false;
                            o.resume_id = None;
                        }
                        Command::LatestSession => {
                            o.fresh = false;
                            o.continue_latest = true;
                            o.resume = false;
                            o.resume_id = None;
                        }
                        Command::NativePicker => {
                            o.fresh = false;
                            o.continue_latest = false;
                            o.resume = true;
                            o.resume_id = None;
                        }
                        _ => unreachable!(),
                    }
                    if matches!(self.rows.get(self.selected), Some(Row::Session(..))) {
                        self.selected=self.rows.iter().position(|r|matches!(r,Row::Project(p) if config::key(p)==config::key(&path))).unwrap_or(self.selected);
                    }
                    self.staged.insert(config::key(&path), o);
                }
            }
            Command::CopyPath => {
                if let Some(path) = path {
                    crate::files::copy(path.display().to_string())?;
                    self.message = t("overview.copied").into_owned();
                }
            }
            Command::ResetDraft => {
                if let Some(path) = path {
                    let key = config::key(&path);
                    self.staged.remove(&key);
                    self.changed_defaults.remove(&key);
                    self.dashboard.summary_cache = None;
                }
            }
            Command::Attention => self.dialog = Some(Dialog::Attention { selected: 0 }),
            Command::Diagnostics => self.dialog = Some(Dialog::Diagnostics),
            Command::Help => self.dialog = Some(Dialog::Help),
            Command::Presets => self.dialog = Some(Dialog::Presets { selected: 0 }),
            Command::SelectAll => {
                let visible = self
                    .rows
                    .iter()
                    .filter_map(Row::path)
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                if visible.iter().all(|p| self.marked.contains(p)) {
                    self.marked.retain(|p| !visible.contains(p));
                } else {
                    for p in visible {
                        if !self.marked.contains(&p) {
                            self.marked.push(p);
                        }
                    }
                }
            }
            Command::SetFilter(filter) => {
                self.config.overview.filter = filter;
                self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                self.rebuild();
            }
            Command::NewProject => {
                let root = path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(PathBuf::from)
                    .or_else(|| self.config.roots.first().cloned())
                    .ok_or_else(|| anyhow::anyhow!("{}", t("no_project")))?;
                self.dialog = Some(Dialog::Input(Input::NewProject(root), String::new()));
            }
            Command::ExportProfile => {
                if let Some(p) = path {
                    let mut value = Project::default();
                    let spec = self.launch_spec(&p);
                    value.agent = Some(spec.agent);
                    value.layout = Some(spec.layout);
                    value.composition = Some(spec.composition);
                    value.title = spec.title;
                    value.color = spec.color;
                    value.agent_args = spec.agent_args;
                    value.resume_latest = spec.resume_latest;
                    value.debug_command = spec.debug_command;
                    value.sessions = spec.sessions;
                    self.edit(w, Edit::ExportProfile(p, value))?;
                }
            }
            Command::Restore => {
                let active = self
                    .workspaces
                    .values()
                    .flatten()
                    .map(|w| w.id.clone())
                    .collect();
                self.dashboard.undo = Some(self.marked.clone());
                for entry in self.recovery.pending(&active) {
                    self.marked
                        .restore(crate::recovery::restore_options(&entry));
                }
                self.dashboard.focus = dashboard::Focus::Queue;
                self.dialog = Some(Dialog::Queue { selected: 0 });
            }
            Command::Last => {
                self.start(
                    w,
                    self.recovery
                        .last
                        .iter()
                        .cloned()
                        .map(|mut o| {
                            o.new_instance = false;
                            o
                        })
                        .collect(),
                )?;
            }
            Command::Favorite => {
                if let Some(p) = path {
                    self.edit(w, Edit::Favorite(p))?
                }
            }
            Command::Note => {
                if let Some(p) = path {
                    let value = self
                        .config
                        .projects
                        .get(&config::key(&p))
                        .map(|p| p.note.clone())
                        .unwrap_or_default();
                    self.dialog = Some(Dialog::Input(Input::Note(p), value))
                }
            }
            Command::Settings => {
                if let Some(path) = path {
                    let project = self
                        .profiles
                        .get(&config::key(&path))
                        .or_else(|| self.config.projects.get(&config::key(&path)))
                        .cloned()
                        .unwrap_or_default();
                    let debug = project.debug_command.clone().unwrap_or_default();
                    let arguments = serde_json::to_string(&project.agent_args)?;
                    self.dialog = Some(Dialog::Settings {
                        path,
                        arguments,
                        project,
                        field: 0,
                        debug,
                    })
                }
            }
            Command::History => {
                if let Some(p) = path {
                    self.open_history(w, p)?
                }
            }
            Command::Stats | Command::Details => {
                let focus = if matches!(command, Command::Stats) {
                    dashboard::Focus::Stats
                } else {
                    dashboard::Focus::Detail
                };
                let inline = if focus == dashboard::Focus::Stats {
                    self.dashboard.size.width >= 160
                        || (self.dashboard.size.width >= 104 && !self.show_detail)
                } else {
                    self.dashboard.size.width >= 104
                };
                if !inline {
                    self.dashboard.overlay = if self.dashboard.overlay == Some(focus) {
                        None
                    } else {
                        Some(focus)
                    };
                } else {
                    if focus == dashboard::Focus::Stats {
                        self.show_stats = !self.show_stats;
                        self.config.overview.statistics = Some(self.show_stats);
                    } else {
                        self.show_detail = !self.show_detail;
                        self.config.overview.detail = Some(self.show_detail);
                    }
                    self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                }
            }
            Command::Filter => {
                self.config.overview.filter = self.config.overview.filter.next();
                self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                self.rebuild()
            }
            Command::Sort => {
                self.config.overview.sort = self.config.overview.sort.next();
                self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                self.discover(w)?;
                self.rebuild()
            }
            Command::Group => {
                let g = self
                    .rows
                    .iter()
                    .take(self.selected + 1)
                    .rev()
                    .find_map(|r| match r {
                        Row::Group(g) => Some(*g),
                        _ => None,
                    });
                if let Some(g) = g {
                    if !self.config.overview.collapsed.remove(&g) {
                        self.config.overview.collapsed.insert(g);
                    }
                    self.edit(w, Edit::Preferences(self.config.overview.clone()))?;
                    self.rebuild()
                }
            }
            Command::Launch => {
                let options = self.queue_options();
                let connects = options
                    .iter()
                    .filter(|o| o.navigation.is_some())
                    .collect::<Vec<_>>();
                if let Some(first) = connects.first() {
                    anyhow::ensure!(
                        connects.len() == options.len(),
                        "Connect tasks and launch projects in separate batches"
                    );
                    anyhow::ensure!(
                        connects.iter().all(|o| o.workspace == first.workspace),
                        "Select tasks from one workspace to view in the current terminal"
                    );
                    return Ok(Outcome::Current(Box::new((*first).clone())));
                }
                self.start(w, options)?;
            }
            Command::SavePreset => {
                anyhow::ensure!(!self.marked.is_empty(), "{}", t("preset_empty"));
                self.dialog = Some(Dialog::Input(Input::Preset, String::new()))
            }
            Command::Preset(name) => {
                let items = self.config.presets.get(&name).cloned().unwrap_or_default();
                self.marked = items.iter().map(|s| s.project.clone()).collect();
                self.staged = items
                    .iter()
                    .map(|s| (config::key(&s.project), s.options()))
                    .collect();
                self.config.overview.collapsed.clear();
                self.query.clear();
                self.rebuild();
                self.message = t("parity.preset_applied").into_owned();
            }
            Command::Retry => self.retry(w)?,
            Command::Refresh => {
                self.stats.clear();
                self.dashboard.sources.clear();
                self.dashboard.contexts.clear();
                self.dashboard.session_pending = None;
                self.dashboard.session_loaded.clear();
                self.dashboard.context_order.clear();
                self.dashboard.source_pending = None;
                self.dashboard.context_pending = None;
                w.source_ticket();
                w.history_ticket();
                self.detail_pending = false;
                self.edit_serial += 1;
                w.send(Action::Reload(self.edit_serial))?;
                self.pending_edits += 1;
                self.refresh_pending = Some(self.edit_serial);
                self.detail_ticket = w.ticket();
                self.scan_ticket = w
                    .scan_generation
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
            }
            Command::Root => self.dialog = Some(Dialog::Input(Input::Root, String::new())),
            Command::Add => self.dialog = Some(Dialog::Input(Input::Add, String::new())),
            Command::Exclude => {
                if let Some(p) = path {
                    self.dialog = Some(Dialog::Input(Input::Exclude, p.display().to_string()))
                }
            }
            Command::Folder => {
                if let Some(p) = path {
                    spawn("explorer.exe", [p.as_os_str()])?
                }
            }
            Command::Editor => {
                if let Some(p) = path {
                    let editor = std::env::var_os("VISUAL")
                        .or_else(|| std::env::var_os("EDITOR"))
                        .unwrap_or_else(|| "code".into());
                    spawn(editor, [p.as_os_str()])?
                }
            }
            Command::Config => spawn(
                "notepad.exe",
                [config::home().join("config.toml").as_os_str()],
            )?,
            Command::Language(l) => {
                self.config.language = l;
                crate::i18n::set(l);
                self.edit(w, Edit::Language(l))?
            }
            Command::Theme(theme) => {
                self.config.theme = theme;
                crate::appearance::set(theme);
                self.edit(w, Edit::Theme(theme))?
            }
            Command::Quit => {
                if self.batch.busy() {
                    self.dialog = Some(Dialog::Batch)
                } else if self.pending_edits > 0 {
                    self.message = t("busy").into()
                } else {
                    return Ok(Outcome::Quit);
                }
            }
        }
        self.dirty = true;
        Ok(Outcome::Stay)
    }
    fn key(&mut self, w: &Workers, key: KeyEvent) -> Result<Outcome> {
        let original = self.dialog.clone();
        let before = self.edit_serial;
        let result = self.key_inner(w, key);
        if result.is_ok()
            && self.edit_serial > before
            && let Some(form @ (Dialog::Input(..) | Dialog::Settings { .. })) = original.clone()
        {
            self.pending_forms.insert(self.edit_serial, form);
        }
        if result.is_err() && self.dialog.is_none() {
            self.dialog = original;
        }
        result
    }
    fn key_inner(&mut self, w: &Workers, key: KeyEvent) -> Result<Outcome> {
        self.dirty = true;
        if let Some(outcome) = self.dashboard_key(w, key)? {
            return Ok(outcome);
        }
        if let Some(mut dialog) = self.dialog.take() {
            if key.code == KeyCode::Esc {
                if let Dialog::Options { editing, .. } = &mut dialog
                    && editing.take().is_some()
                {
                    self.dialog = Some(dialog);
                    return Ok(Outcome::Stay);
                }
                if let Dialog::Palette { original, .. } = dialog {
                    crate::appearance::set(original)
                }
                if matches!(dialog, Dialog::History { .. }) {
                    self.dashboard.history_ticket = w.history_ticket();
                    self.dashboard.history_pending = false;
                }
                return Ok(Outcome::Stay);
            }
            let mut keep = true;
            if let Dialog::Options {
                options, editing, ..
            } = &mut dialog
                && let Some((field, mut value)) = editing.take()
            {
                match key.code {
                    KeyCode::Enter => set_draft_value(options, field, value)?,
                    KeyCode::Tab => {
                        set_draft_value(options, field, value)?;
                        let next = (field + 1) % (2 + options.extra_sessions.len() * 3);
                        *editing = Some((next, draft_value(options, next)));
                    }
                    KeyCode::Delete
                        if key.modifiers.contains(KeyModifiers::CONTROL) && field >= 2 =>
                    {
                        options.extra_sessions.remove((field - 2) / 3);
                    }
                    _ => {
                        if key.code == KeyCode::Backspace {
                            value.pop();
                        } else if let KeyCode::Char(c) = key.code
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        {
                            append_bounded(&mut value, &c.to_string(), 32768);
                        }
                        *editing = Some((field, value));
                    }
                }
                self.dialog = Some(dialog);
                return Ok(Outcome::Stay);
            }
            match &mut dialog {
                Dialog::Attention { selected } => {
                    let pending = self.attention();
                    match key.code {
                        KeyCode::Down => {
                            *selected = (*selected + 1).min(pending.len().saturating_sub(1))
                        }
                        KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Enter => {
                            if let Some((target, _)) = pending.get(*selected)
                                && let Some(work) = self
                                    .workspaces
                                    .values()
                                    .flatten()
                                    .find(|w| w.id == target.workspace)
                            {
                                let mut options = work.options.clone();
                                options.workspace = Some(work.id.clone());
                                options.navigation = Some(Box::new(crate::tasks::Navigation {
                                    project: work.project.clone(),
                                    workspace: work.id.clone(),
                                    task: target.task.clone(),
                                    session: target.session.clone(),
                                }));
                                return Ok(Outcome::Current(Box::new(options)));
                            }
                        }
                        _ => {}
                    }
                }
                Dialog::Help => {
                    keep = false;
                }
                Dialog::Presets { selected } => {
                    let names = self.config.presets.keys().cloned().collect::<Vec<_>>();
                    *selected = (*selected).min(names.len().saturating_sub(1));
                    match key.code {
                        KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Down => {
                            *selected = (*selected + 1).min(names.len().saturating_sub(1))
                        }
                        KeyCode::Enter => {
                            if let Some(name) = names.get(*selected) {
                                self.command(w, Command::Preset(name.clone()))?;
                                keep = false;
                            }
                        }
                        KeyCode::Delete => {
                            if let Some(name) = names.get(*selected) {
                                self.edit(w, Edit::DeletePreset(name.clone()))?;
                            }
                        }
                        KeyCode::F(2) => {
                            if let Some(name) = names.get(*selected) {
                                self.dialog = Some(Dialog::Input(
                                    Input::RenamePreset(name.clone()),
                                    name.clone(),
                                ));
                                keep = false;
                            }
                        }
                        _ => {}
                    }
                }
                Dialog::Input(kind, value) => match key.code {
                    KeyCode::Enter => {
                        let edit = match kind {
                            Input::NewProject(root) => {
                                Edit::NewProject(root.clone(), value.clone())
                            }
                            Input::RenamePreset(old) => {
                                Edit::RenamePreset(old.clone(), value.clone())
                            }
                            Input::Root => Edit::Root(value.clone().into()),
                            Input::Add => Edit::Add(value.clone().into()),
                            Input::Exclude => Edit::Exclude(value.clone().into()),
                            Input::Note(path) => Edit::Note(path.clone(), value.clone()),
                            Input::Preset => Edit::Preset(value.clone(), self.specs()),
                        };
                        self.edit(w, edit)?;
                        keep = false
                    }
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && value.len() < 32768 =>
                    {
                        value.push(c)
                    }
                    _ => {}
                },
                Dialog::Settings {
                    path,
                    project,
                    field,
                    debug,
                    arguments,
                } => match key.code {
                    KeyCode::Tab | KeyCode::Down => {
                        *field = (*field + 1) % (7 + project.sessions.len() * 3)
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        *field = (*field + 6 + project.sessions.len() * 3)
                            % (7 + project.sessions.len() * 3)
                    }
                    KeyCode::Left | KeyCode::Right if *field == 6 => {
                        project.resume_latest = match project.resume_latest {
                            None => Some(true),
                            Some(true) => Some(false),
                            Some(false) => None,
                        };
                    }
                    KeyCode::Left | KeyCode::Right if *field < 2 => {
                        if *field == 0 {
                            project.agent = match project.agent {
                                None => Some(Agent::Codex),
                                Some(Agent::Codex) => Some(Agent::Claude),
                                Some(Agent::Claude) => Some(Agent::Shell),
                                _ => None,
                            }
                        } else {
                            project.layout = match project.layout {
                                None => Some(Layout::Float),
                                Some(Layout::Float) => Some(Layout::Split),
                                Some(Layout::Split) => Some(Layout::Tabs),
                                _ => None,
                            }
                        }
                    }
                    KeyCode::Char('m' | 'M') if key.modifiers.contains(KeyModifiers::ALT) => {
                        project.composition = Some(project.terminal_composition().next());
                    }
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::ALT) => {
                        anyhow::ensure!(project.sessions.len() < 16, "{}", t("input_long"));
                        project.composition = Some(crate::launch::Composition::Custom);
                        project.sessions.push(crate::config::SessionConfig {
                            name: format!("Session {}", project.sessions.len() + 1),
                            command: None,
                            directory: None,
                        });
                        *field = 7 + (project.sessions.len() - 1) * 3;
                    }
                    KeyCode::Char('d')
                        if key.modifiers.contains(KeyModifiers::CONTROL) && *field >= 7 =>
                    {
                        project.sessions.remove((*field - 7) / 3);
                        *field = (*field).min(6 + project.sessions.len() * 3);
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        project.debug_command = (!debug.trim().is_empty()).then(|| debug.clone());
                        project.agent_args = serde_json::from_str(arguments)
                            .map_err(|e| anyhow::anyhow!("{}: {e}", t("parity.arguments")))?;
                        model::validate_settings(project)?;
                        self.edit(w, Edit::Settings(path.clone(), project.clone()))?;
                        keep = false;
                    }
                    KeyCode::Backspace if *field >= 2 => {
                        let mut value = if *field == 5 {
                            arguments.clone()
                        } else {
                            settings_value(project, debug, *field)
                        };
                        value.pop();
                        if *field == 5 {
                            *arguments = value;
                        } else {
                            set_settings_value(project, debug, *field, value);
                        }
                    }
                    KeyCode::Char(c)
                        if *field >= 2
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        let mut value = if *field == 5 {
                            arguments.clone()
                        } else {
                            settings_value(project, debug, *field)
                        };
                        if value.len() + c.len_utf8() <= 32768 {
                            value.push(c);
                            if *field == 5 {
                                *arguments = value;
                            } else {
                                set_settings_value(project, debug, *field, value);
                            }
                        }
                    }
                    _ => {}
                },
                Dialog::Options {
                    options,
                    new_tab,
                    editing,
                } => match key.code {
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *editing = Some((0, draft_value(options, 0)))
                    }
                    KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *editing = Some((1, draft_value(options, 1)))
                    }
                    KeyCode::Char('e')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && !options.extra_sessions.is_empty() =>
                    {
                        *editing = Some((2, draft_value(options, 2)))
                    }
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::ALT) => {
                        anyhow::ensure!(options.extra_sessions.len() < 16, "{}", t("input_long"));
                        options.composition = crate::launch::Composition::Custom;
                        options.extra_sessions.push(crate::config::SessionConfig {
                            name: format!("Session {}", options.extra_sessions.len() + 1),
                            command: None,
                            directory: None,
                        });
                        let field = 2 + (options.extra_sessions.len() - 1) * 3;
                        *editing = Some((field, draft_value(options, field)));
                    }
                    KeyCode::Char('1') => {
                        if options.agent != Agent::Codex {
                            options.resume_id = None;
                        }
                        options.agent = Agent::Codex;
                    }
                    KeyCode::Char('2') => {
                        if options.agent != Agent::Claude {
                            options.resume_id = None;
                        }
                        options.agent = Agent::Claude;
                    }
                    KeyCode::Char('3') => {
                        options.agent = Agent::Shell;
                        options.continue_latest = false;
                        options.fresh = true;
                        options.resume = false;
                        options.resume_id = None
                    }
                    KeyCode::Char('r' | 'R') if options.agent != Agent::Shell => {
                        options.resume = !options.resume;
                        options.resume_id = None;
                        options.fresh = false;
                        options.continue_latest = false;
                    }
                    KeyCode::Char('n' | 'N') => options.new_instance = !options.new_instance,
                    KeyCode::Char('t' | 'T') => *new_tab = !*new_tab,
                    KeyCode::Char('w' | 'W') => options.new_window = !options.new_window,
                    KeyCode::Char('c' | 'C') => {
                        options.continue_latest = !options.continue_latest;
                        options.fresh = !options.continue_latest;
                        options.resume = false;
                        options.resume_id = None;
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.staged
                            .insert(config::key(&options.project), options.clone());
                        if !self.marked.contains(&options.project) {
                            self.marked.push(options.project.clone());
                        }
                        self.message = t("parity.options_staged").into_owned();
                        keep = false;
                    }
                    KeyCode::Char('m' | 'M') => {
                        options.composition = options.effective_composition().next()
                    }
                    KeyCode::Char('l' | 'L') => options.layout = next_layout(options.layout),
                    KeyCode::Enter => {
                        if *new_tab {
                            self.start(w, vec![options.clone()])?
                        } else {
                            anyhow::ensure!(
                                !self.batch.busy() && self.pending_edits == 0,
                                "{}",
                                t("busy")
                            );
                            return Ok(Outcome::Current(Box::new(options.clone())));
                        }
                        keep = false
                    }
                    _ => {}
                },
                Dialog::Palette {
                    query,
                    selected,
                    original,
                } => {
                    match key.code {
                        KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Down => *selected += 1,
                        KeyCode::Backspace => {
                            query.pop();
                            *selected = 0
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && query.len() < 256 =>
                        {
                            query.push(c);
                            *selected = 0
                        }
                        _ => {}
                    }
                    let entries = self.palette_entries(query);
                    *selected = (*selected).min(entries.len().saturating_sub(1));
                    if let Some(e) = entries.get(*selected) {
                        if let Command::Theme(theme) = e.command {
                            crate::appearance::set(theme)
                        } else {
                            crate::appearance::set(*original)
                        }
                        if key.code == KeyCode::Enter {
                            if e.enabled {
                                let command = e.command.clone();
                                crate::appearance::set(*original);
                                return self.command(w, command);
                            }
                            self.message = t("unavailable").into()
                        }
                    }
                }
                Dialog::History {
                    path,
                    agent,
                    selected,
                    scroll,
                } => {
                    let n = self.history.as_ref().map_or(0, |h| {
                        h.sessions
                            .iter()
                            .filter(|c| history::matches(c, &self.dashboard.history_query))
                            .count()
                    });
                    match key.code {
                        KeyCode::Up => {
                            *selected = selected.saturating_sub(1);
                            *scroll = 0
                        }
                        KeyCode::Down => {
                            *selected = (*selected + 1).min(n.saturating_sub(1));
                            *scroll = 0
                        }
                        KeyCode::PageUp => *scroll = scroll.saturating_sub(6),
                        KeyCode::PageDown => *scroll = scroll.saturating_add(6),
                        KeyCode::Enter | KeyCode::Char('p' | 'P' | 'n' | 'N')
                            if key.code == KeyCode::Enter
                                || key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            if key.code == KeyCode::Enter {
                                let id = self
                                    .history
                                    .as_ref()
                                    .and_then(|h| {
                                        h.sessions
                                            .iter()
                                            .filter(|c| {
                                                history::matches(c, &self.dashboard.history_query)
                                            })
                                            .nth(*selected)
                                    })
                                    .map(|c| c.id.clone())
                                    .ok_or_else(|| anyhow::anyhow!("{}", t("history_empty")))?;
                                self.stage_resume(path, *agent, id);
                            } else {
                                let mut options = self.options_for(path);
                                options.agent = *agent;
                                options.new_instance = true;
                                options.workspace = None;
                                options.recovery_id = None;
                                options.resume_id = None;
                                options.continue_latest = false;
                                options.fresh = matches!(key.code, KeyCode::Char('n' | 'N'));
                                options.resume = !options.fresh;
                                self.selected = self.rows.iter().position(|r| matches!(r, Row::Project(p) if config::key(p) == config::key(path))).unwrap_or(self.selected);
                                self.staged.insert(config::key(path), options);
                            }
                            self.message = t("parity.options_staged").into_owned();
                            keep = false;
                            w.history_ticket();
                            self.detail_pending = false;
                        }
                        KeyCode::Backspace => {
                            self.dashboard.history_query.pop();
                            *selected = 0;
                            *scroll = 0;
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
                        {
                            append_bounded(&mut self.dashboard.history_query, &c.to_string(), 256);
                            *selected = 0;
                            *scroll = 0;
                        }
                        KeyCode::Home => {
                            *selected = 0;
                            *scroll = 0;
                        }
                        KeyCode::End => {
                            *selected = n.saturating_sub(1);
                            *scroll = 0;
                        }
                        _ => {}
                    }
                }
                Dialog::Diagnostics => {
                    if key.code == KeyCode::Down {
                        self.detail_scroll = self.detail_scroll.saturating_add(1);
                    } else if key.code == KeyCode::Up {
                        self.detail_scroll = self.detail_scroll.saturating_sub(1);
                    }
                }
                Dialog::Instances { path, selected } => {
                    let work = self
                        .workspaces
                        .get(&config::key(path))
                        .cloned()
                        .unwrap_or_default();
                    match key.code {
                        KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Down => {
                            *selected = (*selected + 1).min(work.len().saturating_sub(1))
                        }
                        KeyCode::Enter => {
                            if let Some(target) = work.get(*selected) {
                                let mut o = self.options_for(path);
                                o.workspace = Some(target.id.clone());
                                self.start(w, vec![o])?;
                                keep = false;
                            }
                        }
                        KeyCode::Char('n' | 'N') => {
                            let mut o = self.options_for(path);
                            o.new_instance = true;
                            self.start(w, vec![o])?;
                            keep = false;
                        }
                        _ => {}
                    }
                }
                Dialog::Queue { selected } => {
                    *selected = (*selected).min(self.marked.len().saturating_sub(1));
                    match key.code {
                        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.start(w, self.queue_options())?;
                            keep = false;
                        }
                        KeyCode::Up
                            if key.modifiers.contains(KeyModifiers::ALT) && *selected > 0 =>
                        {
                            self.dashboard.undo = Some(self.marked.clone());
                            self.marked.0.swap(*selected, *selected - 1);
                            *selected -= 1;
                        }
                        KeyCode::Down
                            if key.modifiers.contains(KeyModifiers::ALT)
                                && *selected + 1 < self.marked.len() =>
                        {
                            self.dashboard.undo = Some(self.marked.clone());
                            self.marked.0.swap(*selected, *selected + 1);
                            *selected += 1;
                        }
                        KeyCode::Up => *selected = selected.saturating_sub(1),
                        KeyCode::Down => {
                            *selected = (*selected + 1).min(self.marked.len().saturating_sub(1))
                        }
                        KeyCode::Delete if !self.marked.is_empty() => {
                            self.dashboard.undo = Some(self.marked.clone());
                            self.marked.0.remove(*selected);
                        }
                        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(previous) = self.dashboard.undo.take() {
                                self.marked = previous;
                            }
                        }
                        _ => {}
                    }
                }
                Dialog::Batch => {
                    match key.code {
                        KeyCode::Up => self.batch_scroll = self.batch_scroll.saturating_sub(1),
                        KeyCode::Down => self.batch_scroll = self.batch_scroll.saturating_add(1),
                        KeyCode::PageUp => self.batch_scroll = self.batch_scroll.saturating_sub(10),
                        KeyCode::PageDown => {
                            self.batch_scroll = self.batch_scroll.saturating_add(10)
                        }
                        _ => {}
                    }
                    if matches!(key.code, KeyCode::Char('r' | 'R')) {
                        self.retry(w)?;
                        keep = false
                    }
                }
            }
            if keep {
                self.dialog = Some(dialog)
            }
            return Ok(Outcome::Stay);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let command = match key.code {
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.rebuild();
                None
            }
            KeyCode::Esc if self.dashboard.overlay.is_some() => {
                self.dashboard.overlay = None;
                w.ticket();
                self.detail_pending = false;
                None
            }
            KeyCode::Esc => Some(Command::Quit),
            KeyCode::Enter if ctrl => Some(Command::Launch),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => Some(Command::Options),
            KeyCode::Enter => {
                if matches!(self.rows.get(self.selected), Some(Row::Group(_))) {
                    Some(Command::Group)
                } else {
                    Some(Command::Open)
                }
            }
            KeyCode::Left | KeyCode::Right => Some(Command::Group),
            KeyCode::Char('!') => Some(Command::Attention),
            KeyCode::F(1) => Some(Command::Help),
            KeyCode::Char('a' | 'A') if ctrl && key.modifiers.contains(KeyModifiers::SHIFT) => {
                Some(Command::SelectAll)
            }
            KeyCode::Char('n') if ctrl => Some(Command::NewProject),
            KeyCode::Char('b') if ctrl => Some(Command::Presets),
            KeyCode::F(2) => Some(Command::Favorite),
            KeyCode::F(3) => Some(Command::Note),
            KeyCode::F(4) => Some(Command::Settings),
            KeyCode::F(5) => Some(Command::Refresh),
            KeyCode::F(6) => Some(Command::Filter),
            KeyCode::F(7) => Some(Command::Sort),
            KeyCode::F(8) => Some(Command::Stats),
            KeyCode::F(9) => Some(Command::History),
            KeyCode::F(10) => Some(Command::Details),
            KeyCode::Char('r') if ctrl => Some(Command::Root),
            KeyCode::Char('a') if ctrl => Some(Command::Add),
            KeyCode::Char('e') if ctrl => Some(Command::Exclude),
            KeyCode::Char('s') if ctrl => Some(Command::SavePreset),
            KeyCode::Char('l') if ctrl => Some(Command::Language(self.config.language.next())),
            KeyCode::Char('t') if ctrl => Some(Command::Theme(self.config.theme.next())),
            KeyCode::Char('c') if ctrl => Some(Command::CopyPath),
            KeyCode::Char('p') if ctrl => {
                self.dialog = Some(Dialog::Palette {
                    query: String::new(),
                    selected: 0,
                    original: crate::appearance::theme(),
                });
                None
            }
            KeyCode::Char(' ') if !ctrl => {
                self.toggle_selected_queue();
                None
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1));
                None
            }
            KeyCode::PageUp if key.modifiers.contains(KeyModifiers::ALT) => {
                self.detail_scroll = self.detail_scroll.saturating_sub(8);
                None
            }
            KeyCode::PageDown if key.modifiers.contains(KeyModifiers::ALT) => {
                self.detail_scroll = self.detail_scroll.saturating_add(8);
                None
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                None
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(self.rows.len().saturating_sub(1));
                None
            }
            KeyCode::Home => {
                self.selected = 0;
                None
            }
            KeyCode::End => {
                self.selected = self.rows.len().saturating_sub(1);
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.rebuild();
                None
            }
            KeyCode::Char(c) if !ctrl => {
                if self.query.len() < 256 {
                    self.query.push(c);
                    self.rebuild();
                }
                None
            }
            _ => None,
        };
        if let Some(command) = command {
            self.command(w, command)
        } else {
            Ok(Outcome::Stay)
        }
    }
    fn code_copy_at(&self, point: ratatui::layout::Position) -> Option<std::sync::Arc<str>> {
        if self.dashboard.viewer.is_some()
            || self
                .dialog
                .as_ref()
                .is_some_and(|dialog| !matches!(dialog, Dialog::History { .. }))
        {
            return None;
        }
        self.dashboard
            .code_hits
            .iter()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, text)| text.clone())
    }
    fn mouse(&mut self, w: &Workers, mouse: crossterm::event::MouseEvent) -> Result<Outcome> {
        self.dirty = true;
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(text) = self.code_copy_at((mouse.column, mouse.row).into())
        {
            crate::files::copy(text.to_string())?;
            self.dashboard.copied_code = Some(Instant::now());
            self.message = t("ui.copied_code_block").into_owned();
            return Ok(Outcome::Stay);
        }
        if let Some(outcome) = self.dashboard_mouse(w, mouse)? {
            return Ok(outcome);
        }
        if self.dialog.is_none()
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some((_, filter)) = self
                .filter_hits
                .iter()
                .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
        {
            return self.command(w, Command::SetFilter(*filter));
        }
        let point = (mouse.column, mouse.row).into();
        let up = matches!(mouse.kind, MouseEventKind::ScrollUp);
        let down = matches!(mouse.kind, MouseEventKind::ScrollDown);
        if self.dialog.is_some() {
            if (up || down) && self.dialog_preview.contains(point) {
                match self.dialog.as_mut() {
                    Some(Dialog::History { scroll, .. }) => {
                        *scroll = if up {
                            scroll.saturating_sub(3)
                        } else {
                            scroll.saturating_add(3)
                        }
                    }
                    Some(Dialog::Batch) => {
                        self.batch_scroll = if up {
                            self.batch_scroll.saturating_sub(3)
                        } else {
                            self.batch_scroll.saturating_add(3)
                        }
                    }
                    _ => {}
                }
                return Ok(Outcome::Stay);
            }
            if let Some((area, offset)) = self.dialog_list
                && area.contains(point)
            {
                let entry = offset + usize::from(mouse.row - area.y);
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    match self.dialog.as_mut() {
                        Some(Dialog::Palette { selected, .. })
                        | Some(Dialog::Instances { selected, .. })
                        | Some(Dialog::Presets { selected }) => *selected = entry,
                        Some(Dialog::History {
                            selected, scroll, ..
                        }) => {
                            *selected = entry;
                            *scroll = 0
                        }
                        _ => {}
                    }
                    if matches!(
                        self.dialog,
                        Some(Dialog::Palette { .. })
                            | Some(Dialog::Presets { .. })
                            | Some(Dialog::Instances { .. })
                    ) {
                        return self.key(w, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                } else if up || down {
                    return self.key(
                        w,
                        KeyEvent::new(
                            if up { KeyCode::Up } else { KeyCode::Down },
                            KeyModifiers::NONE,
                        ),
                    );
                }
            }
            return Ok(Outcome::Stay);
        }
        Ok(Outcome::Stay)
    }
    fn paste(&mut self, text: &str) {
        let clean = text.chars().filter(|c| !c.is_control()).collect::<String>();
        if let Some(Dialog::Settings {
            field,
            project,
            debug,
            arguments,
            ..
        }) = &mut self.dialog
        {
            if *field >= 2 {
                let mut value = if *field == 5 {
                    arguments.clone()
                } else {
                    settings_value(project, debug, *field)
                };
                append_bounded(&mut value, &clean, 32768);
                if *field == 5 {
                    *arguments = value;
                } else {
                    set_settings_value(project, debug, *field, value);
                }
            }
        } else {
            let value = match self.dialog.as_mut() {
                Some(Dialog::Input(_, s)) => Some(s),
                Some(Dialog::Palette { query, .. }) => Some(query),
                Some(Dialog::History {
                    selected, scroll, ..
                }) => {
                    *selected = 0;
                    *scroll = 0;
                    Some(&mut self.dashboard.history_query)
                }
                Some(Dialog::Options {
                    editing: Some((_, value)),
                    ..
                }) => Some(value),
                None => Some(&mut self.query),
                _ => None,
            };
            if let Some(value) = value {
                append_bounded(value, &clean, 32768);
            }
        }
        self.rebuild();
    }
}
fn workspace_signature(
    work: &BTreeMap<String, Vec<crate::protocol::WorkspaceInfo>>,
) -> Vec<String> {
    work.values()
        .flatten()
        .flat_map(|w| {
            w.sessions.iter().map(|s| {
                format!(
                    "{}:{}:{:?}:{:?}:{:?}",
                    w.id, s.id, s.shell_pid, s.status, s.command
                )
            })
        })
        .collect()
}
fn append_bounded(value: &mut String, text: &str, limit: usize) {
    for c in text.chars() {
        if value.len() + c.len_utf8() > limit {
            break;
        }
        value.push(c)
    }
}
fn settings_value(project: &Project, debug: &str, field: usize) -> String {
    match field {
        0 => project
            .agent
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| t("inherit").into_owned()),
        1 => project
            .layout
            .map(|l| format!("{l:?}"))
            .unwrap_or_else(|| t("inherit").into_owned()),
        2 => debug.into(),
        3 => project.title.clone().unwrap_or_default(),
        4 => project.color.clone().unwrap_or_default(),
        5 => serde_json::to_string(&project.agent_args).unwrap_or_default(),
        6 => t(match project.resume_latest {
            None => "inherit",
            Some(true) => "yes",
            Some(false) => "no",
        })
        .into_owned(),
        _ => {
            let session = &project.sessions[(field - 7) / 3];
            match (field - 7) % 3 {
                0 => session.name.clone(),
                1 => session.command.clone().unwrap_or_default(),
                _ => session
                    .directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            }
        }
    }
}
fn draft_value(o: &Options, field: usize) -> String {
    match field {
        0 => o.debug_command.clone().unwrap_or_default(),
        1 => serde_json::to_string(&o.agent_args).unwrap_or_default(),
        _ => o
            .extra_sessions
            .get((field - 2) / 3)
            .map(|s| match (field - 2) % 3 {
                0 => s.name.clone(),
                1 => s.command.clone().unwrap_or_default(),
                _ => s
                    .directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            })
            .unwrap_or_default(),
    }
}
fn set_draft_value(o: &mut Options, field: usize, value: String) -> Result<()> {
    match field {
        0 => o.debug_command = (!value.is_empty()).then_some(value),
        1 => o.agent_args = serde_json::from_str(&value)?,
        _ => {
            let s = o
                .extra_sessions
                .get_mut((field - 2) / 3)
                .ok_or_else(|| anyhow::anyhow!("{}", t("no_matches")))?;
            match (field - 2) % 3 {
                0 => s.name = value,
                1 => s.command = (!value.is_empty()).then_some(value),
                _ => s.directory = (!value.is_empty()).then(|| PathBuf::from(value)),
            }
        }
    }
    model::validate_settings(&Project {
        debug_command: o.debug_command.clone(),
        agent_args: o.agent_args.clone(),
        sessions: o.extra_sessions.clone(),
        ..Default::default()
    })
}
fn set_settings_value(project: &mut Project, debug: &mut String, field: usize, value: String) {
    if field == 2 {
        *debug = value
    } else if field == 3 {
        project.title = (!value.is_empty()).then_some(value);
    } else if field == 4 {
        project.color = (!value.is_empty()).then_some(value);
    } else if field >= 7 {
        let session = &mut project.sessions[(field - 7) / 3];
        match (field - 7) % 3 {
            0 => session.name = value,
            1 => session.command = (!value.is_empty()).then_some(value),
            _ => session.directory = (!value.is_empty()).then(|| PathBuf::from(value)),
        }
    }
}
fn next_layout(layout: Layout) -> Layout {
    match layout {
        Layout::Float => Layout::Split,
        Layout::Split => Layout::Tabs,
        Layout::Tabs => Layout::Float,
    }
}
fn spawn(
    program: impl AsRef<std::ffi::OsStr>,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> Result<()> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()?;
    Ok(())
}
pub fn run(terminal: &mut DefaultTerminal) -> Result<Option<Options>> {
    run_from(terminal, None)
}
pub fn run_from(
    terminal: &mut DefaultTerminal,
    origin: Option<Options>,
) -> Result<Option<Options>> {
    let mut app = Overview::new(Config::load()?);
    if let Some(mut origin) = origin {
        app.return_target = origin.navigation.as_deref().cloned();
        origin.navigation = None;
        if let Some(target) = &app.return_target {
            app.dashboard
                .expanded_projects
                .insert(config::key(&target.project));
        }
        app.focus_after_scan = Some(origin.project.clone());
        origin.workspace = None;
        app.staged.insert(config::key(&origin.project), origin);
    }
    let w = Workers::new();
    app.discover(&w)?;
    let mut events = Events::new()?;
    let mut last_input = None::<Instant>;
    let mut notice = (String::new(), Instant::now());
    loop {
        if notice.0 != app.message {
            notice = (app.message.clone(), Instant::now());
        } else if !app.message.is_empty() && notice.1.elapsed() > Duration::from_secs(6) {
            app.message.clear();
            app.dirty = true;
        }
        if !app.dashboard.hover_displayed
            && app
                .dashboard
                .hover
                .is_some_and(|(_, since)| since.elapsed() >= Duration::from_millis(400))
        {
            app.dirty = true;
        }

        if let Some(viewer) = app.dashboard.viewer.as_mut()
            && viewer.tick(true)
        {
            app.dirty = true;
        }
        if let Some(viewer) = app.dashboard.viewer.as_mut()
            && viewer.document.as_ref().is_some_and(|d| !d.loading)
            && let Some(line) = app.dashboard.viewer_line.take()
        {
            if let Err(error) = viewer.reveal_line(line) {
                viewer.say(error.to_string(), false);
            }
            app.dirty = true;
        }
        if let Err(e) = app.poll(&w).and_then(|_| app.request_stats(&w)) {
            app.message = format!("{e:#}");
            app.dirty = true
        }
        if app.dirty {
            let _perf = crate::perf::span(crate::perf::Metric::OverviewFrame);
            terminal.draw(|f| view::draw(f, &mut app))?;
            app.dirty = false;
            if let Some(at) = last_input.take() {
                crate::perf::record(crate::perf::Metric::OverviewInputToFrame, at.elapsed());
            }
        }
        if let Some(input) = events.read(Duration::from_millis(33))? {
            last_input = Some(Instant::now());
            let outcome = match input.event {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(&w, key),
                Event::Paste(text) => {
                    app.paste(&text);
                    Ok(Outcome::Stay)
                }
                Event::Resize(..) | Event::FocusLost => {
                    app.dashboard.cancel(&mut app.config.overview);
                    app.last_click = None;
                    app.dirty = true;
                    Ok(Outcome::Stay)
                }
                Event::Mouse(mouse) => app.mouse(&w, mouse),
                _ => Ok(Outcome::Stay),
            };
            match outcome {
                Ok(Outcome::Quit) => return Ok(None),
                Ok(Outcome::Current(options)) => return Ok(Some(*options)),
                Err(e) => {
                    app.message = format!("{e:#}");
                    app.dirty = true
                }
                _ => {}
            }
        }
    }
}
pub fn launch_tab(options: &Options) -> Result<()> {
    launch_workspace(options).map(|_| ())
}
fn launch_workspace(options: &Options) -> Result<String> {
    let info = open_workspace(options)?;
    present_workspace(&info.id, options)?;
    Ok(info.id)
}
fn open_workspace(options: &Options) -> Result<crate::protocol::WorkspaceInfo> {
    crate::host::ensure()?;
    match crate::protocol::rpc(crate::protocol::Request::Open {
        options: Box::new(options.clone()),
    })? {
        crate::protocol::Response::Workspace(info) => Ok(*info),
        crate::protocol::Response::Error(e) => anyhow::bail!("{e}"),
        _ => anyhow::bail!("No workspace returned"),
    }
}
pub(crate) fn present_workspace(id: &str, options: &Options) -> Result<()> {
    if options.new_window || crate::windows::focus_workspace(id).is_err() {
        crate::windows::spawn_workspace_view(id, "main", options)?
    }
    Ok(())
}

#[cfg(test)]
mod tests;

impl Overview {
    fn attention(&self) -> Vec<(crate::notifications::Target, crate::activity::State)> {
        let mut rows = self
            .workspaces
            .values()
            .flatten()
            .flat_map(|w| {
                w.sessions.iter().flat_map(|s| {
                    crate::notifications::pending(
                        &crate::notifications::Target {
                            workspace: w.id.clone(),
                            task: s.task.task_id.clone(),
                            session: s.id.clone(),
                            run: s.task.run_id.clone(),
                            result: s.activity.result,
                        },
                        &s.activity,
                    )
                })
            })
            .collect::<Vec<_>>();
        crate::notifications::sort_pending(&mut rows);
        rows
    }
}
