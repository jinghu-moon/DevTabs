//! User-authored content and disposable UI memory have separate write owners.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserData {
    #[serde(default, deserialize_with = "note_paths")]
    pub notes: BTreeMap<String, String>,
    #[serde(default)]
    pub bookmarks: Vec<crate::bookmarks::Entry>,
}
fn note_paths<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Notes;
    impl<'de> serde::de::Visitor<'de> for Notes {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique absolute project paths mapped to note text")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut notes = BTreeMap::new();
            while let Some((path, text)) = map.next_entry::<String, String>()? {
                if !Path::new(&path).is_absolute() {
                    return Err(serde::de::Error::custom(
                        "Note project path must be absolute",
                    ));
                }
                let key = crate::config::key(Path::new(&path));
                if notes.insert(key, text).is_some() {
                    return Err(serde::de::Error::custom("Duplicate note project identity"));
                }
            }
            Ok(notes)
        }
    }
    d.deserialize_map(Notes)
}
impl UserData {
    pub fn read(path: &Path) -> Result<Self> {
        read_json(path, 16 * 1024 * 1024)
            .context("Cannot read user content; original file preserved")
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        crate::config::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.notes.values().all(|n| n.chars().count() <= 2048),
            "Note exceeds 2048 characters"
        );
        ensure!(
            self.notes
                .keys()
                .all(|key| Path::new(key).is_absolute()
                    && crate::config::key(Path::new(key)) == *key),
            "Note project identity must be a normalized absolute path"
        );
        ensure!(self.bookmarks.len() <= 4096, "Bookmark limit reached");
        let mut ids = BTreeSet::new();
        let mut counts = BTreeMap::<String, usize>::new();
        for e in &self.bookmarks {
            ensure!(ids.insert(e.id), "Duplicate bookmark identity");
            ensure!(
                e.project.is_absolute()
                    && e.path.is_absolute()
                    && e.note.chars().count() <= 256
                    && e.anchor.chars().count() <= 128,
                "Invalid bookmark"
            );
            let count = counts.entry(crate::config::key(&e.project)).or_default();
            *count += 1;
            ensure!(*count <= 128, "Bookmark limit reached (128 per project)");
        }
        ensure!(
            serde_json::to_vec(&self.bookmarks)?.len() <= 8 * 1024 * 1024,
            "Bookmarks exceed 8 MiB"
        );
        let bytes = serde_json::to_vec_pretty(self)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "User content exceeds 16 MiB"
        );
        Ok(())
    }
    pub fn note(root: &Path, project: &Path, base: &str, draft: &str) -> Result<Self> {
        let _lock = crate::config::ConfigLock::acquire_at(root)?;
        let path = root.join("user-data.json");
        let mut latest = Self::read(&path)?;
        let key = crate::config::key(project);
        let text = latest.notes.get(&key).map_or("", String::as_str);
        ensure!(
            text == base || text == draft,
            "Note changed in another window; draft retained. Latest note: {text}"
        );
        ensure!(
            draft.chars().count() <= 2048,
            "Note exceeds 2048 characters"
        );
        if text != draft {
            if draft.is_empty() {
                latest.notes.remove(&key);
            } else {
                latest.notes.insert(key, draft.into());
            }
            latest.save(&path)?;
        }
        Ok(latest)
    }
}
pub fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path, max: u64) -> Result<T> {
    match fs::File::open(path) {
        Ok(f) => {
            ensure!(
                f.metadata()?.len() <= max,
                "{} exceeds storage budget",
                path.display()
            );
            let mut bytes = vec![];
            f.take(max + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= max,
                "{} exceeds storage budget",
                path.display()
            );
            Ok(serde_json::from_slice(&bytes)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiState {
    pub projects: BTreeMap<String, Recent>,
    pub overview: OverviewMemory,
    pub floating: Option<[u16; 4]>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recent {
    pub last_opened: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OverviewMemory {
    pub collapsed: BTreeSet<u8>,
}
pub enum UiChange {
    Opened(PathBuf, u64),
    Collapsed(u8, bool),
    Floating([u16; 4]),
}
impl UiState {
    pub fn read(root: &Path) -> Result<Self> {
        read_json(&root.join("ui-state.json"), 1024 * 1024)
    }
    pub fn update(root: &Path, change: UiChange) -> Result<Self> {
        let _lock = crate::config::ConfigLock::acquire_at(root)?;
        let mut latest = Self::read(root)?;
        match change {
            UiChange::Opened(path, at) => {
                let recent = latest
                    .projects
                    .entry(crate::config::key(&path))
                    .or_default();
                recent.last_opened = recent.last_opened.max(at);
            }
            UiChange::Collapsed(group, value) => {
                if value {
                    latest.overview.collapsed.insert(group);
                } else {
                    latest.overview.collapsed.remove(&group);
                }
            }
            UiChange::Floating(rect) => latest.floating = Some(rect),
        }
        let bytes = serde_json::to_vec(&latest)?;
        ensure!(bytes.len() <= 1024 * 1024, "UI memory exceeds budget");
        crate::config::atomic_write(&root.join("ui-state.json"), &bytes)?;
        Ok(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_content_budget_is_independent_and_a_full_file_can_still_be_shrunk() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-content-budget-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let project = root.join("p");
        let path = root.join("user-data.json");
        let mut bytes = serde_json::to_vec(
            &serde_json::json!({"notes": {crate::config::key(&project): "one"}}),
        )
        .unwrap();
        bytes.resize(16 * 1024 * 1024, b' ');
        fs::write(&path, &bytes).unwrap();
        assert_eq!(UserData::read(&path).unwrap().notes.len(), 1);
        UserData::note(&root, &project, "one", "").unwrap();
        assert!(fs::metadata(&path).unwrap().len() < 1024);
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(UserData::note(&root, &project, "one", "").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mut oversized = UserData::default();
        for i in 0..8500 {
            oversized.notes.insert(
                crate::config::key(&root.join(format!("project-{i}"))),
                "x".repeat(2048),
            );
        }
        assert!(
            oversized
                .validate()
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
        assert!(oversized.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn note_archive_paths_normalize_and_duplicate_identities_never_silently_overwrite() {
        let root =
            std::env::temp_dir().join(format!("DevTabs-note-archive-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("user-data.json");
        let project = root.join("Project");
        let a = serde_json::to_string(&project.to_string_lossy()).unwrap();
        let b = serde_json::to_string(&crate::config::key(&project)).unwrap();
        for document in [
            format!("{{\"notes\":{{{a}:\"one\",{a}:\"two\"}}}}"),
            format!("{{\"notes\":{{{a}:\"one\",{b}:\"two\"}}}}"),
            "{\"notes\":{\"relative\":\"unaddressable\"}}".into(),
        ] {
            fs::write(&path, &document).unwrap();
            assert!(UserData::read(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), document);
        }
        fs::write(&path, format!("{{\"notes\":{{{a}:\"one\"}}}}")).unwrap();
        let read = UserData::read(&path).unwrap();
        assert_eq!(read.notes[&crate::config::key(&project)], "one");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn notes_conflict_ui_events_merge_and_neither_writes_preferences() {
        let root = std::env::temp_dir().join(format!("DevTabs-data-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        assert!(
            UserData::read(&root.join("user-data.json"))
                .unwrap()
                .notes
                .is_empty()
        );
        assert!(!root.exists());
        UserData::note(&root, &project, "", "first").unwrap();
        assert!(UserData::note(&root, &project, "", "second").is_err());
        UiState::update(&root, UiChange::Opened(project.clone(), 200)).unwrap();
        UiState::update(&root, UiChange::Opened(project.clone(), 100)).unwrap();
        UiState::update(&root, UiChange::Collapsed(1, true)).unwrap();
        let state = UiState::update(&root, UiChange::Floating([1, 2, 40, 20])).unwrap();
        assert_eq!(
            state.projects[&crate::config::key(&project)].last_opened,
            200
        );
        assert!(state.overview.collapsed.contains(&1));
        assert_eq!(state.floating, Some([1, 2, 40, 20]));
        assert!(!root.join("config.toml").exists());
        let path = root.join("user-data.json");
        fs::write(&path, b"broken").unwrap();
        assert!(UserData::note(&root, &project, "", "new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn note_clears_conflict_and_independent_projects_and_whole_rectangles_are_preserved() {
        let root = std::env::temp_dir().join(format!("DevTabs-note-race-{}", uuid::Uuid::new_v4()));
        let a = root.join("a");
        let b = root.join("b");
        UserData::note(&root, &a, "", "one").unwrap();
        UserData::note(&root, &b, "", "two").unwrap();
        UserData::note(&root, &a, "one", "").unwrap();
        assert!(UserData::note(&root, &a, "one", "stale").is_err());
        let current = UserData::read(&root.join("user-data.json")).unwrap();
        assert!(!current.notes.contains_key(&crate::config::key(&a)));
        assert_eq!(current.notes[&crate::config::key(&b)], "two");
        UiState::update(&root, UiChange::Collapsed(1, true)).unwrap();
        UiState::update(&root, UiChange::Collapsed(2, true)).unwrap();
        UiState::update(&root, UiChange::Floating([1, 2, 40, 20])).unwrap();
        let state = UiState::update(&root, UiChange::Floating([9, 8, 80, 30])).unwrap();
        assert_eq!(state.floating, Some([9, 8, 80, 30]));
        assert_eq!(state.overview.collapsed, BTreeSet::from([1, 2]));
        assert!(!root.join("config.toml").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
