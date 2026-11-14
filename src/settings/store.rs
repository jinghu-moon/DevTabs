//! Comment-preserving preferences with semantic three-way saves.
use super::model::{self, Preferences};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use toml::Value;
use toml_edit::{DocumentMut, Item, Table};

const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub path: String,
    pub message: String,
    pub blocks: bool,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub text: String,
    pub tree: Value,
    pub effective: Preferences,
    pub diagnostics: Vec<Diagnostic>,
}
pub struct SettingsStore {
    root: PathBuf,
}
impl SettingsStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn current() -> Self {
        Self::new(crate::config::home())
    }
    pub fn path(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    pub fn read(&self) -> Result<Snapshot> {
        let text = read_text(&self.path())?;
        Snapshot::parse(text).with_context(|| format!("Read {}", self.path().display()))
    }
    /// A draft is the sparse source tree, never the effective fallback projection.
    pub fn save(&self, base: &Snapshot, draft: &Value) -> Result<Snapshot> {
        let _lock = crate::config::ConfigLock::acquire_at(&self.root)?;
        let latest = self.read()?;
        let merged = merge(Some(&base.tree), Some(draft), Some(&latest.tree), "")?
            .context("Cannot delete configuration root")?;
        if merged == latest.tree {
            return Ok(latest);
        }
        let changed = changes(&latest.tree, &merged, "");
        let proposed = Snapshot::parse(toml::to_string(&merged)?)?;
        for error in proposed.diagnostics.iter().filter(|d| d.blocks) {
            ensure!(
                !changed.iter().any(|p| overlaps(p, &error.path)
                    || (["overview.list_ratio", "overview.detail_ratio"].contains(&p.as_str())
                        && ["overview.list_ratio", "overview.detail_ratio"]
                            .contains(&error.path.as_str()))),
                "{}: {}",
                error.path,
                error.message
            );
        }
        let mut document = if latest.text.is_empty() {
            DocumentMut::new()
        } else {
            latest.text.parse::<DocumentMut>()?
        };
        let desired = toml::to_string_pretty(&merged)?.parse::<DocumentMut>()?;
        sync_table(document.as_table_mut(), desired.as_table());
        let text = document.to_string();
        let result = Snapshot::parse(text.clone())?;
        ensure!(
            read_text(&self.path())? == latest.text,
            "Configuration changed outside DevTabs; retry to merge the latest version"
        );
        // Keep recoverable source bytes; this is save recovery, not format migration.
        if !latest.text.is_empty() {
            crate::config::atomic_write(
                &self.root.join("config.previous.toml"),
                latest.text.as_bytes(),
            )?;
        }
        crate::config::atomic_write(&self.path(), text.as_bytes())?;
        Ok(result)
    }
}
fn read_text(path: &Path) -> Result<String> {
    match fs::File::open(path) {
        Ok(file) => {
            use std::io::Read;
            ensure!(
                file.metadata()?.len() <= MAX_BYTES as u64,
                "Configuration exceeds 4 MiB"
            );
            let mut text = String::new();
            file.take(MAX_BYTES as u64 + 1).read_to_string(&mut text)?;
            ensure!(text.len() <= MAX_BYTES, "Configuration exceeds 4 MiB");
            Ok(text)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}
fn overlaps(a: &str, b: &str) -> bool {
    a == b
        || ['.', '['].iter().any(|separator| {
            a.starts_with(&format!("{b}{separator}")) || b.starts_with(&format!("{a}{separator}"))
        })
}
pub fn rebase(base: &Value, draft: &Value, latest: &Value) -> Result<Value> {
    merge(Some(base), Some(draft), Some(latest), "")?.context("Cannot delete configuration root")
}
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.into()
    } else {
        format!("{path}.{key}")
    }
}
fn entity_path(path: &str, key: &str) -> String {
    format!("{path}[{}]", key.replace(']', "\\]"))
}
fn identity(value: &Value, path: &str) -> Option<String> {
    match path {
        "project" => value
            .get("path")?
            .as_str()
            .map(|p| model::project_key(Path::new(p))),
        "preset" => value.get("name")?.as_str().map(str::to_owned),
        "shortcuts.bindings" => Some(format!(
            "{}@{}",
            value.get("command")?.as_str()?,
            value.get("context")?.as_str()?
        )),
        _ => None,
    }
}
fn keyed<'a>(array: &'a [Value], path: &str) -> Result<Vec<(String, &'a Value)>> {
    let mut keys = BTreeSet::new();
    array
        .iter()
        .map(|v| {
            let key = identity(v, path).context(format!("{path}: missing entity identity"))?;
            ensure!(keys.insert(key.clone()), "{path}: duplicate entity {key}");
            Ok((key, v))
        })
        .collect()
}
fn as_array(v: Option<&Value>) -> Option<&Vec<Value>> {
    v.and_then(Value::as_array)
}
fn find<'a>(v: &[(String, &'a Value)], k: &str) -> Option<&'a Value> {
    v.iter().find(|(id, _)| id == k).map(|(_, v)| *v)
}
fn merge(
    base: Option<&Value>,
    draft: Option<&Value>,
    latest: Option<&Value>,
    path: &str,
) -> Result<Option<Value>> {
    if draft == base {
        return Ok(latest.cloned());
    }
    if latest == base || latest == draft {
        return Ok(draft.cloned());
    }
    if base.is_none() && draft.is_some_and(Value::is_table) && latest.is_some_and(Value::is_table) {
        return merge(Some(&Value::Table(toml::Table::new())), draft, latest, path);
    }
    if let (Some(Value::Table(b)), Some(Value::Table(d)), Some(Value::Table(l))) =
        (base, draft, latest)
    {
        let keys = b
            .keys()
            .chain(d.keys())
            .chain(l.keys())
            .collect::<BTreeSet<_>>();
        let mut out = toml::Table::new();
        for key in keys {
            if let Some(value) = merge(b.get(key), d.get(key), l.get(key), &join(path, key))? {
                out.insert(key.clone(), value);
            }
        }
        return Ok(Some(Value::Table(out)));
    }
    if matches!(path, "project" | "preset" | "shortcuts.bindings") {
        let empty = vec![];
        if let (Some(d), Some(l)) = (as_array(draft), as_array(latest)) {
            let b = keyed(as_array(base).unwrap_or(&empty), path)?;
            let d = keyed(d, path)?;
            let l = keyed(l, path)?;
            let ids =
                |v: &Vec<(String, &Value)>| v.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
            let (bi, di, li) = (ids(&b), ids(&d), ids(&l));
            let surviving = |order: &Vec<String>| {
                order
                    .iter()
                    .filter(|k| bi.contains(k))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let dr = surviving(&di);
            let lr = surviving(&li);
            let brd = bi
                .iter()
                .filter(|k| di.contains(k))
                .cloned()
                .collect::<Vec<_>>();
            let brl = bi
                .iter()
                .filter(|k| li.contains(k))
                .cloned()
                .collect::<Vec<_>>();
            ensure!(
                dr == brd || lr == brl || dr == lr,
                "{path}: conflicting order changes"
            );
            let mut order = if dr != brd { di.clone() } else { li.clone() };
            for k in di.iter().chain(li.iter()) {
                if !order.contains(k) {
                    order.push(k.clone());
                }
            }
            let mut out = vec![];
            for key in order {
                if let Some(v) = merge(
                    find(&b, &key),
                    find(&d, &key),
                    find(&l, &key),
                    &entity_path(path, &key),
                )? {
                    out.push(v);
                }
            }
            return Ok(Some(Value::Array(out)));
        }
    }
    bail!("{path}: concurrent edit conflict; draft retained")
}
fn changes(base: &Value, next: &Value, path: &str) -> Vec<String> {
    if base == next {
        return vec![];
    }
    if let (Value::Array(b), Value::Array(n)) = (base, next)
        && matches!(path, "project" | "preset" | "shortcuts.bindings")
        && let (Ok(b), Ok(n)) = (keyed(b, path), keyed(n, path))
    {
        let ids = b
            .iter()
            .chain(n.iter())
            .map(|(k, _)| k)
            .collect::<BTreeSet<_>>();
        return ids
            .into_iter()
            .flat_map(|id| match (find(&b, id), find(&n, id)) {
                (Some(b), Some(n)) => changes(b, n, &entity_path(path, id)),
                _ => vec![entity_path(path, id)],
            })
            .collect();
    }
    if let (Value::Table(b), Value::Table(n)) = (base, next) {
        let keys = b.keys().chain(n.keys()).collect::<BTreeSet<_>>();
        return keys
            .into_iter()
            .flat_map(|k| match (b.get(k), n.get(k)) {
                (Some(b), Some(n)) => changes(b, n, &join(path, k)),
                _ => vec![join(path, k)],
            })
            .collect();
    }
    vec![path.into()]
}
fn sync_table(target: &mut Table, desired: &Table) {
    let removed = target
        .iter()
        .filter(|(k, _)| !desired.contains_key(k))
        .map(|(k, _)| k.to_owned())
        .collect::<Vec<_>>();
    for key in removed {
        target.remove(&key);
    }
    for (key, item) in desired {
        match target.get_mut(key) {
            Some(old) => sync_item(old, item, key),
            None => {
                target.insert(key, item.clone());
            }
        }
    }
}
fn sync_item(old: &mut Item, new: &Item, path: &str) {
    match (old, new) {
        (Item::Table(o), Item::Table(n)) => sync_table(o, n),
        (Item::ArrayOfTables(o), Item::ArrayOfTables(n)) => {
            let mut result = toml_edit::ArrayOfTables::new();
            for next in n {
                let key = |t: &Table| -> Option<String> {
                    let field = if path == "project" {
                        "path"
                    } else if path == "preset" {
                        "name"
                    } else {
                        "command"
                    };
                    let value = t.get(field)?.as_str()?;
                    Some(if field == "path" {
                        model::project_key(Path::new(value))
                    } else if field == "command" {
                        format!("{value}@{}", t.get("context")?.as_str()?)
                    } else {
                        value.into()
                    })
                };
                let mut table = o
                    .iter()
                    .find(|t| key(t).is_some() && key(t) == key(next))
                    .cloned()
                    .unwrap_or_else(|| next.clone());
                sync_table(&mut table, next);
                result.push(table);
            }
            *o = result;
        }
        (Item::Value(o), Item::Value(n)) => {
            if o.to_string().trim() != n.to_string().trim() {
                let decor = o.decor().clone();
                *o = n.clone();
                *o.decor_mut() = decor;
            }
        }
        (o, n) => *o = n.clone(),
    }
}

impl Snapshot {
    pub fn parse(text: String) -> Result<Self> {
        let tree: Value = if text.is_empty() {
            toml::from_str("schema = 1")?
        } else {
            toml::from_str(&text)?
        };
        ensure!(
            tree.get("schema").and_then(Value::as_integer) == Some(model::SCHEMA as i64),
            "Missing or unsupported schema; original file preserved"
        );
        let mut effective = Preferences::default();
        let mut diagnostics = vec![];
        let root = tree
            .as_table()
            .context("Configuration root must be a table")?;
        for (section, value) in root {
            if section == "schema" {
                continue;
            }
            if matches!(section.as_str(), "project" | "preset") {
                let Some(array) = value.as_array() else {
                    diagnostics.push(Diagnostic {
                        path: section.clone(),
                        message: "Expected array of tables".into(),
                        blocks: true,
                    });
                    continue;
                };
                let mut ids = BTreeSet::new();
                for (index, entry) in array.iter().enumerate() {
                    let id = identity(entry, section).unwrap_or_else(|| index.to_string());
                    let path = entity_path(section, &id);
                    if !ids.insert(id) {
                        diagnostics.push(Diagnostic {
                            path,
                            message: "Duplicate entity".into(),
                            blocks: true,
                        });
                        continue;
                    }
                    if section == "project" {
                        parse_project(entry, &path, &mut effective, &mut diagnostics);
                    } else {
                        let mut single = toml::Table::new();
                        single.insert("schema".into(), Value::Integer(1));
                        single.insert(section.clone(), Value::Array(vec![entry.clone()]));
                        match Value::Table(single).try_into::<Preferences>() {
                            Ok(mut p) => match p.validate() {
                                Ok(()) => effective.preset.append(&mut p.preset),
                                Err(e) => diagnostics.push(Diagnostic {
                                    path,
                                    message: e.to_string(),
                                    blocks: true,
                                }),
                            },
                            Err(e) => diagnostics.push(Diagnostic {
                                path,
                                message: e.to_string(),
                                blocks: true,
                            }),
                        }
                    }
                }
                continue;
            }
            if !matches!(
                section.as_str(),
                "general"
                    | "appearance"
                    | "overview"
                    | "discovery"
                    | "launch"
                    | "terminal"
                    | "sidebar"
                    | "preview"
                    | "notifications"
                    | "shortcuts"
            ) {
                diagnostics.push(Diagnostic {
                    path: section.clone(),
                    message: "Unknown field; preserved and not applied".into(),
                    blocks: false,
                });
                continue;
            }
            let Some(fields) = value.as_table() else {
                diagnostics.push(Diagnostic {
                    path: section.clone(),
                    message: "Expected table".into(),
                    blocks: true,
                });
                continue;
            };
            if section == "shortcuts" {
                let result = value.clone().try_into::<model::Shortcuts>().and_then(|v| {
                    crate::shortcuts::Index::build(&v)
                        .map(|_| v)
                        .map_err(serde::de::Error::custom)
                });
                match result {
                    Ok(v) => effective.shortcuts = v,
                    Err(e) => diagnostics.push(Diagnostic {
                        path: "shortcuts".into(),
                        message: e.to_string(),
                        blocks: true,
                    }),
                }
                continue;
            }
            for (field, value) in fields {
                let path = join(section, field);
                if !known_fields(section).contains(&field.as_str()) {
                    diagnostics.push(Diagnostic {
                        path,
                        message: "Unknown field; preserved and not applied".into(),
                        blocks: false,
                    });
                    continue;
                }
                let mut candidate = Value::try_from(&effective)?;
                candidate
                    .as_table_mut()
                    .unwrap()
                    .get_mut(section)
                    .unwrap()
                    .as_table_mut()
                    .unwrap()
                    .insert(field.clone(), value.clone());
                match candidate.try_into::<Preferences>() {
                    Ok(p) => match p.validate() {
                        Ok(()) => effective = p,
                        Err(e) => diagnostics.push(Diagnostic {
                            path: if section == "launch" {
                                section.clone()
                            } else {
                                path
                            },
                            message: e.to_string(),
                            blocks: true,
                        }),
                    },
                    Err(e) => diagnostics.push(Diagnostic {
                        path,
                        message: e.to_string(),
                        blocks: true,
                    }),
                }
            }
        }
        Ok(Self {
            text,
            tree,
            effective,
            diagnostics,
        })
    }
    pub fn require(&self, path: &str) -> Result<()> {
        if let Some(d) = self
            .diagnostics
            .iter()
            .find(|d| d.blocks && overlaps(path, &d.path))
        {
            bail!("{}: {}", d.path, d.message);
        }
        Ok(())
    }
}
fn known_fields(section: &str) -> &'static [&'static str] {
    match section {
        "general" => &["language"],
        "appearance" => &["theme"],
        "overview" => &[
            "sort",
            "filter",
            "detail",
            "statistics",
            "list_ratio",
            "detail_ratio",
        ],
        "discovery" => &["roots", "added", "excluded"],
        "launch" => &[
            "agent",
            "layout",
            "composition",
            "resume_latest",
            "new_window",
            "shell",
            "agent_args",
            "debug_command",
            "sessions",
            "title",
            "color",
        ],
        "terminal" => &["history_lines"],
        "sidebar" => &[
            "width",
            "agent_ratio",
            "follow",
            "compact",
            "show_hidden",
            "show_ignored",
        ],
        "preview" => &["wrap", "markdown_mode", "image_protocol"],
        "notifications" => &["enabled", "sound", "categories"],
        "shortcuts" => &["prefix", "bindings"],
        _ => &[],
    }
}
fn parse_project(
    entry: &Value,
    path: &str,
    effective: &mut Preferences,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut source = toml::Table::new();
    let Some(project_path) = entry.get("path").and_then(Value::as_str) else {
        diagnostics.push(Diagnostic {
            path: path.into(),
            message: "Project path is required".into(),
            blocks: true,
        });
        return;
    };
    source.insert("path".into(), project_path.into());
    let decode = |source: &toml::Table| -> Result<model::Project> {
        let project: model::Project = Value::Table(source.clone()).try_into()?;
        let mut preferences = Preferences::default();
        preferences.project.push(project.clone());
        preferences.validate()?;
        Ok(project)
    };
    let mut project = match decode(&source) {
        Ok(p) => p,
        Err(e) => {
            diagnostics.push(Diagnostic {
                path: path.into(),
                message: e.to_string(),
                blocks: true,
            });
            return;
        }
    };
    let Some(table) = entry.as_table() else {
        return;
    };
    for (field, value) in table {
        if field == "path" {
            continue;
        }
        if !matches!(field.as_str(), "favorite" | "launch" | "sidebar" | "checks") {
            diagnostics.push(Diagnostic {
                path: join(path, field),
                message: "Unknown field; preserved and not applied".into(),
                blocks: false,
            });
            continue;
        }
        if field == "checks" {
            let Some(checks) = value.as_table() else {
                diagnostics.push(Diagnostic {
                    path: join(path, field),
                    message: "Expected table".into(),
                    blocks: true,
                });
                continue;
            };
            for (kind, spec) in checks {
                if !matches!(kind.as_str(), "prepare" | "check") {
                    diagnostics.push(Diagnostic {
                        path: join(&join(path, field), kind),
                        message: "Unknown field; preserved and not applied".into(),
                        blocks: false,
                    });
                    continue;
                }
                let mut candidate = source.clone();
                let checks = candidate
                    .entry("checks")
                    .or_insert_with(|| Value::Table(toml::Table::new()))
                    .as_table_mut()
                    .unwrap();
                checks.insert(kind.clone(), spec.clone());
                match decode(&candidate) {
                    Ok(p) => {
                        source = candidate;
                        project = p;
                    }
                    Err(e) => diagnostics.push(Diagnostic {
                        path: join(&join(path, field), kind),
                        message: e.to_string(),
                        blocks: true,
                    }),
                }
            }
        } else {
            let mut candidate = source.clone();
            candidate.insert(field.clone(), value.clone());
            match decode(&candidate) {
                Ok(p) => {
                    source = candidate;
                    project = p;
                }
                Err(e) => diagnostics.push(Diagnostic {
                    path: join(path, field),
                    message: e.to_string(),
                    blocks: true,
                }),
            }
        }
    }
    effective.project.push(project);
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("DevTabs-settings-{}", uuid::Uuid::new_v4())))
        }
        fn store(&self) -> SettingsStore {
            SettingsStore::new(&self.0)
        }
        fn write(&self, text: &str) {
            fs::create_dir_all(&self.0).unwrap();
            fs::write(self.store().path(), text).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tree(text: &str) -> Value {
        toml::from_str(text).unwrap()
    }
    #[test]
    fn missing_read_does_not_write_and_broken_files_are_preserved() {
        let f = Fixture::new();
        let s = f.store();
        assert_eq!(s.read().unwrap().effective.schema, 1);
        assert!(!f.0.exists());
        let base = s.read().unwrap();
        for text in ["schema=2", "[broken", "[appearance]\ntheme='dark'"] {
            f.write(text);
            assert!(s.read().is_err());
            assert!(
                s.save(&base, &tree("schema=1\n[appearance]\ntheme='light'"))
                    .is_err()
            );
            assert_eq!(fs::read_to_string(s.path()).unwrap(), text);
        }
    }
    #[test]
    fn disjoint_fields_merge_preserving_comments_unknowns_and_invalid_values() {
        let f = Fixture::new();
        f.write("# personal\nschema=1\n[appearance]\ntheme='bad' # repair me\nfuture=42\n[general]\nlanguage='en-us'\n");
        let s = f.store();
        let base = s.read().unwrap();
        assert!(
            base.diagnostics
                .iter()
                .any(|d| d.path == "appearance.theme" && d.blocks)
        );
        assert!(
            base.diagnostics
                .iter()
                .any(|d| d.path == "appearance.future" && !d.blocks)
        );
        let mut draft = base.tree.clone();
        draft["general"]["language"] = "zh-cn".into();
        let mut external = base.tree.clone();
        external
            .as_table_mut()
            .unwrap()
            .insert("overview".into(), tree("statistics=false"));
        s.save(&base, &external).unwrap();
        let saved = s.save(&base, &draft).unwrap();
        assert_eq!(saved.tree["overview"]["statistics"].as_bool(), Some(false));
        assert_eq!(saved.tree["general"]["language"].as_str(), Some("zh-cn"));
        assert!(saved.text.contains("# personal"));
        assert!(saved.text.contains("# repair me"));
        assert_eq!(saved.tree["appearance"]["theme"].as_str(), Some("bad"));
        assert_eq!(saved.tree["appearance"]["future"].as_integer(), Some(42));
    }
    #[test]
    fn same_field_and_parent_deletion_conflict_without_writing() {
        let f = Fixture::new();
        f.write("schema=1\n[overview]\ndetail=true\nstatistics=true\n");
        let s = f.store();
        let base = s.read().unwrap();
        let mut draft = base.tree.clone();
        draft["overview"]["detail"] = false.into();
        f.write("schema=1\n[overview]\ndetail='broken'\nstatistics=true\n");
        let bytes = fs::read(s.path()).unwrap();
        assert!(s.save(&base, &draft).is_err());
        assert_eq!(fs::read(s.path()).unwrap(), bytes);
        f.write("schema=1\n");
        assert!(s.save(&base, &draft).is_err());
    }
    #[test]
    fn entities_merge_by_identity_and_preserve_order_and_entity_comments() {
        let base = tree(
            "schema=1\n[[project]]\npath='D:/one'\nfavorite=false\n[[project]]\npath='D:/two'\nfavorite=false\n",
        );
        let mut draft = base.clone();
        draft["project"].as_array_mut().unwrap()[0]["favorite"] = true.into();
        let mut latest = base.clone();
        latest["project"].as_array_mut().unwrap().reverse();
        latest["project"].as_array_mut().unwrap()[0]["favorite"] = true.into();
        let merged = merge(Some(&base), Some(&draft), Some(&latest), "")
            .unwrap()
            .unwrap();
        assert_eq!(merged["project"][0]["path"].as_str(), Some("D:/two"));
        assert!(
            merged["project"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["favorite"].as_bool() == Some(true))
        );
        let mut deletion = base.clone();
        deletion["project"].as_array_mut().unwrap().remove(0);
        assert!(merge(Some(&base), Some(&draft), Some(&deletion), "").is_err());
    }
    #[test]
    fn invalid_changed_dependency_rejects_whole_transaction() {
        let f = Fixture::new();
        let s = f.store();
        let base = s.read().unwrap();
        let draft = tree(
            "schema=1\n[appearance]\ntheme='light'\n[overview]\nlist_ratio=7000\ndetail_ratio=7000\n",
        );
        assert!(s.save(&base, &draft).is_err());
        assert!(!s.path().exists());
    }
    #[test]
    fn shortcut_dependency_failure_is_isolated_but_draft_is_atomic() {
        let f = Fixture::new();
        f.write("schema=1\n[shortcuts]\nprefix='bad key'\n");
        let store = f.store();
        let base = store.read().unwrap();
        assert!(base.require("shortcuts").is_err());
        let mut draft = base.tree.clone();
        crate::settings::projection::set(
            &mut draft,
            &["appearance", "theme"],
            Some("light".into()),
        );
        let saved = store.save(&base, &draft).unwrap();
        assert_eq!(saved.tree["shortcuts"]["prefix"].as_str(), Some("bad key"));
        let original = fs::read(store.path()).unwrap();
        crate::settings::projection::set(
            &mut draft,
            &["shortcuts", "prefix"],
            Some("Ctrl+G".into()),
        );
        crate::settings::projection::set(
            &mut draft,
            &["shortcuts", "bindings"],
            Some(
                vec![tree(
                    "command='overview.note.edit'\ncontext='overview'\nadd=['Ctrl+G']\n",
                )]
                .into(),
            ),
        );
        assert!(store.save(&saved, &draft).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), original);
    }
}
