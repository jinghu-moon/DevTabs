//! Bounded, immutable records of disk changes, independent of Agent context.
pub mod service;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Git,
    Observe,
    Snapshot(Vec<PathBuf>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub logical_round: String,
    pub order: u64,
    pub file_count: usize,
    pub id: String,
    pub task: String,
    pub run: String,
    pub mode: Mode,
    pub result: Option<u64>,
    pub started: u64,
    pub ended: Option<u64>,
    pub revision: u64,
    pub read: bool,
    pub late: bool,
    pub reasons: Vec<String>,
    pub changes: Vec<Change>,
    pub root: PathBuf,
    pub head: Option<String>,
    pub baseline: BTreeMap<PathBuf, Content>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Content {
    Missing,
    Blob(String),
    Unknown(String),
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub file: usize,
    pub round: u64,
    pub task: u64,
    pub global: u64,
    pub paths: usize,
    pub time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file: 8 * 1024 * 1024,
            round: 128 * 1024 * 1024,
            task: 512 * 1024 * 1024,
            global: 2 * 1024 * 1024 * 1024,
            paths: 200_000,
            time: Duration::from_secs(5),
        }
    }
}
pub struct Recorder {
    pub home: PathBuf,
    pub limits: Limits,
}
struct Capture {
    started: Instant,
    bytes: u64,
    reasons: Vec<String>,
    keys: BTreeSet<String>,
    task_bytes: u64,
    global_bytes: u64,
}
impl Capture {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: 0,
            reasons: vec![],
            keys: BTreeSet::new(),
            task_bytes: 0,
            global_bytes: 0,
        }
    }
}
fn valid_path(p: &Path) -> bool {
    !p.as_os_str().is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}
fn id(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 128
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid identity"
    );
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn nul_paths(bytes: &[u8]) -> Result<BTreeSet<PathBuf>> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let p = PathBuf::from(std::str::from_utf8(p)?);
            ensure!(valid_path(&p), "Unsafe or unrepresentable Git path");
            Ok(p)
        })
        .collect()
}
impl Recorder {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            limits: Limits::default(),
        }
    }
    pub fn begin(
        &self,
        task: &str,
        run: &str,
        root: &Path,
        mode: Mode,
        late: bool,
    ) -> Result<Record> {
        let _writer = crate::config::ConfigLock::acquire_at(&self.home)?;
        id(task)?;
        id(run)?;
        let root = if mode == Mode::Git {
            crate::tasks::Repository::discover(root)?.root
        } else {
            dunce::canonicalize(root)?
        };
        let mut r = Record {
            logical_round: uuid::Uuid::new_v4().to_string(),
            order: order(),
            file_count: 0,
            id: uuid::Uuid::new_v4().to_string(),
            task: task.into(),
            run: run.into(),
            mode,
            result: None,
            started: crate::config::now(),
            ended: None,
            revision: 0,
            read: false,
            late,
            reasons: vec![],
            changes: vec![],
            root,
            head: None,
            baseline: BTreeMap::new(),
        };
        let mut cap = self.capture(task)?;
        let paths = match &r.mode {
            Mode::Git => {
                r.head = Some(
                    String::from_utf8(self.git(
                        &r.root,
                        &["rev-parse", "--verify", "HEAD^{commit}"],
                        &cap,
                    )?)?
                    .trim()
                    .to_owned(),
                );
                self.exceptions(&r.root, &cap)?
            }
            Mode::Snapshot(scope) => self.scope_paths(&r.root, scope, &cap)?,
            Mode::Observe => BTreeSet::new(),
        };
        for p in paths {
            if r.baseline.len() >= self.limits.paths || cap.started.elapsed() >= self.limits.time {
                cap.reasons.push("Baseline path/time budget reached".into());
                break;
            }
            let content = self.disk(&r.root, &p, &mut cap);
            if let Content::Unknown(e) = &content {
                cap.reasons.push(e.clone());
            }
            r.baseline.insert(p, content);
        }
        r.reasons = cap.reasons;
        self.save(&r)?;
        Ok(r)
    }
    pub fn finish(&self, mut r: Record) -> Result<Record> {
        let _writer = crate::config::ConfigLock::acquire_at(&self.home)?;
        ensure!(r.ended.is_none(), "Round already ended");
        let mut cap = self.capture(&r.task)?;
        let mut paths = r.baseline.keys().cloned().collect::<BTreeSet<_>>();
        let queried = (|| -> Result<()> {
            match &r.mode {
                Mode::Git => {
                    paths.extend(self.exceptions(&r.root, &cap)?);
                    let head = r.head.as_ref().context("Missing starting commit")?;
                    paths.extend(nul_paths(&self.git(
                        &r.root,
                        &[
                            "diff",
                            "--name-only",
                            "-z",
                            "--no-renames",
                            head,
                            "HEAD",
                            "--",
                        ],
                        &cap,
                    )?)?);
                }
                Mode::Snapshot(scope) => paths.extend(self.scope_paths(&r.root, scope, &cap)?),
                Mode::Observe => {}
            }
            Ok(())
        })();
        if let Err(e) = queried {
            cap.reasons
                .push(format!("Candidate coverage incomplete: {e:#}"));
        }
        if r.mode == Mode::Git
            && paths.iter().any(|p| {
                p.file_name()
                    .is_some_and(|n| n == ".gitattributes" || n == ".gitignore")
            })
        {
            cap.reasons.push("Git attribute/ignore rules changed or were dirty; path coverage and representation may be partial".into());
        }
        for p in paths {
            if r.changes.len() >= self.limits.paths || cap.started.elapsed() >= self.limits.time {
                cap.reasons.push("End path/time budget reached".into());
                break;
            }
            let before = if let Some(c) = r.baseline.get(&p) {
                c.clone()
            } else if r.mode == Mode::Git {
                match self.base_content(&r, &p, &mut cap) {
                    Ok(c) => c,
                    Err(e) => Content::Unknown(format!("Starting content unavailable: {e:#}")),
                }
            } else if r.reasons.is_empty() {
                Content::Missing
            } else {
                Content::Unknown("Starting scope coverage was partial; absence unknown".into())
            };
            let after = self.disk(&r.root, &p, &mut cap);
            let reason = match (&before, &after) {
                (Content::Unknown(e), _) | (_, Content::Unknown(e)) => Some(e.clone()),
                _ => None,
            };
            if let Some(e) = &reason {
                cap.reasons.push(e.clone());
            }
            let key = |c: Content| match c {
                Content::Blob(k) => Some(k),
                _ => None,
            };
            let a = key(before);
            let b = key(after);
            if a != b || reason.is_some() {
                r.changes.push(Change {
                    path: p,
                    before: a,
                    after: b,
                    reason,
                });
            }
        }
        r.reasons.extend(cap.reasons);
        r.reasons.sort();
        r.reasons.dedup();
        r.baseline.clear(); // Both sides of confirmed differences are now owned CAS objects.
        r.order = order();
        r.file_count = r.changes.len();
        r.ended = Some(crate::config::now());
        r.revision += 1;
        self.save(&r)?;
        Ok(r)
    }
    fn git(&self, root: &Path, args: &[&str], cap: &Capture) -> Result<Vec<u8>> {
        ensure!(
            cap.started.elapsed() < self.limits.time,
            "Capture deadline exceeded"
        );
        crate::explorer::git_output_timeout(
            root,
            args,
            self.limits.time.saturating_sub(cap.started.elapsed()),
        )
    }
    fn exceptions(&self, root: &Path, cap: &Capture) -> Result<BTreeSet<PathBuf>> {
        let mut paths = nul_paths(&self.git(
            root,
            &["diff", "HEAD", "--name-only", "-z", "--no-renames", "--"],
            cap,
        )?)?;
        paths.extend(nul_paths(&self.git(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            cap,
        )?)?);
        ensure!(
            paths.len() <= self.limits.paths,
            "Candidate path budget exceeded"
        );
        Ok(paths)
    }
    fn scope_paths(
        &self,
        root: &Path,
        scope: &[PathBuf],
        cap: &Capture,
    ) -> Result<BTreeSet<PathBuf>> {
        ensure!(
            !scope.is_empty(),
            "Choose explicit snapshot directories or files"
        );
        let mut out = BTreeSet::new();
        for p in scope {
            ensure!(
                valid_path(p),
                "Scope must be a relative path within the task"
            );
            let selected = root.join(p);
            ensure!(
                dunce::canonicalize(&selected)?.starts_with(root),
                "Scope leaves task"
            );
            for entry in ignore::WalkBuilder::new(selected)
                .follow_links(false)
                .filter_entry(|e| !excluded(e.path()))
                .hidden(false)
                .build()
            {
                ensure!(
                    cap.started.elapsed() < self.limits.time,
                    "Snapshot enumeration deadline exceeded"
                );
                let entry = entry?;
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    let p = entry.path().strip_prefix(root)?.to_path_buf();
                    if !excluded(&p) {
                        out.insert(p);
                    }
                    ensure!(
                        out.len() <= self.limits.paths,
                        "Snapshot path budget exceeded"
                    );
                }
            }
        }
        Ok(out)
    }
    fn disk(&self, root: &Path, path: &Path, cap: &mut Capture) -> Content {
        match (|| -> Result<Content> {
            ensure!(valid_path(path), "Path leaves task");
            let p = root.join(path);
            let meta = match fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Content::Missing),
                Err(e) => return Err(e.into()),
            };
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unsupported file type or link"
            );
            ensure!(
                dunce::canonicalize(&p)?.starts_with(root),
                "Link leaves task"
            );
            ensure!(
                meta.len() <= self.limits.file as u64,
                "File content budget exceeded"
            );
            ensure!(
                cap.started.elapsed() < self.limits.time,
                "Capture deadline exceeded"
            );
            let mut bytes = vec![];
            fs::File::open(&p)?
                .take(self.limits.file as u64 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= self.limits.file, "File grew beyond budget");
            let after = fs::metadata(&p)?;
            ensure!(
                meta.len() == after.len() && meta.modified()? == after.modified()?,
                "File changed while being captured"
            );
            Ok(Content::Blob(self.put(&bytes, cap)?))
        })() {
            Ok(c) => c,
            Err(e) => Content::Unknown(format!("{}: {e:#}", path.display())),
        }
    }
    fn base_content(&self, r: &Record, p: &Path, cap: &mut Capture) -> Result<Content> {
        let path = p.to_str().context("Non UTF-8 path")?.replace('\\', "/");
        let object = format!("{}:{path}", r.head.as_ref().context("Missing commit")?);
        // Determine absence from a successful tree query, never from a failed blob read.
        let tree = self.git(
            &r.root,
            &["ls-tree", "-z", r.head.as_ref().unwrap(), "--", &path],
            cap,
        )?;
        if tree.is_empty() {
            return Ok(Content::Missing);
        }
        ensure!(tree.starts_with(b"100"), "Unsupported Git tree entry");
        let attrs = self.git(
            &r.root,
            &[
                "check-attr",
                "-z",
                "filter",
                "working-tree-encoding",
                "eol",
                "text",
                "--",
                &path,
            ],
            cap,
        )?;
        let a = attrs.split(|b| *b == 0).collect::<Vec<_>>();
        let attr = |name: &[u8]| {
            a.chunks(3)
                .find(|v| v.len() == 3 && v[1] == name)
                .map(|v| v[2])
                .unwrap_or(b"unspecified")
        };
        ensure!(
            [b"unspecified".as_slice(), b"unset"].contains(&attr(b"filter"))
                && attr(b"working-tree-encoding") == b"unspecified",
            "External filter/encoding not supported"
        );
        let size = String::from_utf8(self.git(&r.root, &["cat-file", "-s", &object], cap)?)?
            .trim()
            .parse::<usize>()?;
        ensure!(
            size <= self.limits.file,
            "Starting blob exceeds content budget"
        );
        let mut bytes = self.git(&r.root, &["cat-file", "blob", &object], cap)?;
        let crlf = attr(b"eol") == b"crlf"
            || (attr(b"eol") == b"unspecified"
                && attr(b"text") != b"unset"
                && self
                    .git(&r.root, &["config", "--get", "core.autocrlf"], cap)
                    .ok()
                    .is_some_and(|b| b.starts_with(b"true")));
        if crlf && !bytes.contains(&0) && !bytes.windows(2).any(|w| w == b"\r\n") {
            bytes = bytes
                .into_iter()
                .flat_map(|b| {
                    if b == b'\n' {
                        vec![b'\r', b'\n']
                    } else {
                        vec![b]
                    }
                })
                .collect();
        }
        Ok(Content::Blob(self.put(&bytes, cap)?))
    }
    fn put(&self, bytes: &[u8], cap: &mut Capture) -> Result<String> {
        ensure!(
            bytes.len() <= self.limits.file,
            "File content budget exceeded"
        );
        ensure!(
            cap.bytes + bytes.len() as u64 <= self.limits.round,
            "Round content budget exceeded"
        );
        let key = digest(bytes);
        let path = self.home.join("objects").join(&key);
        if !cap.keys.contains(&key) {
            ensure!(
                cap.task_bytes + bytes.len() as u64 <= self.limits.task,
                "Task history content budget exceeded"
            );
        }
        if !path.exists() {
            ensure!(
                cap.global_bytes + bytes.len() as u64 <= self.limits.global,
                "History storage budget exceeded"
            );
            crate::config::atomic_write(&path, bytes)?;
            cap.global_bytes += bytes.len() as u64;
        }
        if cap.keys.insert(key.clone()) {
            cap.task_bytes += bytes.len() as u64;
        }
        cap.bytes += bytes.len() as u64;
        Ok(key)
    }
    fn capture(&self, task: &str) -> Result<Capture> {
        let mut cap = Capture::new();
        if let Ok(entries) = fs::read_dir(self.home.join("objects")) {
            for entry in entries {
                cap.global_bytes += entry?.metadata()?.len();
            }
        }
        for r in self.list(task)? {
            for k in keys(&r) {
                if cap.keys.insert(k.clone()) {
                    cap.task_bytes +=
                        fs::metadata(self.home.join("objects").join(&k)).map_or(0, |m| m.len());
                }
            }
        }
        Ok(cap)
    }
    /// Remove manifests first. Never reclaim an active or explicitly pinned record.
    pub fn collect(&self, task: &str, pinned: &BTreeSet<String>) -> Result<()> {
        let _writer = crate::config::ConfigLock::acquire_at(&self.home)?;
        let records = self.list(task)?;
        let mut retained = BTreeSet::new();
        for r in records.iter().filter(|r| r.ended.is_some()) {
            if retained.len() < 20 {
                retained.insert(r.logical_round.clone());
            }
        }
        for r in records
            .iter()
            .filter(|r| r.ended.is_some() && !retained.contains(&r.logical_round))
        {
            if !pinned.contains(&r.id) {
                fs::remove_file(
                    self.home
                        .join("rounds")
                        .join(task)
                        .join(format!("{}.json", r.id)),
                )?;
            }
        }
        let mut live = BTreeSet::new();
        if let Ok(tasks) = fs::read_dir(self.home.join("rounds")) {
            for t in tasks {
                let t = t?;
                if let Some(task) = t.file_name().to_str() {
                    for r in self.list(task)? {
                        live.extend(keys(&r));
                    }
                }
            }
        }
        if let Ok(objects) = fs::read_dir(self.home.join("objects")) {
            for o in objects {
                let o = o?;
                if let Some(key) = o.file_name().to_str()
                    && !live.contains(key)
                {
                    fs::remove_file(o.path())?;
                }
            }
        }
        Ok(())
    }
    pub fn content(&self, key: &str) -> Result<Vec<u8>> {
        ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid content key"
        );
        let mut bytes = vec![];
        fs::File::open(self.home.join("objects").join(key))?
            .take(self.limits.file as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= self.limits.file && digest(&bytes) == key,
            "Historical content missing or corrupt"
        );
        Ok(bytes)
    }
    pub fn save(&self, r: &Record) -> Result<()> {
        id(&r.task)?;
        id(&r.id)?;
        let bytes = serde_json::to_vec(r)?;
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "Round manifest budget exceeded"
        );
        crate::config::atomic_write(
            &self
                .home
                .join("rounds")
                .join(&r.task)
                .join(format!("{}.json", r.id)),
            &bytes,
        )
    }
    pub fn load(&self, task: &str, round: &str) -> Result<Record> {
        id(task)?;
        id(round)?;
        let mut bytes = vec![];
        fs::File::open(
            self.home
                .join("rounds")
                .join(task)
                .join(format!("{round}.json")),
        )?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 64 * 1024 * 1024, "Manifest budget exceeded");
        let r: Record = serde_json::from_slice(&bytes)?;
        ensure!(
            r.task == task && r.id == round,
            "Manifest identity mismatch"
        );
        Ok(r)
    }
    pub fn list(&self, task: &str) -> Result<Vec<Record>> {
        id(task)?;
        let mut records = vec![];
        if let Ok(entries) = fs::read_dir(self.home.join("rounds").join(task)) {
            for e in entries {
                let e = e?;
                let name = e.path();
                if let Some(s) = name.file_stem().and_then(|s| s.to_str()) {
                    records.push(self.load(task, s)?);
                }
            }
        }
        records.sort_by(|a, b| (b.order, &b.id).cmp(&(a.order, &a.id)));
        Ok(records)
    }
    pub fn diff(&self, c: &Change) -> Result<String> {
        ensure!(c.reason.is_none(), "{}", c.reason.as_deref().unwrap_or(""));
        let before = c
            .before
            .as_ref()
            .map(|k| self.content(k))
            .transpose()?
            .unwrap_or_default();
        let after = c
            .after
            .as_ref()
            .map(|k| self.content(k))
            .transpose()?
            .unwrap_or_default();
        ensure!(
            !before.contains(&0) && !after.contains(&0),
            "Binary content; text diff unavailable"
        );
        let a = std::str::from_utf8(&before)?;
        let b = std::str::from_utf8(&after)?;
        let diff = similar::TextDiff::configure()
            .timeout(Duration::from_millis(250))
            .diff_lines(a, b)
            .unified_diff()
            .context_radius(3)
            .header("before", "after")
            .to_string();
        if diff.len() > 1024 * 1024 {
            let mut end = 1024 * 1024;
            while !diff.is_char_boundary(end) {
                end -= 1;
            }
            Ok(format!(
                "{}\n[diff display truncated; archived contents retained]",
                &diff[..end]
            ))
        } else {
            Ok(diff)
        }
    }
}
fn keys(r: &Record) -> BTreeSet<String> {
    r.baseline
        .values()
        .filter_map(|c| {
            if let Content::Blob(k) = c {
                Some(k.clone())
            } else {
                None
            }
        })
        .chain(
            r.changes
                .iter()
                .flat_map(|c| [c.before.clone(), c.after.clone()])
                .flatten(),
        )
        .collect()
}
fn order() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos().min(u64::MAX as u128) as u64)
}
pub fn excluded(p: &Path) -> bool {
    p.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(".git" | "target" | "node_modules" | ".next" | "dist" | "artifacts")
        )
    })
}
