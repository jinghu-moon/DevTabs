//! Project-wide work is isolated from the preview/save workers and UI thread.
use anyhow::{Context, Result, bail, ensure};
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BinaryHeap},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub const INDEX_LIMIT: usize = 100_000;
pub const RESULT_LIMIT: usize = 200;
pub const SELECTION_LIMIT: usize = 128;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub width: u16,
    pub agent_ratio: u16,
    pub follow: bool,
    pub compact: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            width: 28,
            agent_ratio: 5000,
            follow: true,
            compact: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Ignored,
    Untracked,
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflict,
}
impl Status {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Ignored => "!",
            Self::Untracked => "?",
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Conflict => "U",
        }
    }
    pub fn color(self, light: bool) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            Self::Conflict | Self::Deleted => {
                if light {
                    Color::Rgb(180, 20, 30)
                } else {
                    Color::Red
                }
            }
            Self::Untracked | Self::Added => {
                if light {
                    Color::Rgb(0, 110, 50)
                } else {
                    Color::Green
                }
            }
            Self::Modified | Self::Renamed => {
                if light {
                    Color::Rgb(145, 95, 0)
                } else {
                    Color::Yellow
                }
            }
            Self::Ignored => Color::DarkGray,
        }
    }
    pub fn changed(self) -> bool {
        self != Self::Ignored
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Git {
    pub entries: BTreeMap<PathBuf, Status>,
    pub inherited: BTreeMap<PathBuf, Status>,
}
impl Git {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.inherited.clear();
    }
}
pub fn git_status(root: &Path) -> Result<Git> {
    let _perf = crate::perf::span(crate::perf::Metric::ProjectGit);
    let bytes = git_output(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--ignored=matching",
            "--untracked-files=normal",
            "--",
            ".",
        ],
    )?;
    // porcelain paths are relative to the worktree, even when -C is a subdirectory.
    let prefix = git_output(root, &["rev-parse", "--show-prefix"])?;
    let prefix = String::from_utf8_lossy(&prefix);
    Ok(parse_git(
        root,
        prefix.trim_end_matches(['\r', '\n']),
        &bytes,
    ))
}
pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    git_output_timeout(root, args, Duration::from_secs(5))
}
pub(crate) fn git_output_timeout(root: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(["--literal-pathspecs", "--no-optional-locks", "-C"])
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(windows)]
    let job = crate::platform::Job::new(0)?;
    let mut child = command.spawn().context("Start git status")?;
    #[cfg(windows)]
    if let Err(e) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    let pipe = child.stdout.take().context("Git output")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr = child.stderr.take().context("Git diagnostic")?;
    let errors = thread::spawn(move || {
        let mut bytes = vec![];
        for chunk in std::io::Read::bytes(std::io::BufReader::new(stderr)) {
            let b = chunk?;
            if bytes.len() < 16384 {
                bytes.push(b);
            }
        }
        Ok::<_, std::io::Error>(bytes)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            #[cfg(windows)]
            let _ = job.terminate();
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git status timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    #[cfg(windows)]
    {
        let _ = job.terminate();
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Git reader failed"))??;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "Git output exceeds 8 MiB");
    let errors = errors
        .join()
        .map_err(|_| anyhow::anyhow!("Git diagnostic reader failed"))??;
    ensure!(
        status.success(),
        "Git rejected operation ({}): {}",
        status,
        crate::preview::safe_text(&String::from_utf8_lossy(&errors))
    );
    Ok(bytes)
}
pub fn parse_git(root: &Path, prefix: &str, bytes: &[u8]) -> Git {
    let mut out = Git::new();
    let mut parts = bytes.split(|b| *b == 0);
    while let Some(record) = parts.next() {
        if record.len() < 4 {
            continue;
        }
        let xy = &record[..2];
        let status = if xy == b"!!" {
            Status::Ignored
        } else if xy == b"??" {
            Status::Untracked
        } else if xy.contains(&b'U') || xy == b"AA" || xy == b"DD" {
            Status::Conflict
        } else if xy.contains(&b'R') {
            Status::Renamed
        } else if xy.contains(&b'D') {
            Status::Deleted
        } else if xy.contains(&b'A') {
            Status::Added
        } else {
            Status::Modified
        };
        if xy.contains(&b'R') || xy.contains(&b'C') {
            let _ = parts.next();
        }
        let Ok(name) = std::str::from_utf8(&record[3..]) else {
            continue;
        };
        let Some(name) = name.strip_prefix(prefix) else {
            continue;
        };
        let p = root.join(name.trim_end_matches('/'));
        if !p.starts_with(root) {
            continue;
        }
        if name.ends_with('/') && matches!(status, Status::Ignored | Status::Untracked) {
            out.inherited.insert(p.clone(), status);
        }
        // Include directory aggregates; ignored/untracked directory entries also cover descendants.
        let mut cursor = Some(p.as_path());
        while let Some(path) = cursor {
            if !path.starts_with(root) {
                break;
            }
            out.entries
                .entry(path.to_owned())
                .and_modify(|old| *old = (*old).max(status))
                .or_insert(status);
            if path == root {
                break;
            }
            cursor = path.parent();
        }
    }
    out
}
pub fn status_for(git: &Git, path: &Path) -> Option<Status> {
    if let Some(status) = git.entries.get(path) {
        return Some(*status);
    }
    path.ancestors()
        .skip(1)
        .find_map(|p| git.inherited.get(p).copied())
}
pub fn relocate(path: &Path, old: &Path, new: &Path) -> PathBuf {
    let relative = path.strip_prefix(old).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        new.to_owned()
    } else {
        new.join(relative)
    }
}

#[derive(Clone, Debug)]
pub enum Operation {
    File { parent: PathBuf, name: String },
    Directory { parent: PathBuf, name: String },
    Rename { path: PathBuf, name: String },
    Trash(Vec<PathBuf>),
}
impl Operation {
    pub fn affected(&self) -> Vec<PathBuf> {
        match self {
            Self::Rename { path, .. } => vec![path.clone()],
            Self::Trash(paths) => paths.clone(),
            _ => Vec::new(),
        }
    }
}
#[derive(Debug, Default)]
pub struct Change {
    pub paths: Vec<(PathBuf, Option<PathBuf>)>,
    pub error: Option<String>,
}
pub fn child_name(parent: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty() && name.chars().count() <= 240,
        "Name must contain 1–240 characters"
    );
    ensure!(
        !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "Invalid filename"
    );
    ensure!(
        Path::new(name).components().count() == 1
            && matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            ),
        "Enter a single filename"
    );
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    ensure!(
        !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !["COM", "LPT"].iter().any(|p| base
                .strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))),
        "Reserved filename"
    );
    ensure!(
        !name.eq_ignore_ascii_case(".git"),
        "Git metadata is protected"
    );
    Ok(parent.join(name))
}
fn guard(root: &Path, path: &Path, allow_root: bool) -> Result<()> {
    ensure!(
        path.starts_with(root) && (allow_root || path != root),
        "Operation must stay inside the project; project root is protected"
    );
    ensure!(
        !path
            .strip_prefix(root)?
            .components()
            .any(|c| !matches!(c, Component::Normal(_))
                || c.as_os_str().eq_ignore_ascii_case(".git")),
        "Unsafe project path"
    );
    let canonical_root = dunce::canonicalize(root)?;
    // Reject directory links/junctions, including the final component. Never traverse outside root.
    let mut at = root.to_path_buf();
    for part in path.strip_prefix(root)?.components() {
        at.push(part);
        let meta = fs::symlink_metadata(&at)?;
        ensure!(
            !crate::files::is_link(&meta),
            "File operations on links are disabled"
        );
        ensure!(
            dunce::canonicalize(&at)?.starts_with(&canonical_root),
            "Path escapes project"
        );
    }
    Ok(())
}
pub fn apply_change(root: &Path, op: Operation) -> Change {
    let _perf = crate::perf::span(crate::perf::Metric::ProjectChange);
    let mut change = Change::default();
    let directory = matches!(&op, Operation::Directory { .. });
    let result = (|| -> Result<()> {
        match op {
            Operation::File { parent, name } | Operation::Directory { parent, name } => {
                guard(root, &parent, true)?;
                let path = child_name(&parent, &name)?;
                create_entry(&path, directory)?;
                change.paths.push((path.clone(), Some(path)));
            }
            Operation::Rename { path, name } => {
                guard(root, &path, false)?;
                let target = child_name(path.parent().context("No parent")?, &name)?;
                ensure!(path != target, "Name is unchanged");
                rename_new(&path, &target)?;
                change.paths.push((path, Some(target)));
            }
            Operation::Trash(paths) => {
                ensure!(
                    !paths.is_empty() && paths.len() <= SELECTION_LIMIT,
                    "Select 1–128 entries"
                );
                let mut paths = paths;
                paths.sort();
                paths.dedup();
                let parents = paths.clone();
                paths.retain(|p| !parents.iter().any(|a| a != p && p.starts_with(a)));
                for p in &paths {
                    guard(root, p, false)?;
                }
                for p in paths {
                    trash::delete(&p).with_context(|| format!("Recycle {}", p.display()))?;
                    change.paths.push((p, None));
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        change.error = Some(format!("{e:#}"));
    }
    change
}
fn create_entry(path: &Path, directory: bool) -> Result<()> {
    if directory {
        fs::create_dir(path)?;
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
    }
    Ok(())
}
fn rename_new(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        ensure!(
            unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(a.as_ptr(), b.as_ptr()) }
                != 0,
            "Rename failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(windows))]
    {
        ensure!(fs::symlink_metadata(to).is_err(), "Target already exists");
        fs::rename(from, to)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub hidden: bool,
    pub ignored: bool,
    pub revision: u64,
}
pub struct Search {
    pub paths: Vec<PathBuf>,
    pub total: usize,
    pub limited: bool,
    pub errors: usize,
    pub scanned: usize,
    pub elapsed: Duration,
}
struct Index {
    scope: Scope,
    paths: Vec<(String, PathBuf)>,
    limited: bool,
    errors: usize,
}
fn index(root: &Path, scope: Scope, cancel: &AtomicU64, id: u64) -> Option<Index> {
    let _perf = crate::perf::span(crate::perf::Metric::ProjectIndex);
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false)
        .ignore(!scope.ignored)
        .git_ignore(!scope.ignored)
        .git_exclude(!scope.ignored)
        .git_global(!scope.ignored)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(64))
        .filter_entry({
            let hidden = scope.hidden;
            move |e| {
                e.file_name() != ".git"
                    && (hidden
                        || e.metadata()
                            .is_ok_and(|m| !crate::files::hidden_attribute(&m)))
            }
        });
    let mut index = Index {
        scope,
        paths: Vec::new(),
        limited: false,
        errors: 0,
    };
    let mut count = 0;
    for entry in builder.build() {
        if cancel.load(Ordering::Relaxed) != id {
            return None;
        }
        count += 1;
        if count > INDEX_LIMIT {
            index.limited = true;
            break;
        }
        match entry {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => {
                let p = e.into_path();
                let label = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                if label.len() <= 4096 {
                    index.paths.push((label, p));
                } else {
                    index.errors += 1;
                }
            }
            Ok(e) if e.depth() == 64 && e.file_type().is_some_and(|t| t.is_dir()) => {
                index.limited = true
            }
            Err(_) => index.errors += 1,
            _ => {}
        }
    }
    index.paths.sort_by(|a, b| a.0.cmp(&b.0));
    Some(index)
}
fn search(index: &Index, query: &str, cancel: &AtomicU64, id: u64) -> Option<Search> {
    let _perf = crate::perf::span(crate::perf::Metric::ProjectSearch);
    let start = Instant::now();
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::new(
        &query.replace('\\', "/"),
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let mut heap = BinaryHeap::new();
    let mut total = 0;
    for (i, (label, _)) in index.paths.iter().enumerate() {
        if i % 256 == 0 && cancel.load(Ordering::Relaxed) != id {
            return None;
        }
        if let Some(score) = pattern.score(Utf32Str::new(label, &mut buf), &mut matcher) {
            total += 1;
            heap.push(std::cmp::Reverse((score, std::cmp::Reverse(i))));
            if heap.len() > RESULT_LIMIT {
                heap.pop();
            }
        }
    }
    let mut hits = heap.into_iter().map(|r| r.0).collect::<Vec<_>>();
    hits.sort_by(|a, b| b.cmp(a));
    Some(Search {
        paths: hits
            .into_iter()
            .map(|(_, i)| index.paths[i.0].1.clone())
            .collect(),
        total,
        limited: index.limited,
        errors: index.errors,
        scanned: index.paths.len(),
        elapsed: start.elapsed(),
    })
}
pub enum Job {
    Search {
        id: u64,
        query: String,
        scope: Scope,
    },
    Git,
    Change(Operation),
    LoadPreferences,
    Language(crate::i18n::Language),
    Theme(crate::appearance::Theme),
    Validate(Vec<PathBuf>),
    External(Vec<PathBuf>),
}
pub enum Completed {
    Search { id: u64, result: Search },
    Git(Result<Git>),
    Change(Change),
    Preferences(Result<ViewDefaults>),
    SavedPreferences(Result<()>),
    Input(Result<Vec<PathBuf>>),
    External(Result<()>),
}
pub struct ViewDefaults {
    pub sidebar: Preferences,
    pub hidden: bool,
    pub ignored: bool,
    pub preview: crate::settings::model::Preview,
}
pub struct Worker {
    tx: SyncSender<Job>,
    pub rx: Receiver<Completed>,
    cancel: Arc<AtomicU64>,
}
impl Worker {
    pub fn new(root: PathBuf) -> Self {
        let (tx, jobs) = mpsc::sync_channel(8);
        let (done, rx) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicU64::new(0));
        let flag = cancel.clone();
        thread::Builder::new()
            .name("project-explorer".into())
            .spawn(move || {
                let mut cache: Option<Index> = None;
                while let Ok(job) = jobs.recv() {
                    let output = match job {
                        Job::Search { id, query, scope } => {
                            if flag.load(Ordering::Relaxed) != id {
                                continue;
                            }
                            if cache.as_ref().is_none_or(|c| c.scope != scope) {
                                cache = index(&root, scope, &flag, id);
                            }
                            let Some(result) =
                                cache.as_ref().and_then(|c| search(c, &query, &flag, id))
                            else {
                                continue;
                            };
                            Completed::Search { id, result }
                        }
                        Job::Git => Completed::Git(git_status(&root)),
                        Job::Change(op) => {
                            cache = None;
                            let mut change = apply_change(&root, op);
                            let store = crate::config::home().join("user-data.json");
                            if store.exists()
                                && change
                                    .paths
                                    .iter()
                                    .any(|(old, new)| new.as_ref() != Some(old))
                                && let Err(e) = crate::bookmarks::update(
                                    &store,
                                    &root,
                                    crate::bookmarks::Operation::Remap(change.paths.clone()),
                                )
                            {
                                let warning = format!(
                                    "Files changed, but bookmarks could not be updated: {e:#}"
                                );
                                change.error =
                                    Some(change.error.map_or(warning.clone(), |old| {
                                        format!("{old}; {warning}")
                                    }));
                            }
                            Completed::Change(change)
                        }
                        Job::LoadPreferences => Completed::Preferences(
                            crate::settings::store::SettingsStore::current()
                                .read()
                                .map(|s| {
                                    let sidebar = s.effective.resolve_sidebar(&root);
                                    ViewDefaults {
                                        sidebar: crate::settings::projection::sidebar(&sidebar),
                                        hidden: sidebar.show_hidden.unwrap_or(false),
                                        ignored: sidebar.show_ignored.unwrap_or(false),
                                        preview: s.effective.preview,
                                    }
                                }),
                        ),
                        Job::Language(language) => Completed::SavedPreferences(
                            crate::config::Config::update(|c| {
                                c.language = language;
                                Ok(())
                            })
                            .map(|_| ()),
                        ),
                        Job::Theme(theme) => Completed::SavedPreferences(
                            crate::config::Config::update(|c| {
                                c.theme = theme;
                                Ok(())
                            })
                            .map(|_| ()),
                        ),
                        Job::Validate(paths) => Completed::Input(validate_paths(paths)),
                        Job::External(paths) => Completed::External((|| {
                            ensure!(paths.len() <= SELECTION_LIMIT, "Too many files");
                            for p in paths {
                                crate::files::external(&p, false)?;
                            }
                            Ok(())
                        })()),
                    };
                    if done.send(output).is_err() {
                        break;
                    }
                }
            })
            .expect("Project worker");
        Self { tx, rx, cancel }
    }
    pub fn send(&self, job: Job) -> Result<()> {
        self.tx
            .try_send(job)
            .map_err(|_| anyhow::anyhow!(crate::i18n::t("ui.project_worker_busy_retry_shortly")))
    }
    pub fn search(&self, id: u64, query: String, scope: Scope) -> Result<()> {
        self.cancel.store(id, Ordering::Relaxed);
        self.send(Job::Search { id, query, scope })
    }
    pub fn cancel(&self) {
        self.cancel.fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub fn validate_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    ensure!(
        !paths.is_empty() && paths.len() <= SELECTION_LIMIT,
        "Select 1–128 files"
    );
    let mut size = 0;
    for p in &paths {
        ensure!(
            fs::metadata(p).is_ok_and(|m| m.is_file()),
            "File no longer exists: {}",
            p.display()
        );
        size += crate::files::path_input(p)?.len() + 1;
    }
    ensure!(size <= 32 * 1024, "Combined paths exceed 32 KiB");
    Ok(paths)
}
pub fn paths_input(paths: &[PathBuf]) -> Result<String> {
    ensure!(
        !paths.is_empty() && paths.len() <= SELECTION_LIMIT,
        "Select 1–128 files"
    );
    let text = paths
        .iter()
        .map(|p| crate::files::path_input(p))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    ensure!(text.len() <= 32 * 1024, "Combined paths exceed 32 KiB");
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuzzy_result_heap_is_bounded_deterministic_and_cancellable() {
        let index = Index {
            scope: Scope {
                hidden: false,
                ignored: false,
                revision: 0,
            },
            paths: (0..10_000)
                .map(|i| {
                    (
                        format!("file{i:05}.rs"),
                        PathBuf::from(format!("file{i:05}.rs")),
                    )
                })
                .collect(),
            limited: false,
            errors: 0,
        };
        let flag = AtomicU64::new(1);
        let hits = search(&index, "", &flag, 1).unwrap();
        assert_eq!(hits.total, 10_000);
        assert_eq!(hits.paths.len(), RESULT_LIMIT);
        assert_eq!(hits.paths[0], PathBuf::from("file00000.rs"));
        assert_eq!(hits.paths[199], PathBuf::from("file00199.rs"));
        flag.store(2, Ordering::Relaxed);
        assert!(search(&index, "rs", &flag, 1).is_none());
    }
}
