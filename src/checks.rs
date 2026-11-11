//! Explicit managed checks, isolated from Agent and Debug input streams.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub run: String,
    pub checkout: PathBuf,
    pub head: Option<String>,
    pub task: String,
    pub result: u64,
    pub before: String,
    pub after: String,
    pub exit: Option<i32>,
    pub timeout: bool,
    pub stale: bool,
    pub output: String,
    pub elapsed_ms: u64,
}
pub fn run(task: &str, result: u64, root: &Path, spec: &Spec) -> Result<Report> {
    let _lease = crate::lifecycle::lease(&crate::config::home(), task, false, false)?;
    run_with_executable(task, result, root, spec, &std::env::current_exe()?)
}
pub fn run_with_executable(
    task: &str,
    result: u64,
    root: &Path,
    spec: &Spec,
    exe: &Path,
) -> Result<Report> {
    ensure!(
        !spec.argv.is_empty()
            && spec.argv.len() <= 128
            && spec
                .argv
                .iter()
                .all(|s| s.len() <= 8192 && !s.contains('\0')),
        "Invalid check command"
    );
    ensure!(
        (1..=3_600_000).contains(&spec.timeout_ms),
        "Check timeout must be 1 ms to 1 hour"
    );
    let root = dunce::canonicalize(root)?;
    let cwd = dunce::canonicalize(if spec.cwd.is_relative() {
        root.join(&spec.cwd)
    } else {
        spec.cwd.clone()
    })?;
    ensure!(cwd.starts_with(&root), "Check cwd leaves task checkout");
    let before = fingerprint(&root)?;
    let start = Instant::now();
    let job = crate::platform::Job::new(0)?;
    let mut cmd = Command::new(exe);
    cmd.arg("__check-child")
        .arg(&job.name)
        .args(&spec.argv)
        .current_dir(&cwd)
        .envs(&spec.env)
        .env_remove("DEVTABS_HOOK_CONTEXT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().context("Start managed check")?;
    let reader = |pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut b = vec![];
            let _ = pipe.take(128 * 1024 + 1).read_to_end(&mut b);
            b
        })
    };
    let out = reader(Box::new(child.stdout.take().unwrap()));
    let err = reader(Box::new(child.stderr.take().unwrap()));
    let (exit, timeout) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s.code(), false),
            Ok(None) => {}
            Err(e) => {
                let _ = job.terminate();
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        }
        if start.elapsed() >= Duration::from_millis(spec.timeout_ms) {
            let _ = job.terminate();
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let _ = job.terminate();
    drop(job);
    let mut bytes = out.join().unwrap_or_default();
    bytes.extend(err.join().unwrap_or_default());
    let truncated = bytes.len() > 128 * 1024;
    bytes.truncate(128 * 1024);
    let output = format!(
        "{}{}",
        crate::preview::copy_text(&String::from_utf8_lossy(&bytes)),
        if truncated {
            "\n[output truncated]"
        } else {
            ""
        }
    );
    let after = fingerprint(&root).unwrap_or_default();
    Ok(Report {
        run: String::new(),
        checkout: root.clone(),
        head: crate::tasks::Repository::discover(&root)
            .and_then(|r| r.head())
            .ok(),
        task: task.into(),
        result,
        stale: before != after || after.is_empty(),
        before,
        after,
        exit,
        timeout,
        output,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}
/// HEAD plus actual dirty contents; a clean HEAD alone is not a code revision.
pub fn fingerprint(root: &Path) -> Result<String> {
    let mut h = Sha256::new();
    let mut paths = std::collections::BTreeSet::new();
    if let Ok(repo) = crate::tasks::Repository::discover(root) {
        h.update(repo.head()?.as_bytes());
        let status = crate::explorer::git_status(&repo.root)?;
        for p in status.entries.keys() {
            paths.insert(if p.is_absolute() {
                p.clone()
            } else {
                repo.root.join(p)
            });
        }
    } else {
        for e in ignore::WalkBuilder::new(root)
            .hidden(false)
            .follow_links(false)
            .filter_entry(|e| !crate::rounds::excluded(e.path()))
            .build()
        {
            let e = e?;
            if e.file_type().is_some_and(|t| t.is_file()) {
                paths.insert(e.path().to_path_buf());
            }
        }
    }
    ensure!(
        paths.len() <= 200_000,
        "Check revision path budget exceeded"
    );
    let mut used = 0;
    for p in paths {
        h.update(
            p.strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .as_bytes(),
        );
        if !p.exists() {
            h.update(b"missing");
            continue;
        }
        ensure!(
            !fs::symlink_metadata(&p)?.file_type().is_symlink(),
            "Check revision contains a link"
        );
        let meta = fs::metadata(&p)?;
        used += meta.len();
        ensure!(
            meta.len() <= 8 * 1024 * 1024 && used <= 128 * 1024 * 1024,
            "Check revision content budget exceeded"
        );
        let mut file = fs::File::open(p)?;
        let mut buf = [0; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
pub fn child(job: &str, args: &[std::ffi::OsString]) -> Result<i32> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::{
                JobObjects::{AssignProcessToJobObject, OpenJobObjectW},
                SystemServices::JOB_OBJECT_ASSIGN_PROCESS,
                Threading::GetCurrentProcess,
            },
        };
        let name = job.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let h = OpenJobObjectW(JOB_OBJECT_ASSIGN_PROCESS, 0, name.as_ptr());
        ensure!(!h.is_null(), "Check job missing");
        let ok = AssignProcessToJobObject(h, GetCurrentProcess());
        CloseHandle(h);
        ensure!(ok != 0, "Cannot join check job");
    }
    #[cfg(not(windows))]
    let _ = job;
    let mut command = Command::new(args.first().context("Missing check program")?);
    command.args(&args[1..]);
    Ok(command.status()?.code().unwrap_or(1))
}
pub fn initialize(task: &str, root: &Path, home: &Path) -> Result<()> {
    let _lease = crate::lifecycle::lease(home, task, false, false)?;
    initialize_with_executable(task, root, home, &std::env::current_exe()?)
}
pub fn initialize_with_executable(task: &str, root: &Path, home: &Path, exe: &Path) -> Result<()> {
    let profile = crate::project_profile::Profile::load(root)?.unwrap_or_default();
    ensure!(profile.prepare.len() <= 16, "At most 16 preparation steps");
    for (i, spec) in profile.prepare.iter().enumerate() {
        let key = format!("{:x}", spec.timeout_ms);
        let data = serde_json::to_vec(spec)?;
        let hash = Sha256::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let path = home
            .join("preparation")
            .join(task)
            .join(format!("{i}-{key}-{hash}.json"));
        if path.exists() {
            continue;
        }
        let report = run_with_executable(task, 0, root, spec, exe)?;
        ensure!(
            !report.timeout && report.exit == Some(0),
            "Preparation step {} failed: {}",
            i + 1,
            report.output
        );
        crate::config::atomic_write(&path, &serde_json::to_vec(&report)?)?;
    }
    Ok(())
}
