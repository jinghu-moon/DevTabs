//! Terminal settings centre. Every page edits the same sparse source draft.
mod support;
#[cfg(test)]
mod tests;
use super::{
    draft::{self, Draft, Part, Path},
    fields::{self, Field, Kind},
    store::{SettingsStore, Snapshot},
};
use anyhow::{Context, Result};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers as Mods, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use std::{path::PathBuf, sync::mpsc};
use support::option_label;
use toml::Value;

fn tr(zh: &str, en: &str) -> String {
    if crate::i18n::language() == crate::i18n::Language::ZhCn {
        zh.into()
    } else {
        en.into()
    }
}
#[derive(Clone)]
struct Page {
    path: Path,
    field: Field,
    selected: usize,
    original: Option<Value>,
}
struct Choice {
    path: Path,
    field: Field,
    selected: usize,
}
struct PathProbe {
    path: Path,
    value: String,
    result: String,
}
struct Edit {
    path: Path,
    field: Field,
    area: TextArea<'static>,
    map_key: Option<String>,
}
#[derive(Clone)]
enum Hit {
    SearchResult(usize),
    Choice(usize),
    SetChoice(Path, Field, usize),
    Slider(Path, i64),
    Issues,
    Issue(usize),
    Probe,
    PickProject,
    Picked(usize),
    ApplyPreset,
    Recapture,
    Copy,
    RestoreToggle(usize),
    RestoreApply,
    RestoreCancel,
    ResultClose,
    FieldHelp,
    Controls,
    Control(usize),
    PathDecision(u8),
    Step(i64),
    Override,
    ConfirmReset,
    ConfirmDelete,
    UseLatest,
    KeepMine,
    Page(usize),
    Row(usize),
    Open,
    Back,
    Save,
    Reset,
    Add,
    Delete,
    Up,
    Down,
    Inherit,
    Done,
    Cancel,
    Close,
    Reload,
    Discard,
    Continue,
    Shortcut(super::shortcut_ui::Action),
    Action(super::operations::Action),
}
pub enum Outcome {
    Stay,
    Closed,
    Saved,
}
enum Reply {
    Saved(Result<Snapshot, String>),
    Reloaded(Result<Snapshot, String>),
    Operation(Result<String, String>),
    Restore(Result<super::operations::RestorePlan, String>),
    Projects(Result<Vec<String>, String>),
}
pub struct SettingsUi {
    pub draft: Option<Draft>,
    pages: Vec<Field>,
    category: usize,
    stack: Vec<Page>,
    edit: Option<Edit>,
    search: String,
    searching: bool,
    search_selected: usize,
    nav: bool,
    close_prompt: bool,
    reset_prompt: bool,
    delete_prompt: Option<(Path, String)>,
    closing_after_save: bool,
    pub message: String,
    pub error: Option<String>,
    hits: Vec<(Rect, Hit)>,
    pointer: Option<(u16, u16)>,
    area: Rect,
    pending: Option<mpsc::Receiver<Reply>>,
    shortcuts: super::shortcut_ui::Manager,
    operation_path: String,
    operation_confirm: Option<super::operations::Action>,
    committed_language: crate::i18n::Language,
    project_placeholder: Option<(PathBuf, usize)>,
    conflict: Option<Snapshot>,
    conflict_scroll: u16,
    root: PathBuf,
    choice: Option<Choice>,
    path_decision: Option<(Path, Value)>,
    project_picker: Option<(Path, Field, Vec<String>, usize)>,
    footer_focus: Option<usize>,
    control_menu: Option<usize>,
    issues_open: bool,
    issue_selected: usize,
    probe: Option<mpsc::Receiver<PathProbe>>,
    probes: Vec<PathProbe>,
    restore: Option<super::operations::RestorePlan>,
    restore_selected: usize,
    pub can_queue: bool,
    pub image_status: String,
    operation_result: String,
    operation_scroll: u16,
    result_open: bool,
    restore_scroll: u16,
    pub queued_preset: Option<Vec<crate::launch::Options>>,
}
impl SettingsUi {
    pub fn open(project: Option<PathBuf>) -> Self {
        Self::at(SettingsStore::current().read(), project)
    }
    pub fn at(source: Result<Snapshot>, project: Option<PathBuf>) -> Self {
        let (draft, error) = match source {
            Ok(s) => (Some(Draft::new(s)), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let mut ui = Self {
            draft,
            pages: fields::pages(),
            category: 0,
            stack: vec![],
            edit: None,
            search: String::new(),
            searching: false,
            search_selected: 0,
            nav: true,
            close_prompt: false,
            reset_prompt: false,
            delete_prompt: None,
            closing_after_save: false,
            message: String::new(),
            error,
            hits: vec![],
            pointer: None,
            area: Rect::default(),
            pending: None,
            shortcuts: Default::default(),
            operation_path: String::new(),
            operation_confirm: None,
            committed_language: crate::i18n::language(),
            project_placeholder: None,
            conflict: None,
            conflict_scroll: 0,
            root: crate::config::home(),
            choice: None,
            path_decision: None,
            project_picker: None,
            footer_focus: None,
            control_menu: None,
            issues_open: false,
            issue_selected: 0,
            probe: None,
            probes: vec![],
            queued_preset: None,
            restore: None,
            restore_selected: 0,
            can_queue: true,
            image_status: tr(
                "未探测；总览没有活动图片终端能力结果。",
                "Not probed; overview has no active image-terminal capability result.",
            ),
            operation_result: tr(
                "宿主 / Hooks / 通知注册：尚未确认，请显式诊断。",
                "Host / Hooks / registration: unconfirmed; request diagnostics.",
            ),
            operation_scroll: 0,
            result_open: false,
            restore_scroll: 0,
        };
        if let Some(path) = project {
            ui.category = 8;
            ui.nav = false;
            if let Some(d) = &mut ui.draft {
                let found = d
                    .tree
                    .get("project")
                    .and_then(Value::as_array)
                    .and_then(|v| {
                        v.iter().position(|p| {
                            p.get("path").and_then(Value::as_str).is_some_and(|p| {
                                super::model::project_key(std::path::Path::new(p))
                                    == super::model::project_key(&path)
                            })
                        })
                    });
                let i = if let Some(i) = found {
                    i
                } else {
                    let mut project = toml::Table::new();
                    project.insert("path".into(), path.to_string_lossy().to_string().into());
                    let mut values = d
                        .tree
                        .get("project")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let i = values.len();
                    values.push(Value::Table(project));
                    // An empty project has no behavioral overrides; entering F4 does not add it.
                    let original = d.tree.clone();
                    ui.project_placeholder = Some((path.clone(), i));
                    ui.stack.push(Page {
                        path: draft::key(&[], "project"),
                        field: ui.pages[8].clone(),
                        selected: i,
                        original: original.get("project").cloned(),
                    });
                    i
                };
                if ui.stack.is_empty() {
                    ui.stack.push(Page {
                        path: draft::key(&[], "project"),
                        field: ui.pages[8].clone(),
                        selected: i,
                        original: d.tree.get("project").cloned(),
                    });
                }
                let p = draft::index(&draft::key(&[], "project"), i);
                ui.stack.push(Page {
                    original: draft::get(&d.tree, &p).cloned(),
                    path: p,
                    field: fields::project(),
                    selected: 0,
                });
            }
        }
        ui
    }
    pub fn at_root(root: PathBuf, project: Option<PathBuf>) -> Self {
        let mut ui = Self::at(SettingsStore::new(&root).read(), project);
        ui.shortcuts.root = root.clone();
        ui.root = root;
        ui
    }
    pub fn set_field(&mut self, path: &[Part], value: Option<Value>) -> Result<()> {
        self.change(path, value)
    }
    pub fn focus_path(&mut self, path: &[Part]) -> Result<()> {
        let Some(Part::Key(category)) = path.first() else {
            anyhow::bail!("Missing category");
        };
        self.category = self
            .pages
            .iter()
            .position(|f| f.key == category)
            .context("Unknown category")?;
        self.stack.clear();
        self.nav = false;
        let mut page = self.page();
        for part in &path[1..] {
            let children = self.children(&page);
            let i = children
                .iter()
                .position(|(p, _)| p.last() == Some(part))
                .context("Field not available")?;
            page.selected = i;
            self.stack.push(page.clone());
            let (path, field) = children[i].clone();
            page = Page {
                original: self
                    .draft
                    .as_ref()
                    .and_then(|d| draft::get(&d.tree, &path))
                    .cloned(),
                path,
                field,
                selected: 0,
            };
        }
        Ok(())
    }
    pub fn observe_raw(&mut self, raw: &crate::shortcuts::ledger::RawKey) {
        if self.category == 10 {
            self.shortcuts.observe_raw(raw);
        }
    }
    pub fn shortcuts_page(&mut self) {
        self.category = 10;
        self.stack.clear();
        self.nav = false;
    }
    pub fn dirty(&self) -> bool {
        self.edit.is_some() || self.draft.as_ref().is_some_and(Draft::dirty)
    }
    fn page(&self) -> Page {
        self.stack.last().cloned().unwrap_or_else(|| Page {
            path: draft::key(&[], self.pages[self.category].key),
            field: self.pages[self.category].clone(),
            selected: 0,
            original: None,
        })
    }
    fn selected(&self) -> usize {
        self.stack.last().map_or(0, |p| p.selected)
    }
    fn select(&mut self, n: usize) {
        if self.stack.is_empty() {
            let mut p = self.page();
            p.selected = n;
            self.stack.push(p);
        } else {
            self.stack.last_mut().unwrap().selected = n;
        }
    }
    fn children(&self, page: &Page) -> Vec<(Path, Field)> {
        match &page.field.kind {
            Kind::Object(fields) => fields
                .iter()
                .map(|f| (draft::key(&page.path, f.key), f.clone()))
                .collect(),
            Kind::List { item, .. } => {
                self.value(&page.path, &page.field)
                    .as_array()
                    .map_or(vec![], |a| {
                        (0..a.len())
                            .map(|i| {
                                let mut field = *item.clone();
                                if page.field.key == "argv" && i == 0 {
                                    field.zh = "程序（必填）";
                                    field.en = "Program (required)";
                                }
                                (draft::index(&page.path, i), field)
                            })
                            .collect()
                    })
            }
            Kind::Map => self
                .draft
                .as_ref()
                .and_then(|d| draft::get(&d.tree, &page.path))
                .and_then(Value::as_table)
                .map_or(vec![], |a| {
                    a.keys()
                        .map(|k| {
                            (
                                draft::key(&page.path, k),
                                Field {
                                    key: "",
                                    zh: "环境变量",
                                    en: "Environment variable",
                                    help: "空值保留。Empty values are retained.",
                                    kind: Kind::Text {
                                        multiline: false,
                                        path: false,
                                    },
                                    default: "".into(),
                                    optional: false,
                                    readonly: false,
                                },
                            )
                        })
                        .collect()
                }),
            _ => vec![],
        }
    }
    fn value(&self, path: &[Part], f: &Field) -> Value {
        if matches!(path.first(),Some(Part::Key(k)) if k=="project")
            && matches!(path.last(),Some(Part::Key(k)) if k=="path")
            && let Some((project, _)) = &self.project_placeholder
        {
            return project.to_string_lossy().to_string().into();
        }
        if path == draft::key(&draft::key(&[], "overview"), "list_ratio")
            && self
                .draft
                .as_ref()
                .is_some_and(|d| draft::get(&d.tree, path).is_none())
            && self
                .draft
                .as_ref()
                .and_then(|d| d.tree.get("overview"))
                .and_then(|v| v.get("statistics"))
                .and_then(Value::as_bool)
                == Some(false)
        {
            return 4400.into();
        }
        self.draft
            .as_ref()
            .and_then(|d| draft::get(&d.tree, path))
            .cloned()
            .unwrap_or_else(|| {
                self.draft
                    .as_ref()
                    .map_or_else(|| f.default.clone(), |d| d.inherited(path, f))
            })
    }
    fn change(&mut self, path: &[Part], v: Option<Value>) -> Result<()> {
        let d = self
            .draft
            .as_mut()
            .context("Repair the original configuration before saving")?;
        if matches!(path.first(),Some(Part::Key(k)) if k=="project")
            && v.is_some()
            && let Some((project, i)) = self.project_placeholder.take()
        {
            let mut values = d
                .tree
                .get("project")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if values.len() == i {
                let mut entry = toml::Table::new();
                entry.insert("path".into(), project.to_string_lossy().to_string().into());
                values.push(Value::Table(entry));
                draft::put(
                    &mut d.tree,
                    &draft::key(&[], "project"),
                    Some(values.into()),
                )?;
            }
        }
        draft::put(&mut d.tree, path, v)?;
        // Remove only empty containers created by this draft. Explicit empty arrays/maps remain values.
        for n in (1..path.len()).rev() {
            let parent = &path[..n];
            if draft::get(&d.base.tree, parent).is_none()
                && !matches!(parent.last(),Some(Part::Key(k)) if k=="env")
                && (draft::get(&d.tree, parent)
                    .and_then(Value::as_table)
                    .is_some_and(toml::Table::is_empty)
                    || (matches!(parent, [Part::Key(k)] if k == "project" || k == "preset")
                        && draft::get(&d.tree, parent)
                            .and_then(Value::as_array)
                            .is_some_and(Vec::is_empty)))
            {
                draft::put(&mut d.tree, parent, None)?;
            }
        }

        self.preview();
        Ok(())
    }
    fn preview(&mut self) {
        if let Some(d) = &self.draft {
            let theme = d
                .tree
                .get("appearance")
                .and_then(|v| v.get("theme"))
                .cloned()
                .map(|v| v.try_into())
                .transpose();
            if let Ok(theme) = theme {
                crate::appearance::set(theme.unwrap_or_default());
            }
        }
    }
    fn row(&self) -> Option<(Path, Field)> {
        let p = self.page();
        self.children(&p).get(self.selected()).cloned()
    }
    fn open_row(&mut self) -> Result<()> {
        let Some((path, field)) = self.row() else {
            return Ok(());
        };
        let value = self.value(&path, &field);
        if field.readonly && !value.as_str().unwrap_or("").is_empty() {
            crate::files::copy(value.as_str().unwrap_or("").into())?;
            self.message = tr("已复制完整路径", "Copied full path");
            return Ok(());
        }
        if self.inherited_lock(&path, &field) {
            self.message = tr(
                "当前继承全局值；按 O 或“设为覆盖”后编辑。",
                "Inherited value. Press O or Override before editing.",
            );
            return Ok(());
        }
        match &field.kind {
            Kind::Actions | Kind::Object(_) | Kind::List { .. } | Kind::Map => {
                let original = self
                    .draft
                    .as_ref()
                    .and_then(|d| draft::get(&d.tree, &path))
                    .cloned();
                if field.key == "check" && original.is_none() {
                    self.change(&path, Some(initial(&field)))?;
                }
                self.stack.push(Page {
                    path,
                    field,
                    selected: 0,
                    original,
                });
            }
            Kind::Toggle { inverse: _ } => {
                self.change(&path, Some((!value.as_bool().unwrap_or(false)).into()))?
            }
            Kind::Choice(values) => {
                let old = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                let selected = values.iter().position(|v| *v == old).unwrap_or(0);
                self.choice = Some(Choice {
                    path,
                    field,
                    selected,
                });
            }
            _ => self.start_edit(path, field, value, None),
        }
        Ok(())
    }
    fn start_edit(&mut self, path: Path, field: Field, value: Value, map_key: Option<String>) {
        let value = match (&field.kind, value) {
            (Kind::Number { percent: true, .. }, Value::Integer(v)) => {
                format!("{}.{:02}", v / 100, v % 100)
            }
            (_, Value::String(s)) => s,
            (_, Value::Integer(v)) => v.to_string(),
            _ => String::new(),
        };
        let mut area = TextArea::new(value.split('\n').map(str::to_string).collect());
        area.set_cursor_style(Style::new().bg(crate::appearance::palette().accent));
        self.edit = Some(Edit {
            path,
            field,
            area,
            map_key,
        });
    }
    fn finish_edit(&mut self) -> Result<()> {
        let Some(edit) = &self.edit else {
            return Ok(());
        };
        let text = edit.area.lines().join("\n");
        let value = match edit.field.kind {
            Kind::Number {
                min, max, percent, ..
            } => {
                let n = if percent {
                    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
                    anyhow::ensure!(
                        !whole.is_empty()
                            && whole.bytes().all(|b| b.is_ascii_digit())
                            && fraction.bytes().all(|b| b.is_ascii_digit())
                            && fraction.len() <= 2,
                        "Use a positive percentage with at most two decimal places"
                    );
                    whole
                        .parse::<i64>()?
                        .checked_mul(100)
                        .and_then(|v| v.checked_add(format!("{fraction:0<2}").parse::<i64>().ok()?))
                        .context("Percentage is too large")?
                } else {
                    text.parse()?
                };
                anyhow::ensure!((min..=max).contains(&n), "Range {min}..{max}");
                Value::Integer(n)
            }
            _ => text.into(),
        };
        let path = edit.path.clone();
        let map_key = edit.map_key.clone();
        if path == draft::key(&[], "$archive") {
            self.operation_path = value.as_str().unwrap_or("").into();
            self.edit = None;
            return Ok(());
        }
        if let Some(k) = map_key {
            let mut parent = path.clone();
            parent.pop();
            let name = value.as_str().unwrap_or("");
            anyhow::ensure!(
                !name.trim().is_empty() && !name.contains(['=', '\0']),
                "Invalid environment variable name"
            );
            let table =
                draft::get(&self.draft.as_ref().unwrap().tree, &parent).and_then(Value::as_table);
            anyhow::ensure!(
                !table.is_some_and(|t| t
                    .keys()
                    .any(|existing| existing.eq_ignore_ascii_case(name) && existing != &k)),
                "Duplicate environment variable (case insensitive)"
            );
            let new = draft::key(&parent, name);
            let previous = table
                .and_then(|t| t.get(&k))
                .cloned()
                .unwrap_or_else(|| "".into());
            if table.is_some_and(|t| t.contains_key(&k)) {
                self.change(&path, None)?;
            }
            self.change(&new, Some(previous))?;
        } else {
            let is_project = matches!(edit.field.kind, Kind::Text { path: true, .. })
                && matches!(path.last(),Some(Part::Key(k)) if k=="project")
                && matches!(path.first(),Some(Part::Key(k)) if k=="preset");
            let previous = draft::get(&self.draft.as_ref().unwrap().tree, &path)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if is_project && !previous.is_empty() && value.as_str() != Some(&previous) {
                self.path_decision = Some((path, value));
                self.edit = None;
                return Ok(());
            }
            self.change(&path, Some(value))?;
            if is_project && previous.is_empty() {
                if let Err(error) = self.recapture() {
                    if let Some(entry) = self.entry_path() {
                        self.change(&draft::key(&entry, "launch"), None)?;
                    }
                    return Err(error);
                }
            }
        }
        self.edit = None;
        self.start_probe();
        Ok(())
    }
    fn add(&mut self) -> Result<()> {
        let page = self.page();
        match page.field.kind.clone() {
            Kind::List { item, .. } => {
                let mut values = self
                    .value(&page.path, &page.field)
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut value = initial(&item);
                if page.field.key == "sessions" {
                    let mut n = 1;
                    loop {
                        let name = format!("Session {n}");
                        if !values
                            .iter()
                            .any(|v| v.get("name").and_then(Value::as_str) == Some(&name))
                        {
                            value
                                .as_table_mut()
                                .unwrap()
                                .insert("name".into(), name.into());
                            break;
                        }
                        n += 1;
                    }
                }
                let i = values.len();
                self.select(i);
                if let Kind::Object(_) = item.kind {
                    values.push(value.clone());
                    self.change(&page.path, Some(values.into()))?;
                    self.stack.push(Page {
                        path: draft::index(&page.path, i),
                        field: *item,
                        selected: 0,
                        original: None,
                    });
                } else {
                    self.start_edit(draft::index(&page.path, i), *item, value, None);
                }
            }
            Kind::Map => {
                let path = draft::key(&page.path, "NEW_VARIABLE");
                let field = Field {
                    key: "",
                    zh: "变量名称",
                    en: "Variable name",
                    help: "",
                    kind: Kind::Text {
                        multiline: false,
                        path: false,
                    },
                    default: "".into(),
                    optional: false,
                    readonly: false,
                };
                self.start_edit(
                    path,
                    field,
                    "NEW_VARIABLE".into(),
                    Some("NEW_VARIABLE".into()),
                );
            }
            _ => {}
        }
        Ok(())
    }
    fn move_row(&mut self, delta: isize) -> Result<()> {
        let page = self.page();
        if !matches!(page.field.kind, Kind::List { ordered: true, .. }) {
            return Ok(());
        }
        let mut v = self
            .value(&page.path, &page.field)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let i = self.selected();
        let to = i.saturating_add_signed(delta);
        if i < v.len() && to < v.len() {
            v.swap(i, to);
            self.change(&page.path, Some(v.into()))?;
            self.select(to);
        }
        Ok(())
    }
    fn leave_child(&mut self, cancel: bool) -> Result<()> {
        if self.stack.len() > 1 {
            let page = self.stack.pop().unwrap();
            if cancel
                && (page.original.is_some()
                    || self
                        .draft
                        .as_ref()
                        .is_some_and(|d| draft::get(&d.tree, &page.path).is_some()))
            {
                self.change(&page.path, page.original)?;
            }
        } else {
            self.nav = true;
        }
        Ok(())
    }
    fn save(&mut self) -> Result<()> {
        if self.pending.is_some() {
            return Ok(());
        }
        self.finish_edit()?;
        anyhow::ensure!(
            self.path_decision.is_none(),
            "Choose whether to keep or recapture the launch snapshot before saving"
        );
        let d = self
            .draft
            .as_ref()
            .context("Original configuration needs repair")?;
        let base = d.base.clone();
        let tree = d.tree.clone();
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Reply::Saved(
                SettingsStore::new(root)
                    .save(&base, &tree)
                    .map_err(|e| format!("{e:#}")),
            ));
        });
        self.message = tr("正在保存全部草稿…", "Saving all drafts…");
        Ok(())
    }
    pub fn reload(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Reply::Reloaded(
                SettingsStore::new(root)
                    .read()
                    .map_err(|e| format!("{e:#}")),
            ));
        });
    }
    pub fn tick(&mut self) -> Option<Outcome> {
        let transfer = self.draft.as_mut().is_some_and(|d| self.shortcuts.tick(d));
        let probed = self.poll_probe() || transfer;
        if self.pending.is_none() && probed {
            return Some(Outcome::Stay);
        }
        let result = self.pending.as_ref().and_then(|r| r.try_recv().ok())?;
        self.pending = None;
        match result {
            Reply::Saved(Ok(s)) => {
                self.committed_language = s.effective.general.language.unwrap_or_default();
                crate::i18n::set(self.committed_language);
                let theme = s.effective.appearance.theme.unwrap_or_default();
                crate::appearance::set(theme);
                self.draft.as_mut().unwrap().saved(s);
                self.error = None;
                self.message = tr(
                    "已保存。新建终端和文档采用启动/视图默认；通知待宿主确认。",
                    "Saved. Launch/view defaults apply to new terminals/documents; notification adoption is pending.",
                );
                self.close_prompt = false;
                return Some(if self.closing_after_save {
                    Outcome::Closed
                } else {
                    Outcome::Saved
                });
            }
            Reply::Reloaded(Ok(s)) => {
                let result = if let Some(d) = &mut self.draft {
                    d.reload(s)
                } else {
                    self.draft = Some(Draft::new(s));
                    Ok(())
                };
                if let Err(e) = result {
                    self.conflict = SettingsStore::new(&self.root).read().ok();
                    self.error = Some(format!("{e:#}"));
                } else {
                    self.error = None;
                    self.message = tr("已重载并合并草稿", "Reloaded and merged drafts");
                    self.preview();
                }
            }
            Reply::Operation(Ok(s)) => {
                self.operation_result = s.clone();
                self.operation_scroll = 0;
                self.result_open = true;
                self.message = s;
            }
            Reply::Projects(result) => match result {
                Ok(values) => {
                    if let Some(p) = &mut self.project_picker {
                        p.2 = values;
                    }
                }
                Err(e) => {
                    self.project_picker = None;
                    self.error = Some(e);
                }
            },
            Reply::Restore(Ok(plan)) => {
                self.restore = Some(plan);
                self.restore_selected = 0;
            }
            Reply::Restore(Err(e)) => self.error = Some(e),
            Reply::Saved(Err(e)) => {
                if e.contains("conflict") || e.contains("changed outside") {
                    if let Ok(latest) = SettingsStore::new(&self.root).read() {
                        self.conflict = Some(latest);
                    }
                }
                self.error = Some(e);
                self.closing_after_save = false;
            }
            Reply::Reloaded(Err(e)) | Reply::Operation(Err(e)) => {
                self.operation_result = e.clone();
                self.result_open = true;
                self.error = Some(e);
                self.closing_after_save = false;
            }
        }
        Some(Outcome::Stay)
    }
    fn discard(&mut self) {
        // Recheck the latest committed theme instead of restoring the opening value.
        if let Ok(latest) = SettingsStore::new(&self.root).read() {
            let theme = latest.effective.appearance.theme.unwrap_or_default();
            crate::appearance::set(theme);
            self.committed_language = latest.effective.general.language.unwrap_or_default();
        } else if let Some(d) = &self.draft {
            crate::appearance::set(d.committed_theme);
        }
        crate::i18n::set(self.committed_language);
    }
    pub fn paste(&mut self, text: &str) {
        if self.close_prompt
            || self.reset_prompt
            || self.delete_prompt.is_some()
            || self.operation_confirm.is_some()
            || self.restore.is_some()
            || self.conflict.is_some()
            || self.result_open
            || self.path_decision.is_some()
            || self.project_picker.is_some()
            || self.control_menu.is_some()
            || self.pending.is_some()
            || self.shortcuts.pending()
        {
            return;
        }
        if let Some(e) = &mut self.edit {
            e.area.insert_str(text);
        } else if self.category == 10 && self.shortcuts.paste(text) {
        } else if self.searching {
            self.search.push_str(&text.replace(['\r', '\n'], ""));
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<Outcome> {
        if key.kind == KeyEventKind::Release {
            return Ok(Outcome::Stay);
        }
        if self.pending.is_some() || self.shortcuts.pending() {
            return Ok(Outcome::Stay);
        }
        if let Some(selected) = self.control_menu {
            match key.code {
                KeyCode::Esc => self.control_menu = None,
                KeyCode::Up => self.control_menu = Some(selected.saturating_sub(1)),
                KeyCode::Down => {
                    self.control_menu =
                        Some((selected + 1).min(self.controls().len().saturating_sub(1)))
                }
                KeyCode::Enter => return self.activate(Some(Hit::Control(selected))),
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if let Some((_, _, values, selected)) = &mut self.project_picker {
            match key.code {
                KeyCode::Esc => self.project_picker = None,
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => *selected = (*selected + 1).min(values.len().saturating_sub(1)),
                KeyCode::Enter => {
                    let i = *selected;
                    self.activate(Some(Hit::Picked(i)))?;
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.delete_prompt.is_some() {
            if key.code == KeyCode::Esc {
                self.delete_prompt = None;
            } else if key.code == KeyCode::Enter && key.modifiers == Mods::CONTROL {
                self.activate(Some(Hit::ConfirmDelete))?;
            }
            return Ok(Outcome::Stay);
        }
        if self.path_decision.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.path_decision = None;
                }
                KeyCode::Char('k') => {
                    self.decide_path(0)?;
                }
                KeyCode::Char('r') => {
                    self.decide_path(1)?;
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.result_open {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.result_open = false,
                KeyCode::Up => self.operation_scroll = self.operation_scroll.saturating_sub(1),
                KeyCode::Down => self.operation_scroll = self.operation_scroll.saturating_add(1),
                KeyCode::PageUp => self.operation_scroll = self.operation_scroll.saturating_sub(10),
                KeyCode::PageDown => {
                    self.operation_scroll = self.operation_scroll.saturating_add(10)
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.restore.is_some() {
            return self.restore_key(key);
        }
        if self.operation_confirm.is_some() {
            match key.code {
                KeyCode::Esc => self.operation_confirm = None,
                KeyCode::Enter if key.modifiers == Mods::CONTROL => {
                    let action = self.operation_confirm.clone().unwrap();
                    self.run_operation(action)?;
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if let Some(choice) = &mut self.choice {
            let len = if let Kind::Choice(v) = &choice.field.kind {
                v.len()
            } else {
                0
            };
            match key.code {
                KeyCode::Esc => self.choice = None,
                KeyCode::Up | KeyCode::Left => choice.selected = choice.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Right => {
                    choice.selected = (choice.selected + 1).min(len.saturating_sub(1))
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let i = choice.selected;
                    self.choose(i)?;
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.issues_open {
            match key.code {
                KeyCode::Esc => self.issues_open = false,
                KeyCode::Up => self.issue_selected = self.issue_selected.saturating_sub(1),
                KeyCode::Down => {
                    self.issue_selected =
                        (self.issue_selected + 1).min(self.issues().len().saturating_sub(1))
                }
                KeyCode::Enter => return self.activate(Some(Hit::Issue(self.issue_selected))),
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.category == 10
            && self.edit.is_none()
            && !self.nav
            && !self.close_prompt
            && !self.reset_prompt
            && self.conflict.is_none()
            && self.footer_focus.is_none()
        {
            if let Some(d) = &mut self.draft {
                if self.shortcuts.key(key, d)? {
                    return Ok(Outcome::Stay);
                }
            }
        }
        if self.pending.is_some() || self.shortcuts.pending() {
            return Ok(Outcome::Stay);
        }
        if self.conflict.is_some() {
            match key.code {
                KeyCode::Esc => self.conflict = None,
                KeyCode::Char('l') => return self.activate(Some(Hit::UseLatest)),
                KeyCode::Char('m') => return self.activate(Some(Hit::KeepMine)),
                KeyCode::Up => self.conflict_scroll = self.conflict_scroll.saturating_sub(3),
                KeyCode::Down => self.conflict_scroll = self.conflict_scroll.saturating_add(3),
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if self.close_prompt {
            return match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => {
                    self.close_prompt = false;
                    Ok(Outcome::Stay)
                }
                (KeyCode::Char('d'), Mods::NONE) => {
                    self.discard();
                    Ok(Outcome::Closed)
                }
                (KeyCode::Char('s'), Mods::NONE) => {
                    self.closing_after_save = true;
                    self.save()?;
                    Ok(Outcome::Stay)
                }
                _ => Ok(Outcome::Stay),
            };
        }
        if self.reset_prompt {
            if key.code == KeyCode::Enter && key.modifiers == Mods::CONTROL {
                self.reset_page()?;
            } else if key.code == KeyCode::Esc {
                self.reset_prompt = false;
            }
            return Ok(Outcome::Stay);
        }
        if key.modifiers == Mods::CONTROL && key.code == KeyCode::Char('s') {
            self.save()?;
            return Ok(Outcome::Stay);
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => self.searching = false,
                KeyCode::Up => self.search_selected = self.search_selected.saturating_sub(1),
                KeyCode::Down => {
                    self.search_selected = (self.search_selected + 1)
                        .min(self.search_results().len().saturating_sub(1))
                }
                KeyCode::Enter => {
                    let found = self.search_results();
                    if let Some((path, _)) = found.get(self.search_selected) {
                        self.focus_path(path)?;
                    }
                    self.searching = false;
                    self.search.clear();
                }
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Char(c) if !key.modifiers.intersects(Mods::CONTROL | Mods::ALT) => {
                    self.search.push(c)
                }
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        if let Some(e) = &mut self.edit {
            if key.code == KeyCode::Esc {
                self.edit = None;
                return Ok(Outcome::Stay);
            }
            if key.code == KeyCode::Enter
                && (!matches!(
                    e.field.kind,
                    Kind::Text {
                        multiline: true,
                        ..
                    }
                ) || key.modifiers == Mods::CONTROL)
            {
                self.finish_edit()?;
                return Ok(Outcome::Stay);
            }
            e.area.input(key);
            return Ok(Outcome::Stay);
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let backwards = key.code == KeyCode::BackTab || key.modifiers.contains(Mods::SHIFT);
            match (self.nav, self.footer_focus, backwards) {
                (true, _, false) => {
                    self.nav = false;
                    self.footer_focus = None;
                }
                (false, None, false) => self.footer_focus = Some(usize::from(!self.dirty())),
                (false, Some(_), false) => {
                    self.nav = true;
                    self.footer_focus = None;
                }
                (true, _, true) => {
                    self.nav = false;
                    self.footer_focus = Some(usize::from(!self.dirty()));
                }
                (false, Some(_), true) => self.footer_focus = None,
                (false, None, true) => self.nav = true,
            }
            return Ok(Outcome::Stay);
        }
        if let Some(i) = self.footer_focus {
            match key.code {
                KeyCode::Left | KeyCode::Up => {
                    self.footer_focus = Some(i.saturating_sub(1).max(usize::from(!self.dirty())))
                }
                KeyCode::Right | KeyCode::Down => self.footer_focus = Some((i + 1).min(4)),
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let actions = [Hit::Save, Hit::Reset, Hit::Reload, Hit::Issues, Hit::Close];
                    return self.activate(Some(actions[i].clone()));
                }
                KeyCode::Esc => self.footer_focus = None,
                _ => {}
            }
            return Ok(Outcome::Stay);
        }
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if self.stack.len() > 1 {
                    self.leave_child(true)?;
                } else if self.dirty() {
                    self.close_prompt = true;
                } else {
                    self.discard();
                    return Ok(Outcome::Closed);
                }
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => self.nav = !self.nav,
            (KeyCode::Char('/'), Mods::NONE) => {
                self.searching = true;
                self.search_selected = 0;
                self.search.clear();
            }
            (KeyCode::F(5), Mods::NONE) => self.reload(),
            (KeyCode::F(1), Mods::NONE) => {
                return self.activate(Some(Hit::FieldHelp));
            }
            (KeyCode::F(8), Mods::NONE) => {
                self.issues_open = true;
                self.issue_selected = 0;
            }
            (KeyCode::F(7), Mods::NONE) => self.recapture()?,
            (KeyCode::F(9), Mods::NONE) => self.apply_preset()?,
            (KeyCode::Char('a'), Mods::NONE) => {
                self.control_menu = Some(0);
            }
            (KeyCode::Char('o'), Mods::NONE) => {
                return self.activate(Some(Hit::Override));
            }
            (KeyCode::Char('p'), Mods::ALT) => self.start_probe(),
            (KeyCode::Char('l'), Mods::ALT) => self.pick_project()?,
            (KeyCode::Char('c'), Mods::ALT) => {
                return self.activate(Some(Hit::Copy));
            }
            (KeyCode::Enter, Mods::CONTROL) => self.leave_child(false)?,
            (KeyCode::F(2), Mods::NONE) => {
                if let Some((p, f)) = self.row() {
                    if f.optional {
                        self.change(&p, None)?;
                    }
                }
            }
            (KeyCode::F(6), Mods::NONE) => self.reset_prompt = true,
            (KeyCode::Up, Mods::ALT) => self.move_row(-1)?,
            (KeyCode::Down, Mods::ALT) => self.move_row(1)?,
            (KeyCode::Up, _) => {
                if self.nav {
                    self.category = self.category.saturating_sub(1);
                    self.stack.clear();
                } else {
                    self.select(self.selected().saturating_sub(1));
                }
            }
            (KeyCode::Down, _) => {
                if self.nav {
                    self.category = (self.category + 1).min(self.pages.len() - 1);
                    self.stack.clear();
                } else {
                    let len =
                        if self.category == 12 || matches!(self.page().field.kind, Kind::Actions) {
                            self.actions().len() + 1
                        } else {
                            self.children(&self.page()).len()
                        };
                    self.select((self.selected() + 1).min(len.saturating_sub(1)));
                }
            }
            (KeyCode::Left, Mods::NONE)
                if !self.nav
                    && self
                        .row()
                        .is_some_and(|(_, f)| matches!(f.kind, Kind::Number { .. })) =>
            {
                self.fine_step(-1)?
            }
            (KeyCode::Right, Mods::NONE)
                if !self.nav
                    && self
                        .row()
                        .is_some_and(|(_, f)| matches!(f.kind, Kind::Number { .. })) =>
            {
                self.fine_step(1)?
            }
            (KeyCode::Left, _) => {
                if self.stack.len() > 1 {
                    self.leave_child(false)?;
                } else {
                    self.nav = true;
                }
            }
            (KeyCode::Right, _) | (KeyCode::Enter, _) | (KeyCode::Char(' '), Mods::NONE) => {
                if self.nav {
                    self.nav = false;
                } else if self.category == 12 || matches!(self.page().field.kind, Kind::Actions) {
                    if self.selected() == 0 {
                        if self.category == 11 {
                            self.run_operation(super::operations::Action::Diagnostics)?;
                        } else {
                            self.edit_archive();
                        }
                    } else if let Some(a) = self.actions().get(self.selected() - 1) {
                        self.run_operation(a.clone())?;
                    }
                } else {
                    self.open_row()?;
                }
            }
            (KeyCode::Insert, _) | (KeyCode::Char('+'), Mods::NONE) => self.add()?,
            (KeyCode::Delete, _) => {
                self.delete_row()?;
            }
            (KeyCode::Char('e'), Mods::NONE) => {
                if let Some((p, f)) = self.row() {
                    let value = self.value(&p, &f);
                    if !self.inherited_lock(&p, &f)
                        && (!f.readonly || value.as_str().is_some_and(str::is_empty))
                        && !matches!(f.kind, Kind::Object(_) | Kind::List { .. } | Kind::Map)
                    {
                        self.start_edit(p, f, value, None);
                    }
                }
            }
            _ => {}
        }
        Ok(Outcome::Stay)
    }
    pub fn mouse(&mut self, event: MouseEvent) -> Result<Outcome> {
        self.pointer = Some((event.column, event.row));
        if event.kind == MouseEventKind::Moved {
            return Ok(Outcome::Stay);
        }
        if self.pending.is_some() || self.shortcuts.pending() {
            return Ok(Outcome::Stay);
        }
        if matches!(
            event.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            let up = event.kind == MouseEventKind::ScrollUp;
            if self.result_open {
                self.operation_scroll = if up {
                    self.operation_scroll.saturating_sub(3)
                } else {
                    self.operation_scroll.saturating_add(3)
                };
                return Ok(Outcome::Stay);
            }
            if self.conflict.is_some() {
                self.conflict_scroll = if up {
                    self.conflict_scroll.saturating_sub(3)
                } else {
                    self.conflict_scroll.saturating_add(3)
                };
                return Ok(Outcome::Stay);
            }
            if self.restore.is_some() {
                self.restore_scroll = if up {
                    self.restore_scroll.saturating_sub(3)
                } else {
                    self.restore_scroll.saturating_add(3)
                };
                return Ok(Outcome::Stay);
            }
            if self.edit.is_some()
                || self.choice.is_some()
                || self.path_decision.is_some()
                || self.project_picker.is_some()
                || self.control_menu.is_some()
                || self.close_prompt
                || self.reset_prompt
                || self.delete_prompt.is_some()
                || self.operation_confirm.is_some()
                || self.issues_open
                || self.searching
                || (self.category == 10 && self.shortcuts.exclusive())
            {
                return Ok(Outcome::Stay);
            }
        }
        if event.kind == MouseEventKind::ScrollUp {
            self.select(self.selected().saturating_sub(3));
            return Ok(Outcome::Stay);
        }
        if event.kind == MouseEventKind::ScrollDown {
            self.select(
                (self.selected() + 3).min(self.children(&self.page()).len().saturating_sub(1)),
            );
            return Ok(Outcome::Stay);
        }
        if self.pending.is_some() || self.shortcuts.pending() {
            return Ok(Outcome::Stay);
        }
        if event.kind == MouseEventKind::Drag(MouseButton::Left) {
            let action = self
                .hits
                .iter()
                .rev()
                .find(|(r, h)| {
                    matches!(h, Hit::Slider(..)) && r.contains((event.column, event.row).into())
                })
                .map(|(_, h)| h.clone());
            return self.activate(action);
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Ok(Outcome::Stay);
        }
        let action = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains((event.column, event.row).into()))
            .map(|(_, h)| h.clone());
        if self.category == 10
            && self.shortcuts.exclusive()
            && !matches!(action, Some(Hit::Shortcut(_)))
        {
            return Ok(Outcome::Stay);
        }
        self.activate(action)
    }
    fn activate(&mut self, action: Option<Hit>) -> Result<Outcome> {
        if action.as_ref().is_some_and(|h| !self.hit_enabled(h)) {
            return Ok(Outcome::Stay);
        }
        match action {
            Some(Hit::RestoreToggle(i)) => self.restore_toggle(i),
            Some(Hit::RestoreApply) => self.restore_apply()?,
            Some(Hit::RestoreCancel) => self.restore = None,
            Some(Hit::ResultClose) => self.result_open = false,
            Some(Hit::Controls) => self.control_menu = Some(0),
            Some(Hit::Control(i)) => {
                let action = self.controls().get(i).map(|(_, h)| h.clone());
                self.control_menu = None;
                return self.activate(action);
            }
            Some(Hit::FieldHelp) => {
                if let Some((path, field)) = self.row() {
                    self.operation_result = format!(
                        "{}\n{:?}\n{}\n{}\n{}",
                        field.label(),
                        path,
                        self.value(&path, &field),
                        field.help,
                        tr(
                            "O 覆盖 · F2 恢复默认/继承 · E 精确编辑 · Alt+C 复制 · Alt+P 探测路径",
                            "O override · F2 default/inherit · E edit exact value · Alt+C copy · Alt+P probe path"
                        )
                    );
                    self.operation_scroll = 0;
                    self.result_open = true;
                }
            }
            Some(Hit::PathDecision(which)) => self.decide_path(which)?,

            Some(Hit::SetChoice(path, field, i)) => {
                self.choice = Some(Choice {
                    path,
                    field,
                    selected: i,
                });
                self.choose(i)?;
            }
            Some(Hit::Choice(i)) => self.choose(i)?,
            Some(Hit::Slider(path, value)) => {
                self.change(&path, Some(value.into()))?;
            }
            Some(Hit::Issues) => {
                self.issues_open = true;
                self.issue_selected = 0;
            }
            Some(Hit::Issue(i)) => {
                if let Some((path, _)) = self.issues().get(i) {
                    self.focus_path(path)?;
                }
                self.issues_open = false;
            }
            Some(Hit::Probe) => self.start_probe(),
            Some(Hit::PickProject) => self.pick_project()?,
            Some(Hit::Picked(i)) => {
                if let Some((path, field, values, _)) = self.project_picker.take() {
                    if let Some(value) = values.get(i) {
                        self.start_edit(path, field, value.clone().into(), None);
                        self.finish_edit()?;
                    }
                }
            }
            Some(Hit::Recapture) => self.recapture()?,
            Some(Hit::ApplyPreset) => self.apply_preset()?,
            Some(Hit::Copy) => {
                if let Some((path, field)) = self.row() {
                    let v = self.value(&path, &field);
                    crate::files::copy(
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()),
                    )?;
                }
            }
            Some(Hit::SearchResult(i)) => {
                if let Some((path, _)) = self.search_results().get(i).cloned() {
                    self.focus_path(&path)?;
                }
                self.searching = false;
                self.search.clear();
            }
            Some(Hit::UseLatest) => {
                if let Some(latest) = self.conflict.take() {
                    self.draft.as_mut().unwrap().saved(latest);
                    self.preview();
                }
            }
            Some(Hit::KeepMine) => {
                if let Some(latest) = self.conflict.take() {
                    let d = self.draft.as_mut().unwrap();
                    let tree = super::store::reapply(&d.base.tree, &d.tree, &latest.tree)?;
                    d.saved(latest);
                    d.tree = tree;
                    self.error = None;
                    self.preview();
                }
            }
            Some(Hit::Step(delta)) => self.step(delta)?,
            Some(Hit::Override) => {
                if let Some((path, field)) = self.row() {
                    let value = self.value(&path, &field);
                    self.change(&path, Some(value))?;
                }
            }
            Some(Hit::ConfirmReset) => {
                self.reset_page()?;
            }
            Some(Hit::Page(i)) => {
                self.finish_edit()?;
                self.category = i;
                self.stack.clear();
                self.nav = false;
            }
            Some(Hit::Row(i)) => {
                self.nav = false;
                self.footer_focus = None;
                self.select(i);
                self.open_row()?;
            }
            Some(Hit::Save) => {
                self.closing_after_save = self.close_prompt;
                self.save()?;
            }
            Some(Hit::Reset) => self.reset_prompt = true,
            Some(Hit::Back) => self.leave_child(false)?,
            Some(Hit::Cancel) => {
                if self.project_picker.is_some() {
                    self.project_picker = None;
                } else if self.edit.is_some() {
                    self.edit = None;
                } else {
                    self.leave_child(true)?;
                }
            }
            Some(Hit::Done) => {
                if self.edit.is_some() {
                    self.finish_edit()?;
                } else {
                    self.leave_child(false)?;
                }
            }
            Some(Hit::Add) => self.add()?,
            Some(Hit::Delete) => self.delete_row()?,
            Some(Hit::ConfirmDelete) => {
                if let Some((path, _)) = self.delete_prompt.take() {
                    self.change(&path, None)?;
                    self.select(
                        self.selected()
                            .min(self.children(&self.page()).len().saturating_sub(1)),
                    );
                }
            }
            Some(Hit::Up) => self.move_row(-1)?,
            Some(Hit::Down) => self.move_row(1)?,
            Some(Hit::Inherit) => {
                if let Some((p, f)) = self.row() {
                    if f.optional {
                        self.change(&p, None)?;
                    }
                }
            }
            Some(Hit::Reload) => self.reload(),
            Some(Hit::Close) => {
                if self.dirty() {
                    self.close_prompt = true;
                } else {
                    self.discard();
                    return Ok(Outcome::Closed);
                }
            }
            Some(Hit::Discard) => {
                self.discard();
                return Ok(Outcome::Closed);
            }
            Some(Hit::Continue) => {
                self.close_prompt = false;
                self.reset_prompt = false;
                self.delete_prompt = None;
                self.operation_confirm = None;
                self.conflict = None;
            }
            Some(Hit::Shortcut(action)) => {
                if let Some(d) = &mut self.draft {
                    self.shortcuts.action(action, d)?;
                }
            }
            Some(Hit::Action(action)) => self.run_operation(action)?,
            Some(Hit::Open) => self.edit_archive(),
            None => {}
        }
        Ok(Outcome::Stay)
    }
    fn button(&mut self, f: &mut Frame, r: Rect, label: String, hit: Hit) {
        if r.width == 0 || r.height == 0 {
            return;
        }
        let p = crate::appearance::palette();
        let enabled = self.hit_enabled(&hit);
        let style = if !enabled {
            Style::new().fg(p.muted)
        } else if Self::hit_dangerous(&hit) {
            Style::new().fg(p.error)
        } else if matches!(hit, Hit::Save) {
            Style::new()
                .bg(p.accent)
                .fg(if p.bg == ratatui::style::Color::Reset {
                    ratatui::style::Color::Black
                } else {
                    p.bg
                })
        } else {
            Style::new().fg(p.accent)
        };
        f.render_widget(Paragraph::new(format!("[{label}] ")).style(style), r);
        self.hits.push((r, hit));
    }
    fn hit_enabled(&self, hit: &Hit) -> bool {
        match hit {
            Hit::Save => self.dirty(),
            Hit::Step(delta) => self.row().is_some_and(|(path, f)| {
                let Kind::Number { min, max, .. } = f.kind else {
                    return false;
                };
                let value = self.value(&path, &f).as_integer().unwrap_or(min);
                !self.inherited_lock(&path, &f)
                    && if *delta < 0 { value > min } else { value < max }
            }),
            _ => true,
        }
    }
    fn hit_dangerous(hit: &Hit) -> bool {
        use super::operations::Action;
        matches!(
            hit,
            Hit::Delete
                | Hit::ConfirmDelete
                | Hit::Discard
                | Hit::Reset
                | Hit::ConfirmReset
                | Hit::Action(
                    Action::ClearStats
                        | Action::ResetMemory
                        | Action::HookRemove(_)
                        | Action::Unregister
                        | Action::RestoreContent
                )
        )
    }
    fn draw_hover(&self, f: &mut Frame) {
        if self.pending.is_some() || self.shortcuts.pending() {
            return;
        }
        if let Some(point) = self.pointer
            && let Some((r, hit)) = self
                .hits
                .iter()
                .rev()
                .find(|(r, _)| r.contains(point.into()))
            && self.hit_enabled(hit)
        {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    f.buffer_mut()[(x, y)]
                        .set_style(Style::new().add_modifier(ratatui::style::Modifier::UNDERLINED));
                }
            }
        }
    }
    fn buttons(&mut self, f: &mut Frame, r: Rect, items: Vec<(String, Hit)>) {
        use unicode_width::UnicodeWidthStr;
        let mut x = r.x;
        let mut y = r.y;
        for (label, hit) in items {
            let w = (label.width() as u16 + 4).min(r.width);
            if x + w > r.right() {
                x = r.x;
                y += 1;
            }
            if y >= r.bottom() {
                break;
            }
            self.button(f, Rect::new(x, y, w, 1), label, hit);
            x += w;
        }
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        self.area = frame.area();
        self.hits.clear();
        let area = self.area;
        crate::ui::clear_popup(frame, area);
        let palette = crate::appearance::palette();
        frame.render_widget(
            Block::bordered()
                .title(format!(
                    " {}{} ",
                    tr("设置", "Settings"),
                    if self.dirty() { " ●" } else { "" }
                ))
                .style(crate::appearance::style()),
            area,
        );
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let footer = inner.height.min(if inner.width < 55 { 4 } else { 3 });
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(footer),
        );
        let wide = content.width >= 90;
        let nav_w = if wide { 23 } else { 0 };
        if wide {
            for (i, p) in self.pages.iter().enumerate().take(content.height as usize) {
                let r = Rect::new(content.x, content.y + i as u16, nav_w, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {}",
                        if i == self.category { "▸" } else { " " },
                        p.label()
                    ))
                    .style(if i == self.category {
                        Style::new().fg(palette.accent)
                    } else {
                        Style::default()
                    }),
                    r,
                );
                self.hits.push((r, Hit::Page(i)));
            }
        } else {
            let i = self.category;
            self.button(
                frame,
                Rect::new(content.x, content.y, content.width, 1),
                format!("{} / {}   Tab ↑↓", i + 1, self.pages[i].label()),
                Hit::Page(i),
            );
        }
        let body = Rect::new(
            content.x + nav_w,
            content.y + u16::from(!wide),
            content.width - nav_w,
            content.height.saturating_sub(u16::from(!wide)),
        );
        if self.draft.is_none() {
            frame.render_widget(
                Paragraph::new(format!(
                    "{}\n{}\n{}",
                    tr(
                        "原配置无法解析，保留原文件。",
                        "Cannot parse configuration; original bytes are preserved."
                    ),
                    self.error.as_deref().unwrap_or(""),
                    SettingsStore::new(&self.root).path().display()
                ))
                .wrap(Wrap { trim: false }),
                body,
            );
            self.buttons(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 2),
                vec![
                    (
                        tr("打开原文件", "Open original"),
                        Hit::Action(super::operations::Action::OpenConfig),
                    ),
                    (tr("重载", "Reload"), Hit::Reload),
                    (tr("关闭", "Close"), Hit::Close),
                ],
            );
            self.draw_hover(frame);
            return;
        }
        if self.category == 10 && self.stack.is_empty() {
            let d = self.draft.as_mut().unwrap();
            let hits = self.shortcuts.draw(frame, body, d);
            self.hits
                .extend(hits.into_iter().map(|(r, a)| (r, Hit::Shortcut(a))));
        } else if self.category == 12 || matches!(self.page().field.kind, Kind::Actions) {
            self.draw_operations(frame, body);
        } else {
            let page = self.page();
            let breadcrumb = self
                .stack
                .iter()
                .map(|p| p.field.label())
                .collect::<Vec<_>>()
                .join(" / ");
            frame.render_widget(
                Paragraph::new(if breadcrumb.is_empty() {
                    page.field.label().to_string()
                } else {
                    breadcrumb
                })
                .style(Style::new().fg(palette.accent)),
                Rect::new(body.x, body.y, body.width, 1),
            );
            let rows = self.children(&page);
            let issues = self.issues();
            let view = Rect::new(
                body.x,
                body.y + 1,
                body.width,
                body.height.saturating_sub(4),
            );
            let slots = (view.height / 3).max(1) as usize;
            let first = self.selected().saturating_sub(slots - 1);
            for (i, (path, field)) in rows.iter().enumerate().skip(first) {
                let y = view.y + ((i - first) * 3) as u16;
                if y >= view.bottom() {
                    break;
                }
                let value = self.value(path, field);
                let own = draft::get(&self.draft.as_ref().unwrap().tree, path).is_some();
                let label = if matches!(page.field.kind, Kind::List { .. }) {
                    format!("{}. {}", i + 1, item_label(&value, field))
                } else if matches!(page.field.kind, Kind::Map) {
                    match path.last() {
                        Some(Part::Key(k)) => k.clone(),
                        _ => field.label().into(),
                    }
                } else {
                    field.label().into()
                };
                let source = if own {
                    if matches!(path.first(),Some(Part::Key(k)) if k=="preset") {
                        tr("固定快照", "Frozen snapshot")
                    } else {
                        tr("覆盖", "Override")
                    }
                } else if matches!(path.first(),Some(Part::Key(k)) if k=="project")
                    && path.len() >= 4
                {
                    tr(
                        "继承全局 / 代码默认 · O 覆盖",
                        "Global / code default · O to override",
                    )
                } else {
                    tr("代码默认", "Code default")
                };
                let issue = issues
                    .iter()
                    .find(|(p, _)| p.starts_with(path) || path.starts_with(p));
                let contextual = self.contextual_help(path, field, &value);
                let hint = issue
                    .map(|(_, msg)| msg.as_str())
                    .or_else(|| {
                        self.probes
                            .iter()
                            .find(|p| p.path == *path && value.as_str() == Some(&p.value))
                            .map(|p| p.result.as_str())
                    })
                    .unwrap_or(if field.key == "image_protocol" {
                        &self.image_status
                    } else {
                        &contextual
                    });
                let r = Rect::new(view.x, y, view.width, (view.bottom() - y).min(3));
                let style = if i == self.selected() && !self.nav {
                    Style::new().bg(palette.selected)
                } else {
                    Style::default()
                };
                let display = if matches!(page.field.kind, Kind::Map) {
                    tr(
                        "（值已折叠；Enter 查看/编辑）",
                        "(value hidden; Enter to view/edit)",
                    )
                } else {
                    display(&value, field)
                };
                let row = vec![
                    Line::from(vec![
                        Span::styled(
                            format!("{} {}", if i == self.selected() { "▸" } else { " " }, label),
                            Style::new().fg(palette.fg),
                        ),
                        Span::styled(format!("   {source}"), Style::new().fg(palette.muted)),
                    ]),
                    Line::from(format!("  {display}")),
                    Line::from(Span::styled(
                        hint,
                        Style::new().fg(if issue.is_some() {
                            palette.error
                        } else {
                            palette.muted
                        }),
                    )),
                ];
                frame.render_widget(Paragraph::new(row).style(style), r);
                self.hits.push((r, Hit::Row(i)));
                if r.height > 1 {
                    self.draw_control(
                        frame,
                        Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(2), 1),
                        path,
                        field,
                        &value,
                    );
                }
            }
            if rows.is_empty() {
                frame.render_widget(
                    Paragraph::new(tr(
                        "暂无条目。Insert 或“添加”创建。",
                        "No entries. Use Insert or Add.",
                    )),
                    view,
                );
            }
            self.buttons(
                frame,
                Rect::new(body.x, body.bottom().saturating_sub(3), body.width, 3),
                if body.width < 90 {
                    vec![(tr("操作 A", "Actions A"), Hit::Controls)]
                } else {
                    self.controls()
                },
            );
        }
        let foot = Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer),
            inner.width,
            footer,
        );
        self.buttons(
            frame,
            foot,
            vec![
                (tr("保存 Ctrl+S", "Save Ctrl+S"), Hit::Save),
                (tr("恢复 F6", "Reset F6"), Hit::Reset),
                (tr("重载 F5", "Reload F5"), Hit::Reload),
                (tr("错误 F8", "Issues F8"), Hit::Issues),
                (tr("关闭 Esc", "Close Esc"), Hit::Close),
            ],
        );
        if let Some(i) = self.footer_focus {
            let label = [
                tr("保存全部", "Save all"),
                tr("恢复本页", "Reset page"),
                tr("重载", "Reload"),
                tr("错误列表", "Issues"),
                tr("关闭", "Close"),
            ][i]
                .clone();
            frame.render_widget(
                Paragraph::new(format!("◀ {label} ▶ Enter · Tab"))
                    .style(Style::new().bg(palette.selected)),
                Rect::new(foot.x, foot.y, foot.width, 1),
            );
        }
        if foot.height > 1 {
            let msg = self.error.as_ref().unwrap_or(&self.message);
            frame.render_widget(
                Paragraph::new(crate::preview::safe_text(msg)).style(Style::new().fg(
                    if self.error.is_some() {
                        palette.error
                    } else {
                        palette.muted
                    },
                )),
                Rect::new(foot.x, foot.bottom() - 1, foot.width, 1),
            );
        }
        if self.searching {
            crate::ui::clear_popup(frame, body);
            frame.render_widget(
                Paragraph::new(format!("/ {}", self.search)),
                Rect::new(body.x, body.y, body.width, 1),
            );
            let visible = body.height.saturating_sub(1) as usize;
            let first = self
                .search_selected
                .saturating_sub(visible.saturating_sub(1));
            self.hits.clear();
            for (i, (_, label)) in self
                .search_results()
                .iter()
                .enumerate()
                .skip(first)
                .take(visible)
            {
                let r = Rect::new(body.x, body.y + 1 + (i - first) as u16, body.width, 1);
                frame.render_widget(
                    Paragraph::new(label.clone()).style(if i == self.search_selected {
                        Style::new().bg(palette.selected)
                    } else {
                        Style::default()
                    }),
                    r,
                );
                self.hits.push((r, Hit::SearchResult(i)));
            }
        }
        if self.control_menu.is_some() {
            self.draw_controls_menu(frame);
        }
        if self.restore.is_some() {
            self.draw_restore(frame);
        }
        if self.choice.is_some() {
            self.draw_choice(frame);
        }
        if self.issues_open {
            self.draw_issues(frame);
        }
        if self.edit.is_some() {
            self.draw_edit(frame);
        }
        if self.close_prompt
            || self.reset_prompt
            || self.delete_prompt.is_some()
            || self.operation_confirm.is_some()
        {
            self.draw_confirmation(frame);
        }
        if self.project_picker.is_some() {
            self.draw_project_picker(frame);
        }
        if self.path_decision.is_some() {
            self.draw_path_decision(frame);
        }
        if self.result_open {
            self.draw_operation_result(frame);
        }
        if self.conflict.is_some() {
            self.draw_conflict(frame);
        }
        self.draw_hover(frame);
    }
    fn draw_edit(&mut self, f: &mut Frame) {
        self.hits.clear();
        let area = dialog(self.area, 90, 22);
        crate::ui::clear_popup(f, area);
        let e = self.edit.as_mut().unwrap();
        f.render_widget(
            Block::bordered()
                .title(e.field.label())
                .style(crate::appearance::style()),
            area,
        );
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        let body = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(3),
        );
        f.render_widget(&e.area, body);
        if let Some(error) = &self.error {
            f.render_widget(
                Paragraph::new(error.clone())
                    .style(Style::new().fg(crate::appearance::palette().error)),
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
            );
        }
        if inner.height >= 3 {
            f.render_widget(
                Paragraph::new(format!(
                    "{}  {}",
                    e.field.help,
                    tr(
                        "Enter 确认；多行 Ctrl+Enter；Esc 取消",
                        "Enter confirms; multiline Ctrl+Enter; Esc cancels"
                    )
                ))
                .wrap(Wrap { trim: false }),
                Rect::new(inner.x, inner.bottom() - 3, inner.width, 2),
            );
        }
        self.buttons(
            f,
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            vec![
                (tr("完成", "Done"), Hit::Done),
                (tr("取消", "Cancel"), Hit::Cancel),
            ],
        );
    }
    fn draw_confirmation(&mut self, f: &mut Frame) {
        self.hits.clear();
        let area = dialog(self.area, 76, 10);
        crate::ui::clear_popup(f, area);
        f.render_widget(
            Block::bordered()
                .title(tr("确认", "Confirm"))
                .style(crate::appearance::style()),
            area,
        );
        let r = area.inner(ratatui::layout::Margin::new(1, 1));
        let text = if let Some((_, label)) = &self.delete_prompt {
            format!(
                "{}\n{}\nCtrl+Enter / Esc",
                label,
                tr(
                    "仅从配置草稿中删除；不会删除项目目录或任务。保存全部后生效。",
                    "Remove from the draft only; project directories and tasks remain. Save all to apply."
                )
            )
        } else if self.close_prompt {
            tr(
                "保留全部草稿继续编辑，或保存/放弃并关闭。默认继续编辑。",
                "Continue editing, save all and close, or discard and close. Default: continue.",
            )
        } else if self.reset_prompt {
            tr(
                "仅删除本页偏好覆盖。项目目录、预设、收藏、检查和用户内容保留。Ctrl+Enter 确认，Esc 返回。",
                "Remove this page's preference overrides. Projects, presets, favorites, checks and content remain. Ctrl+Enter confirms; Esc returns.",
            )
        } else {
            format!(
                "{} {:?}\n{}\n{}\n{}\nCtrl+Enter 确认 / confirm; Esc 取消 / cancel",
                tr("立即执行：", "Execute immediately:"),
                self.operation_confirm,
                self.root.display(),
                self.operation_path,
                tr(
                    "操作与设置草稿独立；不会随放弃草稿撤销。",
                    "This operation is independent of the draft and cannot be undone by discarding settings."
                )
            )
        };
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), r);
        let mut buttons = vec![(tr("继续编辑", "Continue editing"), Hit::Continue)];
        if self.close_prompt {
            buttons.extend([
                (tr("保存并关闭 S", "Save and close S"), Hit::Save),
                (tr("放弃并关闭 D", "Discard and close D"), Hit::Discard),
            ]);
        } else if self.delete_prompt.is_some() {
            buttons.push((tr("确认删除配置项", "Confirm removal"), Hit::ConfirmDelete));
        } else if self.reset_prompt {
            buttons.push((tr("确认恢复本页", "Confirm reset"), Hit::ConfirmReset));
        } else if let Some(action) = self.operation_confirm.clone() {
            buttons.push((tr("确认立即执行", "Confirm operation"), Hit::Action(action)));
        }
        self.buttons(
            f,
            Rect::new(r.x, r.bottom().saturating_sub(2), r.width, 2),
            buttons,
        );
    }
    fn step(&mut self, delta: i64) -> Result<()> {
        if let Some((path, field)) = self.row()
            && let Kind::Number { min, max, step, .. } = field.kind
        {
            if self.inherited_lock(&path, &field) {
                return Ok(());
            }
            let v = self.value(&path, &field).as_integer().unwrap_or(min);
            self.change(
                &path,
                Some(v.saturating_add(delta * step).clamp(min, max).into()),
            )?;
        }
        Ok(())
    }
    fn edit_archive(&mut self) {
        let field = Field {
            key: "",
            zh: "归档/导出路径",
            en: "Archive/export path",
            help: "使用独立绝对路径；覆盖目标需确认。Use a separate absolute path; overwriting requires confirmation.",
            kind: Kind::Text {
                multiline: false,
                path: true,
            },
            default: "".into(),
            optional: false,
            readonly: false,
        };
        self.start_edit(
            draft::key(&[], "$archive"),
            field,
            self.operation_path.clone().into(),
            None,
        );
    }
    fn delete_row(&mut self) -> Result<()> {
        if matches!(self.page().field.kind, Kind::List { .. } | Kind::Map)
            && let Some((p, f)) = self.row()
        {
            if matches!(p.as_slice(), [Part::Key(k), Part::Index(_)] if k == "project" || k == "preset")
            {
                self.delete_prompt = Some((p.clone(), item_label(&self.value(&p, &f), &f)));
            } else {
                self.change(&p, None)?;
                self.select(
                    self.selected()
                        .min(self.children(&self.page()).len().saturating_sub(1)),
                );
            }
        }
        Ok(())
    }
    fn reset_page(&mut self) -> Result<()> {
        let p = self.page();
        let d = self.draft.as_mut().context("Repair configuration first")?;
        if self.category == 10 {
            draft::put(&mut d.tree, &draft::key(&[], "shortcuts"), None)?;
        } else if let Kind::Object(fields) = p.field.kind {
            d.remove_overrides(&p.path, &fields)?;
        }
        self.reset_prompt = false;
        self.preview();
        Ok(())
    }
    fn search_results(&self) -> Vec<(Path, String)> {
        fn collect(
            tree: &Value,
            path: Path,
            f: &Field,
            out: &mut Vec<(Path, String)>,
            query: &str,
        ) {
            let current = draft::get(tree, &path);
            let identity = current
                .and_then(Value::as_table)
                .and_then(|v| {
                    v.get("path")
                        .or_else(|| v.get("name"))
                        .or_else(|| v.get("project"))
                })
                .map(ToString::to_string)
                .unwrap_or_default();
            let label = format!(
                "{} / {} / {} {} {}",
                f.zh,
                f.en,
                f.key,
                identity,
                current
                    .filter(|v| !v.is_table() && !v.is_array())
                    .map(ToString::to_string)
                    .unwrap_or_default()
            );
            if label.to_lowercase().contains(query) {
                out.push((path.clone(), label));
            }
            match &f.kind {
                Kind::Object(fields) => {
                    for child in fields {
                        collect(tree, draft::key(&path, child.key), child, out, query);
                    }
                }
                Kind::List { item, .. } => {
                    if let Some(entries) = current.and_then(Value::as_array) {
                        for i in 0..entries.len() {
                            collect(tree, draft::index(&path, i), item, out, query);
                        }
                    }
                }
                // Environment values are private and must not enter global search results.
                _ => {}
            }
        }
        let mut out = vec![];
        let query = self.search.to_lowercase();
        if let Some(d) = &self.draft {
            for p in &self.pages {
                collect(&d.tree, draft::key(&[], p.key), p, &mut out, &query);
            }
        }
        out
    }
    fn draw_conflict(&mut self, f: &mut Frame) {
        self.hits.clear();
        let area = dialog(self.area, 100, self.area.height.saturating_sub(2));
        crate::ui::clear_popup(f, area);
        f.render_widget(
            Block::bordered()
                .title(tr("保存冲突", "Save conflict"))
                .style(crate::appearance::style()),
            area,
        );
        let r = area.inner(ratatui::layout::Margin::new(1, 1));
        let d = self.draft.as_ref().unwrap();
        let latest = self.conflict.as_ref().unwrap();
        let text = format!(
            "{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}",
            self.error.as_deref().unwrap_or(""),
            tr("打开时", "When opened"),
            toml::to_string_pretty(&d.base.tree).unwrap_or_default(),
            tr("最新已保存", "Latest committed"),
            latest.text,
            tr("我的修改", "My draft"),
            toml::to_string_pretty(&d.tree).unwrap_or_default()
        );
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .scroll((self.conflict_scroll, 0)),
            Rect::new(r.x, r.y, r.width, r.height.saturating_sub(3)),
        );
        self.buttons(
            f,
            Rect::new(r.x, r.bottom().saturating_sub(3), r.width, 3),
            vec![
                (tr("返回编辑 Esc", "Return to editing Esc"), Hit::Continue),
                (
                    tr("采用最新并放弃草稿", "Use latest; discard draft"),
                    Hit::UseLatest,
                ),
                (
                    tr(
                        "保留我的修改/重新新增；稍后再保存",
                        "Retain/re-add my edits; save again",
                    ),
                    Hit::KeepMine,
                ),
            ],
        );
    }
    fn run_operation(&mut self, action: super::operations::Action) -> Result<()> {
        if matches!(
            action,
            super::operations::Action::InspectContent | super::operations::Action::RestoreContent
        ) {
            if self.pending.is_some() {
                return Ok(());
            }
            let root = self.root.clone();
            let archive = self.operation_path.clone();
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(Reply::Restore(
                    super::operations::inspect_restore(&root, &archive)
                        .map_err(|e| format!("{e:#}")),
                ));
            });
            return Ok(());
        }
        if action.destructive() && self.operation_confirm.as_ref() != Some(&action) {
            self.operation_confirm = Some(action);
            return Ok(());
        }
        if self.pending.is_some() {
            return Ok(());
        }
        self.operation_confirm = None;
        let archive = self.operation_path.clone();
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Reply::Operation(
                super::operations::execute_at(&root, &action, &archive)
                    .map_err(|e| format!("{e:#}")),
            ));
        });
        Ok(())
    }
    fn actions(&self) -> Vec<super::operations::Action> {
        use super::operations::Action::*;
        super::operations::actions()
            .into_iter()
            .filter(|a| {
                let integration = matches!(
                    a,
                    HookDiagnostic(_)
                        | HookInstall(_)
                        | HookRemove(_)
                        | Register
                        | Unregister
                        | TestNotification
                );
                if self.category == 11 {
                    integration || matches!(a, Diagnostics)
                } else {
                    !integration
                }
            })
            .collect()
    }
    fn draw_operations(&mut self, f: &mut Frame, r: Rect) {
        if r.height == 0 {
            return;
        }
        let integration = self.category == 11;
        let info = if integration {
            format!(
                "{}\n{}\n{}",
                tr(
                    "宿主 / Hooks / 系统注册均需显式诊断；未确认不表示已关闭。",
                    "Host / Hooks / registration require diagnostics; unconfirmed does not mean disabled."
                ),
                tr(
                    "操作立即执行，独立于设置草稿。测试通知可能有声音。",
                    "Operations execute immediately, independently of the draft. Tests may play sound."
                ),
                self.operation_result.lines().next().unwrap_or("")
            )
        } else {
            format!(
                "DevTabs {} · schema {} · {}\nconfig.toml 偏好 / preferences · user-data.json 内容 / content · ui-state.json 记忆 / memory\nruntime/ 任务记录 / task records · cache/ 可重算统计 / statistics\n{} {}",
                env!("CARGO_PKG_VERSION"),
                self.draft.as_ref().map_or(0, |d| d.base.effective.schema),
                self.root.display(),
                tr("归档路径（Enter 编辑）：", "Archive path (Enter edits):"),
                self.operation_path
            )
        };
        let ih = r
            .height
            .saturating_sub(3)
            .min(if integration { 3 } else { 4 });
        f.render_widget(
            Paragraph::new(info).wrap(Wrap { trim: false }),
            Rect::new(r.x, r.y, r.width, ih),
        );
        let actions = self.actions();
        let mut rows = vec![];
        if !integration {
            rows.push((
                tr("归档路径：", "Archive path:") + &self.operation_path,
                Hit::Open,
            ));
        } else {
            rows.push((
                tr("刷新采用状态 / 诊断", "Refresh adoption / diagnostics"),
                Hit::Action(super::operations::Action::Diagnostics),
            ));
        }
        rows.extend(actions.into_iter().map(|a| {
            let (zh, en) = a.label();
            (tr(zh, en), Hit::Action(a))
        }));
        let available = r.height.saturating_sub(ih).max(1) as usize;
        let first = self.selected().saturating_sub(available - 1);
        for (i, (label, hit)) in rows.into_iter().enumerate().skip(first).take(available) {
            let row = Rect::new(r.x, r.y + ih + (i - first) as u16, r.width, 1);
            f.render_widget(
                Paragraph::new(format!(
                    "{} [{}]",
                    if self.selected() == i { "▸" } else { " " },
                    label
                ))
                .style({
                    let p = crate::appearance::palette();
                    let s = if self.selected() == i {
                        Style::new().bg(p.selected)
                    } else {
                        Style::default()
                    };
                    if Self::hit_dangerous(&hit) {
                        s.fg(p.error)
                    } else {
                        s
                    }
                }),
                row,
            );
            self.hits.push((row, hit));
        }
    }
    fn draw_operation_result(&mut self, f: &mut Frame) {
        self.hits.clear();
        let r = dialog(self.area, 110, self.area.height.saturating_sub(2));
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr(
                    "操作结果 · ↑↓/PgUp/PgDn 滚动 · Esc 返回",
                    "Operation result · ↑↓/PgUp/PgDn scroll · Esc back",
                ))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        f.render_widget(
            Paragraph::new(crate::preview::safe_text(&self.operation_result))
                .wrap(Wrap { trim: false })
                .scroll((self.operation_scroll, 0)),
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(1),
            ),
        );
        self.buttons(
            f,
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            vec![(tr("返回", "Back"), Hit::ResultClose)],
        );
    }
}
fn dialog(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
fn display(value: &Value, field: &Field) -> String {
    match (&field.kind, value) {
        (Kind::Toggle { inverse }, Value::Boolean(v)) => format!(
            "[{}] {}",
            if *v ^ inverse { "x" } else { " " },
            if *v ^ inverse {
                tr("开启", "On")
            } else {
                tr("关闭", "Off")
            }
        ),
        (Kind::Choice(_), v) => {
            let s = v
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string());
            format!("{} ▾", option_label(field.key, &s))
        }
        (Kind::Number { percent: true, .. }, Value::Integer(n)) => {
            format!("{}.{:02}%", n / 100, n % 100)
        }
        (_, Value::String(s)) => {
            if s.is_empty() {
                tr("（空）", "(empty)")
            } else {
                crate::preview::safe_text(&s.lines().next().unwrap_or("").to_string())
            }
        }
        (_, Value::Array(a)) => format!("{} {}  ▸", a.len(), tr("个条目", "entries")),
        (_, Value::Table(t)) => format!("{} {}  ▸", t.len(), tr("个字段", "fields")),
        _ => value.to_string(),
    }
}
fn item_label(value: &Value, field: &Field) -> String {
    if let Value::Table(t) = value {
        for key in ["name", "path", "project"] {
            if let Some(v) = t.get(key).and_then(Value::as_str) {
                return v.into();
            }
        }
    }
    if let Some(s) = value.as_str() {
        return if s.is_empty() {
            tr("空参数", "Empty argument")
        } else {
            crate::preview::safe_text(s)
        };
    }
    field.label().into()
}
fn initial(f: &Field) -> Value {
    if let Kind::Object(fields) = &f.kind {
        Value::Table(
            fields
                .iter()
                .filter(|f| !f.optional || f.key == "path" || f.key == "name")
                .map(|f| (f.key.into(), initial(f)))
                .collect(),
        )
    } else {
        f.default.clone()
    }
}
