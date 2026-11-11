//! Frontend-owned bounded filesystem work, lazy tree and native change notifications.
use crate::preview;
use anyhow::{Context, Result, ensure};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, ReadDir},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use tui_tree_widget::{TreeItem, TreeState};
use unicode_width::UnicodeWidthStr;

pub const MAX_NODES: usize = 10_000;
pub const BATCH: usize = 500;
pub const MAX_CURSORS: usize = 8;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Key {
    Entry(PathBuf),
    More(PathBuf),
    Notice(PathBuf),
}
#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub link: bool,
    pub icon: char,
    pub color: Color,
}
#[derive(Default, Debug)]
pub struct Directory {
    pub children: Vec<PathBuf>,
    pub complete: bool,
    pub loading: bool,
    pub error: String,
    pub generation: u64,
    seen: HashSet<PathBuf>,
    scanned: usize,
}

pub enum Job {
    Bookmarks {
        project: PathBuf,
        id: u64,
        operation: crate::bookmarks::Operation,
    },
    Edit {
        loaded: Arc<preview::Loaded>,
        id: u64,
    },
    Save {
        request: crate::editor::Save,
    },
    Directory {
        root: PathBuf,
        path: PathBuf,
        generation: u64,
        reset: bool,
        hidden: bool,
        ignored: bool,
        capacity: usize,
    },
    Read {
        path: PathBuf,
        id: u64,
    },
    Render {
        editor: bool,
        wrap: bool,
        path: PathBuf,
        id: u64,
        source: Arc<str>,
        markdown: bool,
        width: u16,
        light: bool,
        viewport: preview::CodeViewport,
        generation: Arc<std::sync::atomic::AtomicU64>,
        complete: bool,
        reading: preview::reading::ReadingOptions,
    },
    Highlight {
        path: PathBuf,
        id: u64,
        source: Arc<str>,
        viewport: preview::CodeViewport,
        light: bool,
        generation: Arc<std::sync::atomic::AtomicU64>,
        expected: u64,
    },
    Companion {
        path: PathBuf,
        id: u64,
        source: Arc<str>,
        width: u16,
        light: bool,
        viewport: preview::CodeViewport,
    },
    Diff {
        path: PathBuf,
        id: u64,
        old: Arc<str>,
        new: Arc<str>,
    },
    Close(PathBuf),
}
pub enum Completed {
    Image {
        path: PathBuf,
        id: u64,
    },
    Bookmarks {
        id: u64,
        result: Result<Vec<crate::bookmarks::Entry>>,
    },
    Companion {
        path: PathBuf,
        id: u64,
        result: preview::Rendered,
    },
    Diff {
        path: PathBuf,
        id: u64,
        result: preview::tools::Diff,
    },
    Edit {
        id: u64,
        result: Result<Box<crate::editor::Editor>>,
    },
    Save {
        path: PathBuf,
        revision: u64,
        source: String,
        result: Result<preview::Loaded>,
    },
    Directory {
        path: PathBuf,
        generation: u64,
        batch: Result<(Vec<Node>, bool, usize)>,
    },
    Read {
        path: PathBuf,
        id: u64,
        result: Result<preview::Loaded>,
    },
    Render {
        path: PathBuf,
        id: u64,
        result: preview::Rendered,
        stats: Option<preview::HighlightStats>,
        markdown_stats: Option<preview::MarkdownStats>,
    },
    Highlight {
        path: PathBuf,
        id: u64,
        window: Option<Arc<preview::HighlightWindow>>,
        stats: preview::HighlightStats,
    },
}
struct Cursor {
    iter: ReadDir,
    matcher: ignore::IncrementalIgnore,
    generation: u64,
    scanned: usize,
}
struct Queued {
    job: Job,
    at: Option<Instant>,
}
pub struct Workers {
    tx: SyncSender<Queued>,
    render_tx: SyncSender<Queued>,
    pub rx: Receiver<Completed>,
}
impl Default for Workers {
    fn default() -> Self {
        Self::new()
    }
}
impl Workers {
    pub fn new() -> Self {
        let (tx, jobs) = mpsc::sync_channel::<Queued>(16);
        let (render_tx, render_jobs) = mpsc::sync_channel::<Queued>(2);
        let (done, rx) = mpsc::sync_channel(8);
        let jobs = Arc::new(Mutex::new(jobs));
        let render_jobs = Arc::new(Mutex::new(render_jobs));
        let cursors = Arc::new(Mutex::new(HashMap::<PathBuf, Cursor>::new()));
        for index in 0..2 {
            let jobs = if index == 0 {
                jobs.clone()
            } else {
                render_jobs.clone()
            };
            let done = done.clone();
            let cursors = cursors.clone();
            thread::Builder::new().name(format!("files-{index}")).spawn(move||{
                // Oniguruma's ParseState is not Send. Own it on the render worker.
                let mut code_cache = preview::CodeHighlighter::default();
                let mut markdown_cache = preview::MarkdownCache::default();
                loop{
                // Acquire cursor ownership in receive order: Close/reset must not
                // overtake a newer enumeration on the second worker.
                let (job,mut cursor_guard)={
                    let queue=jobs.lock().unwrap_or_else(|e|e.into_inner());
                    let job=queue.recv().map(|queued| {
                        if let Some(at)=queued.at {crate::perf::record(crate::perf::Metric::QueueWait,at.elapsed());}
                        queued.job
                    });
                    let guard=if matches!(job,Ok(Job::Directory{..}|Job::Close(_))) {Some(cursors.lock().unwrap_or_else(|e|e.into_inner()))} else {None};
                    (job,guard)
                };
                let Ok(job)=job else {break;};
                let completion=match job {
                    Job::Bookmarks{project,id,operation} => Some(Completed::Bookmarks{id,result:crate::bookmarks::update(&crate::config::home().join("bookmarks.json"),&project,operation)}),
                    Job::Edit{loaded,id}=>Some(Completed::Edit{id,result:crate::editor::Editor::new(loaded).map(Box::new)}),
                    Job::Save{request}=>{let result=crate::editor::save(&request);Some(Completed::Save{path:request.snapshot.path.clone(),revision:request.revision,source:request.source,result})},
                    Job::Directory{root,path,generation,reset,hidden,ignored,capacity}=>{
                        let _perf=crate::perf::span(crate::perf::Metric::Directory);
                        let result=(||->Result<(Vec<Node>,bool,usize)>{
                            let mut all=cursor_guard.take().expect("ordered cursor ownership");
                            if reset {all.remove(&path);}
                            if !all.contains_key(&path){
                                ensure!(all.len()<MAX_CURSORS,"Directory cursor budget full; collapse a branch then retry");
                                let mut builder=ignore::WalkBuilder::new(&root);builder.hidden(false);
                                let matcher=builder.build_matchers().remove(0);
                                all.insert(path.clone(),Cursor{iter:fs::read_dir(&path)?,matcher,generation,scanned:0});
                            }
                            let cursor=all.get_mut(&path).context("Directory cursor missing")?;
                            ensure!(cursor.generation==generation,"Directory changed; refresh to restart");
                            let start=Instant::now();let mut nodes=vec![];let mut complete=false;
                            for _ in 0..BATCH.min(capacity){
                                let Some(entry)=cursor.iter.next() else {complete=true;break;};let entry=entry?;cursor.scanned+=1;
                                let full=entry.path();let meta=fs::symlink_metadata(&full)?;
                                let link=is_link(&meta);let directory=meta.is_dir()&&!link;
                                let name=entry.file_name().to_string_lossy().into_owned();
                                if !hidden && (name==".git" || hidden_attribute(&meta)){continue;}
                                if !ignored && cursor.matcher.matched(full.strip_prefix(&root)?,directory).is_ignore(){continue;}
                                let icon=if directory {devicons::FileIcon{icon:'\u{f115}',color:"#7e8ea8"}} else {devicons::FileIcon::from(&full)};
                                let color=icon.color.parse::<Color>().unwrap_or(Color::Gray);
                                nodes.push(Node{path:full,name,directory,link,icon:icon.icon,color});
                                if start.elapsed()>=Duration::from_millis(20){break;}
                            }
                            let scanned=cursor.scanned;if complete{all.remove(&path);}
                            Ok((nodes,complete,scanned))
                        })();
                        Some(Completed::Directory{path,generation,batch:result})
                    }
                    Job::Read{path,id}=>{if preview::image::supported_signature(&path){Some(Completed::Image{path,id})}else{let result=preview::read(&path);Some(Completed::Read{path,id,result})}}
                    Job::Render{path,id,source,markdown,width,light,editor,viewport,generation,complete,reading,wrap}=>{
                        let code=if markdown {None} else {code_cache.render(source.clone(),&path,viewport,light,&generation,id)};
                        let mut result=if editor {preview::render_editor_plain(source,width,light)} else if markdown {markdown_cache.render_reading(source,width,light,viewport,complete,&reading)} else {preview::render_plain(source,&path,false,width,light)};
                        if !markdown {result.window=code;}
                        if wrap && !editor { result.wrap_code(width.saturating_sub(if markdown {0} else {6}) as usize); }
                        Some(Completed::Render{path,id,result,stats:(!markdown).then(||code_cache.stats()),markdown_stats:markdown.then(||markdown_cache.stats())})
                    }
                    Job::Highlight{path,id,source,viewport,light,generation,expected}=>{
                        let window=code_cache.render(source,&path,viewport,light,&generation,expected);
                        Some(Completed::Highlight{path,id,window,stats:code_cache.stats()})
                    }
                    Job::Companion{path,id,source,width,light,viewport}=>{
                        let mut result=preview::render_plain(source.clone(),&path,false,width,light);
                        let generation=std::sync::atomic::AtomicU64::new(id);
                        result.window=code_cache.render(source,&path,viewport,light,&generation,id);
                        Some(Completed::Companion{path,id,result})
                    },
                    Job::Diff{path,id,old,new}=>Some(Completed::Diff{path,id,result:preview::tools::diff(&old,&new)}),
                    Job::Close(path)=>{cursor_guard.take().expect("ordered cursor ownership").remove(&path);None}
                };
                if let Some(completion)=completion {
                    let _perf=crate::perf::span(crate::perf::Metric::ResultWait);
                    if done.send(completion).is_err(){break;}
                }
            }}).expect("filesystem worker");
        }
        Self { tx, render_tx, rx }
    }
    pub fn send(&self, job: Job) -> Result<()> {
        let tx = if matches!(
            job,
            Job::Render { .. } | Job::Highlight { .. } | Job::Companion { .. } | Job::Diff { .. }
        ) {
            &self.render_tx
        } else {
            &self.tx
        };
        tx.try_send(Queued {
            job,
            at: crate::perf::enabled().then(Instant::now),
        })
        .map_err(|_| {
            crate::perf::record(crate::perf::Metric::QueueFull, Duration::ZERO);
            anyhow::anyhow!(crate::i18n::t("ui.file_work_queue_full_retry"))
        })
    }
}

#[cfg(windows)]
pub fn is_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
pub fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}
#[cfg(windows)]
pub(crate) fn hidden_attribute(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 2 != 0
}
#[cfg(not(windows))]
pub(crate) fn hidden_attribute(_: &fs::Metadata) -> bool {
    false
}

pub struct Files {
    refresh_pending: VecDeque<PathBuf>,
    pub git: crate::explorer::Git,
    pub modified_only: bool,
    pub compact: bool,
    pub selected_paths: HashSet<PathBuf>,
    revision: u64,
    pub root: PathBuf,
    pub nodes: HashMap<PathBuf, Node>,
    pub dirs: HashMap<PathBuf, Directory>,
    pub state: TreeState<Key>,
    pub hidden: bool,
    pub ignored: bool,
    pub icons: bool,
    pub light: bool,
    generation: u64,
    watcher: Option<RecommendedWatcher>,
    watched: HashSet<PathBuf>,
    changes: Receiver<notify::Result<notify::Event>>,
    change_tx: SyncSender<notify::Result<notify::Event>>,
    overflow: Arc<std::sync::atomic::AtomicBool>,
    pending: HashSet<PathBuf>,
    changed_at: Option<Instant>,
    pub watch_error: String,
    pub pinned: Option<PathBuf>,
    restore: Option<PathBuf>,
}
impl Files {
    pub fn new(root: PathBuf) -> Self {
        let (change_tx, changes) = mpsc::sync_channel(128);
        let mut nodes = HashMap::new();
        nodes.insert(
            root.clone(),
            Node {
                path: root.clone(),
                name: root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                directory: true,
                link: false,
                icon: '\u{f115}',
                color: Color::Gray,
            },
        );
        let mut state = TreeState::default();
        state.open(vec![Key::Entry(root.clone())]);
        state.select(vec![Key::Entry(root.clone())]);
        Self {
            refresh_pending: VecDeque::new(),
            git: Default::default(),
            modified_only: false,
            compact: false,
            selected_paths: Default::default(),
            revision: 0,
            root,
            nodes,
            dirs: HashMap::new(),
            state,
            hidden: false,
            ignored: false,
            icons: true,
            light: false,
            generation: 0,
            watcher: None,
            watched: HashSet::new(),
            changes,
            change_tx,
            overflow: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending: HashSet::new(),
            changed_at: None,
            watch_error: String::new(),
            pinned: None,
            restore: None,
        }
    }
    pub fn identifier(&self, path: &Path) -> Vec<Key> {
        let projected = self.compact_target(path);
        let path = projected.as_path();
        let mut chain = vec![Key::Entry(self.root.clone())];
        if let Ok(relative) = path.strip_prefix(&self.root) {
            let mut p = self.root.clone();
            for c in relative.components() {
                p.push(c);
                if self.compact_target(&p) == p {
                    chain.push(Key::Entry(p.clone()));
                }
            }
        }
        chain
    }
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn compact_target(&self, path: &Path) -> PathBuf {
        let mut at = path.to_path_buf();
        if !self.compact || at == self.root {
            return at;
        }
        for _ in 0..64 {
            let Some(d) = self.dirs.get(&at).filter(|d| {
                (d.complete || d.loading) && d.error.is_empty() && d.children.len() == 1
            }) else {
                break;
            };
            let next = &d.children[0];
            if !self.nodes.get(next).is_some_and(|n| n.directory && !n.link) {
                break;
            }
            at = next.clone();
        }
        at
    }
    pub fn reproject(&mut self) {
        let opened: Vec<_> = self
            .state
            .opened()
            .iter()
            .filter_map(|id| match id.last() {
                Some(Key::Entry(p)) => Some((
                    p.clone(),
                    (1..id.len()).all(|i| self.state.opened().contains(&id[..i])),
                )),
                _ => None,
            })
            .collect();
        let selected = self.selected_path();
        self.state.close_all();
        for (p, visible) in opened {
            let id = self.identifier(&p);
            if visible {
                for i in 1..id.len() {
                    self.state.open(id[..i].to_vec());
                }
            }
            self.state.open(id);
        }
        if let Some(p) = selected {
            let id = self.identifier(&p);
            if self.state.selected() != id {
                self.state.select(id);
            }
        }
        self.touch();
    }
    pub fn selected(&self) -> Option<Key> {
        self.state.selected().last().cloned()
    }
    pub fn selected_path(&self) -> Option<PathBuf> {
        match self.selected()? {
            Key::Entry(p) => Some(p),
            _ => None,
        }
    }
    pub fn selected_file(&self) -> Option<PathBuf> {
        let p = self.selected_path()?;
        self.nodes.get(&p).filter(|n| !n.directory).map(|_| p)
    }
    pub fn request(&mut self, path: &Path, reset: bool, io: &Workers) -> Result<()> {
        if self.dirs.get(path).is_some_and(|d| d.loading) {
            return Ok(());
        }
        if self.nodes.len() >= MAX_NODES && !reset {
            self.release_collapsed(io);
        }
        // Reserve one batch for every in-flight directory; never consume an
        // iterator entry which cannot subsequently enter the bounded cache.
        let reserved = self.dirs.values().filter(|d| d.loading).count() * BATCH;
        let reusable = self.dirs.get(path).map_or(0, |d| {
            if reset {
                d.children.len()
            } else {
                d.children.len().saturating_sub(d.seen.len())
            }
        });
        let capacity = MAX_NODES
            .saturating_sub(self.nodes.len() + reserved)
            .saturating_add(reusable.min(BATCH))
            .min(BATCH);
        ensure!(
            capacity > 0,
            crate::i18n::t("ui.tree_cache_full_collapse_branches_and_release_cache_f2")
        );
        let dir = self.dirs.entry(path.into()).or_default();
        if dir.loading {
            return Ok(());
        }
        if reset || dir.generation == 0 {
            self.generation += 1;
            dir.generation = self.generation;
            dir.complete = false;
            dir.seen.clear();
        }
        io.send(Job::Directory {
            root: self.root.clone(),
            path: path.into(),
            generation: dir.generation,
            reset,
            hidden: self.hidden,
            ignored: self.ignored,
            capacity,
        })?;
        dir.loading = true;
        dir.error.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
    pub fn apply(
        &mut self,
        path: PathBuf,
        generation: u64,
        batch: Result<(Vec<Node>, bool, usize)>,
    ) {
        let _perf = crate::perf::span(crate::perf::Metric::TreeMerge);
        let Some(dir) = self.dirs.get_mut(&path) else {
            return;
        };
        if dir.generation != generation {
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        dir.loading = false;
        let mut removed_paths = Vec::new();
        match batch {
            Ok((nodes, complete, scanned)) => {
                dir.scanned = scanned;
                let mut existing: HashSet<PathBuf> = dir.children.iter().cloned().collect();
                for node in nodes {
                    dir.seen.insert(node.path.clone());
                    if existing.insert(node.path.clone()) {
                        if self.nodes.len() >= MAX_NODES {
                            dir.error =
                                crate::i18n::t("ui.tree_cache_full_release_collapsed_cache_f2")
                                    .into();
                            continue;
                        }
                        dir.children.push(node.path.clone());
                    }
                    self.nodes.insert(node.path.clone(), node);
                }
                if complete {
                    let removed: Vec<_> = dir
                        .children
                        .iter()
                        .filter(|p| !dir.seen.contains(*p))
                        .cloned()
                        .collect();
                    dir.children.retain(|p| dir.seen.contains(p));
                    for p in removed {
                        self.nodes.retain(|n, _| !n.starts_with(&p));
                        removed_paths.push(p);
                    }
                }
                dir.children.sort_by_cached_key(|path| {
                    let n = &self.nodes[path];
                    (!n.directory, n.name.to_lowercase(), n.name.clone())
                });
                dir.complete = complete;
            }
            Err(e) => dir.error = format!("{e:#}"),
        }
        for p in removed_paths {
            self.dirs.retain(|n, _| !n.starts_with(&p));
            self.selected_paths.retain(|n| !n.starts_with(&p));
        }
        if self.compact {
            self.reproject();
        }
        if let Some(restore) = self.restore.clone() {
            if self.nodes.contains_key(&restore) {
                let id = self.identifier(&restore);
                if self.state.selected() != id {
                    self.state.select(id);
                }
                self.restore = None;
            } else if self.dirs.values().all(|d| d.complete && !d.loading) {
                self.restore = None;
                self.repair_selection();
            }
        } else {
            self.repair_selection();
        }
    }
    pub fn repair_selection(&mut self) {
        if let Some(Key::Entry(mut p)) = self.selected() {
            loop {
                let id = self.identifier(&p);
                let visible = self.nodes.contains_key(&p)
                    && id
                        .iter()
                        .enumerate()
                        .skip(1)
                        .all(|(i, _)| self.state.opened().contains(&id[..i]));
                if visible {
                    if self.state.selected() != id {
                        self.state.select(id);
                    }
                    break;
                }
                if p == self.root || !p.pop() {
                    self.state.select(vec![Key::Entry(self.root.clone())]);
                    break;
                }
            }
        }
    }
    pub fn toggle(&mut self, path: &Path, io: &Workers) -> Result<()> {
        self.revision = self.revision.wrapping_add(1);
        let id = self.identifier(path);
        if self.state.opened().contains(&id) {
            self.state.close(&id);
            io.send(Job::Close(path.into()))?;
            if let Some(d) = self.dirs.get_mut(path) {
                d.generation = 0;
                d.loading = false;
            }
            self.repair_selection();
        } else {
            self.state.open(id);
            if !self.dirs.get(path).is_some_and(|d| d.complete) {
                self.request(path, false, io)?;
            }
        }
        Ok(())
    }
    pub fn collapse(&mut self, io: &Workers) {
        self.revision = self.revision.wrapping_add(1);
        self.state.close_all();
        self.state.open(vec![Key::Entry(self.root.clone())]);
        let paths: Vec<_> = self
            .dirs
            .keys()
            .filter(|p| **p != self.root)
            .cloned()
            .collect();
        for p in paths {
            let _ = io.send(Job::Close(p.clone()));
            if let Some(d) = self.dirs.get_mut(&p) {
                d.generation = 0;
                d.loading = false;
            }
        }
        self.repair_selection();
    }
    pub fn release_collapsed(&mut self, io: &Workers) {
        self.revision = self.revision.wrapping_add(1);
        let selected = self.selected_path();
        let remove: Vec<_> = self
            .dirs
            .keys()
            .filter(|p| {
                **p != self.root
                    && !self.state.opened().contains(&self.identifier(p))
                    && !selected.as_ref().is_some_and(|s| s.starts_with(p))
                    && !self.pinned.as_ref().is_some_and(|s| s.starts_with(p))
            })
            .cloned()
            .collect();
        for dir in remove {
            let _ = io.send(Job::Close(dir.clone()));
            self.dirs.remove(&dir);
            let dead: Vec<_> = self
                .nodes
                .keys()
                .filter(|p| **p != dir && p.starts_with(&dir))
                .cloned()
                .collect();
            for p in dead {
                self.nodes.remove(&p);
                self.dirs.remove(&p);
            }
        }
    }
    pub fn refresh(&mut self, io: &Workers) -> Result<()> {
        self.revision = self.revision.wrapping_add(1);
        self.restore = self.selected_path();
        self.refresh_pending.clear();
        for (path, dir) in &mut self.dirs {
            if dir.loading {
                let _ = io.send(Job::Close(path.clone()));
            }
            dir.loading = false;
            dir.generation = 0;
            if path != &self.root {
                self.refresh_pending.push_back(path.clone());
            }
        }
        self.refresh_pending.push_front(self.root.clone());
        self.pump_refresh(io);
        Ok(())
    }
    pub fn pump_refresh(&mut self, io: &Workers) {
        let Some(path) = self.refresh_pending.front().cloned() else {
            return;
        };
        if !self.nodes.contains_key(&path) {
            self.refresh_pending.pop_front();
            return;
        }
        match self.request(&path, true, io) {
            Ok(()) => {
                self.refresh_pending.pop_front();
            }
            Err(e) => {
                if let Some(dir) = self.dirs.get_mut(&path) {
                    dir.error = e.to_string();
                }
            }
        }
    }
    pub fn refresh_paths(&mut self, paths: &HashSet<PathBuf>, io: &Workers) -> Result<()> {
        if paths.iter().any(|p| {
            p.file_name()
                .is_some_and(|n| n == ".gitignore" || n == ".ignore")
        }) {
            return self.refresh(io);
        }
        let targets: Vec<_> = self
            .dirs
            .keys()
            .filter(|d| {
                paths
                    .iter()
                    .any(|p| p == *d || p.parent() == Some(d.as_path()))
            })
            .cloned()
            .collect();
        for p in targets {
            if let Some(d) = self.dirs.get_mut(&p) {
                d.loading = false;
                d.generation = 0;
            }
            self.request(&p, true, io)?;
        }
        Ok(())
    }
    pub fn items(&self, preview: Option<&Path>, width: usize) -> Vec<TreeItem<'static, Key>> {
        let _perf = crate::perf::span(crate::perf::Metric::TreeItems);
        vec![self.item(&self.root, preview, width, 0)]
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn item(
        &self,
        path: &Path,
        preview: Option<&Path>,
        width: usize,
        depth: usize,
    ) -> TreeItem<'static, Key> {
        let start = path;
        let projected = self.compact_target(path);
        let path = projected.as_path();
        let n = &self.nodes[path];
        let icon = if self.icons {
            format!("{} ", n.icon)
        } else if n.directory {
            "d ".into()
        } else {
            "f ".into()
        };
        let status = crate::explorer::status_for(&self.git, path);
        let badge = format!(
            "{}{}{}",
            if n.link { " @" } else { "" },
            if preview == Some(path) { " [P]" } else { "" },
            status.map_or_else(String::new, |s| format!(" {}", s.badge()))
        );
        let label = if path != start {
            format!(
                "{} / {}",
                self.nodes[start].name,
                path.strip_prefix(start)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', " / ")
            )
        } else {
            n.name.clone()
        };
        let name_width = width.saturating_sub(depth * 2 + 6 + badge.len());
        let name = preview::truncate_name(&preview::safe_text(&label), name_width);
        let name = format!(
            "{name}{}",
            " ".repeat(name_width.saturating_sub(name.width()))
        );
        let color = if self.light { Color::Blue } else { n.color };
        let line = Line::from(vec![
            Span::styled(icon, Style::new().fg(color)),
            Span::raw(name),
            Span::styled(
                format!(
                    "{}{}",
                    if n.link { " @" } else { "" },
                    if preview == Some(path) { " [P]" } else { "" }
                ),
                Style::new().fg(Color::Cyan),
            ),
            Span::styled(
                status.map_or_else(String::new, |s| format!(" {}", s.badge())),
                Style::new().fg(status.map_or(Color::Reset, |s| s.color(self.light))),
            ),
        ]);
        let mut children = Vec::new();
        if n.directory {
            if let Some(dir) = self.dirs.get(path) {
                for child in &dir.children {
                    if self.nodes.contains_key(child)
                        && (!self.modified_only
                            || crate::explorer::status_for(&self.git, child)
                                .is_some_and(|s| s.changed()))
                    {
                        children.push(self.item(child, preview, width, depth + 1));
                    }
                }
                let status = if dir.loading {
                    crate::i18n::t("ui.loading").into_owned()
                } else if !dir.error.is_empty() {
                    format!("Error: {} (Enter retries)", dir.error)
                } else if !dir.complete {
                    format!("Load more · {} loaded", dir.children.len())
                } else if self.modified_only && children.is_empty() {
                    crate::i18n::t("ui.no_modified_entries").into()
                } else if dir.children.is_empty() {
                    if dir.scanned == 0 {
                        crate::i18n::t("ui.empty_directory").into()
                    } else {
                        crate::i18n::t("ui.no_visible_entries_check_filters").into()
                    }
                } else {
                    String::new()
                };
                if !status.is_empty() {
                    children.push(TreeItem::new_leaf(
                        if !dir.complete && !dir.loading {
                            Key::More(path.into())
                        } else {
                            Key::Notice(path.into())
                        },
                        status,
                    ));
                }
            } else {
                children.push(TreeItem::new_leaf(
                    Key::More(path.into()),
                    crate::i18n::t("ui.expand_to_load"),
                ));
            }
        }
        TreeItem::new(Key::Entry(path.into()), line, children).expect("unique entry paths")
    }
    pub fn watch(&mut self, active: bool, preview: Option<&Path>) {
        self.watch_images(active, preview, &[]);
    }
    pub fn watch_images(&mut self, active: bool, preview: Option<&Path>, images: &[PathBuf]) {
        let _perf = crate::perf::span(crate::perf::Metric::Watch);
        if !active {
            self.watcher = None;
            self.watched.clear();
            return;
        }
        if self.watcher.is_none() {
            if !self.watch_error.is_empty() {
                return;
            }
            let tx = self.change_tx.clone();
            let overflow = self.overflow.clone();
            match notify::recommended_watcher(move |event| {
                if tx.try_send(event).is_err() {
                    overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }) {
                Ok(w) => {
                    self.watcher = Some(w);
                    self.watch_error.clear();
                }
                Err(e) => {
                    self.watch_error = format!("Auto refresh unavailable: {e}; F5 retries");
                    return;
                }
            }
        }
        let mut wanted: HashSet<PathBuf> = self.dirs.keys().cloned().collect();
        wanted.insert(self.root.clone());
        if let Some(p) = preview.and_then(Path::parent) {
            wanted.insert(p.into());
        }
        wanted.extend(
            images
                .iter()
                .filter_map(|p| p.parent().map(Path::to_path_buf)),
        );
        let watcher = self.watcher.as_mut().expect("watcher");
        for p in self.watched.difference(&wanted) {
            let _ = watcher.unwatch(p);
        }
        for p in wanted.difference(&self.watched) {
            if let Err(e) = watcher.watch(p, RecursiveMode::NonRecursive) {
                self.watch_error = format!("Auto refresh unavailable for {}: {e}; F5", p.display());
            }
        }
        self.watched = wanted;
    }
    pub fn retry_watch(&mut self) {
        self.watcher = None;
        self.watched.clear();
        self.watch_error.clear();
    }
    pub fn changes(&mut self) -> Option<HashSet<PathBuf>> {
        while let Ok(event) = self.changes.try_recv() {
            self.changed_at = Some(Instant::now());
            match event {
                Ok(e) => {
                    if !matches!(e.kind, notify::EventKind::Access(_)) {
                        for p in e.paths {
                            if self.pending.len() < 128 {
                                self.pending.insert(p);
                            } else {
                                self.overflow
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
                Err(e) => {
                    self.watch_error = format!("Watch error: {e}; F5");
                    self.overflow
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        if self
            .overflow
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.pending.insert(self.root.clone());
            self.changed_at = Some(Instant::now());
        }
        if self
            .changed_at
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(200))
        {
            self.changed_at = None;
            Some(std::mem::take(&mut self.pending))
        } else {
            None
        }
    }
}

pub fn path_input(path: &Path) -> Result<String> {
    ensure!(
        path.is_absolute(),
        crate::i18n::t("ui.expected_absolute_file_path")
    );
    let s = path
        .to_str()
        .context(crate::i18n::t("ui.path_is_not_valid_unicode"))?;
    ensure!(
        !s.chars().any(char::is_control) && !s.contains('"'),
        crate::i18n::t("ui.path_contains_characters_that_cannot_be_safely_inserted")
    );
    Ok(format!("\"{s}\" "))
}
pub fn copy(text: String) -> Result<()> {
    arboard::Clipboard::new()?.set_text(text)?;
    Ok(())
}
pub fn external(path: &Path, reveal: bool) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        if reveal {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        command.creation_flags(0x08000000).spawn()?;
    }
    #[cfg(not(windows))]
    {
        let _ = (path, reveal);
        anyhow::bail!("Windows required");
    }
    Ok(())
}
