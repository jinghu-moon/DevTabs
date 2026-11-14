//! Domain view used by existing overview code; it is never serialized as a file.
use super::{
    data::{UiChange, UiState, UserData},
    model as m,
    store::SettingsStore,
};
use crate::{
    config::{Config, Project},
    overview::model::LaunchSpec,
};
use anyhow::Result;
use std::path::Path;

pub fn load() -> Result<Config> {
    let root = crate::config::home();
    let mut source = SettingsStore::new(&root).read()?;
    let mut view = from_preferences(&source.effective);
    match UserData::read(&root.join("user-data.json")) {
        Ok(data) => {
            for (key, note) in data.notes {
                view.projects.entry(key).or_default().note = note;
            }
        }
        Err(error) => source.diagnostics.push(super::store::Diagnostic {
            path: "user-data.json".into(),
            message: format!("{error:#}"),
            blocks: false,
        }),
    }
    match UiState::read(&root) {
        Ok(state) => {
            view.overview.collapsed = state.overview.collapsed;
            for (key, recent) in state.projects {
                view.projects.entry(key).or_default().last_opened = recent.last_opened;
            }
        }
        Err(e) => eprintln!("UI memory: {e:#}"),
    }
    view.baseline = Some(serde_json::to_value(&view)?);
    view.source = Some(source);
    Ok(view)
}
pub fn from_preferences(p: &m::Preferences) -> Config {
    let mut view = Config {
        language: p.general.language.unwrap_or_default(),
        theme: p.appearance.theme.unwrap_or_default(),
        default_agent: p.launch.agent,
        roots: p.discovery.roots.clone(),
        added: p.discovery.added.clone(),
        excluded: p.discovery.excluded.clone(),
        ..Default::default()
    };
    view.overview.sort = p.overview.sort.unwrap_or_default();
    view.overview.filter = p.overview.filter.unwrap_or_default();
    view.overview.detail = p.overview.detail;
    view.overview.statistics = p.overview.statistics;
    view.overview.list_ratio = p.overview.list_ratio;
    view.overview.detail_ratio = p.overview.detail_ratio;
    for project in &p.project {
        let l = &project.launch;
        view.projects.insert(
            crate::config::key(&project.path),
            Project {
                title: l.title.clone(),
                color: l.color.clone(),
                agent_args: l.agent_args.clone().unwrap_or_default(),
                resume_latest: l.resume_latest,
                agent: l.agent,
                debug_command: l.debug_command.clone(),
                layout: l.layout,
                composition: l.composition,
                sessions: l.sessions.clone().unwrap_or_default(),
                favorite: project.favorite,
                sidebar: sidebar(&p.resolve_sidebar(&project.path)),
                ..Default::default()
            },
        );
    }
    for preset in &p.preset {
        view.presets.insert(
            preset.name.clone(),
            preset
                .entries
                .iter()
                .map(|s| LaunchSpec::from_options(&s.options()))
                .collect(),
        );
    }
    view
}
pub fn sidebar(p: &m::Sidebar) -> crate::explorer::Preferences {
    let default = crate::explorer::Preferences::default();
    crate::explorer::Preferences {
        width: p.width.unwrap_or(default.width),
        agent_ratio: p.agent_ratio.unwrap_or(default.agent_ratio),
        follow: p.follow.unwrap_or(default.follow),
        compact: p.compact.unwrap_or(default.compact),
    }
}
pub fn save(view: &Config) -> Result<()> {
    let store = SettingsStore::current();
    let base = match &view.source {
        Some(s) => s.clone(),
        None => store.read()?,
    };
    let old: Config = match &view.baseline {
        Some(v) => serde_json::from_value(v.clone())?,
        None => Config::default(),
    };
    let mut draft = base.tree.clone();
    macro_rules! field {($old:expr,$new:expr,$($key:expr),+) => {if changed(&$old,&$new) { set(&mut draft,&[$($key),+],toml::Value::try_from(&$new).ok()); }};}
    field!(old.language, view.language, "general", "language");
    field!(old.theme, view.theme, "appearance", "theme");
    if old.default_agent != view.default_agent {
        set(
            &mut draft,
            &["launch", "agent"],
            view.default_agent
                .map(|v| toml::Value::try_from(v).unwrap()),
        );
    }
    field!(old.roots, view.roots, "discovery", "roots");
    field!(old.added, view.added, "discovery", "added");
    field!(old.excluded, view.excluded, "discovery", "excluded");
    macro_rules! overview {($($f:ident),*)=>{$(field!(old.overview.$f,view.overview.$f,"overview",stringify!($f));)*};}
    overview!(sort, filter, detail, statistics, list_ratio, detail_ratio);
    let old_keys = old
        .projects
        .keys()
        .chain(view.projects.keys())
        .collect::<std::collections::BTreeSet<_>>();
    for key in old_keys {
        let before = old.projects.get(key).cloned().unwrap_or_default();
        let after = view.projects.get(key).cloned().unwrap_or_default();
        let before_json = serde_json::to_value(&before)?;
        let after_json = serde_json::to_value(&after)?;
        if before_json == after_json {
            continue;
        }
        let fields = [
            "title",
            "color",
            "agent_args",
            "resume_latest",
            "agent",
            "debug_command",
            "layout",
            "composition",
            "sessions",
        ];
        let settings_changed = before.favorite != after.favorite
            || before.sidebar != after.sidebar
            || fields.iter().any(|f| before_json[f] != after_json[f]);
        if settings_changed {
            let entries = draft
                .as_table_mut()
                .unwrap()
                .entry("project")
                .or_insert_with(|| toml::Value::Array(vec![]))
                .as_array_mut()
                .unwrap();
            let index = entries.iter().position(|p| {
                p.get("path")
                    .and_then(toml::Value::as_str)
                    .is_some_and(|s| crate::config::key(Path::new(s)) == *key)
            });
            let index = index.unwrap_or_else(|| {
                let mut p = toml::Table::new();
                p.insert("path".into(), key.clone().into());
                entries.push(toml::Value::Table(p));
                entries.len() - 1
            });
            let project = &mut entries[index];
            if before.favorite != after.favorite {
                set(project, &["favorite"], Some(after.favorite.into()));
            }
            for field in fields {
                if before_json[field] != after_json[field] {
                    let value = &after_json[field];
                    let value = if value.is_null() {
                        None
                    } else {
                        Some(json_toml(value)?)
                    };
                    set(project, &["launch", field], value);
                }
            }
            for field in ["width", "agent_ratio", "follow", "compact"] {
                if before_json["sidebar"][field] != after_json["sidebar"][field] {
                    set(
                        project,
                        &["sidebar", field],
                        Some(json_toml(&after_json["sidebar"][field])?),
                    );
                }
            }
        }
    }
    if changed(&old.presets, &view.presets) {
        let mut presets = vec![];
        for (name, entries) in &view.presets {
            let mut snapshots = vec![];
            for spec in entries {
                let options = spec.options();
                anyhow::ensure!(
                    options.resume_id.is_none(),
                    "Choose latest, fresh or picker before saving a specific conversation as a preset"
                );
                let strategy = if options.fresh {
                    m::Conversation::Fresh
                } else if options.resume {
                    m::Conversation::Picker
                } else {
                    m::Conversation::Latest
                };
                snapshots.push(m::LaunchSnapshot::capture(&options, strategy));
            }
            presets.push(m::Preset {
                name: name.clone(),
                entries: snapshots,
            });
        }
        set(
            &mut draft,
            &["preset"],
            Some(toml::Value::try_from(presets)?),
        );
    }
    store.save(&base, &draft)?;
    // These are domain operations, deliberately excluded from the preference transaction.
    for (key, after) in &view.projects {
        let before = old.projects.get(key).cloned().unwrap_or_default();
        if before.note != after.note {
            UserData::note(
                &crate::config::home(),
                Path::new(key),
                &before.note,
                &after.note,
            )?;
        }
        if before.last_opened != after.last_opened {
            UiState::update(
                &crate::config::home(),
                UiChange::Opened(key.into(), after.last_opened),
            )?;
        }
    }
    for group in old
        .overview
        .collapsed
        .symmetric_difference(&view.overview.collapsed)
    {
        UiState::update(
            &crate::config::home(),
            UiChange::Collapsed(*group, view.overview.collapsed.contains(group)),
        )?;
    }
    Ok(())
}
fn json_toml(value: &serde_json::Value) -> Result<toml::Value> {
    Ok(toml::Value::try_from(value)?)
}
pub fn set(tree: &mut toml::Value, path: &[&str], value: Option<toml::Value>) {
    let Some((key, rest)) = path.split_first() else {
        return;
    };
    let table = tree.as_table_mut().expect("settings table");
    if rest.is_empty() {
        if let Some(v) = value {
            table.insert((*key).into(), v);
        } else {
            table.remove(*key);
        }
    } else {
        if value.is_none() && !table.contains_key(*key) {
            return;
        }
        let child = table
            .entry(*key)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        set(child, rest, value);
    }
}
fn changed<A: serde::Serialize, B: serde::Serialize>(a: &A, b: &B) -> bool {
    serde_json::to_value(a).unwrap() != serde_json::to_value(b).unwrap()
}
