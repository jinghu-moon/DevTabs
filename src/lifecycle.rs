//! Explicit, journaled task resource operations. No automatic commit, stash or force removal.
use crate::tasks::{Catalog, Repository, Task, git, text};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    List,
    Plan {
        source: String,
        target: String,
        cleanup: bool,
        require_check: bool,
    },
    Apply {
        operation: String,
        confirm: String,
    },
    Query {
        operation: String,
    },
    Archive {
        task: String,
        confirm: String,
    },
    Close {
        task: String,
        confirm: String,
    },
}
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Kind {
    FastForward,
    Cleanup,
}
/// OS locks cover all DevTabs builds sharing this home and release on process death.
pub fn lease(home: &Path, task: &str, views: bool, exclusive: bool) -> Result<fs::File> {
    ensure!(uuid::Uuid::parse_str(task).is_ok(), "Invalid task lease ID");
    let dir = home.join(if views { "view-leases" } else { "task-leases" });
    fs::create_dir_all(&dir)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(format!("{task}.lock")))?;
    if exclusive {
        file.try_lock().context("Task is used by a terminal/view in this or another DevTabs host; close/detach it before applying")?;
    } else {
        file.try_lock_shared()
            .context("Task resource operation is in progress")?;
    }
    Ok(file)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub kind: Kind,
    pub source: Task,
    pub target: Task,
    pub source_head: String,
    pub target_head: String,
    pub phase: String,
    pub error: Option<String>,
    pub git_dir: PathBuf,
    pub resource: String,
    pub target_resource: String,
    pub branch_resource: String,
    pub check: Option<crate::checks::Report>,
}
fn stamp(path: &Path) -> Result<String> {
    let m = fs::symlink_metadata(path)?;
    ensure!(!m.file_type().is_symlink(), "Linked resource is protected");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            m.file_attributes() & 0x400 == 0,
            "Reparse resource is protected"
        );
    }
    Ok(format!("{:?}", m.created()?))
}
pub fn identity(root: &Path) -> Result<String> {
    let r = Repository::discover(root)?;
    ensure!(
        r.root == dunce::canonicalize(root)?,
        "Task path is not the checkout root"
    );
    let data = format!(
        "{}|{}|{}|{}",
        r.git_dir.display(),
        r.common.display(),
        stamp(root)?,
        stamp(&r.git_dir)?
    );
    Ok(Sha256::digest(data.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn branch_identity(task: &Task) -> Result<String> {
    let r = Repository::discover(&task.checkout)?;
    branch_at(&r.root, &task.branch)
}
fn branch_at(root: &Path, branch: &str) -> Result<String> {
    let r = Repository::discover(root)?;
    let p = r.common.join("logs/refs/heads").join(branch);
    let reference = r.common.join("refs/heads").join(branch);
    let mut h = Sha256::new();
    for p in [p, reference] {
        if p.exists() {
            h.update(stamp(&p)?.as_bytes());
            h.update(format!("{:?}", fs::metadata(&p)?.modified()?).as_bytes());
            let mut b = vec![];
            fs::File::open(p)?
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut b)?;
            ensure!(b.len() <= 8 * 1024 * 1024, "Branch identity exceeds budget");
            h.update(&b);
        }
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn path(home: &Path, id: &str) -> Result<PathBuf> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok(),
        "Invalid lifecycle operation ID"
    );
    Ok(home.join("lifecycle").join(format!("{id}.json")))
}
fn save(home: &Path, op: &Operation) -> Result<()> {
    crate::config::atomic_write(&path(home, &op.id)?, &serde_json::to_vec_pretty(op)?)
}
pub fn load(home: &Path, id: &str) -> Result<Operation> {
    let mut b = vec![];
    fs::File::open(path(home, id)?)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut b)?;
    ensure!(b.len() <= 1024 * 1024, "Lifecycle operation exceeds budget");
    let op: Operation = serde_json::from_slice(&b)?;
    ensure!(op.id == id, "Lifecycle identity mismatch");
    Ok(op)
}
fn clean(task: &Task, ignored: bool) -> Result<()> {
    let mut args = vec!["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    if ignored {
        args.push("--ignored=matching");
    }
    ensure!(
        git(&task.checkout, &args)?.is_empty(),
        "Task has local changes{}; resources retained",
        if ignored {
            ", untracked or ignored files"
        } else {
            " or untracked files"
        }
    );
    Ok(())
}
fn checkout(task: &Task) -> Result<Repository> {
    let r = Repository::discover(&task.checkout)?;
    ensure!(
        r.root == dunce::canonicalize(&task.checkout)?,
        "Checkout identity changed"
    );
    ensure!(
        text(&r.root, &["branch", "--show-current"])? == task.branch && !task.branch.is_empty(),
        "Task branch changed or detached; resources retained"
    );
    Ok(r)
}
fn ancestor(root: &Path, a: &str, b: &str) -> Result<()> {
    git(root, &["merge-base", "--is-ancestor", a, b])
        .context("Not a fast-forward/merged result; use the task terminal to review")?;
    Ok(())
}
fn proof(home: &Path, task: &Task) -> Result<crate::checks::Report> {
    let bytes = fs::read(home.join("checks").join(format!("{}.json", task.id)))?;
    ensure!(bytes.len() <= 1024 * 1024, "Check record exceeds budget");
    let report: crate::checks::Report = serde_json::from_slice(&bytes)?;
    validate_proof(task, &report)?;
    Ok(report)
}
fn validate_proof(task: &Task, r: &crate::checks::Report) -> Result<()> {
    ensure!(
        r.task == task.id
            && dunce::canonicalize(&r.checkout)? == dunce::canonicalize(&task.checkout)?
            && !r.run.is_empty()
            && r.exit == Some(0)
            && !r.timeout
            && !r.stale
            && r.before == r.after
            && r.after == crate::checks::fingerprint(&task.checkout)?
            && r.head.as_ref() == Some(&checkout(task)?.head()?),
        "Check is missing, failed or stale for this source checkout/revision"
    );
    Ok(())
}
pub fn plan(
    home: &Path,
    source: &str,
    target: &str,
    cleanup: bool,
    require_check: bool,
) -> Result<Operation> {
    let _lock = crate::config::ConfigLock::acquire_at(home)?;
    let catalog = Catalog::load(home)?;
    let source = catalog
        .tasks
        .get(source)
        .context("Source task missing")?
        .clone();
    let target = catalog
        .tasks
        .get(target)
        .context("Target task missing")?
        .clone();
    ensure!(
        source.id != target.id,
        "Select distinct source and target tasks"
    );
    let a = checkout(&source)?;
    let b = checkout(&target)?;
    ensure!(
        a.common == b.common,
        "Tasks belong to different repositories"
    );
    let source_head = a.head()?;
    let target_head = b.head()?;
    if cleanup {
        ensure!(
            source.owned && a.git_dir != a.common,
            "Original/external checkout is protected; archive it without deleting resources"
        );
        ensure!(
            catalog.resources.get(&source.id) == Some(&identity(&source.checkout)?),
            "No matching creation ownership; resources retained"
        );
        ensure!(
            catalog
                .operations
                .values()
                .any(|o| matches!(o.phase.as_str(), "Ready" | "Attached")
                    && o.plan.task.id == source.id
                    && o.plan.task.checkout == source.checkout
                    && o.plan.task.branch == source.branch),
            "Creation transaction ownership is unavailable"
        );
        clean(&source, true)?;
        ancestor(&a.root, &source_head, &target_head)?;
    } else {
        clean(&source, false)?;
        clean(&target, false)?;
        ancestor(&a.root, &target_head, &source_head)?;
    }
    let check = if require_check {
        Some(proof(home, &source)?)
    } else {
        None
    };
    let resource = identity(&source.checkout)?;
    let branch_resource = branch_identity(&source)?;
    let target_resource = identity(&target.checkout)?;
    let op = Operation {
        id: uuid::Uuid::new_v4().to_string(),
        kind: if cleanup {
            Kind::Cleanup
        } else {
            Kind::FastForward
        },
        source,
        target,
        source_head,
        target_head,
        phase: "Prepared".into(),
        error: None,
        git_dir: a.git_dir,
        resource,
        target_resource,
        branch_resource,
        check,
    };
    save(home, &op)?;
    Ok(op)
}
pub fn archive(home: &Path, task: &str) -> Result<Task> {
    let _lock = crate::config::ConfigLock::acquire_at(home)?;
    let mut c = Catalog::load(home)?;
    let t = c.tasks.get_mut(task).context("Unknown task")?;
    t.archived = true;
    let t = t.clone();
    c.save(home)?;
    Ok(t)
}
pub fn execute(home: &Path, id: &str) -> Result<Operation> {
    let _perf = crate::perf::span(crate::perf::Metric::TaskLifecycle);
    let _lock = crate::config::ConfigLock::acquire_at(home)?;
    let mut op = load(home, id)?;
    if op.phase == "Complete" {
        return Ok(op);
    }
    op.error = None;
    let outcome = (|| -> Result<()> {
        let mut _leases = vec![lease(home, &op.source.id, false, true)?];
        if matches!(op.kind, Kind::FastForward) {
            _leases.push(lease(home, &op.target.id, false, true)?);
        }
        if matches!(op.kind, Kind::Cleanup) {
            _leases.push(lease(home, &op.source.id, true, true)?);
        }
        let catalog = Catalog::load(home)?;
        let current = catalog
            .tasks
            .get(&op.source.id)
            .context("Task association removed")?;
        ensure!(
            current.checkout == op.source.checkout && current.branch == op.source.branch,
            "Task binding changed"
        );
        let target_binding = catalog
            .tasks
            .get(&op.target.id)
            .context("Target task association removed")?;
        ensure!(
            target_binding.checkout == op.target.checkout
                && target_binding.branch == op.target.branch
                && identity(&op.target.checkout)? == op.target_resource,
            "Target resource/binding changed; create a new operation"
        );
        let target = checkout(&op.target)?;
        ensure!(
            target.head()? == op.target_head
                || matches!(op.kind, Kind::FastForward) && target.head()? == op.source_head,
            "Target HEAD changed; create a new operation"
        );
        match op.kind {
            Kind::FastForward => {
                let source = checkout(&op.source)?;
                ensure!(
                    identity(&op.source.checkout)? == op.resource
                        && source.head()? == op.source_head,
                    "Source HEAD/resource changed; create a new operation"
                );
                clean(&op.source, false)?;
                clean(&op.target, false)?;
                if let Some(r) = &op.check {
                    validate_proof(&op.source, r)?;
                }
                if target.head()? != op.source_head {
                    ensure!(target.head()? == op.target_head, "Target moved");
                    ancestor(&target.root, &op.target_head, &op.source_head)?;
                    op.phase = "ApplyingFastForward".into();
                    save(home, &op)?;
                    git(
                        &target.root,
                        &[
                            "-c",
                            "core.hooksPath=NUL",
                            "merge",
                            "--ff-only",
                            "--no-overwrite-ignore",
                            "--no-edit",
                            &op.source_head,
                        ],
                    )?;
                }
                ensure!(
                    checkout(&op.target)?.head()? == op.source_head,
                    "Git result changed externally; inspect before retry"
                );
                op.phase = "Complete".into();
                save(home, &op)?;
            }
            Kind::Cleanup => {
                ensure!(
                    current.owned && catalog.resources.get(&current.id) == Some(&op.resource),
                    "Ownership changed; resources retained"
                );
                ancestor(&target.root, &op.source_head, &op.target_head)?;
                if matches!(op.phase.as_str(), "Prepared" | "RemovingDirectory") {
                    if !op.source.checkout.exists() && op.phase == "RemovingDirectory" {
                        op.phase = "DirectoryRemoved".into();
                        save(home, &op)?;
                    } else {
                        let source = checkout(&op.source)?;
                        ensure!(
                            source.git_dir == op.git_dir
                                && identity(&op.source.checkout)? == op.resource
                                && source.head()? == op.source_head
                                && branch_identity(&op.source)? == op.branch_resource,
                            "Source resources changed; create a new operation"
                        );
                        clean(&op.source, true)?;
                        op.phase = "RemovingDirectory".into();
                        save(home, &op)?;
                        git(
                            &target.root,
                            &[
                                "-c",
                                "core.hooksPath=NUL",
                                "worktree",
                                "remove",
                                op.source
                                    .checkout
                                    .to_str()
                                    .context("Non Unicode checkout")?,
                            ],
                        )?;
                        op.phase = "DirectoryRemoved".into();
                        save(home, &op)?;
                    }
                }
                ensure!(
                    !op.source.checkout.exists(),
                    "Checkout path was recreated; it will not be removed"
                );
                let reference = format!("refs/heads/{}", op.source.branch);
                let head = text(&target.root, &["rev-parse", "--verify", &reference]);
                if let Ok(head) = head {
                    ensure!(
                        head == op.source_head
                            && branch_at(&target.root, &op.source.branch)? == op.branch_resource,
                        "Branch was changed or recreated; it will not be deleted"
                    );
                    op.phase = "DeletingBranch".into();
                    save(home, &op)?;
                    git(&target.root, &["branch", "-d", "--", &op.source.branch])?;
                } else {
                    ensure!(
                        op.phase == "DeletingBranch",
                        "Branch disappeared externally; inspect resources"
                    );
                }
                let mut c = Catalog::load(home)?;
                c.tasks
                    .get_mut(&op.source.id)
                    .context("Task missing")?
                    .archived = true;
                c.resources.remove(&op.source.id);
                c.save(home)?;
                op.phase = "Complete".into();
                save(home, &op)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        op.error = Some(format!("{e:#}"));
        save(home, &op)?;
        return Err(e);
    }
    Ok(op)
}
