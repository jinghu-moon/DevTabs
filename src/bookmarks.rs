//! Persistent reading bookmarks. Worker-only I/O, bounded data and atomic updates.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: uuid::Uuid,
    pub project: PathBuf,
    pub path: PathBuf,
    pub byte: usize,
    pub line: usize,
    pub anchor: String,
    pub markdown: bool,
    pub note: String,
}
#[derive(Debug)]
pub enum Operation {
    List,
    Put(Entry),
    Delete(uuid::Uuid),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    entries: Vec<Entry>,
}
const MAX_FILE: u64 = 8 * 1024 * 1024;

pub fn update(file: &Path, project: &Path, operation: Operation) -> Result<Vec<Entry>> {
    let _lock = crate::config::ConfigLock::acquire()?;
    let mut store = match std::fs::File::open(file) {
        Ok(f) => {
            ensure!(
                f.metadata()?.len() <= MAX_FILE,
                "Bookmark store exceeds 8 MiB"
            );
            let mut bytes = Vec::new();
            f.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_FILE,
                "Bookmark store exceeds 8 MiB"
            );
            let s: Store = serde_json::from_slice(&bytes)
                .context("Cannot read bookmarks; original file preserved")?;
            ensure!(
                s.version == 1 && s.entries.len() <= 4096,
                "Unsupported bookmark store"
            );
            s
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store {
            version: 1,
            entries: vec![],
        },
        Err(e) => return Err(e.into()),
    };
    let key = crate::config::key(project);
    let belongs = |e: &Entry| crate::config::key(&e.project) == key;
    let write = !matches!(operation, Operation::List);
    match operation {
        Operation::List => {}
        Operation::Put(entry) => {
            ensure!(
                belongs(&entry) && entry.path.is_absolute() && entry.project.is_absolute(),
                "Invalid bookmark project/path"
            );
            ensure!(
                entry.note.chars().count() <= 256 && entry.anchor.chars().count() <= 128,
                "Bookmark text exceeds limit"
            );
            store.entries.retain(|e| !belongs(e) || e.id != entry.id);
            ensure!(
                store.entries.len() < 4096
                    && store.entries.iter().filter(|e| belongs(e)).count() < 128,
                "Bookmark limit reached (128 per project)"
            );
            store.entries.push(entry);
        }
        Operation::Delete(id) => store.entries.retain(|e| !belongs(e) || e.id != id),
    }
    if write {
        let bytes = serde_json::to_vec_pretty(&store)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE,
            "Bookmark store exceeds 8 MiB"
        );
        crate::config::atomic_write(file, &bytes)?;
    }
    Ok(store.entries.into_iter().filter(belongs).collect())
}

/// Match the closest unchanged source line after edits; otherwise use the saved
/// line, bounded to the currently loaded source. Never seek inside a UTF-8 codepoint.
pub fn position(entry: &Entry, source: &str) -> usize {
    let lines = source
        .split_inclusive('\n')
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len();
            Some((start, line.trim_end_matches(['\r', '\n'])))
        })
        .collect::<Vec<_>>();
    if let Some((_, (at, _))) = lines
        .iter()
        .enumerate()
        .filter(|(_, (_, line))| !entry.anchor.is_empty() && line.starts_with(&entry.anchor))
        .min_by_key(|(i, _)| i.abs_diff(entry.line))
    {
        return *at;
    }
    lines.get(entry.line).map_or_else(
        || {
            let mut byte = entry.byte.min(source.len());
            while !source.is_char_boundary(byte) {
                byte -= 1;
            }
            byte
        },
        |(at, _)| *at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("DevTabs-bookmarks-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn entry(&self) -> Entry {
            Entry {
                id: uuid::Uuid::new_v4(),
                project: self.0.clone(),
                path: self.0.join("文件.md"),
                byte: 8,
                line: 1,
                anchor: "中文".into(),
                markdown: true,
                note: "继续阅读".into(),
            }
        }
        fn file(&self) -> PathBuf {
            self.0.join("bookmarks.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn store_roundtrip_updates_notes_and_keeps_projects_separate() {
        let f = Fixture::new();
        let e = f.entry();
        let id = e.id;
        assert!(update(&f.file(), &f.0, Operation::List).unwrap().is_empty());
        assert!(!f.file().exists());
        update(&f.file(), &f.0, Operation::Put(e.clone())).unwrap();
        let other = f.0.join("other");
        let mut e2 = f.entry();
        e2.project = other.clone();
        update(&f.file(), &other, Operation::Put(e2)).unwrap();
        let mut e = e;
        e.note = "修改备注".into();
        let entries = update(&f.file(), &f.0, Operation::Put(e)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].note, "修改备注");
        assert!(
            update(&f.file(), &f.0, Operation::Delete(id))
                .unwrap()
                .is_empty()
        );
        assert_eq!(update(&f.file(), &other, Operation::List).unwrap().len(), 1);
    }
    #[test]
    fn corrupt_unsupported_oversized_and_invalid_updates_preserve_original() {
        let f = Fixture::new();
        for bytes in [
            b"{broken".to_vec(),
            b"{\"version\":2,\"entries\":[]}".to_vec(),
            vec![b' '; MAX_FILE as usize + 1],
        ] {
            std::fs::write(f.file(), &bytes).unwrap();
            assert!(update(&f.file(), &f.0, Operation::Put(f.entry())).is_err());
            assert_eq!(std::fs::read(f.file()).unwrap(), bytes);
        }
        std::fs::remove_file(f.file()).unwrap();
        update(&f.file(), &f.0, Operation::Put(f.entry())).unwrap();
        let bytes = std::fs::read(f.file()).unwrap();
        let mut invalid = f.entry();
        invalid.note = "x".repeat(257);
        assert!(update(&f.file(), &f.0, Operation::Put(invalid)).is_err());
        let mut invalid = f.entry();
        invalid.path = "relative.md".into();
        assert!(update(&f.file(), &f.0, Operation::Put(invalid)).is_err());
        assert_eq!(std::fs::read(f.file()).unwrap(), bytes);
    }
    #[test]
    fn concurrent_writers_merge_without_lost_bookmarks() {
        let f = Fixture::new();
        let handles = (0..12)
            .map(|_| {
                let file = f.file();
                let project = f.0.clone();
                let entry = f.entry();
                std::thread::spawn(move || update(&file, &project, Operation::Put(entry)).unwrap())
            })
            .collect::<Vec<_>>();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(update(&f.file(), &f.0, Operation::List).unwrap().len(), 12);
    }
    #[test]
    fn bookmark_budget_rejects_add_but_allows_note_edits() {
        let f = Fixture::new();
        let entries = (0..128).map(|_| f.entry()).collect::<Vec<_>>();
        std::fs::write(
            f.file(),
            serde_json::to_vec(&Store {
                version: 1,
                entries: entries.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        let bytes = std::fs::read(f.file()).unwrap();
        assert!(update(&f.file(), &f.0, Operation::Put(f.entry())).is_err());
        assert_eq!(std::fs::read(f.file()).unwrap(), bytes);
        let mut e = entries[0].clone();
        e.note = "edit at limit".into();
        assert_eq!(
            update(&f.file(), &f.0, Operation::Put(e)).unwrap().len(),
            128
        );
    }
    #[test]
    fn position_finds_nearest_anchor_after_edits_and_bounds_utf8_fallback() {
        let f = Fixture::new();
        let mut e = f.entry();
        assert_eq!(position(&e, "inserted\nfirst\n中文\nlast"), 15);
        e.line = 3;
        assert_eq!(position(&e, "中文\nx\ny\n中文\n"), 11);
        e.line = 100;
        e.byte = 4;
        e.anchor = "missing".into();
        assert_eq!(position(&e, "中文"), 3);
        assert_eq!(position(&e, ""), 0);
        e.line = 1;
        assert_eq!(position(&e, "changed\nsecond\n"), 8);
    }
}
