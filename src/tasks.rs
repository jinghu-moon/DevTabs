//! Persistent task identities and owned Git worktrees. Resource work runs outside host locks.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Agent,
    Debug,
    Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Binding {
    pub task_id: String,
    pub run_id: String,
    pub role: Role,
    pub initial_cwd: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Navigation {
    pub project: PathBuf,
    pub workspace: String,
    pub task: String,
    pub session: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub project: PathBuf,
    pub checkout: PathBuf,
    pub cwd: PathBuf,
    pub branch: String,
    pub base: String,
    pub owned: bool,
    pub archived: bool,
    pub options: crate::launch::Options,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub operation: String,
    pub workspace: Option<String>,
    pub task: Task,
    pub source: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub plan: Plan,
    pub phase: String,
    pub error: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
    pub tasks: BTreeMap<String, Task>,
    pub operations: BTreeMap<String, Operation>,
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common: PathBuf,
}
impl Repository {
    pub fn discover(cwd: &Path) -> Result<Self> {
        let canonical =
            |s: String| -> Result<PathBuf> { Ok(dunce::canonicalize(PathBuf::from(s))?) };
        Ok(Self {
            root: canonical(text(cwd, &["rev-parse", "--show-toplevel"])?)?,
            git_dir: canonical(text(cwd, &["rev-parse", "--absolute-git-dir"])?)?,
            common: canonical(text(
                cwd,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?)?,
        })
    }
    pub fn head(&self) -> Result<String> {
        text(&self.root, &["rev-parse", "--verify", "HEAD^{commit}"])
    }
    pub fn stamp(&self) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::hash::DefaultHasher::new();
        for path in [
            self.git_dir.join("HEAD"),
            self.git_dir.join("index"),
            self.common.join("packed-refs"),
        ] {
            if let Ok(meta) = std::fs::metadata(&path) {
                meta.len().hash(&mut hash);
                meta.modified().ok().hash(&mut hash);
            }
        }
        text(&self.root, &["show-ref", "--head"])
            .unwrap_or_default()
            .hash(&mut hash);
        vec![hash.finish()]
    }
}
pub fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    crate::explorer::git_output(root, args)
}
pub fn text(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git(root, args)?)?
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}
/// Complete paths, independent of the truncated project-pulse hotspot ranking.
pub fn changed_paths(task: &Task) -> Result<std::collections::BTreeSet<PathBuf>> {
    let mut paths = std::collections::BTreeSet::new();
    for args in [
        vec![
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            task.base.as_str(),
            "HEAD",
            "--",
        ],
        vec!["diff", "--name-only", "-z", "--no-renames", "HEAD", "--"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        let bytes = git(&task.checkout, &args)?;
        for p in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            paths.insert(PathBuf::from(std::str::from_utf8(p)?));
        }
        ensure!(
            paths.len() <= 200_000,
            "Task path coverage exceeded budget; overlap unknown"
        );
    }
    Ok(paths)
}
pub fn overlaps(tasks: &[Task]) -> Result<BTreeMap<PathBuf, Vec<String>>> {
    let mut all = BTreeMap::<PathBuf, Vec<String>>::new();
    for task in tasks {
        for path in changed_paths(task)? {
            all.entry(path).or_default().push(task.id.clone());
        }
    }
    all.retain(|_, tasks| tasks.len() > 1);
    Ok(all)
}
impl Catalog {
    pub fn load(home: &Path) -> Result<Self> {
        use std::io::Read;
        let path = home.join("tasks.json");
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 8 * 1024 * 1024, "Task catalog exceeds 8 MiB");
                Ok(serde_json::from_slice(&bytes)?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, home: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, "Task catalog exceeds 8 MiB");
        crate::config::atomic_write(&home.join("tasks.json"), &bytes)
    }
    pub fn main_task(&mut self, options: &crate::launch::Options) -> Result<Task> {
        let repo = Repository::discover(&options.project).ok();
        let checkout = repo
            .as_ref()
            .map_or_else(|| options.project.clone(), |r| r.root.clone());
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Main".into(),
            project: options.project.clone(),
            checkout,
            cwd: options.project.clone(),
            branch: repo
                .as_ref()
                .and_then(|r| text(&r.root, &["branch", "--show-current"]).ok())
                .unwrap_or_default(),
            base: repo
                .as_ref()
                .and_then(|r| r.head().ok())
                .unwrap_or_default(),
            owned: false,
            archived: false,
            options: crate::recovery::clean(options.clone()),
        };
        self.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }
    pub fn plan(
        &self,
        source: &str,
        name: &str,
        base: &str,
        path: Option<PathBuf>,
    ) -> Result<Plan> {
        let source = self.tasks.get(source).context("Unknown source task")?;
        ensure!(
            !base.starts_with('-') && !base.is_empty(),
            "Invalid base reference"
        );
        ensure!(
            !name.trim().is_empty() && name.chars().count() <= 120,
            "Task name must contain 1–120 characters"
        );
        let repo = Repository::discover(&source.cwd)?;
        let commit = text(
            &repo.root,
            &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .take(32)
            .collect();
        let slug = slug.trim_matches('-');
        let slug = if slug.is_empty() { "task" } else { slug };
        let branch = format!("devtabs/{slug}-{}", &id[..8]);
        let parent = repo.root.parent().context("Repository has no parent")?;
        let path = path.unwrap_or_else(|| {
            parent
                .join(format!(
                    "{}-worktrees",
                    repo.root.file_name().unwrap_or_default().to_string_lossy()
                ))
                .join(format!("{slug}-{}", &id[..8]))
        });
        ensure!(path.is_absolute(), "Worktree path must be absolute");
        ensure!(
            !path
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "Worktree path must not contain parent traversal"
        );
        // Resolve existing ancestors before containment checks, including junctions.
        let ancestor = path
            .ancestors()
            .find(|p| p.exists())
            .context("No existing path ancestor")?;
        let path = dunce::canonicalize(ancestor)?.join(path.strip_prefix(ancestor)?);
        ensure!(
            !path.starts_with(&repo.root) && !repo.root.starts_with(&path),
            "Worktree must not overlap source checkout"
        );
        ensure!(!path.exists(), "Worktree target already exists");
        let module = source
            .cwd
            .strip_prefix(&repo.root)
            .context("Task cwd outside checkout")?;
        let cwd = path.join(module);
        let mut options = crate::recovery::clean(source.options.clone());
        options.project = cwd.clone();
        options.resume_id = None;
        options.resume = false;
        options.continue_latest = false;
        options.fresh = true;
        options.new_instance = false;
        for extra in &mut options.extra_sessions {
            if let Some(old) = &extra.directory
                && old.is_absolute()
            {
                let relative = old
                    .strip_prefix(&repo.root)
                    .context("Extra terminal directory outside checkout")?;
                extra.directory = Some(path.join(relative));
            }
        }
        Ok(Plan {
            operation: uuid::Uuid::new_v4().to_string(),
            workspace: None,
            source: repo.root,
            task: Task {
                id,
                name: name.trim().into(),
                project: source.project.clone(),
                checkout: path,
                cwd,
                branch,
                base: commit,
                owned: true,
                archived: false,
                options,
            },
        })
    }
    pub fn create(&mut self, home: &Path, plan: &Plan) -> Result<Task> {
        if let Some(previous) = self.operations.get(&plan.operation) {
            ensure!(
                serde_json::to_vec(&previous.plan)? == serde_json::to_vec(plan)?,
                "Operation identity conflict"
            );
            if let Some(task) = self.tasks.get(&plan.task.id) {
                ensure!(
                    task.cwd.is_dir(),
                    "Task directory is missing; resources retained"
                );
                return Ok(task.clone());
            }
        } else {
            ensure!(
                !plan.task.checkout.exists(),
                "Worktree target already exists"
            );
            self.operations.insert(
                plan.operation.clone(),
                Operation {
                    plan: plan.clone(),
                    phase: "Prepared".into(),
                    error: None,
                },
            );
            self.save(home)?;
        }
        let result = (|| -> Result<Task> {
            if plan.task.checkout.exists() {
                // Only reconcile an exact persisted intent, never names/prefixes.
                let repo = Repository::discover(&plan.task.checkout)?;
                ensure!(
                    repo.common == Repository::discover(&plan.source)?.common,
                    "Worktree belongs to another repository; resources retained"
                );
                ensure!(
                    repo.root == dunce::canonicalize(&plan.task.checkout)?,
                    "Unexpected checkout target"
                );
                ensure!(
                    text(&repo.root, &["branch", "--show-current"])? == plan.task.branch,
                    "Worktree ownership conflict"
                );
                ensure!(
                    repo.head()? == plan.task.base,
                    "Created worktree has changed; keep it for review"
                );
            } else {
                let target = plan
                    .task
                    .checkout
                    .to_str()
                    .context("Non-Unicode worktree path")?;
                git(
                    &plan.source,
                    &[
                        "worktree",
                        "add",
                        "-b",
                        &plan.task.branch,
                        target,
                        &plan.task.base,
                    ],
                )?;
            }
            let cwd =
                dunce::canonicalize(&plan.task.cwd).context("Module missing in task worktree")?;
            ensure!(
                cwd.starts_with(dunce::canonicalize(&plan.task.checkout)?),
                "Module leaves task checkout"
            );
            self.tasks.insert(plan.task.id.clone(), plan.task.clone());
            self.resources.insert(
                plan.task.id.clone(),
                crate::lifecycle::identity(&plan.task.checkout)?,
            );
            Ok(plan.task.clone())
        })();
        let op = self.operations.get_mut(&plan.operation).unwrap();
        op.phase = if result.is_ok() { "Ready" } else { "Failed" }.into();
        op.error = result.as_ref().err().map(|e| format!("{e:#}"));
        self.save(home)?;
        result
    }
}
