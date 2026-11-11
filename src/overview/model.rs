//! Pure overview state: stable identities, ordered selections and reproducible launches.
use crate::{
    config::{self, Config, Project, SessionConfig},
    i18n::t,
    launch::{Agent, Layout, Options},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    #[default]
    All,
    Running,
    Dirty,
    Attention,
    Pending,
    Codex,
    Claude,
    Shell,
}
impl Filter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Running,
            Self::Running => Self::Attention,
            Self::Attention => Self::Dirty,
            Self::Dirty => Self::Pending,
            Self::Pending => Self::Codex,
            Self::Codex => Self::Claude,
            Self::Claude => Self::Shell,
            Self::Shell => Self::All,
        }
    }
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::All => t("all"),
            Self::Running => t("running"),
            Self::Dirty => t("dirty"),
            Self::Attention => t("overview.attention"),
            Self::Pending => t("parity.pending"),
            Self::Codex => "Codex".into(),
            Self::Claude => "Claude".into(),
            Self::Shell => "Shell".into(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    #[default]
    Recent,
    Name,
    Size,
    Activity,
}
impl Sort {
    pub fn next(self) -> Self {
        match self {
            Self::Recent => Self::Name,
            Self::Name => Self::Size,
            Self::Size => Self::Activity,
            Self::Activity => Self::Recent,
        }
    }
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        t(match self {
            Self::Recent => "recent",
            Self::Name => "name",
            Self::Size => "size",
            Self::Activity => "activity",
        })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub filter: Filter,
    pub sort: Sort,
    pub collapsed: BTreeSet<u8>,
    pub detail: Option<bool>,
    pub statistics: Option<bool>,
    /// Basis points, independent of workspace panel sizes.
    pub list_ratio: Option<u16>,
    pub detail_ratio: Option<u16>,
    #[serde(skip)]
    pub pending: BTreeSet<String>,
    #[serde(skip)]
    pub attention: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    pub project: PathBuf,
    pub agent: Agent,
    pub layout: Layout,
    #[serde(default)]
    pub composition: crate::launch::Composition,
    pub debug_command: Option<String>,
    pub sessions: Vec<SessionConfig>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub agent_args: Vec<String>,
    #[serde(default)]
    pub resume_latest: Option<bool>,
    #[serde(default)]
    pub launch: Option<Options>,
}
impl LaunchSpec {
    pub fn from_options(o: &Options) -> Self {
        Self {
            project: o.project.clone(),
            agent: o.agent,
            layout: o.layout,
            composition: o.composition,
            debug_command: o.debug_command.clone(),
            sessions: o.extra_sessions.clone(),
            title: o.title.clone(),
            color: o.color.clone(),
            agent_args: o.agent_args.clone(),
            resume_latest: Some(o.continue_latest),
            launch: Some(crate::recovery::clean(o.clone())),
        }
    }
    pub fn from_config(path: &Path, c: &Config) -> Self {
        let p = c
            .projects
            .get(&config::key(path))
            .cloned()
            .unwrap_or_default();
        Self::from_project(path, c.default_agent, p)
    }
    pub fn from_project(path: &Path, default_agent: Option<Agent>, p: Project) -> Self {
        Self {
            project: path.into(),
            agent: p.agent.or(default_agent).unwrap_or(Agent::Codex),
            layout: p.layout.unwrap_or_default(),
            composition: p.terminal_composition(),
            debug_command: p.debug_command,
            sessions: p.sessions,
            title: p.title,
            color: p.color,
            agent_args: p.agent_args,
            resume_latest: p.resume_latest,
            launch: None,
        }
    }
    pub fn options(&self) -> Options {
        if let Some(o) = &self.launch {
            return crate::recovery::clean(o.clone());
        }
        static TEMPLATE: std::sync::OnceLock<Options> = std::sync::OnceLock::new();
        let mut o = TEMPLATE
            .get_or_init(|| Options::parse_from(["devtabs", "--agent", "shell"]))
            .clone();
        o.project = self.project.clone();
        o.agent = self.agent;
        o.layout = self.layout;
        o.composition = self.composition;
        o.debug_command = self.debug_command.clone();
        o.extra_sessions = self.sessions.clone();
        o.title = self.title.clone();
        o.color = self.color.clone();
        o.agent_args = self.agent_args.clone();
        o.continue_latest = self.resume_latest.unwrap_or(true) && self.agent != Agent::Shell;
        o
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitSummary {
    pub branch: String,
    pub upstream: String,
    pub ahead: u32,
    pub behind: u32,
    pub changed: usize,
    pub conflicts: usize,
    pub files: Vec<String>,
    pub statuses: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchState {
    Queued,
    Opening,
    Opened(String),
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct LaunchItem {
    pub options: Options,
    pub state: LaunchState,
}
#[derive(Default)]
pub struct Batch {
    pub items: Vec<LaunchItem>,
    pub generation: u64,
}
impl Batch {
    pub fn busy(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.state, LaunchState::Queued | LaunchState::Opening))
    }
    pub fn start(&mut self, options: Vec<Options>) -> anyhow::Result<Vec<(usize, Options)>> {
        anyhow::ensure!(!self.busy(), "{}", t("launch_active"));
        self.generation += 1;
        self.items = options
            .into_iter()
            .map(|options| LaunchItem {
                options,
                state: LaunchState::Queued,
            })
            .collect();
        Ok(self
            .items
            .iter()
            .enumerate()
            .map(|(i, x)| (i, x.options.clone()))
            .collect())
    }
    pub fn retry(&mut self) -> Vec<(usize, Options)> {
        if self.busy() {
            return vec![];
        }
        self.generation += 1;
        self.items
            .iter_mut()
            .enumerate()
            .filter_map(|(i, x)| {
                if matches!(x.state, LaunchState::Failed(_)) {
                    x.state = LaunchState::Queued;
                    Some((i, x.options.clone()))
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn update(&mut self, generation: u64, index: usize, state: LaunchState) {
        if generation == self.generation
            && let Some(item) = self.items.get_mut(index)
        {
            item.state = state
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Group(u8),
    Project(PathBuf),
    Session(PathBuf, Agent, String),
    Task(PathBuf, String, String),
}
impl Row {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Project(p) | Self::Session(p, ..) | Self::Task(p, ..) => Some(p),
            _ => None,
        }
    }
}
pub fn group(path: &Path, c: &Config, now: u64) -> u8 {
    let p = c.projects.get(&config::key(path));
    if c.overview.pending.contains(&config::key(path)) {
        4
    } else if p.is_some_and(|p| p.favorite) {
        0
    } else {
        let age = p
            .filter(|p| p.last_opened > 0)
            .map(|p| now.saturating_sub(p.last_opened));
        match age {
            Some(a) if a < 7 * 86400 => 1,
            Some(a) if a < 30 * 86400 => 2,
            Some(_) => 3,
            None => 5,
        }
    }
}
pub fn project_group(
    path: &Path,
    c: &Config,
    now: u64,
    active: &BTreeSet<String>,
    git: &BTreeMap<String, Result<GitSummary, String>>,
) -> u8 {
    let key = config::key(path);
    if c.overview.attention.contains(&key)
        || git
            .get(&key)
            .is_some_and(|g| g.as_ref().is_ok_and(|g| g.conflicts > 0))
    {
        6
    } else if active.contains(&key) {
        7
    } else {
        group(path, c, now)
    }
}
fn group_rank(g: u8) -> u8 {
    match g {
        6 => 0,
        7 => 1,
        4 => 2,
        0 => 3,
        1 => 4,
        2 => 5,
        3 => 6,
        _ => 7,
    }
}
pub fn group_label(group: u8) -> std::borrow::Cow<'static, str> {
    t(match group {
        0 => "favorites",
        6 => "overview.attention",
        7 => "running",
        4 => "parity.pending",
        1 => "week",
        2 => "month",
        5 => "overview.other_projects",
        _ => "older",
    })
}
pub fn rows(
    paths: &[PathBuf],
    c: &Config,
    query: &str,
    active: &BTreeSet<String>,
    git: &BTreeMap<String, Result<GitSummary, String>>,
    sizes: &BTreeMap<String, u64>,
    activity: &BTreeMap<String, u64>,
) -> Vec<Row> {
    let _perf = crate::perf::span(crate::perf::Metric::OverviewFilter);
    let query = query.to_lowercase();
    let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
    let mut needle_buffer = vec![];
    let needle = nucleo_matcher::Utf32Str::new(&query, &mut needle_buffer);
    let mut paths = paths
        .iter()
        .filter(|p| {
            let key = config::key(p);
            let s = c.projects.get(&key);
            let agent = s
                .and_then(|s| s.agent)
                .or(c.default_agent)
                .unwrap_or(Agent::Codex);
            let label = format!(
                "{} {} {}",
                p.display(),
                s.map_or("", |p| p.note.as_str()),
                s.and_then(|p| p.title.as_deref()).unwrap_or("")
            )
            .to_lowercase();
            let mut buffer = vec![];
            (query.is_empty()
                || matcher
                    .fuzzy_match(nucleo_matcher::Utf32Str::new(&label, &mut buffer), needle)
                    .is_some())
                && match c.overview.filter {
                    Filter::All => true,
                    Filter::Attention => project_group(p, c, config::now(), active, git) == 6,
                    Filter::Running => active.contains(&key),
                    Filter::Pending => c.overview.pending.contains(&key),
                    Filter::Dirty => git
                        .get(&key)
                        .is_some_and(|s| s.as_ref().is_ok_and(|s| s.changed > 0)),
                    Filter::Codex => agent == Agent::Codex,
                    Filter::Claude => agent == Agent::Claude,
                    Filter::Shell => agent == Agent::Shell,
                }
        })
        .cloned()
        .collect::<Vec<_>>();
    let now = config::now();
    paths.sort_by(|a, b| {
        let ka = config::key(a);
        let kb = config::key(b);
        let primary = match c.overview.sort {
            Sort::Recent => c
                .projects
                .get(&kb)
                .map_or(0, |p| p.last_opened)
                .cmp(&c.projects.get(&ka).map_or(0, |p| p.last_opened)),
            Sort::Name => a
                .file_name()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(&b.file_name().unwrap_or_default().to_ascii_lowercase()),
            Sort::Size => sizes.get(&kb).cmp(&sizes.get(&ka)),
            Sort::Activity => activity.get(&kb).cmp(&activity.get(&ka)),
        };
        group_rank(project_group(a, c, now, active, git))
            .cmp(&group_rank(project_group(b, c, now, active, git)))
            .then(primary)
            .then(ka.cmp(&kb))
    });
    let mut out = vec![];
    let mut previous = None;
    for p in paths {
        let g = project_group(&p, c, now, active, git);
        if previous != Some(g) {
            out.push(Row::Group(g));
            previous = Some(g)
        }
        if !c.overview.collapsed.contains(&g) || !query.is_empty() {
            out.push(Row::Project(p))
        }
    }
    out
}
pub fn toggle_selection(selected: &mut Vec<PathBuf>, path: PathBuf) {
    if let Some(i) = selected
        .iter()
        .position(|p| config::key(p) == config::key(&path))
    {
        selected.remove(i);
    } else {
        selected.push(path)
    }
}
/// One ordered queue. Ordinary entries track the live draft; recovery entries
/// carry a frozen journal identity, so two instances at one path stay distinct.
#[derive(Clone, Debug)]
pub struct QueueEntry {
    pub path: PathBuf,
    pub restore: Option<Options>,
    pub connect: Option<Options>,
}
#[derive(Clone, Debug, Default)]
pub struct Queue(pub Vec<QueueEntry>);
impl Queue {
    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.0.iter().map(|e| &e.path)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, path: &Path) -> bool {
        self.0.iter().any(|e| {
            e.restore.is_none() && e.connect.is_none() && config::key(&e.path) == config::key(path)
        })
    }
    pub fn push(&mut self, path: PathBuf) {
        if !self.contains(&path) {
            self.0.push(QueueEntry {
                path,
                restore: None,
                connect: None,
            });
        }
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&PathBuf) -> bool) {
        self.0
            .retain(|e| e.restore.is_some() || e.connect.is_some() || keep(&e.path));
    }
    pub fn toggle(&mut self, path: PathBuf) {
        if let Some(i) = self.0.iter().position(|e| {
            e.restore.is_none() && e.connect.is_none() && config::key(&e.path) == config::key(&path)
        }) {
            self.0.remove(i);
        } else {
            self.push(path);
        }
    }
    pub fn restore(&mut self, options: Options) {
        if !self.0.iter().any(|e| {
            e.restore
                .as_ref()
                .is_some_and(|o| o.recovery_id == options.recovery_id)
        }) {
            self.0.push(QueueEntry {
                path: options.project.clone(),
                restore: Some(options),
                connect: None,
            });
        }
    }
    pub fn toggle_task(&mut self, options: Options) {
        if let Some(i) = self.0.iter().position(|e| {
            e.connect
                .as_ref()
                .is_some_and(|o| o.navigation == options.navigation)
        }) {
            self.0.remove(i);
        } else {
            self.0.push(QueueEntry {
                path: options.project.clone(),
                restore: None,
                connect: Some(options),
            });
        }
    }
}
impl FromIterator<PathBuf> for Queue {
    fn from_iter<T: IntoIterator<Item = PathBuf>>(items: T) -> Self {
        let mut q = Self::default();
        for p in items {
            q.push(p)
        }
        q
    }
}
impl From<Vec<PathBuf>> for Queue {
    fn from(items: Vec<PathBuf>) -> Self {
        items.into_iter().collect()
    }
}
#[cfg(test)]
impl PartialEq<Vec<PathBuf>> for Queue {
    fn eq(&self, paths: &Vec<PathBuf>) -> bool {
        self.iter().eq(paths.iter())
    }
}
pub fn validate_settings(p: &Project) -> anyhow::Result<()> {
    let mut o = Options::parse_from(["devtabs", "--agent", "shell"]);
    o.title = p.title.clone();
    o.color = p.color.clone();
    o.agent_args = p.agent_args.clone();
    crate::project_profile::validate_launch(&o)?;
    anyhow::ensure!(
        p.note.chars().count() <= 2048 && p.debug_command.as_ref().is_none_or(|s| s.len() <= 32768),
        "{}",
        t("input_long")
    );
    anyhow::ensure!(p.sessions.len() <= 16, "{}", t("input_long"));
    let mut names = BTreeSet::new();
    for s in &p.sessions {
        anyhow::ensure!(
            !s.name.trim().is_empty()
                && s.name.len() <= 128
                && !s.name.chars().any(char::is_control)
                && names.insert(s.name.to_lowercase()),
            "{}",
            t("invalid_name")
        );
        anyhow::ensure!(
            s.command.as_ref().is_none_or(|s| s.len() <= 32768),
            "{}",
            t("input_long")
        )
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_group_and_filter_without_losing_selection_order() {
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");
        let mut c = Config::default();
        c.projects.insert(
            config::key(&a),
            Project {
                favorite: true,
                ..Default::default()
            },
        );
        c.projects.insert(
            config::key(&b),
            Project {
                agent: Some(Agent::Claude),
                ..Default::default()
            },
        );
        let rows = rows(
            &[a.clone(), b.clone()],
            &c,
            "",
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert!(matches!(rows.first(), Some(Row::Group(0))));
        let mut marked = Vec::new();
        toggle_selection(&mut marked, b.clone());
        toggle_selection(&mut marked, a.clone());
        assert_eq!(marked, vec![b, a]);
    }
    #[test]
    fn batch_retries_only_failures() {
        let mut batch = Batch::default();
        let o = Options::parse_from(["devtabs", "--agent", "shell"]);
        batch.start(vec![o.clone(), o]).unwrap();
        batch.update(batch.generation, 0, LaunchState::Opened("a".into()));
        batch.update(batch.generation, 1, LaunchState::Failed("x".into()));
        assert_eq!(batch.retry().len(), 1);
    }
}
