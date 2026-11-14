use crate::{
    config::SessionConfig,
    launch::{Agent, Composition, Layout, Options},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub schema: u32,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub overview: Overview,
    #[serde(default)]
    pub discovery: Discovery,
    #[serde(default)]
    pub launch: LaunchOverrides,
    #[serde(default)]
    pub terminal: Terminal,
    #[serde(default)]
    pub sidebar: Sidebar,
    #[serde(default)]
    pub preview: Preview,
    #[serde(default)]
    pub notifications: Notifications,
    #[serde(default)]
    pub project: Vec<Project>,
    #[serde(default)]
    pub preset: Vec<Preset>,
    #[serde(default)]
    pub shortcuts: Shortcuts,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            general: General::default(),
            appearance: Appearance::default(),
            overview: Overview::default(),
            discovery: Discovery::default(),
            launch: LaunchOverrides::default(),
            terminal: Terminal::default(),
            sidebar: Sidebar::default(),
            preview: Preview::default(),
            notifications: Notifications::default(),
            project: vec![],
            preset: vec![],
            shortcuts: Shortcuts::default(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub language: Option<crate::i18n::Language>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Option<crate::appearance::Theme>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Overview {
    pub sort: Option<crate::overview::model::Sort>,
    pub filter: Option<crate::overview::model::Filter>,
    pub detail: Option<bool>,
    pub statistics: Option<bool>,
    pub list_ratio: Option<u16>,
    pub detail_ratio: Option<u16>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Discovery {
    pub roots: Vec<PathBuf>,
    pub added: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchOverrides {
    pub agent: Option<Agent>,
    pub layout: Option<Layout>,
    pub composition: Option<Composition>,
    pub resume_latest: Option<bool>,
    pub new_window: Option<bool>,
    pub shell: Option<PathBuf>,
    pub agent_args: Option<Vec<String>>,
    pub debug_command: Option<String>,
    pub sessions: Option<Vec<SessionConfig>>,
    pub title: Option<String>,
    pub color: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    pub history_lines: Option<u16>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    pub width: Option<u16>,
    pub agent_ratio: Option<u16>,
    pub follow: Option<bool>,
    pub compact: Option<bool>,
    pub show_hidden: Option<bool>,
    pub show_ignored: Option<bool>,
}
impl Sidebar {
    pub fn overlay(&mut self, next: &Self) {
        macro_rules! overlay { ($($f:ident),*) => { $(if next.$f.is_some() { self.$f=next.$f; })* } }
        overlay!(
            width,
            agent_ratio,
            follow,
            compact,
            show_hidden,
            show_ignored
        );
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownMode {
    #[default]
    Rendered,
    Source,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageProtocol {
    #[default]
    Auto,
    Sixel,
    Halfblock,
}
impl From<ImageProtocol> for crate::preview::image::ProtocolChoice {
    fn from(value: ImageProtocol) -> Self {
        match value {
            ImageProtocol::Auto => Self::Auto,
            ImageProtocol::Sixel => Self::Sixel,
            ImageProtocol::Halfblock => Self::Halfblock,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    pub wrap: Option<bool>,
    pub markdown_mode: Option<MarkdownMode>,
    pub image_protocol: Option<ImageProtocol>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: Option<bool>,
    pub sound: Option<bool>,
    pub categories: BTreeMap<String, bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Checks {
    pub prepare: Vec<crate::checks::Spec>,
    pub check: Option<crate::checks::Spec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub path: PathBuf,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub launch: LaunchOverrides,
    #[serde(default)]
    pub sidebar: Sidebar,
    #[serde(default)]
    pub checks: Checks,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub prefix: Option<String>,
    pub bindings: Vec<Binding>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Sequence {
    Chord(String),
    Steps(Vec<String>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub command: String,
    pub context: String,
    #[serde(default)]
    pub add: Vec<Sequence>,
    #[serde(default)]
    pub remove: Vec<Sequence>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub preferred: Option<Sequence>,
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub entries: Vec<LaunchSnapshot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Conversation {
    Latest,
    Fresh,
    Picker,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchValues {
    pub agent: Agent,
    pub layout: Layout,
    pub composition: Composition,
    pub conversation: Conversation,
    pub new_window: bool,
    pub new_instance: bool,
    pub no_profile: bool,
    pub shell: PathBuf,
    pub agent_args: Vec<String>,
    pub debug_command: String,
    pub sessions: Vec<SessionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}
impl Default for LaunchValues {
    fn default() -> Self {
        Self {
            agent: Agent::Codex,
            layout: Layout::Float,
            composition: Composition::AgentDebug,
            conversation: Conversation::Latest,
            new_window: false,
            new_instance: false,
            no_profile: false,
            shell: "pwsh".into(),
            agent_args: vec![],
            debug_command: String::new(),
            sessions: vec![],
            title: None,
            color: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotTerminal {
    pub history_lines: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSnapshot {
    pub project: PathBuf,
    pub launch: LaunchValues,
    pub terminal: SnapshotTerminal,
}
#[derive(Clone, Debug)]
pub struct ResolvedLaunch {
    pub snapshot: LaunchSnapshot,
    pub sources: BTreeMap<String, &'static str>,
}
impl LaunchValues {
    fn overlay(
        &mut self,
        v: &LaunchOverrides,
        source: &'static str,
        sources: &mut BTreeMap<String, &'static str>,
    ) {
        macro_rules! copy { ($($f:ident),*) => { $(if let Some(value)=&v.$f {self.$f=value.clone();sources.insert(stringify!($f).into(),source);})* } }
        copy!(
            agent,
            layout,
            composition,
            new_window,
            shell,
            agent_args,
            debug_command,
            sessions
        );
        if let Some(value) = v.resume_latest {
            self.conversation = if value {
                Conversation::Latest
            } else {
                Conversation::Fresh
            };
            sources.insert("resume_latest".into(), source);
        }
        for (key, value, target) in [
            ("title", &v.title, &mut self.title),
            ("color", &v.color, &mut self.color),
        ] {
            if let Some(value) = value {
                *target = (!value.is_empty()).then(|| value.clone());
                sources.insert(key.into(), source);
            }
        }
    }
}
impl Preferences {
    pub fn project(&self, path: &Path) -> Option<&Project> {
        let key = project_key(path);
        self.project.iter().find(|p| project_key(&p.path) == key)
    }
    pub fn resolve(&self, path: &Path, explicit: &LaunchOverrides) -> ResolvedLaunch {
        let mut launch = LaunchValues::default();
        let mut sources = BTreeMap::new();
        launch.overlay(&self.launch, "global", &mut sources);
        if let Some(p) = self.project(path) {
            launch.overlay(&p.launch, "project", &mut sources);
        }
        launch.overlay(explicit, "invocation", &mut sources);
        ResolvedLaunch {
            snapshot: LaunchSnapshot {
                project: path.into(),
                launch,
                terminal: SnapshotTerminal {
                    history_lines: self.terminal.history_lines.unwrap_or(5000),
                },
            },
            sources,
        }
    }
    pub fn resolve_sidebar(&self, path: &Path) -> Sidebar {
        let mut value = self.sidebar.clone();
        if let Some(project) = self.project(path) {
            value.overlay(&project.sidebar);
        }
        value
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == SCHEMA, "Unsupported configuration schema");
        ensure!(
            self.launch.title.is_none() && self.launch.color.is_none(),
            "launch.title/color are project-only"
        );
        validate_launch(&self.launch)?;
        validate_sidebar(&self.sidebar)?;
        if let Some(v) = self.terminal.history_lines {
            ensure!(v <= 20000, "terminal.history_lines must be 0..20000");
        }
        for value in [self.overview.list_ratio, self.overview.detail_ratio]
            .into_iter()
            .flatten()
        {
            ensure!(
                (1..10000).contains(&value),
                "overview ratio must be 1..9999"
            );
        }
        if let (Some(a), Some(b)) = (self.overview.list_ratio, self.overview.detail_ratio) {
            ensure!(
                u32::from(a) + u32::from(b) < 10000,
                "overview ratios must leave space for statistics"
            );
        }
        let mut keys = std::collections::BTreeSet::new();
        for p in &self.project {
            ensure!(p.path.is_absolute(), "Project path must be absolute");
            ensure!(keys.insert(project_key(&p.path)), "Duplicate project path");
            validate_launch(&p.launch)?;
            validate_sidebar(&p.sidebar)?;
            ensure!(p.checks.prepare.len() <= 16, "At most 16 preparation steps");
            for spec in p.checks.prepare.iter().chain(p.checks.check.iter()) {
                validate_check(spec)?;
            }
        }
        for p in self
            .discovery
            .roots
            .iter()
            .chain(&self.discovery.added)
            .chain(&self.discovery.excluded)
        {
            ensure!(p.is_absolute(), "Discovery path must be absolute");
        }
        let mut names = std::collections::BTreeSet::new();
        for preset in &self.preset {
            ensure!(
                !preset.name.trim().is_empty()
                    && preset.name.chars().count() <= 128
                    && names.insert(&preset.name),
                "Invalid or duplicate preset name"
            );
            ensure!(!preset.entries.is_empty(), "Preset must contain a launch");
            for entry in &preset.entries {
                entry.validate()?;
            }
        }
        for category in self.notifications.categories.keys() {
            ensure!(
                ["end", "failure", "permission", "input"].contains(&category.as_str()),
                "Unknown notification category"
            );
        }
        Ok(())
    }
}
pub fn project_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if text.len() == 3 && text.as_bytes()[1] == b':' {
        return text.to_owned();
    }
    text.trim_end_matches('\\').to_owned()
}
pub fn validate_sidebar(v: &Sidebar) -> Result<()> {
    if let Some(w) = v.width {
        ensure!((20..=80).contains(&w), "sidebar.width must be 20..80");
    }
    if let Some(r) = v.agent_ratio {
        ensure!(
            (1..10000).contains(&r),
            "sidebar.agent_ratio must be 1..9999"
        );
    }
    Ok(())
}
pub fn validate_launch(v: &LaunchOverrides) -> Result<()> {
    if let Some(shell) = &v.shell {
        ensure!(
            !shell.as_os_str().is_empty()
                && (shell.is_absolute() || !shell.to_string_lossy().contains(['\\', '/'])),
            "Expected PowerShell program name or absolute path"
        );
    }
    let mut o = Options::parse_for_view();
    o.title = v.title.clone().filter(|s| !s.is_empty());
    o.color = v.color.clone().filter(|s| !s.is_empty());
    o.agent_args = v.agent_args.clone().unwrap_or_default();
    crate::project_profile::validate_launch(&o)?;
    for command in v.debug_command.iter().chain(
        v.sessions
            .iter()
            .flatten()
            .filter_map(|s| s.command.as_ref()),
    ) {
        ensure!(
            command.len() <= 32768 && !command.contains('\0'),
            "Command exceeds limit or contains NUL"
        );
    }
    let sessions = v.sessions.as_deref().unwrap_or_default();
    ensure!(sessions.len() <= 16, "At most 16 extra sessions");
    let mut names = std::collections::BTreeSet::new();
    for session in sessions {
        ensure!(
            !session.name.trim().is_empty()
                && session.name.len() <= 128
                && !session.name.chars().any(char::is_control)
                && names.insert(session.name.to_lowercase()),
            "Invalid or duplicate session name"
        );
    }
    Ok(())
}
pub fn validate_check(spec: &crate::checks::Spec) -> Result<()> {
    ensure!(
        !spec.argv.is_empty()
            && spec.argv.len() <= 128
            && !spec.argv[0].is_empty()
            && spec
                .argv
                .iter()
                .all(|s| s.len() <= 8192 && !s.contains('\0')),
        "Invalid check argv"
    );
    ensure!(
        (1..=3_600_000).contains(&spec.timeout_ms),
        "Check timeout must be 1..3600000 ms"
    );
    ensure!(
        !spec.cwd.as_os_str().is_empty(),
        "Check cwd cannot be empty"
    );
    let mut names = std::collections::BTreeSet::new();
    for (key, value) in &spec.env {
        ensure!(
            !key.is_empty()
                && !key.contains(['=', '\0'])
                && !value.contains('\0')
                && names.insert(key.to_lowercase()),
            "Invalid or duplicate environment variable"
        );
    }
    Ok(())
}
impl LaunchSnapshot {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.project.is_absolute(),
            "Snapshot project path must be absolute"
        );
        ensure!(
            self.terminal.history_lines <= 20000,
            "Invalid snapshot history"
        );
        validate_launch(&LaunchOverrides {
            agent: Some(self.launch.agent),
            layout: Some(self.launch.layout),
            composition: Some(self.launch.composition),
            shell: Some(self.launch.shell.clone()),
            agent_args: Some(self.launch.agent_args.clone()),
            debug_command: Some(self.launch.debug_command.clone()),
            sessions: Some(self.launch.sessions.clone()),
            title: self.launch.title.clone(),
            color: self.launch.color.clone(),
            ..Default::default()
        })
    }
    pub fn capture(o: &Options, conversation: Conversation) -> Self {
        Self {
            project: o.project.clone(),
            launch: LaunchValues {
                agent: o.agent,
                layout: o.layout,
                composition: o.composition,
                conversation,
                new_window: o.new_window,
                new_instance: o.new_instance,
                no_profile: o.no_profile,
                shell: o.shell.clone(),
                agent_args: o.agent_args.clone(),
                debug_command: o.debug_command.clone().unwrap_or_default(),
                sessions: o.extra_sessions.clone(),
                title: o.title.clone(),
                color: o.color.clone(),
            },
            terminal: SnapshotTerminal {
                history_lines: o.history,
            },
        }
    }
    pub fn options(&self) -> Options {
        let mut o = Options::parse_for_view();
        let v = &self.launch;
        o.project = self.project.clone();
        o.agent = v.agent;
        o.layout = v.layout;
        o.composition = v.composition;
        o.continue_latest = v.conversation == Conversation::Latest;
        o.fresh = v.conversation == Conversation::Fresh;
        o.resume = v.conversation == Conversation::Picker;
        o.new_window = v.new_window;
        o.new_instance = v.new_instance;
        o.no_profile = v.no_profile;
        o.shell = v.shell.clone();
        o.agent_args = v.agent_args.clone();
        o.debug_command = (!v.debug_command.is_empty()).then(|| v.debug_command.clone());
        o.extra_sessions = v.sessions.clone();
        o.title = v.title.clone();
        o.color = v.color.clone();
        o.history = self.terminal.history_lines;
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inheritance_preserves_false_empty_collections_and_argument_boundaries() {
        let p: Preferences = toml::from_str(
            r#"
schema=1
[launch]
agent="claude"
resume_latest=true
agent_args=["--model","中文 名称",""]
debug_command="Write-Output inherited"
[[project]]
path='D:\repo'
[project.launch]
resume_latest=false
agent_args=[]
debug_command=""
"#,
        )
        .unwrap();
        let r = p.resolve(Path::new(r"D:\repo"), &Default::default());
        assert_eq!(r.snapshot.launch.agent, Agent::Claude);
        assert_eq!(r.snapshot.launch.conversation, Conversation::Fresh);
        assert!(r.snapshot.launch.agent_args.is_empty());
        assert!(r.snapshot.launch.debug_command.is_empty());
        assert_eq!(r.sources["agent_args"], "project");
        let other = p.resolve(Path::new(r"D:\other"), &Default::default());
        assert_eq!(
            other.snapshot.launch.agent_args,
            ["--model", "中文 名称", ""]
        );
    }
    #[test]
    fn custom_sessions_never_implicitly_change_composition() {
        let mut p = Preferences::default();
        p.launch.sessions = Some(vec![SessionConfig {
            name: "extra".into(),
            command: None,
            directory: None,
        }]);
        assert_eq!(
            p.resolve(Path::new(r"D:\repo"), &Default::default())
                .snapshot
                .launch
                .composition,
            Composition::AgentDebug
        );
    }
    #[test]
    fn snapshot_requires_complete_values_and_keeps_launch_capabilities() {
        let mut o = Options::parse_for_view();
        o.project = r"D:\repo".into();
        o.history = 42;
        o.new_instance = true;
        o.new_window = true;
        o.no_profile = true;
        o.fresh = true;
        o.agent_args = vec!["中文 空格".into(), "".into()];
        let snapshot = LaunchSnapshot::capture(&o, Conversation::Fresh);
        let text = toml::to_string(&snapshot).unwrap();
        let r: LaunchSnapshot = toml::from_str(&text).unwrap();
        assert!(
            r.options().fresh
                && r.options().new_instance
                && r.options().no_profile
                && r.options().new_window
        );
        assert_eq!(r.options().history, 42);
        assert_eq!(r.options().agent_args, o.agent_args);
        assert!(
            toml::from_str::<LaunchSnapshot>(&text.replace("new_instance = true\n", "")).is_err()
        );
        assert!(toml::from_str::<Preferences>("").is_err());
    }
    #[test]
    fn example_is_valid_and_optional_titles_do_not_inherit_into_snapshots() {
        let doc = include_str!("../../docs/53-Unified-Configuration-Design.md");
        let fence = char::from(96).to_string().repeat(3);
        let example = doc
            .split(&format!("{fence}toml\n"))
            .nth(1)
            .unwrap()
            .split(&fence)
            .next()
            .unwrap();
        let p: Preferences = toml::from_str(example).unwrap();
        p.validate().unwrap();
        let mut snapshot = p.preset[0].entries[0].clone();
        snapshot.launch.title = None;
        let roundtrip: LaunchSnapshot =
            toml::from_str(&toml::to_string(&snapshot).unwrap()).unwrap();
        assert!(roundtrip.options().title.is_none());
    }
    #[test]
    fn validation_keeps_scope_size_and_ratio_contracts() {
        let mut p = Preferences::default();
        p.launch.title = Some("global".into());
        assert!(p.validate().is_err());
        p.launch.title = None;
        p.terminal.history_lines = Some(0);
        p.validate().unwrap();
        p.overview.list_ratio = Some(5000);
        p.overview.detail_ratio = Some(5000);
        assert!(p.validate().is_err());
        assert_eq!(
            project_key(Path::new("D:/Repo/")),
            project_key(Path::new(r"d:\repo"))
        );
        assert_eq!(project_key(Path::new(r"D:\")), r"d:\");
    }
    #[test]
    fn check_environment_names_are_case_insensitive_and_zero_timeout_is_invalid() {
        let mut spec = crate::checks::Spec {
            argv: vec!["cargo".into(), "test".into()],
            cwd: ".".into(),
            env: BTreeMap::new(),
            timeout_ms: 1,
        };
        validate_check(&spec).unwrap();
        spec.env.insert("PATH".into(), "a".into());
        spec.env.insert("path".into(), "b".into());
        assert!(validate_check(&spec).is_err());
        spec.env.clear();
        spec.timeout_ms = 0;
        assert!(validate_check(&spec).is_err());
    }
}
