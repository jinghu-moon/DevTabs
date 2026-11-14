//! Explicit project-template export and launch validation; templates are never auto-loaded.
use crate::{
    config::{self, Config, Project, SessionConfig},
    launch::{Agent, Layout, Options},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub prepare: Vec<crate::checks::Spec>,
    pub check: Option<crate::checks::Spec>,
    pub agent: Option<Agent>,
    pub layout: Option<Layout>,
    pub composition: Option<crate::launch::Composition>,
    pub title: Option<String>,
    pub color: Option<String>,
    pub agent_args: Option<Vec<String>>,
    pub resume_latest: Option<bool>,
    pub debug_command: Option<String>,
    pub sessions: Option<Vec<SessionConfig>>,
}
impl Profile {
    pub fn from_project(p: &Project) -> Self {
        Self {
            prepare: vec![],
            check: None,
            agent: p.agent,
            layout: p.layout,
            composition: p.composition,
            title: p.title.clone(),
            color: p.color.clone(),
            agent_args: Some(p.agent_args.clone()),
            resume_latest: p.resume_latest,
            debug_command: Some(p.debug_command.clone().unwrap_or_default()),
            sessions: Some(p.sessions.clone()),
        }
    }
    pub fn apply(self, p: &mut Project) {
        macro_rules! copy { ($($field:ident),*) => {$ (if self.$field.is_some() {p.$field=self.$field;})*}; }
        copy!(agent, layout, composition, title, color, resume_latest);
        if let Some(v) = self.debug_command {
            p.debug_command = (!v.is_empty()).then_some(v);
        }
        if let Some(v) = self.agent_args {
            p.agent_args = v;
        }
        if let Some(v) = self.sessions {
            p.sessions = v;
        }
    }
    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(".devtabs.toml");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut text = String::new();
        file.take(128 * 1024 + 1).read_to_string(&mut text)?;
        ensure!(text.len() <= 128 * 1024, "Project profile exceeds 128 KiB");
        Ok(Some(
            toml::from_str(&text).with_context(|| format!("Read {}", path.display()))?,
        ))
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        let mut p = Project::default();
        self.clone().apply(&mut p);
        crate::overview::model::validate_settings(&p)?;
        config::atomic_write(
            &root.join(".devtabs.toml"),
            toml::to_string_pretty(self)?.as_bytes(),
        )
    }
}
pub fn effective(c: &Config, path: &Path) -> Result<Project> {
    let mut p = c
        .projects
        .get(&config::key(path))
        .cloned()
        .unwrap_or_default();
    if let Some(source) = &c.source {
        let key = crate::settings::model::project_scope(path);
        source.require("launch")?;
        source.require(&format!("{key}.launch"))?;
        let resolved = source.effective.resolve(path, &Default::default());
        let v = resolved.snapshot.launch;
        p.agent = Some(v.agent);
        p.layout = Some(v.layout);
        p.composition = Some(v.composition);
        p.title = v.title;
        p.color = v.color;
        p.agent_args = v.agent_args;
        p.resume_latest = Some(v.conversation == crate::settings::model::Conversation::Latest);
        p.debug_command = Some(v.debug_command);
        p.sessions = v.sessions;
    }
    crate::overview::model::validate_settings(&p)?;
    Ok(p)
}
pub fn validate_launch(o: &Options) -> Result<()> {
    ensure!(
        o.title
            .as_ref()
            .is_none_or(|s| s.len() <= 256 && !s.chars().any(char::is_control)),
        "Invalid workspace title"
    );
    ensure!(
        o.color.as_deref().is_none_or(valid_color),
        "Color must be #RRGGBB"
    );
    ensure!(
        o.agent_args.len() <= 64
            && o.agent_args
                .iter()
                .all(|s| s.len() <= 4096 && !s.contains(['\0', '\r', '\n'])),
        "Invalid Agent arguments"
    );
    Ok(())
}
pub fn valid_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Creates only a new, single directory; existing directories are never modified.
pub fn create_project(root: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && !name.starts_with('.')
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)),
        "Invalid project name"
    );
    let stem = name.split('.').next().unwrap().to_ascii_uppercase();
    ensure!(
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9')),
        "Reserved project name"
    );
    let root = dunce::canonicalize(root)?;
    let path = root.join(name);
    std::fs::create_dir(&path).context("Create new project directory")?;
    // The Git invocation is bounded and hidden by the existing process helper.
    if let Err(e) = crate::explorer::git_output(&path, &["init"]) {
        return Err(e).with_context(|| {
            format!(
                "Directory created at {}; Git initialization failed",
                path.display()
            )
        });
    }
    Ok(path)
}
