//! One bounded worker queue per host. Snapshot I/O never takes the input mutex.
use super::*;
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, mpsc};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Seal,
    Begin { scope: Vec<PathBuf> },
    End,
    List,
    Page { round: String, offset: usize },
    Result { run: String, result: u64 },
    Open { round: String, path: Option<usize> },
    Ack { round: String, revision: u64 },
    Observe { paths: Vec<PathBuf>, gap: bool },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct View {
    pub detail: bool,
    pub paths: Vec<PathBuf>,
    pub offset: usize,
    pub records: Vec<Record>,
    pub diff: Option<String>,
    pub error: Option<String>,
    pub busy: bool,
}
struct Observation {
    _watcher: notify::RecommendedWatcher,
    receiver: mpsc::Receiver<notify::Result<notify::Event>>,
    overflow: Arc<std::sync::atomic::AtomicBool>,
}
pub struct Service {
    leases: BTreeMap<String, std::fs::File>,
    recorder: Recorder,
    active: BTreeMap<String, (Record, Option<Observation>)>,
    pinned: BTreeMap<String, Instant>,
}
impl Service {
    pub fn new(home: PathBuf) -> Self {
        Self {
            leases: BTreeMap::new(),
            recorder: Recorder::new(home),
            active: BTreeMap::new(),
            pinned: BTreeMap::new(),
        }
    }
    pub fn act(
        &mut self,
        task: &str,
        run: &str,
        root: &Path,
        action: Action,
        late: bool,
    ) -> Result<View> {
        let result = (|| -> Result<View> {
            match action {
                Action::Seal => {
                    if self.active.get(task).is_some_and(|(r, _)| r.run == run) {
                        return self.act(task, run, root, Action::End, late);
                    }
                    Ok(View::default())
                }
                Action::Begin { scope } => {
                    ensure!(
                        !self.active.contains_key(task),
                        "A recording is already active"
                    );
                    let lease = if uuid::Uuid::parse_str(task).is_ok() {
                        Some(crate::lifecycle::lease(
                            self.recorder
                                .home
                                .parent()
                                .context("History root missing")?,
                            task,
                            false,
                            false,
                        )?)
                    } else {
                        None
                    };
                    self.pinned
                        .retain(|_, at| at.elapsed() < Duration::from_secs(600));
                    self.recorder
                        .collect(task, &self.pinned.keys().cloned().collect())?;
                    let mode = if !scope.is_empty() {
                        Mode::Snapshot(scope)
                    } else if crate::tasks::Repository::discover(root)
                        .and_then(|r| r.head())
                        .is_ok()
                    {
                        Mode::Git
                    } else {
                        Mode::Observe
                    };
                    let observer = if mode == Mode::Observe {
                        Some(observe(root)?)
                    } else {
                        None
                    };
                    let r = self.recorder.begin(task, run, root, mode, late)?;
                    self.active.insert(task.into(), (r, observer));
                    if let Some(lease) = lease {
                        self.leases.insert(task.into(), lease);
                    }
                    Ok(View {
                        records: self.recorder.list(task)?,
                        ..View::default()
                    })
                }
                Action::End => {
                    let (r, obs) = self
                        .active
                        .get_mut(task)
                        .context("No recording is active")?;
                    ensure!(r.run == run, "Recording belongs to an earlier Agent run");
                    if let Some(o) = obs {
                        drain(r, o);
                    }
                    let mut r = r.clone();
                    let r = if r.mode == Mode::Observe {
                        r.ended = Some(crate::config::now());
                        r.revision += 1;
                        r.file_count = r.changes.len();
                        r.order = order();
                        self.recorder.save(&r)?;
                        r
                    } else {
                        self.recorder.finish(r)?
                    };
                    self.active.remove(task);
                    self.leases.remove(task);
                    let mut view = View {
                        records: self.recorder.list(task)?,
                        ..View::default()
                    };
                    if let Some(i) = view.records.iter().position(|x| x.id == r.id) {
                        view.records.swap(0, i);
                    }
                    Ok(view)
                }
                Action::List => {
                    if let Some((r, Some(o))) = self.active.get_mut(task) {
                        drain(r, o);
                        self.recorder.save(r)?;
                    }
                    Ok(View {
                        records: self.recorder.list(task)?,
                        ..View::default()
                    })
                }
                Action::Page { round, offset } => {
                    ensure!(
                        self.pinned.len() < 64 || self.pinned.contains_key(&round),
                        "Close older history views before opening more"
                    );
                    self.pinned.insert(round.clone(), Instant::now());
                    let mut r = self.recorder.load(task, &round)?;
                    let paths = r.changes.iter().map(|c| r.root.join(&c.path)).collect();
                    r.changes = r.changes.into_iter().skip(offset).take(100).collect();
                    Ok(View {
                        records: vec![r],
                        paths,
                        offset,
                        detail: true,
                        ..View::default()
                    })
                }
                Action::Result { run, result } => {
                    let r=self.recorder.list(task)?.into_iter().find(|r|r.run==run&&r.result==Some(result)).context("This result is still capturing or its history is unavailable; no latest-round substitution was made")?;
                    Ok(View {
                        records: vec![r],
                        ..View::default()
                    })
                }
                Action::Open { round, path } => {
                    ensure!(
                        self.pinned.len() < 64 || self.pinned.contains_key(&round),
                        "Close older history views before opening more"
                    );
                    self.pinned.insert(round.clone(), Instant::now());
                    let mut r = self.recorder.load(task, &round)?;
                    let paths = r.changes.iter().map(|c| r.root.join(&c.path)).collect();
                    let diff = path
                        .map(|p| {
                            self.recorder
                                .diff(r.changes.get(p).context("Path index out of range")?)
                        })
                        .transpose()?;
                    let offset = path.unwrap_or(0) / 100 * 100;
                    r.changes = r.changes.into_iter().skip(offset).take(100).collect();
                    Ok(View {
                        records: vec![r],
                        paths,
                        offset,
                        diff,
                        detail: true,
                        ..View::default()
                    })
                }
                Action::Ack { round, revision } => {
                    let mut r = self.recorder.load(task, &round)?;
                    ensure!(r.revision == revision, "Result revision changed");
                    r.read = true;
                    self.recorder.save(&r)?;
                    Ok(View {
                        records: self.recorder.list(task)?,
                        ..View::default()
                    })
                }
                Action::Observe { paths, gap } => {
                    let (r, _) = self.active.get_mut(task).context("No active observation")?;
                    ensure!(r.mode == Mode::Observe, "Not an observation recording");
                    if gap {
                        r.reasons.push("Observer gap; incomplete".into());
                    }
                    for p in paths {
                        if valid_path(&p)
                            && !excluded(&p)
                            && r.changes.len() < 4096
                            && !r.changes.iter().any(|c| c.path == p)
                        {
                            r.changes.push(Change {
                                path: p,
                                before: None,
                                after: None,
                                reason: Some("Observed path; previous content unavailable".into()),
                            });
                        }
                    }
                    Ok(View {
                        records: vec![r.clone()],
                        ..View::default()
                    })
                }
            }
        })();
        result.map(|mut v| {
            v.busy = self.active.contains_key(task);
            v.records.truncate(20);
            if v.paths.is_empty()
                && let Some(r) = v.records.first()
            {
                v.paths = r.changes.iter().map(|c| r.root.join(&c.path)).collect();
            }
            if v.error.is_none() {
                v.error = std::fs::read_to_string(
                    self.recorder
                        .home
                        .join("errors")
                        .join(format!("{task}.txt")),
                )
                .ok();
            }
            for r in &mut v.records {
                r.baseline.clear();
                r.changes.truncate(100);
            }
            v
        })
    }
    pub fn automatic(
        &mut self,
        task: &str,
        run: &str,
        root: &Path,
        begin: bool,
        fresh: bool,
        result: u64,
    ) -> Result<()> {
        if let Some(old) = self
            .active
            .get(task)
            .filter(|(r, _)| r.run != run)
            .map(|(r, _)| r.run.clone())
        {
            self.act(task, &old, root, Action::End, true)?;
        }
        if self.active.get(task).is_some_and(|(r, _)| !r.late) {
            return Ok(());
        }
        if begin {
            if fresh && self.active.contains_key(task) {
                self.act(task, run, root, Action::End, true)?;
            }
            if !self.active.contains_key(task) {
                self.act(task, run, root, Action::Begin { scope: vec![] }, true)?;
            }
        } else if let Some((r, obs)) = self.active.get_mut(task) {
            if r.mode == Mode::Observe {
                if let Some(o) = obs {
                    drain(r, o);
                }
                let mut snapshot = r.clone();
                snapshot.id = uuid::Uuid::new_v4().to_string();
                snapshot.result = Some(result);
                snapshot.ended = Some(crate::config::now());
                snapshot.revision += 1;
                snapshot.file_count = snapshot.changes.len();
                snapshot.order = order();
                self.recorder.save(&snapshot)?;
            } else {
                let mut snapshot = r.clone();
                snapshot.id = uuid::Uuid::new_v4().to_string();
                snapshot.result = Some(result);
                self.recorder.finish(snapshot)?;
            }
        }
        Ok(())
    }
}
fn observe(root: &Path) -> Result<Observation> {
    let (tx, rx) = mpsc::sync_channel(4096);
    let overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = overflow.clone();
    let mut watcher = notify::recommended_watcher(move |e| {
        if tx.try_send(e).is_err() {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(Observation {
        _watcher: watcher,
        receiver: rx,
        overflow,
    })
}
#[derive(Clone)]
pub struct Worker {
    tx: mpsc::SyncSender<Job>,
    outstanding: Arc<std::sync::atomic::AtomicUsize>,
}
struct Job {
    task: String,
    run: String,
    root: PathBuf,
    action: Option<Action>,
    automatic: Option<(bool, bool, u64)>,
    reply: mpsc::SyncSender<std::result::Result<View, String>>,
}
impl Worker {
    pub fn spawn(home: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Job>(64);
        let outstanding = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = outstanding.clone();
        std::thread::Builder::new()
            .name("devtabs-records".into())
            .spawn(move || {
                let mut service = Service::new(home);
                let mut queues = BTreeMap::<String, std::collections::VecDeque<Job>>::new();
                let mut order = std::collections::VecDeque::<String>::new();
                loop {
                    if order.is_empty() {
                        let Ok(j) = rx.recv() else {
                            break;
                        };
                        order.push_back(j.task.clone());
                        queues.entry(j.task.clone()).or_default().push_back(j);
                    }
                    for j in rx.try_iter().take(64) {
                        if !queues.contains_key(&j.task) {
                            order.push_back(j.task.clone());
                        }
                        queues.entry(j.task.clone()).or_default().push_back(j);
                    }
                    let task = order.pop_front().unwrap();
                    let queue = queues.get_mut(&task).unwrap();
                    let j = queue.pop_front().unwrap();
                    if queue.is_empty() {
                        queues.remove(&task);
                    } else {
                        order.push_back(task);
                    }
                    let result = if let Some((begin, fresh, result)) = j.automatic {
                        service
                            .automatic(&j.task, &j.run, &j.root, begin, fresh, result)
                            .map(|_| View::default())
                    } else {
                        service.act(&j.task, &j.run, &j.root, j.action.unwrap(), false)
                    };
                    let result = result.map_err(|e| format!("{e:#}"));
                    if let Err(e) = &result {
                        let _ = crate::config::atomic_write(
                            &service
                                .recorder
                                .home
                                .join("errors")
                                .join(format!("{}.txt", j.task)),
                            e.as_bytes(),
                        );
                    }
                    let _ = j.reply.send(result);
                    counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            })?;
        Ok(Self { tx, outstanding })
    }
    fn submit(&self, job: Job) -> Result<()> {
        use std::sync::atomic::Ordering;
        self.outstanding
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < 64).then_some(n + 1)
            })
            .map_err(|_| anyhow::anyhow!("Recording queue full"))?;
        if self.tx.try_send(job).is_err() {
            self.outstanding.fetch_sub(1, Ordering::Relaxed);
            anyhow::bail!("Recording queue unavailable");
        }
        Ok(())
    }
    pub fn request(
        &self,
        task: String,
        run: String,
        root: PathBuf,
        action: Action,
    ) -> Result<View> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit(Job {
            task,
            run,
            root,
            action: Some(action),
            automatic: None,
            reply: tx,
        })?;
        rx.recv_timeout(Duration::from_secs(12))
            .context("Recording still pending; query history before retry")?
            .map_err(anyhow::Error::msg)
    }
    pub fn event(
        &self,
        task: String,
        run: String,
        root: PathBuf,
        begin: bool,
        fresh: bool,
        result: u64,
    ) -> Result<()> {
        let (tx, _) = mpsc::sync_channel(1);
        self.submit(Job {
            task,
            run,
            root,
            action: None,
            automatic: Some((begin, fresh, result)),
            reply: tx,
        })
    }
    pub fn seal(&self, task: String, run: String, root: PathBuf) -> Result<()> {
        let (tx, _) = mpsc::sync_channel(1);
        self.submit(Job {
            task,
            run,
            root,
            action: Some(Action::Seal),
            automatic: None,
            reply: tx,
        })
    }
}

fn drain(r: &mut Record, o: &Observation) {
    if o.overflow.swap(false, std::sync::atomic::Ordering::Relaxed) {
        r.reasons
            .push("Watcher queue overflow; observation incomplete".into());
    }
    let mut paths = r
        .changes
        .iter()
        .map(|c| c.path.clone())
        .collect::<BTreeSet<_>>();
    for e in o.receiver.try_iter() {
        match e {
            Ok(e) => {
                for p in e.paths {
                    if let Ok(p) = p.strip_prefix(&r.root)
                        && valid_path(p)
                        && !excluded(p)
                    {
                        if paths.len() < 4096 {
                            paths.insert(p.to_path_buf());
                        } else {
                            r.reasons.push("Observation path budget reached".into());
                        }
                    }
                }
            }
            Err(e) => r.reasons.push(format!("Watcher gap: {e}")),
        }
    }
    r.changes = paths
        .into_iter()
        .map(|path| Change {
            path,
            before: None,
            after: None,
            reason: Some("Observed file event; prior content unavailable".into()),
        })
        .collect();
    r.reasons.sort();
    r.reasons.dedup();
}
