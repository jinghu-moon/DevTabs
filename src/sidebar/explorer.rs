//! Explorer interaction state; filesystem and project scans live in crate::explorer.
use super::*;
use crate::explorer::{
    self as project, Completed as Done, Job as Work, Operation, Preferences, Scope,
};

pub(super) struct Explorer {
    pub(super) worker: project::Worker,
    pub preferences: Preferences,
    pub busy: bool,
    picker: Option<Picker>,
    form: Option<Form>,
    anchor: Option<PathBuf>,
    suppress_preview: bool,
    resizing: bool,
    resize_preview: bool,
    resize_origin: Preferences,
    hover: Option<(PathBuf, Instant)>,
    initialized: bool,
    preferences_changed: Option<Instant>,
    preferences_touched: bool,
    git_pending: bool,
    git_at: Option<Instant>,
    git_error: String,
    revision: u64,
    serial: u64,
    pub metrics: String,
}
struct Picker {
    value: String,
    paths: Vec<PathBuf>,
    selected: usize,
    id: u64,
    due: Option<Instant>,
    pending: bool,
    status: String,
}
#[derive(Clone, Copy)]
enum NameKind {
    File,
    Directory,
    Rename,
}
enum Form {
    Name {
        kind: NameKind,
        target: PathBuf,
        value: String,
    },
    Delete(Vec<PathBuf>),
}
impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            worker: project::Worker::new(root),
            preferences: Default::default(),
            busy: false,
            picker: None,
            form: None,
            anchor: None,
            suppress_preview: false,
            resizing: false,
            resize_preview: false,
            resize_origin: Preferences::default(),
            hover: None,
            initialized: false,
            preferences_changed: None,
            preferences_touched: false,
            git_pending: false,
            git_at: None,
            git_error: String::new(),
            revision: 0,
            serial: 0,
            metrics: String::new(),
        }
    }
    pub fn modal(&self) -> bool {
        self.picker.is_some() || self.form.is_some() || self.busy || self.resizing
    }
    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.git_at = None;
        if let Some(p) = &mut self.picker {
            p.due = Some(Instant::now());
            p.pending = true;
        }
    }
    fn preferences_changed(&mut self) {
        self.preferences_touched = true;
        self.preferences_changed = Some(Instant::now());
    }
}
impl Sidebar {
    pub fn cycle_language(&mut self) -> Result<()> {
        let language = crate::i18n::preference().next();
        self.explorer.worker.send(Work::Language(language))?;
        crate::i18n::set(language);
        self.files.reproject();
        self.menu = None;
        Ok(())
    }
    pub fn explorer_metrics(&self) -> &str {
        &self.explorer.metrics
    }
    pub fn sidebar_preferences(&self) -> &Preferences {
        &self.explorer.preferences
    }
    pub(crate) fn resizing(&self) -> bool {
        self.explorer.resizing
    }
    pub fn search_results(&self) -> Option<&[PathBuf]> {
        self.explorer
            .picker
            .as_ref()
            .filter(|p| !p.pending)
            .map(|p| p.paths.as_slice())
    }
    pub fn project_busy(&self) -> bool {
        self.explorer.busy
    }
    pub fn quick_open(&mut self) {
        if self.modal() {
            return;
        }
        // The lazy watcher deliberately does not watch every unopened directory.
        // Reopening the picker obtains a fresh project index; keystrokes reuse it.
        self.explorer.invalidate();
        self.cancel_drag(false);
        self.explorer.picker = Some(Picker {
            value: String::new(),
            paths: Vec::new(),
            selected: 0,
            id: 0,
            due: Some(Instant::now() - Duration::from_secs(1)),
            pending: true,
            status: crate::i18n::t("ui.indexing_project").into(),
        });
    }
    pub(super) fn explorer_tick(&mut self, active: bool) -> bool {
        let mut changed = false;
        if active
            && !self.explorer.initialized
            && self.explorer.worker.send(Work::LoadPreferences).is_ok()
        {
            self.explorer.initialized = true;
        }
        if active
            && !self.explorer.git_pending
            && self
                .explorer
                .git_at
                .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
            && self.explorer.worker.send(Work::Git).is_ok()
        {
            self.explorer.git_pending = true;
            self.explorer.git_at = Some(Instant::now());
        }
        if self
            .explorer
            .preferences_changed
            .is_some_and(|t| t.elapsed() > Duration::from_millis(500))
            && self
                .explorer
                .worker
                .send(Work::Preferences(self.explorer.preferences.clone()))
                .is_ok()
        {
            self.explorer.preferences_changed = None;
        }
        if let Some(p) = &mut self.explorer.picker
            && p.due
                .is_some_and(|t| t.elapsed() > Duration::from_millis(120))
        {
            self.explorer.serial = self.explorer.serial.wrapping_add(1);
            let id = self.explorer.serial;
            if self
                .explorer
                .worker
                .search(
                    id,
                    p.value.clone(),
                    Scope {
                        hidden: self.files.hidden,
                        ignored: self.files.ignored,
                        revision: self.explorer.revision,
                    },
                )
                .is_ok()
            {
                p.id = id;
                p.due = None;
                p.pending = true;
            }
        }
        while let Ok(done) = self.explorer.worker.rx.try_recv() {
            changed = true;
            match done {
                Done::Preferences(result) => match result {
                    Ok(mut p) if !self.explorer.preferences_touched => {
                        p.width = p.width.clamp(20, 80);
                        p.agent_ratio = p.agent_ratio.clamp(1, 9999);
                        self.explorer.preferences = p;
                        self.files.compact = self.explorer.preferences.compact;
                        self.files.reproject();
                    }
                    Err(e) => self.say(format!("Sidebar settings: {e:#}"), false),
                    _ => {}
                },
                Done::SavedPreferences(result) => {
                    if let Err(e) = result {
                        self.say(format!("Cannot save sidebar settings: {e:#}"), false);
                    }
                }
                Done::Search { id, result } => {
                    self.explorer.metrics = format!(
                        "{} indexed · {} matches · {:.2} ms fuzzy search",
                        result.scanned,
                        result.total,
                        result.elapsed.as_secs_f64() * 1000.0
                    );
                    if let Some(p) = &mut self.explorer.picker
                        && p.id == id
                        && p.due.is_none()
                    {
                        p.paths = result.paths;
                        p.selected = 0;
                        p.pending = false;
                        p.status = format!(
                            "{} matches · {} files{}{}",
                            result.total,
                            result.scanned,
                            if result.limited {
                                crate::i18n::t("ui.index_depth_limit_reached")
                            } else {
                                "".into()
                            },
                            if result.errors > 0 {
                                crate::i18n::t("ui.entries_skipped")
                            } else {
                                "".into()
                            }
                        );
                    }
                }
                Done::Git(result) => {
                    self.explorer.git_pending = false;
                    match result {
                        Ok(git) => {
                            self.explorer.git_error.clear();
                            if self.files.git != git {
                                self.files.git = git;
                                self.files.touch();
                            }
                        }
                        Err(e) => {
                            self.explorer.git_error = e.to_string();
                            if !self.files.git.is_empty() {
                                self.files.git.clear();
                                self.files.touch();
                            }
                        }
                    }
                }
                Done::Change(change) => {
                    self.explorer.busy = false;
                    self.apply_file_changes(&change);
                    if let Some(e) = change.error {
                        self.say(e, false);
                    } else {
                        self.say(crate::i18n::t("ui.file_operation_completed"), true);
                    }
                }
                Done::Input(result) => {
                    self.checking_input = false;
                    match result {
                        Ok(paths) => self.ready_input = Some(paths),
                        Err(e) => self.say(format!("Paths not sent: {e:#}"), false),
                    }
                }
                Done::External(result) => {
                    if let Err(e) = result {
                        self.say(format!("External open: {e:#}"), false);
                    }
                }
            }
        }
        // A tooltip only causes a redraw when it first becomes visible.
        if self.explorer.hover.as_ref().is_some_and(|(_, t)| {
            t.elapsed() > Duration::from_millis(450) && t.elapsed() < Duration::from_millis(550)
        }) {
            changed = true;
        }
        changed
    }
    fn apply_file_changes(&mut self, change: &project::Change) {
        if change.paths.is_empty() {
            return;
        }
        let mut reopen = None;
        for (old, new) in &change.paths {
            if new.as_ref() == Some(old) {
                self.locate_pending = Some(old.clone());
                continue;
            }
            if let Some(d) = &self.document
                && d.path.starts_with(old)
            {
                reopen = new.as_ref().map(|n| project::relocate(&d.path, old, n));
                self.document = None;
                self.image_worker.release();
                self.preview_visible = false;
            }
            self.tools.paths.retain_mut(|p| {
                if p.starts_with(old) {
                    if let Some(new) = new {
                        *p = project::relocate(p, old, new);
                    } else {
                        return false;
                    }
                }
                true
            });
            self.tools.parked.retain(|d| !d.path.starts_with(old));
            self.reader.remap_paths(old, new.as_deref());
            self.files.selected_paths.retain(|p| !p.starts_with(old));
        }
        self.explorer.invalidate();
        self.explorer.anchor = None;
        self.tree_view = None;
        let _ = self.files.refresh(&self.io);
        if let Some(path) = reopen {
            if let Err(e) = self.open(path, true) {
                self.say(e.to_string(), false);
            }
        } else if self.document.is_none() {
            self.focus = Focus::Tree;
        }
    }
    pub(super) fn request_operation(&mut self, op: Operation) -> Result<()> {
        ensure!(
            !self.explorer.busy,
            crate::i18n::t("ui.a_file_operation_is_already_running")
        );
        let affected = op.affected();
        if self.dirty()
            && self
                .document
                .as_ref()
                .is_some_and(|d| affected.iter().any(|p| d.path.starts_with(p)))
        {
            self.prompt = Some(Prompt::Unsaved(Pending::FileOperation(op)));
            return Ok(());
        }
        ensure!(
            !self
                .tools
                .parked
                .iter()
                .any(|d| d.editor().is_some_and(|e| e.dirty() || e.saving)
                    && affected.iter().any(|p| d.path.starts_with(p))),
            crate::i18n::t("ui.save_the_affected_tabs_before_changing_files")
        );
        self.explorer.worker.cancel();
        self.explorer.worker.send(Work::Change(op))?;
        self.explorer.busy = true;
        self.gesture = Gesture::Idle;
        self.say(crate::i18n::t("ui.working_on_files"), false);
        Ok(())
    }
    pub(super) fn open_paths(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        ensure!(
            !paths.is_empty(),
            crate::i18n::t("ui.select_at_least_one_file_to_preview")
        );
        ensure!(
            paths.len() <= 8,
            crate::i18n::t("ui.preview_supports_at_most_8_tabs_select_fewer_files")
        );
        if self.dirty() {
            self.prompt = Some(Prompt::Unsaved(Pending::OpenPaths(paths)));
            return Ok(());
        }
        for path in paths {
            self.open(path, true)?;
        }
        Ok(())
    }
    pub(super) fn external_paths(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        if self.dirty()
            && self
                .document
                .as_ref()
                .is_some_and(|d| paths.contains(&d.path))
        {
            self.prompt = Some(Prompt::Unsaved(Pending::ExternalPaths(paths)));
            return Ok(());
        }
        self.explorer.worker.send(Work::External(paths))
    }
    fn selection(&self, target: &std::path::Path, files_only: bool) -> Vec<PathBuf> {
        let mut paths = if self.focus == Focus::Tree && self.files.selected_paths.contains(target) {
            self.files
                .selected_paths
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![target.to_owned()]
        };
        if files_only {
            paths.retain(|p| self.files.nodes.get(p).is_none_or(|n| !n.directory));
        }
        paths.sort();
        paths
    }
    pub(crate) fn insert_selection(&self, target: PathBuf) -> Action {
        let paths = self.selection(&target, true);
        Action::Insert(paths)
    }
    pub fn validate_selected_paths(&self, paths: Vec<PathBuf>) -> Result<()> {
        self.explorer.worker.send(Work::Validate(paths))
    }
    pub(super) fn explorer_command(
        &mut self,
        c: Command,
        target: &std::path::Path,
    ) -> Result<Option<Action>> {
        use Command::*;
        if self.read_only
            && matches!(
                c,
                NewFile | NewDirectory | Rename | Trash | Follow | Compact | Modified
            )
        {
            return Ok(Some(Action::None));
        }
        match c {
            QuickOpen => self.quick_open(),
            Follow => {
                self.explorer.preferences.follow = !self.explorer.preferences.follow;
                self.explorer.preferences_changed();
                if self.explorer.preferences.follow {
                    self.locate_pending = self.document.as_ref().map(|d| d.path.clone());
                    self.locate_explicit = false;
                }
            }
            Compact => {
                self.files.compact = !self.files.compact;
                self.explorer.preferences.compact = self.files.compact;
                self.explorer.preferences_changed();
                self.files.reproject();
            }
            Modified => {
                self.files.modified_only = !self.files.modified_only;
                self.files.touch();
                self.tree_view = None;
                self.files.state.select_first();
            }
            NewFile | NewDirectory | Rename => {
                let kind = match c {
                    NewFile => NameKind::File,
                    NewDirectory => NameKind::Directory,
                    _ => NameKind::Rename,
                };
                if matches!(kind, NameKind::Rename) {
                    ensure!(
                        target != self.files.root,
                        crate::i18n::t("explorer.protected_root")
                    );
                }
                let base = if matches!(kind, NameKind::Rename)
                    || self.files.nodes.get(target).is_some_and(|n| n.directory)
                {
                    target.to_owned()
                } else {
                    target.parent().unwrap_or(&self.files.root).to_owned()
                };
                let value = if matches!(kind, NameKind::Rename) {
                    base.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    String::new()
                };
                self.explorer.form = Some(Form::Name {
                    kind,
                    target: base,
                    value,
                });
            }
            Trash => {
                let paths = self.selection(target, false);
                ensure!(
                    !paths.contains(&self.files.root),
                    crate::i18n::t("explorer.protected_root")
                );
                self.explorer.form = Some(Form::Delete(paths));
            }
            PreviewSelected => self.open_paths(self.selection(target, true))?,
            Insert if self.focus == Focus::Tree => {
                return Ok(Some(self.insert_selection(target.to_owned())));
            }
            CopyAbsolute | CopyRelative if self.focus == Focus::Tree => {
                let paths = self.selection(target, false);
                let text = paths
                    .iter()
                    .map(|p| {
                        let p = if matches!(c, CopyRelative) {
                            p.strip_prefix(&self.files.root).unwrap_or(p)
                        } else {
                            p
                        };
                        if p.as_os_str().is_empty() {
                            ".".into()
                        } else {
                            p.to_string_lossy().into_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                files::copy(text)?;
                self.say(format!("Copied {} paths", paths.len()), true);
            }
            External if self.focus == Focus::Tree => {
                self.external_paths(self.selection(target, false))?
            }
            _ => return Ok(None),
        }
        Ok(Some(Action::None))
    }
    pub(super) fn select_tree(&mut self, modifiers: KeyModifiers, previous: Option<PathBuf>) {
        let Some(path) = self.files.selected_path() else {
            return;
        };
        self.explorer.suppress_preview =
            modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        if modifiers.contains(KeyModifiers::SHIFT) {
            let anchor = self
                .explorer
                .anchor
                .clone()
                .or(previous)
                .unwrap_or_else(|| path.clone());
            self.explorer.anchor = Some(anchor.clone());
            let rows = self.visible_paths();
            if let (Some(a), Some(b)) = (
                rows.iter().position(|p| p == &anchor),
                rows.iter().position(|p| p == &path),
            ) {
                if !modifiers.contains(KeyModifiers::CONTROL) {
                    self.files.selected_paths.clear();
                }
                self.files.selected_paths.extend(
                    rows[a.min(b)..=a.max(b)]
                        .iter()
                        .take(
                            project::SELECTION_LIMIT
                                .saturating_sub(self.files.selected_paths.len()),
                        )
                        .cloned(),
                );
            }
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            if !self.files.selected_paths.remove(&path)
                && self.files.selected_paths.len() < project::SELECTION_LIMIT
            {
                self.files.selected_paths.insert(path.clone());
            }
            self.explorer.anchor = Some(path);
        } else {
            if !self.files.selected_paths.contains(&path) {
                self.files.selected_paths.clear();
                self.files.selected_paths.insert(path.clone());
            }
            self.explorer.anchor = Some(path);
        }
    }
    fn visible_paths(&self) -> Vec<PathBuf> {
        self.tree_view.as_ref().map_or_else(Vec::new, |v| {
            self.files
                .state
                .flatten(&v.items)
                .iter()
                .filter_map(|r| match r.identifier.last() {
                    Some(Key::Entry(p)) => Some(p.clone()),
                    _ => None,
                })
                .collect()
        })
    }
    pub(super) fn click_preview_allowed(&self) -> bool {
        !self.explorer.suppress_preview && self.files.selected_paths.len() <= 1
    }
    pub(super) fn explorer_paste(&mut self, text: &str) -> bool {
        let value = if let Some(p) = &mut self.explorer.picker {
            p.due = Some(Instant::now());
            p.pending = true;
            Some(&mut p.value)
        } else if let Some(Form::Name { value, .. }) = &mut self.explorer.form {
            Some(value)
        } else {
            None
        };
        if let Some(value) = value {
            let left = 256usize.saturating_sub(value.chars().count());
            value.extend(text.chars().filter(|c| !c.is_control()).take(left));
            true
        } else {
            self.explorer.busy || self.explorer.form.is_some()
        }
    }
    pub(super) fn explorer_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if self.read_only {
            return Ok(None);
        }
        if self.explorer.resizing {
            if key.code == KeyCode::Esc {
                self.explorer.resizing = false;
                self.explorer.preferences = self.explorer.resize_origin.clone();
                self.update_layout(self.tools.area, self.tools.split);
            }
            return Ok(Some(Action::None));
        }
        if self.explorer.busy {
            return Ok(Some(Action::None));
        }
        if self.prompt.is_some() || self.menu.is_some() {
            return Ok(None);
        }
        if self.explorer.picker.is_some() {
            let p = self.explorer.picker.as_mut().unwrap();
            match key.code {
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                    p.value.clear();
                    p.pending = true;
                    p.due = Some(Instant::now());
                }
                KeyCode::F(5) => {
                    self.explorer.revision = self.explorer.revision.wrapping_add(1);
                    p.pending = true;
                    p.due = Some(Instant::now() - Duration::from_secs(1));
                }
                KeyCode::Esc => {
                    self.explorer.worker.cancel();
                    self.explorer.picker = None;
                }
                KeyCode::Enter if !p.pending => {
                    if let Some(path) = p.paths.get(p.selected).cloned() {
                        self.explorer.picker = None;
                        self.open(path, true)?;
                    }
                }
                KeyCode::Up => p.selected = p.selected.saturating_sub(1),
                KeyCode::Down => p.selected = (p.selected + 1).min(p.paths.len().saturating_sub(1)),
                KeyCode::PageUp => p.selected = p.selected.saturating_sub(10),
                KeyCode::PageDown => {
                    p.selected = (p.selected + 10).min(p.paths.len().saturating_sub(1))
                }
                KeyCode::Backspace => {
                    p.value.pop();
                    p.pending = true;
                    p.due = Some(Instant::now());
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && p.value.chars().count() < 256 =>
                {
                    p.value.push(c);
                    p.pending = true;
                    p.due = Some(Instant::now());
                }
                _ => {}
            }
            return Ok(Some(Action::None));
        }
        if self.explorer.form.is_some() {
            match key.code {
                KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                    if let Some(Form::Name { value, .. }) = &mut self.explorer.form {
                        value.clear();
                    }
                }
                KeyCode::Esc => self.explorer.form = None,
                KeyCode::Enter => {
                    let form = self.explorer.form.take().unwrap();
                    let op = match &form {
                        Form::Name {
                            kind,
                            target,
                            value,
                        } => {
                            let parent = if matches!(kind, NameKind::Rename) {
                                target.parent().unwrap_or(&self.files.root)
                            } else {
                                target
                            };
                            if let Err(e) = project::child_name(parent, value) {
                                self.explorer.form = Some(form);
                                self.say(e.to_string(), false);
                                return Ok(Some(Action::None));
                            }
                            match kind {
                                NameKind::File => Operation::File {
                                    parent: target.clone(),
                                    name: value.clone(),
                                },
                                NameKind::Directory => Operation::Directory {
                                    parent: target.clone(),
                                    name: value.clone(),
                                },
                                NameKind::Rename => Operation::Rename {
                                    path: target.clone(),
                                    name: value.clone(),
                                },
                            }
                        }
                        Form::Delete(paths) => Operation::Trash(paths.clone()),
                    };
                    self.request_operation(op)?;
                }
                KeyCode::Backspace => {
                    if let Some(Form::Name { value, .. }) = &mut self.explorer.form {
                        value.pop();
                    }
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if let Some(Form::Name { value, .. }) = &mut self.explorer.form
                        && value.chars().count() < 240
                    {
                        value.push(c);
                    }
                }
                _ => {}
            }
            return Ok(Some(Action::None));
        }
        if self.modal() {
            return Ok(None);
        }
        if key.code == KeyCode::Esc && !matches!(self.gesture, Gesture::Idle) {
            return Ok(None);
        }
        if key.code == KeyCode::Char('p') && key.modifiers == KeyModifiers::CONTROL {
            self.quick_open();
            return Ok(Some(Action::None));
        }
        if key.code == KeyCode::F(5) {
            self.explorer.invalidate();
        }
        if self.focus != Focus::Tree {
            return Ok(None);
        }
        let target = self
            .files
            .selected_path()
            .unwrap_or_else(|| self.files.root.clone());
        let command = match key.code {
            KeyCode::Delete => Some(Command::Trash),
            KeyCode::F(4) => Some(Command::Rename),
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => Some(Command::NewFile),
            KeyCode::Char('n' | 'N')
                if key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT =>
            {
                Some(Command::NewDirectory)
            }
            KeyCode::Char('g' | 'G') => Some(Command::Modified),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                Some(Command::CopyAbsolute)
            }
            KeyCode::Enter if self.files.selected_paths.len() > 1 => Some(Command::PreviewSelected),
            _ => None,
        };
        if let Some(c) = command {
            return self.explorer_command(c, &target);
        }
        if key.code == KeyCode::Esc && !self.files.selected_paths.is_empty() {
            self.files.selected_paths.clear();
            self.explorer.anchor = None;
            return Ok(Some(Action::None));
        }
        if matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        ) {
            let previous = self.files.selected_path();
            match key.code {
                KeyCode::Up => {
                    self.files.state.key_up();
                }
                KeyCode::Down => {
                    self.files.state.key_down();
                }
                KeyCode::Home => {
                    self.files.state.select_first();
                }
                KeyCode::End => {
                    self.files.state.select_last();
                }
                KeyCode::PageUp => {
                    for _ in 0..10 {
                        self.files.state.key_up();
                    }
                }
                _ => {
                    for _ in 0..10 {
                        self.files.state.key_down();
                    }
                }
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::SHIFT)
            {
                return Ok(Some(Action::None));
            }
            if !key.modifiers.contains(KeyModifiers::SHIFT) {
                self.files.selected_paths.clear();
            }
            self.select_tree(key.modifiers, previous);
            return Ok(Some(Action::None));
        }
        Ok(None)
    }
    fn picker_rect(&self) -> Rect {
        let a = self.tools.area;
        let w = a.width.saturating_sub(4).min(90);
        let h = a.height.saturating_sub(4).min(22);
        Rect::new(
            a.x + (a.width - w) / 2,
            a.y + 2.min(a.height.saturating_sub(h)),
            w,
            h,
        )
    }
    pub(super) fn explorer_mouse(&mut self, event: MouseEvent) -> Result<Option<Action>> {
        let point = (event.column, event.row);
        if self.explorer.picker.is_some() {
            let rect = self.picker_rect();
            let p = self.explorer.picker.as_mut().unwrap();
            let page = rect.height.saturating_sub(4) as usize;
            let first = p.selected.saturating_sub(page.saturating_sub(1));
            match event.kind {
                MouseEventKind::ScrollUp => p.selected = p.selected.saturating_sub(3),
                MouseEventKind::ScrollDown => {
                    p.selected = (p.selected + 3).min(p.paths.len().saturating_sub(1))
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if !rect.contains(point.into()) {
                        self.explorer.worker.cancel();
                        self.explorer.picker = None;
                    } else if event.row >= rect.y + 2
                        && event.row < rect.bottom().saturating_sub(2)
                        && !p.pending
                    {
                        let index = first + (event.row - rect.y - 2) as usize;
                        if let Some(path) = p.paths.get(index).cloned() {
                            self.explorer.picker = None;
                            self.open(path, true)?;
                        }
                    }
                }
                _ => {}
            }
            return Ok(Some(Action::None));
        }
        if self.explorer.busy || self.explorer.form.is_some() {
            return Ok(Some(Action::None));
        }
        if self.modal() && !self.explorer.resizing {
            return Ok(None);
        }
        if self.explorer.resizing {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    if self.explorer.resize_preview {
                        if let (Some(agent), Some(preview)) =
                            (self.geometry.agent, self.geometry.preview)
                            && !self.geometry.overlay
                        {
                            let total = agent.width + preview.width;
                            let desired =
                                event.column.saturating_sub(agent.x).clamp(62, total - 42);
                            // Basis points keep normal terminal widths cell-accurate.
                            self.explorer.preferences.agent_ratio =
                                (u32::from(desired) * 10000 / u32::from(total)) as u16;
                        }
                    } else {
                        self.explorer.preferences.width = event
                            .column
                            .saturating_sub(self.tools.area.x)
                            .saturating_add(1)
                            .clamp(20, 80);
                    }
                    self.update_layout(self.tools.area, self.tools.split);
                    if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                        self.explorer.resizing = false;
                        self.explorer.preferences_changed();
                    }
                }
                _ => {}
            }
            return Ok(Some(Action::None));
        }
        if self.enlarged.is_none()
            && !self.geometry.overlay
            && self.reader.drag.is_none()
            && matches!(self.gesture, Gesture::Idle)
            && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(preview) = self.geometry.preview
            && event.row > preview.y
            && event.row < preview.bottom().saturating_sub(1)
            && (event.column == preview.x || event.column == preview.x.saturating_sub(1))
        {
            self.explorer.resizing = true;
            self.explorer.resize_preview = true;
            self.explorer.resize_origin = self.explorer.preferences.clone();
            return Ok(Some(Action::None));
        }
        if let Some(rect) = self.geometry.tree {
            if self.enlarged.is_none()
                && (event.column == rect.right()
                    || (event.column == rect.right().saturating_sub(1)
                        && (!self.scrollbar_hit(point)
                            || event.modifiers.contains(KeyModifiers::ALT))))
                && event.row >= rect.y
                && event.row < rect.bottom()
                && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && self.reader.drag.is_none()
                && matches!(self.gesture, Gesture::Idle)
            {
                self.explorer.resizing = true;
                self.explorer.resize_preview = false;
                self.explorer.resize_origin = self.explorer.preferences.clone();
                return Ok(Some(Action::None));
            }
            if event.row == rect.y
                && rect.contains(point.into())
                && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            {
                let expanded = self.focus == Focus::Tree
                    || self.hover.is_some_and(|p| rect.contains(p.into()));
                self.focus = Focus::Tree;
                if event.column >= rect.right().saturating_sub(6) {
                    self.menu();
                } else if expanded && event.column >= rect.right().saturating_sub(10) {
                    self.quick_open();
                }
                return Ok(Some(Action::None));
            }
            if matches!(event.kind, MouseEventKind::Moved) {
                let hover = if rect.contains(point.into()) {
                    self.tree_hit(point)
                        .and_then(|id| id.last())
                        .and_then(|k| match k {
                            Key::Entry(p) => Some(p.clone()),
                            _ => None,
                        })
                } else {
                    None
                };
                if self.explorer.hover.as_ref().map(|(p, _)| p) != hover.as_ref() {
                    self.explorer.hover = hover.map(|p| (p, Instant::now()));
                }
                if rect.contains(point.into()) {
                    return Ok(Some(Action::None));
                }
            }
        }
        Ok(None)
    }
    pub(super) fn draw_explorer_header(&self, frame: &mut Frame) {
        if let Some(rect) = self.geometry.tree {
            let expanded =
                self.focus == Focus::Tree || self.hover.is_some_and(|p| rect.contains(p.into()));
            let right = Rect::new(rect.right().saturating_sub(10), rect.y, 9, 1);
            frame.render_widget(
                Paragraph::new(if expanded {
                    crate::i18n::t("button.symbol_1")
                } else {
                    "    [···]".into()
                })
                .style(Style::new().fg(Color::Cyan)),
                right,
            );
            let status = format!(
                " {}{}{} · {} sel · {} ",
                if self.files.hidden { "H" } else { "h" },
                if self.files.ignored { "I" } else { "i" },
                if self.files.modified_only { " Δ" } else { "" },
                self.files.selected_paths.len(),
                if self.explorer.git_pending {
                    crate::i18n::t("ui.git")
                } else if !self.explorer.git_error.is_empty() {
                    crate::i18n::t("ui.no_git")
                } else {
                    "Git".into()
                }
            );
            frame.render_widget(
                Paragraph::new(preview::truncate_name(
                    &status,
                    rect.width.saturating_sub(2) as usize,
                ))
                .style(Style::new().fg(Color::DarkGray)),
                Rect::new(
                    rect.x + 1,
                    rect.bottom() - 1,
                    rect.width.saturating_sub(2),
                    1,
                ),
            );
        }
    }
    pub(super) fn draw_explorer(&self, frame: &mut Frame) {
        if let Some(p) = &self.explorer.picker {
            let rect = self.picker_rect();
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Block::bordered()
                    .title(crate::i18n::t("project_files"))
                    .border_style(Style::new().fg(Color::Cyan)),
                rect,
            );
            let inner = crate::ui::content(rect);
            if inner.height == 0 {
                return;
            }
            frame.render_widget(
                Paragraph::new(preview::truncate_name(
                    &format!("> {}", p.value),
                    inner.width as usize,
                )),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let page = rect.height.saturating_sub(4) as usize;
            let first = p.selected.saturating_sub(page.saturating_sub(1));
            for (i, path) in p.paths.iter().enumerate().skip(first).take(page) {
                let label = path
                    .strip_prefix(&self.files.root)
                    .unwrap_or(path)
                    .to_string_lossy();
                frame.render_widget(
                    Paragraph::new(preview::truncate_name(
                        &preview::safe_text(&label),
                        inner.width as usize,
                    ))
                    .style(if i == p.selected {
                        Style::new().fg(Color::Black).bg(Color::Cyan)
                    } else {
                        Style::new()
                    }),
                    Rect::new(inner.x, inner.y + 1 + (i - first) as u16, inner.width, 1),
                );
            }
            let status = if p.pending {
                crate::i18n::t("searching")
            } else {
                std::borrow::Cow::Borrowed(p.status.as_str())
            };
            frame.render_widget(
                Paragraph::new(preview::truncate_name(&status, inner.width as usize))
                    .style(Style::new().fg(Color::DarkGray)),
                Rect::new(inner.x, rect.bottom().saturating_sub(2), inner.width, 1),
            );
        } else if let Some(form) = &self.explorer.form {
            let text = match form {
                Form::Name {
                    kind,
                    target,
                    value,
                } => format!(
                    "{}\n{}\n> {}\nEnter confirm · Esc cancel",
                    match kind {
                        NameKind::File => crate::i18n::t("ui.new_file"),
                        NameKind::Directory => crate::i18n::t("ui.new_directory"),
                        NameKind::Rename => crate::i18n::t("ui.rename"),
                    },
                    preview::safe_text(&target.to_string_lossy()),
                    value
                ),
                Form::Delete(paths) => format!(
                    "Move {} entries to Recycle Bin?\n{}\nEnter confirm · Esc cancel",
                    paths.len(),
                    paths
                        .iter()
                        .take(4)
                        .map(|p| preview::safe_text(&p.to_string_lossy()))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            };
            let a = frame.area();
            let w = a.width.saturating_sub(4).min(90);
            let h = a.height.saturating_sub(2).min(9);
            let r = Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
            frame.render_widget(Clear, r);
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::bordered()
                        .title(crate::i18n::t("files_title"))
                        .border_style(Style::new().fg(Color::Cyan)),
                ),
                r,
            );
        } else if !self.modal()
            && let Some((path, since)) = &self.explorer.hover
            && since.elapsed() > Duration::from_millis(450)
        {
            let a = frame.area();
            if a.height > 3 {
                let text = preview::safe_text(&path.to_string_lossy());
                let w = (text.width() as u16).saturating_add(4).min(a.width);
                let r = Rect::new(a.x, a.bottom() - 3, w, 3);
                frame.render_widget(Clear, r);
                frame.render_widget(Paragraph::new(text).block(Block::bordered()), r);
            }
        }
    }
}
