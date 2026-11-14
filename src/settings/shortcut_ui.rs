//! Shortcut management never writes preferences independently of the outer draft.
use super::{
    draft::{self, Draft},
    model::{self, Binding, Sequence as ModelSequence},
};
use crate::shortcuts::{Chord, Index, Sequence, command_contexts};
use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as Mods};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
#[derive(Clone, Debug)]
pub enum Action {
    Select(usize),
    Add,
    Record,
    Text,
    Modify,
    Scroll(i16),
    Finish,
    Cancel,
    Remove(usize),
    Prefer(usize),
    Disable,
    Restore,
    Prefix,
    Test,
    Context,
    Search,
    Note,
    Import,
    Export,
    ReplaceConflict,
    Scope,
    Filter,
    PreferTest,
    RestoreKey(usize),
    ConfirmExport,
    Details,
    Menu,
    Removed,
    RecordSequence,
    ConfirmRisk,
    ClearNotes,
    ClearNote,
    ExportEffective,
    Inactive,
    DeleteInactive(usize),
}
enum Transfer {
    Import(model::Shortcuts),
    Export(String),
}
enum Mode {
    Browse,
    Menu(usize),
    Removed(usize),
    Inactive(usize),
    Search,
    Record(Vec<Chord>),
    Text(TextArea<'static>),
    Prefix(TextArea<'static>),
    Test { received: Vec<String> },
    Note(TextArea<'static>),
    Import(TextArea<'static>),
    Export(TextArea<'static>),
}
pub struct Manager {
    selected: usize,
    binding: usize,
    search: String,
    context: usize,
    mode: Mode,
    pub error: Option<String>,
    pub message: String,
    conflict: Option<(String, String, ModelSequence)>,
    scope: usize,
    filter: usize,
    detail: bool,
    committed_router: crate::shortcuts::Router,
    proposed_router: crate::shortcuts::Router,
    test_keys: Vec<Chord>,
    last_test: Option<(String, String, Sequence)>,
    export_confirm: Option<String>,
    pub root: std::path::PathBuf,
    transfer: Option<std::sync::mpsc::Receiver<Result<Transfer, String>>>,
    raw_events: std::collections::VecDeque<String>,
    replacing: Option<(String, String, Sequence)>,
    detail_scroll: u16,
    candidate: Option<(model::Shortcuts, toml::Value)>,
    record_sequence: bool,
    native_down: Option<crate::shortcuts::ledger::PhysicalKey>,
    record_pending: Option<(crate::shortcuts::ledger::PhysicalKey, Chord)>,
    note_key: Option<String>,
    export_effective: bool,
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            selected: 0,
            binding: 0,
            search: String::new(),
            context: 0,
            mode: Mode::Browse,
            error: None,
            message: String::new(),
            conflict: None,
            scope: 0,
            filter: 0,
            detail: false,
            committed_router: Default::default(),
            proposed_router: Default::default(),
            test_keys: vec![],
            last_test: None,
            export_confirm: None,
            root: crate::config::home(),
            transfer: None,
            raw_events: Default::default(),
            replacing: None,
            detail_scroll: 0,
            candidate: None,
            record_sequence: false,
            native_down: None,
            record_pending: None,
            note_key: None,
            export_effective: false,
        }
    }
}
fn settings(d: &Draft) -> Result<model::Shortcuts> {
    d.tree.get("shortcuts").cloned().map_or_else(
        || Ok(Default::default()),
        |v| v.try_into().map_err(Into::into),
    )
}
fn set(d: &mut Draft, p: model::Shortcuts) -> Result<()> {
    draft::put(
        &mut d.tree,
        &draft::key(&[], "shortcuts"),
        Some(toml::Value::try_from(p)?),
    )
}
static CONTEXTS: std::sync::LazyLock<Vec<&str>> = std::sync::LazyLock::new(|| {
    let mut out = vec![
        "terminal.input",
        "overview",
        "tree",
        "editor",
        "preview",
        "preview.image",
    ];
    for &c in crate::shortcuts::CONTEXTS {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
});
impl Manager {
    /// All entry paths share the same validation and risk/preview confirmation.
    fn candidate(&mut self, d: &mut Draft, p: model::Shortcuts, preview: bool) -> Result<()> {
        Index::build(&p)?;
        let risks = crate::shortcuts::configuration_risks(&p);
        let existing = crate::shortcuts::configuration_risks(&settings(d)?);
        let added = risks
            .into_iter()
            .filter(|s| !existing.contains(s))
            .collect::<Vec<_>>();
        if preview || !added.is_empty() {
            self.message = format!(
                "Review draft / 审阅草稿: {} binding overrides. {}\n{}\nConfirm explicitly; system/input-method keys may never arrive / 确认后仅改草稿，系统/输入法按键可能无法送达",
                p.bindings.len(),
                if preview {
                    "Import preview / 导入预览"
                } else {
                    "Risk / 风险"
                },
                added.join(", ")
            );
            self.candidate = Some((p, toml::Value::try_from(settings(d)?)?));
            return Ok(());
        }
        set(d, p)
    }
    fn commands(&self, d: &Draft) -> Vec<crate::shortcuts::Command> {
        let settings = settings(d).unwrap_or_default();
        let index = Index::load(&settings);
        let query = self.search.to_lowercase();
        index
            .commands
            .iter()
            .filter(|c| {
                let bindings = index
                    .bindings
                    .iter()
                    .filter(|b| b.command == c.id)
                    .collect::<Vec<_>>();
                let edits = settings
                    .bindings
                    .iter()
                    .filter(|b| b.command == c.id)
                    .collect::<Vec<_>>();
                let search = format!(
                    "{} {} {}",
                    c.id,
                    crate::shortcuts::command_label(c.id),
                    bindings
                        .iter()
                        .flat_map(|b| b.keys.iter().map(Sequence::display))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .to_lowercase()
                .contains(&query);
                search
                    && match self.filter {
                        1 => crate::shortcuts::contexts_overlap(c.context, CONTEXTS[self.context]),
                        2 => !edits.is_empty(),
                        3 => bindings.iter().all(|b| b.disabled || b.keys.is_empty()),
                        4 => {
                            !index.diagnostics.is_empty()
                                && index.diagnostics.iter().any(|e| e.contains(c.id))
                        }
                        5 => edits.iter().any(|b| !b.notes.is_empty()),
                        _ => true,
                    }
            })
            .cloned()
            .collect()
    }
    fn command(&self, d: &Draft) -> Option<crate::shortcuts::Command> {
        let mut c = self.commands(d).get(self.selected).cloned()?;
        let contexts = command_contexts(c.id, c.context);
        c.context = contexts[self.scope.min(contexts.len() - 1)];
        Some(c)
    }
    fn edit_binding<'a>(p: &'a mut model::Shortcuts, id: &str, context: &str) -> &'a mut Binding {
        let i = p
            .bindings
            .iter()
            .position(|b| b.command == id && b.context == context)
            .unwrap_or_else(|| {
                p.bindings.push(Binding {
                    command: id.into(),
                    context: context.into(),
                    add: vec![],
                    remove: vec![],
                    disabled: false,
                    preferred: None,
                    notes: Default::default(),
                });
                p.bindings.len() - 1
            });
        &mut p.bindings[i]
    }
    fn modify(&mut self, d: &mut Draft, edit: impl FnOnce(&mut Binding)) -> Result<()> {
        let command = self.command(d).context("Select a command")?;
        let mut p = settings(d)?;
        edit(Self::edit_binding(&mut p, command.id, command.context));
        // Keep invalid drafts visible; outer Save all enforces all-or-nothing validation.
        self.error = Index::build(&p).err().map(|e| format!("{e:#}"));
        set(d, p)
    }
    fn effective_keys(&self, d: &Draft) -> Vec<Sequence> {
        let Some(c) = self.command(d) else {
            return vec![];
        };
        let i = Index::load(&settings(d).unwrap_or_default());
        i.bindings
            .iter()
            .find(|b| b.command == c.id && b.context == c.context)
            .map_or_else(|| c.defaults.clone(), |b| b.keys.clone())
    }
    pub fn reset_all(d: &mut Draft, keep_notes: bool) -> Result<()> {
        if !keep_notes {
            return draft::put(&mut d.tree, &draft::key(&[], "shortcuts"), None);
        }
        let mut p = settings(d)?;
        p.prefix = None;
        p.bindings.retain_mut(|b| {
            b.add.clear();
            b.remove.clear();
            b.preferred = None;
            b.disabled = false;
            !b.notes.is_empty()
        });
        set(d, p)
    }
    fn append(&mut self, d: &mut Draft, sequence: Sequence) -> Result<()> {
        let command = self.command(d).context("Select a command")?;
        let mut p = settings(d)?;
        let model = sequence.model();
        let index = Index::load(&p);
        if let Some((id, context, old)) = &self.replacing {
            ensure!(
                id == command.id
                    && context == command.context
                    && self.effective_keys(d).contains(old),
                "The selected binding changed; cancel and select it again"
            );
        }
        if self.effective_keys(d).contains(&sequence)
            && self
                .replacing
                .as_ref()
                .is_none_or(|(_, _, old)| old == &sequence)
        {
            self.message = "Already bound / 已存在此绑定".into();
            self.replacing = None;
            self.mode = Mode::Browse;
            self.error = None;
            return Ok(());
        }
        let collisions = index
            .bindings
            .iter()
            .filter(|b| {
                b.command != command.id
                    && !b.disabled
                    && b.keys.contains(&sequence)
                    && crate::shortcuts::contexts_overlap(&b.context, command.context)
            })
            .map(|b| format!("{} [{}]", b.command, b.context))
            .collect::<Vec<_>>();
        if !collisions.is_empty() {
            self.conflict = Some((command.id.into(), command.context.into(), model.clone()));
            self.error = Some(format!(
                "{} [{}] / {} on {}",
                command.id,
                command.context,
                collisions.join(", "),
                sequence.display()
            ));
            anyhow::bail!("{}", self.error.as_deref().unwrap());
        }

        self.remove_replaced(&mut p, &sequence)?;
        let binding = Self::edit_binding(&mut p, command.id, command.context);
        binding.remove.retain(|s| *s != model);
        if !self.effective_keys(d).contains(&sequence) && !binding.add.contains(&model) {
            binding.add.push(model.clone());
        }
        if let Err(e) = Index::build(&p) {
            self.conflict = Some((command.id.into(), command.context.into(), model));
            return Err(e);
        }
        self.candidate(d, p, false)?;
        self.replacing = None;
        self.mode = Mode::Browse;
        self.error = None;
        Ok(())
    }
    fn remove_replaced(&self, p: &mut model::Shortcuts, replacement: &Sequence) -> Result<()> {
        if let Some((id, context, old)) = &self.replacing {
            let prefix = Index::load(p).prefix;
            let preferred = Self::edit_binding(p, id, context)
                .preferred
                .as_ref()
                .is_some_and(|s| Sequence::parse(s, prefix).is_ok_and(|s| &s == old));
            Self::delete_key(p, id, context, old)?;
            if preferred {
                Self::edit_binding(p, id, context).preferred = Some(replacement.model());
            }
        }
        Ok(())
    }
    fn delete_key(p: &mut model::Shortcuts, id: &str, context: &str, key: &Sequence) -> Result<()> {
        let prefix = Index::load(p).prefix;
        let binding = Self::edit_binding(p, id, context);
        binding
            .add
            .retain(|s| !Sequence::parse(s, prefix).is_ok_and(|s| &s == key));
        if binding
            .preferred
            .as_ref()
            .is_some_and(|s| Sequence::parse(s, prefix).is_ok_and(|s| &s == key))
        {
            binding.preferred = None;
        }
        if Index::build(p)?
            .binding(id, context)
            .is_some_and(|b| b.keys.contains(key))
        {
            let binding = Self::edit_binding(p, id, context);
            if !binding
                .remove
                .iter()
                .any(|s| Sequence::parse(s, prefix).is_ok_and(|s| &s == key))
            {
                binding.remove.push(key.model());
            }
        }
        Ok(())
    }
    pub fn action(&mut self, action: Action, d: &mut Draft) -> Result<()> {
        if self.candidate.is_some() && !matches!(action, Action::ConfirmRisk | Action::Cancel) {
            anyhow::bail!("Confirm or cancel the pending candidate first / 请先确认或取消候选");
        }
        match action {
            Action::ConfirmRisk => {
                let (p, base) = self.candidate.as_ref().context("No candidate pending")?;
                ensure!(
                    toml::Value::try_from(settings(d)?)? == *base,
                    "Shortcut draft changed; cancel and review again"
                );
                Index::build(p)?;
                set(d, p.clone())?;
                self.candidate = None;
                self.replacing = None;
                self.mode = Mode::Browse;
                self.message =
                    "Accepted into draft; Save all to apply / 已确认草稿，保存全部后生效".into();
            }
            Action::ClearNotes => self.modify(d, |b| b.notes.clear())?,
            Action::ClearNote => {
                let key = self
                    .note_key
                    .clone()
                    .context("Select a deletion history first")?;
                self.modify(d, |b| {
                    b.notes.remove(&key);
                })?;
            }
            Action::Inactive => self.mode = Mode::Inactive(0),
            Action::DeleteInactive(i) => {
                let mut p = settings(d)?;
                let index = Index::build(&p)?;
                let indices = p
                    .bindings
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| {
                        index
                            .commands
                            .iter()
                            .find(|c| c.id == b.command)
                            .is_none_or(|c| {
                                !command_contexts(c.id, c.context).contains(&b.context.as_str())
                            })
                    })
                    .map(|(n, _)| n)
                    .collect::<Vec<_>>();
                if let Some(n) = indices.get(i) {
                    p.bindings.remove(*n);
                    set(d, p)?;
                }
            }
            Action::Menu => self.mode = Mode::Menu(0),
            Action::Removed => self.mode = Mode::Removed(0),
            Action::Details => {
                self.detail = true;
                self.detail_scroll = 0;
            }
            Action::Scroll(delta) => {
                self.detail = true;
                self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
            }
            Action::Filter => {
                self.filter = (self.filter + 1) % 6;
                self.selected = 0;
                self.scope = 0;
            }
            Action::Scope => {
                self.note_key = None;
                if let Some(c) = self.commands(d).get(self.selected) {
                    self.scope = (self.scope + 1) % command_contexts(c.id, c.context).len();
                }
            }
            Action::RestoreKey(i) => {
                let c = self.command(d).context("Select a command")?;
                let p = settings(d)?;
                if let Some(key) = p
                    .bindings
                    .iter()
                    .find(|b| b.command == c.id && b.context == c.context)
                    .and_then(|b| b.remove.get(i))
                    .cloned()
                {
                    self.note_key = Some(Sequence::parse(&key, Index::load(&p).prefix)?.display());
                    self.modify(d, |b| {
                        b.remove.retain(|s| s != &key);
                    })?;
                    self.message="Binding restored; history note retained. Clear notes to remove it / 已恢复绑定并保留历史备注，可清除备注".into();
                }
            }
            Action::PreferTest => {
                let (id, context, key) = self
                    .last_test
                    .clone()
                    .context("Test an effective command binding first")?;
                let mut p = settings(d)?;
                let index = Index::build(&p)?;
                ensure!(
                    index.resolve(&context, &key.0)
                        == crate::shortcuts::Resolution::Execute(
                            index
                                .commands
                                .iter()
                                .find(|c| c.id == id)
                                .context("Command disappeared")?
                                .id
                        ),
                    "The tested key is no longer an effective draft binding"
                );
                let b = Self::edit_binding(&mut p, &id, &context);
                b.preferred = Some(key.model());
                Index::build(&p)?;
                set(d, p)?;
                self.message =
                    "Preferred display changed in draft; other keys remain active".into();
            }
            Action::ConfirmExport => {
                let path = self.export_confirm.take().context("No export pending")?;
                let effective = self.export_effective;
                let root = self.root.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.transfer = Some(rx);
                self.message = "Exporting… / 正在导出…".into();
                std::thread::spawn(move || {
                    let _ = tx.send(
                        super::operations::execute_at(
                            &root,
                            &if effective {
                                super::operations::Action::ExportEffectiveShortcuts
                            } else {
                                super::operations::Action::ExportShortcuts
                            },
                            &path,
                        )
                        .map(Transfer::Export)
                        .map_err(|e| format!("{e:#}")),
                    );
                });
            }
            Action::Select(i) => {
                self.note_key = None;
                self.selected = i;
                self.binding = 0;
                self.scope = 0;
                self.detail = true;
                self.detail_scroll = 0;
                self.mode = Mode::Browse;
            }
            Action::Search => self.mode = Mode::Search,
            Action::Add | Action::Record | Action::RecordSequence => {
                self.record_sequence = matches!(action, Action::RecordSequence);
                self.record_pending = None;
                self.native_down = None;
                self.replacing = None;
                self.mode = Mode::Record(vec![]);
            }
            Action::Text => {
                self.replacing = None;
                self.mode = Mode::Text(TextArea::default());
            }
            Action::Modify => {
                let command = self.command(d).context("Select a command")?;
                let old = self
                    .effective_keys(d)
                    .get(self.binding)
                    .cloned()
                    .context("Select an effective binding to modify")?;
                self.mode = Mode::Text(TextArea::new(vec![
                    old.0
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" -> "),
                ]));
                self.replacing = Some((command.id.into(), command.context.into(), old));
            }
            Action::Prefix => {
                let p = settings(d)?.prefix.unwrap_or_else(|| "Ctrl+G".into());
                self.mode = Mode::Prefix(TextArea::new(vec![p]));
            }
            Action::Cancel => {
                self.candidate = None;
                self.record_pending = None;
                self.native_down = None;
                self.export_confirm = None;
                self.mode = Mode::Browse;
                self.conflict = None;
                self.replacing = None;
            }
            Action::Finish => self.finish(d)?,
            Action::Test => {
                self.mode = Mode::Test { received: vec![] };
                self.committed_router.cancel();
                self.proposed_router.cancel();
                self.test_keys.clear();
                self.last_test = None;
                self.raw_events.clear();
            }
            Action::Context => {
                self.context = (self.context + 1) % CONTEXTS.len();
                self.committed_router.cancel();
                self.proposed_router.cancel();
                self.test_keys.clear();
            }
            Action::Remove(i) => {
                let keys = self.effective_keys(d);
                if let Some(key) = keys.get(i) {
                    self.note_key = Some(key.display());
                    let c = self.command(d).context("Select a command")?;
                    let mut p = settings(d)?;
                    Self::delete_key(&mut p, c.id, c.context, key)?;
                    set(d, p)?;
                    // Removed aliases can have historical notes without becoming tombstones.
                    self.message="Deleted selected binding. Note adds an optional explanation / 已删除选中绑定，可添加备注".into();
                }
            }
            Action::Prefer(i) => {
                let c = self.command(d).context("Select a command")?;
                ensure!(
                    !Index::load(&settings(d)?)
                        .bindings
                        .iter()
                        .any(|b| b.command == c.id && b.context == c.context && b.disabled),
                    "Enable the command before choosing its preferred display"
                );
                let keys = self.effective_keys(d);
                if let Some(key) = keys.get(i) {
                    let key = key.model();
                    self.modify(d, |b| b.preferred = Some(key))?;
                }
            }
            Action::Disable => self.modify(d, |b| b.disabled = !b.disabled)?,
            Action::Restore => {
                let c = self.command(d).context("Select a command")?;
                let mut p = settings(d)?;
                p.bindings.retain_mut(|b| {
                    if b.command != c.id || b.context != c.context {
                        return true;
                    }
                    b.add.clear();
                    b.remove.clear();
                    b.disabled = false;
                    b.preferred = None;
                    !b.notes.is_empty()
                });
                set(d, p)?;
            }
            Action::Note => self.mode = Mode::Note(TextArea::default()),
            Action::Import => self.mode = Mode::Import(TextArea::default()),
            Action::Export | Action::ExportEffective => {
                self.export_effective = matches!(action, Action::ExportEffective);
                self.mode = Mode::Export(TextArea::default());
            }
            Action::ReplaceConflict => {
                let (id, context, sequence) =
                    self.conflict.clone().context("No pending conflict")?;
                let mut p = settings(d)?;
                let index = Index::load(&p);
                let key = Sequence::parse(&sequence, index.prefix)?;
                // Replacement is explicit and applies to every overlapping binding shown by validation.
                for b in index.bindings.iter().filter(|b| {
                    b.command != id
                        && b.keys.contains(&key)
                        && crate::shortcuts::contexts_overlap(&b.context, &context)
                }) {
                    let binding = Self::edit_binding(&mut p, &b.command, &b.context);
                    binding.add.retain(|s| s != &sequence);
                    if !binding.remove.contains(&sequence) {
                        binding.remove.push(sequence.clone());
                    }
                }
                self.remove_replaced(&mut p, &key)?;
                let b = Self::edit_binding(&mut p, &id, &context);
                b.add.push(sequence);
                Index::build(&p)?;
                self.candidate(d, p, false)?;
                self.conflict = None;
                self.replacing = None;
                self.mode = Mode::Browse;
                self.error = None;
            }
        }
        Ok(())
    }
    fn finish(&mut self, d: &mut Draft) -> Result<()> {
        let prefix = Index::load(&settings(d).unwrap_or_default()).prefix;
        match &self.mode {
            Mode::Record(chords) => {
                ensure!(
                    self.record_pending.is_none(),
                    "Release the recorded key first / 请先松开录制的键"
                );
                ensure!(!chords.is_empty(), "Press one or two chords first");
                self.append(d, Sequence(chords.clone()))?;
            }
            Mode::Text(area) => {
                let text = area.lines().join("");
                let parts = text.split(" -> ").map(str::to_string).collect::<Vec<_>>();
                let value = if parts.len() == 1 {
                    ModelSequence::Chord(parts[0].clone())
                } else {
                    ModelSequence::Steps(parts)
                };
                self.append(d, Sequence::parse(&value, prefix)?)?;
            }
            Mode::Prefix(area) => {
                let text = area.lines().join("");
                let chord = Chord::parse(&text)?;
                let mut p = settings(d)?;
                p.prefix = Some(chord.to_string());
                Index::build(&p)?;
                self.candidate(d, p, false)?;
                self.mode = Mode::Browse;
            }
            Mode::Note(area) => {
                let text = area.lines().join("\n");
                let c = self.command(d).context("Select a command")?;
                let p = settings(d)?;
                let removed = p
                    .bindings
                    .iter()
                    .find(|b| b.command == c.id && b.context == c.context)
                    .and_then(|b| {
                        b.remove
                            .get(self.binding.min(b.remove.len().saturating_sub(1)))
                    })
                    .cloned();
                let key = self
                    .note_key
                    .clone()
                    .or_else(|| {
                        removed
                            .as_ref()
                            .and_then(|s| Sequence::parse(s, prefix).ok())
                            .map(|s| s.display())
                    })
                    .context("Delete or select a binding before adding its note")?;
                self.modify(d, |b| {
                    if text.is_empty() {
                        b.notes.remove(&key);
                    } else {
                        b.notes.insert(key, text);
                    }
                })?;
                self.mode = Mode::Browse;
            }
            Mode::Import(area) => {
                let path = area.lines().join("");
                let (tx, rx) = std::sync::mpsc::channel();
                self.transfer = Some(rx);
                self.message = "Reading import… / 正在读取导入…".into();
                std::thread::spawn(move || {
                    let result = (|| -> Result<Transfer> {
                        let meta = std::fs::metadata(&path)?;
                        ensure!(meta.len() <= 4 * 1024 * 1024, "Import exceeds 4 MiB");
                        let source = super::store::Snapshot::parse(std::fs::read_to_string(path)?)?;
                        source.require("shortcuts")?;
                        Ok(Transfer::Import(source.effective.shortcuts))
                    })()
                    .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(result);
                });
            }
            Mode::Export(area) => {
                let path = area.lines().join("");
                ensure!(
                    std::path::Path::new(&path).is_absolute(),
                    "Choose an absolute export path"
                );
                self.export_confirm = Some(path);
                self.message="Export committed shortcuts only. Confirm export using the separate button; existing destination will be replaced.".into();
            }
            _ => self.mode = Mode::Browse,
        }
        Ok(())
    }
    pub fn pending(&self) -> bool {
        self.transfer.is_some()
    }
    pub fn tick(&mut self, d: &mut Draft) -> bool {
        let Some(result) = self.transfer.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.transfer = None;
        let applied = (|| -> Result<()> {
            match result.map_err(anyhow::Error::msg)? {
                Transfer::Export(message) => self.message = message,
                Transfer::Import(incoming) => {
                    let mut p = settings(d)?;
                    for b in incoming.bindings {
                        p.bindings
                            .retain(|old| old.command != b.command || old.context != b.context);
                        p.bindings.push(b);
                    }
                    if incoming.prefix.is_some() {
                        p.prefix = incoming.prefix;
                    }
                    Index::build(&p)?;
                    self.candidate(d, p, true)?;
                }
            }
            self.mode = Mode::Browse;
            Ok(())
        })();
        if let Err(e) = applied {
            self.error = Some(format!("{e:#}"));
        }
        true
    }
    pub fn observe_raw(&mut self, raw: &crate::shortcuts::ledger::RawKey) {
        if let Mode::Record(chords) = &mut self.mode {
            if raw.down {
                self.native_down = Some(raw.physical);
            } else if self
                .record_pending
                .as_ref()
                .is_some_and(|(p, _)| *p == raw.physical)
            {
                let (_, chord) = self.record_pending.take().unwrap();
                if chords.len() < if self.record_sequence { 2 } else { 1 } {
                    chords.push(chord);
                }
                self.native_down = None;
            }
            return;
        }
        if !matches!(self.mode, Mode::Test { .. }) {
            return;
        }
        if self.raw_events.len() == 8 {
            self.raw_events.pop_front();
        }
        self.raw_events.push_back(format!(
            "vk={} scan={} enhanced={} {} repeat={} ctrl=0x{:x} unicode=0x{:x}",
            raw.physical.vk,
            raw.physical.scan,
            raw.physical.enhanced,
            if raw.down { "down" } else { "up" },
            raw.repeat,
            raw.control,
            raw.unicode
        ));
    }
    pub fn key(&mut self, key: KeyEvent, d: &mut Draft) -> Result<bool> {
        if self.pending() {
            return Ok(true);
        }
        if self.candidate.is_some() {
            if key.code == KeyCode::Esc {
                self.action(Action::Cancel, d)?;
            } else if key.code == KeyCode::Enter && key.modifiers == Mods::CONTROL {
                self.action(Action::ConfirmRisk, d)?;
            }
            return Ok(true);
        }
        let committed = Index::load(&d.base.effective.shortcuts);
        let proposed = Index::build(&settings(d)?);
        let removed_count = self
            .command(d)
            .and_then(|c| {
                settings(d).ok().and_then(|p| {
                    p.bindings
                        .into_iter()
                        .find(|b| b.command == c.id && b.context == c.context)
                        .map(|b| b.remove.len())
                })
            })
            .unwrap_or(0);
        match &mut self.mode {
            Mode::Inactive(selected) => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = selected.saturating_add(1),
                    KeyCode::Delete => {
                        let n = *selected;
                        self.action(Action::DeleteInactive(n), d)?;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            Mode::Menu(selected) => {
                let actions = menu_actions();
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(actions.len() - 1),
                    KeyCode::Enter => {
                        let action = actions[*selected].1.clone();
                        self.mode = Mode::Browse;
                        self.action(action, d)?;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            Mode::Removed(selected) => {
                let count = removed_count;
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(count.saturating_sub(1)),
                    KeyCode::Enter => {
                        let i = *selected;
                        self.action(Action::RestoreKey(i), d)?;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            Mode::Record(chords) => {
                if matches!(
                    key.kind,
                    crossterm::event::KeyEventKind::Repeat
                        | crossterm::event::KeyEventKind::Release
                ) {
                    return Ok(true);
                }
                if !matches!(key.code, KeyCode::Modifier(_)) {
                    let chord = Chord::event(key);
                    if let Some(physical) = self.native_down.take() {
                        self.record_pending.get_or_insert((physical, chord));
                        return Ok(true);
                    }
                    if chords.len() < if self.record_sequence { 2 } else { 1 } {
                        chords.push(chord);
                    }
                    if let Some(w) = chord.warning() {
                        self.message = w.into();
                    }
                }
                return Ok(true);
            }
            Mode::Test { received } => {
                let chord = Chord::event(key);
                let context = CONTEXTS[self.context];
                if key.kind == crossterm::event::KeyEventKind::Press
                    && !self.proposed_router.waiting()
                {
                    self.last_test = None;
                }
                if key.kind == crossterm::event::KeyEventKind::Press
                    && !matches!(key.code, KeyCode::Modifier(_))
                {
                    self.test_keys.push(chord);
                }
                let result = self.committed_router.route(&committed, context, key);
                let prediction = match &proposed {
                    Ok(i) => {
                        let r = self.proposed_router.route(i, context, key);
                        if let crate::shortcuts::Resolution::Execute(id) = r {
                            self.last_test =
                                Some((id.into(), context.into(), Sequence(self.test_keys.clone())));
                        }
                        format!("{r:?}")
                    }
                    Err(e) => e.to_string(),
                };
                received.push(format!(
                    "{chord} {:?} {:?}: committed={result:?}; draft={prediction}",
                    key.kind, key.state
                ));
                if received.len() > 32 {
                    received.remove(0);
                }
                if !self.committed_router.waiting() && !self.proposed_router.waiting() {
                    self.test_keys.clear();
                }
                return Ok(true);
            }
            Mode::Text(area)
            | Mode::Prefix(area)
            | Mode::Note(area)
            | Mode::Import(area)
            | Mode::Export(area) => {
                if key.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                    self.replacing = None;
                } else if key.code == KeyCode::Enter {
                    self.finish(d)?;
                } else {
                    area.input(key);
                }
                return Ok(true);
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => self.mode = Mode::Browse,
                    KeyCode::Backspace => {
                        self.search.pop();
                        self.selected = 0;
                    }
                    KeyCode::Char(c) if !key.modifiers.intersects(Mods::CONTROL | Mods::ALT) => {
                        self.search.push(c);
                        self.selected = 0;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            Mode::Browse => {}
        }
        let action = match (key.code, key.modifiers) {
            (KeyCode::Char('a'), Mods::NONE) => Some(Action::Menu),
            (KeyCode::Char('v'), Mods::NONE) => Some(Action::Removed),
            (KeyCode::Char('r'), Mods::NONE) => Some(Action::Record),
            (KeyCode::Char('e'), Mods::NONE) => Some(Action::Text),
            (KeyCode::Char('m'), Mods::NONE) => Some(Action::Modify),
            (KeyCode::PageUp, Mods::NONE) => Some(Action::Scroll(-3)),
            (KeyCode::PageDown, Mods::NONE) => Some(Action::Scroll(3)),
            (KeyCode::Char('p'), Mods::NONE) => Some(Action::Prefix),
            (KeyCode::Char('t'), Mods::NONE) => Some(Action::Test),
            (KeyCode::Char('d'), Mods::NONE) => Some(Action::Disable),
            (KeyCode::Char('u'), Mods::NONE) => Some(Action::Restore),
            (KeyCode::Char('n'), Mods::NONE) => Some(Action::Note),
            (KeyCode::Char('i'), Mods::NONE) => Some(Action::Import),
            (KeyCode::Char('x'), Mods::NONE) => Some(Action::Export),
            (KeyCode::Char('c'), Mods::NONE) => Some(Action::Scope),
            (KeyCode::Char('f'), Mods::NONE) => Some(Action::Filter),
            (KeyCode::Delete, Mods::NONE) => Some(Action::Remove(self.binding)),
            (KeyCode::Char(' '), Mods::NONE) => Some(Action::Prefer(self.binding)),
            (KeyCode::Left, Mods::NONE) => {
                self.binding = self.binding.saturating_sub(1);
                return Ok(true);
            }
            (KeyCode::Right, Mods::NONE) => {
                self.binding =
                    (self.binding + 1).min(self.effective_keys(d).len().saturating_sub(1));
                return Ok(true);
            }
            (KeyCode::Enter, Mods::NONE) => Some(Action::Details),
            (KeyCode::Esc, Mods::NONE) if self.detail => {
                self.detail = false;
                return Ok(true);
            }
            _ => None,
        };
        if let Some(action) = action {
            self.action(action, d)?;
            return Ok(true);
        }
        match (key.code, key.modifiers) {
            (KeyCode::Up, Mods::NONE) => {
                self.note_key = None;
                self.selected = self.selected.saturating_sub(1);
                self.detail_scroll = 0;
                self.binding = 0;
                Ok(true)
            }
            (KeyCode::Down, Mods::NONE) => {
                self.note_key = None;
                self.selected = (self.selected + 1).min(self.commands(d).len().saturating_sub(1));
                self.detail_scroll = 0;
                self.binding = 0;
                Ok(true)
            }
            (KeyCode::Char('/'), Mods::NONE) => {
                self.mode = Mode::Search;
                Ok(true)
            }
            (KeyCode::Insert, _) => {
                self.action(Action::Record, d)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub fn paste(&mut self, text: &str) -> bool {
        match &mut self.mode {
            Mode::Text(a) | Mode::Prefix(a) | Mode::Note(a) | Mode::Import(a) | Mode::Export(a) => {
                a.insert_str(text);
                true
            }
            Mode::Search => {
                self.search.push_str(&text.replace(['\r', '\n'], ""));
                true
            }
            Mode::Menu(_)
            | Mode::Removed(_)
            | Mode::Inactive(_)
            | Mode::Record(_)
            | Mode::Test { .. } => true,
            Mode::Browse => false,
        }
    }
    pub fn exclusive(&self) -> bool {
        self.candidate.is_some() || !matches!(self.mode, Mode::Browse | Mode::Search)
    }
    pub fn draw(&mut self, f: &mut Frame, r: Rect, d: &Draft) -> Vec<(Rect, Action)> {
        let mut hits = vec![];
        let p = crate::appearance::palette();
        if r.width == 0 || r.height == 0 {
            return hits;
        }
        if let Mode::Inactive(selected) = &self.mode {
            let index = Index::load(&settings(d).unwrap_or_default());
            let p = settings(d).unwrap_or_default();
            let rows = p
                .bindings
                .iter()
                .filter(|b| {
                    index
                        .commands
                        .iter()
                        .find(|c| c.id == b.command)
                        .is_none_or(|c| {
                            !command_contexts(c.id, c.context).contains(&b.context.as_str())
                        })
                })
                .collect::<Vec<_>>();
            f.render_widget(
                Paragraph::new(
                    "Inactive entries / 失效配置：保留原始差异和备注；点击删除或 Esc 返回",
                ),
                Rect::new(r.x, r.y, r.width, 1),
            );
            let first = selected.saturating_sub(r.height.saturating_sub(3) as usize);
            for (i, b) in rows
                .iter()
                .enumerate()
                .skip(first)
                .take(r.height.saturating_sub(2) as usize)
            {
                let row = Rect::new(r.x, r.y + 1 + (i - first) as u16, r.width, 1);
                f.render_widget(
                    Paragraph::new(single_line(
                        &format!(
                            "{} {} [{}] {:?} [删除]",
                            if i == *selected { "▸" } else { " " },
                            b.command,
                            b.context,
                            b.notes
                        ),
                        r.width as usize,
                    )),
                    row,
                );
                hits.push((row, Action::DeleteInactive(i)));
            }
            let row = Rect::new(r.x, r.bottom() - 1, r.width, 1);
            f.render_widget(Paragraph::new("[返回 Back / Esc]"), row);
            hits.push((row, Action::Cancel));
            return hits;
        }
        if let Mode::Menu(selected) | Mode::Removed(selected) = &self.mode {
            let selected = *selected;
            let rows = if matches!(self.mode, Mode::Menu(_)) {
                menu_actions()
                    .into_iter()
                    .map(|(s, a)| (s.to_string(), a))
                    .collect::<Vec<_>>()
            } else {
                self.command(d)
                    .and_then(|c| {
                        settings(d).ok().and_then(|p| {
                            p.bindings
                                .into_iter()
                                .find(|b| b.command == c.id && b.context == c.context)
                        })
                    })
                    .map_or(vec![], |b| {
                        b.remove
                            .iter()
                            .enumerate()
                            .map(|(i, key)| {
                                (
                                    format!(
                                        "Restore / 恢复 {} · {}",
                                        Sequence::parse(
                                            key,
                                            Index::load(&settings(d).unwrap_or_default()).prefix
                                        )
                                        .map(|s| s.display())
                                        .unwrap_or_default(),
                                        b.notes
                                            .get(
                                                &Sequence::parse(
                                                    key,
                                                    Index::load(&settings(d).unwrap_or_default())
                                                        .prefix
                                                )
                                                .map(|s| s.display())
                                                .unwrap_or_default()
                                            )
                                            .cloned()
                                            .unwrap_or_default()
                                    ),
                                    Action::RestoreKey(i),
                                )
                            })
                            .collect()
                    })
            };
            f.render_widget(Block::default().style(crate::appearance::style()), r);
            let available = r.height.saturating_sub(2).max(1) as usize;
            let first = selected.saturating_sub(available - 1);
            for (i, (label, action)) in rows.into_iter().enumerate().skip(first).take(available) {
                let row = Rect::new(r.x, r.y + (i - first) as u16, r.width, 1);
                f.render_widget(
                    Paragraph::new(format!(
                        "{} {}",
                        if i == selected { "▸" } else { " " },
                        label
                    )),
                    row,
                );
                hits.push((row, action));
            }
            let row = Rect::new(r.x, r.bottom().saturating_sub(1), r.width, 1);
            f.render_widget(Paragraph::new("[返回 Back / Esc]"), row);
            hits.push((row, Action::Cancel));
            return hits;
        }
        let committed = Index::load(&d.base.effective.shortcuts);
        let proposed = Index::build(&settings(d).unwrap_or_default());
        let index = proposed.as_ref().unwrap_or(&committed);
        f.render_widget(
            Paragraph::new(format!(
                "Shortcuts / 快捷键   Prefix: {}   / {}\nTerminal owned / 终端占用: {}",
                index.prefix,
                format!(
                    "{}  Filter F: {}",
                    self.search,
                    [
                        "all",
                        "current context",
                        "modified",
                        "unbound",
                        "internal conflict",
                        "deletion notes"
                    ][self.filter]
                ),
                index.terminal_owned().join(", ")
            ))
            .style(Style::new().fg(p.accent)),
            Rect::new(r.x, r.y, r.width, r.height.min(2)),
        );
        let commands = self.commands(d);
        let wide = r.width >= 90;
        let left_w = if wide { r.width / 2 } else { r.width };
        let max = r.height.saturating_sub(12) as usize;
        let first = self.selected.saturating_sub(max.saturating_sub(1));
        if !self.exclusive() && (wide || !self.detail) {
            for (i, c) in commands.iter().enumerate().skip(first).take(max) {
                let row = Rect::new(r.x, r.y + 2 + (i - first) as u16, left_w, 1);
                f.render_widget(
                    Paragraph::new(format!(
                        "{} {}  {}",
                        if i == self.selected { "▸" } else { " " },
                        single_line(
                            &crate::shortcuts::command_label(c.id),
                            left_w.saturating_sub(index.preferred(c.id, c.context).len() as u16 + 5)
                                as usize
                        ),
                        index.preferred(c.id, c.context)
                    ))
                    .style(if i == self.selected {
                        Style::new().bg(p.selected)
                    } else {
                        Style::default()
                    }),
                    row,
                );
                hits.push((row, Action::Select(i)));
            }
        }
        let detail = if !wide && self.detail {
            Rect::new(r.x, r.y + 2, r.width, r.height.saturating_sub(10))
        } else if wide {
            Rect::new(
                r.x + left_w,
                r.y + 2,
                r.width - left_w,
                r.height.saturating_sub(8),
            )
        } else {
            Rect::new(r.x, r.y + 2 + max as u16, r.width, 4)
        };
        if let Some(c) = self.command(d) {
            let keys = self.effective_keys(d);
            let mut lines = vec![
                single_line(
                    &crate::shortcuts::command_label(c.id),
                    detail.width as usize,
                ),
                single_line(
                    &format!("Scope / 区域 C: {} · #{}", c.context, self.binding + 1),
                    detail.width as usize,
                ),
            ];
            for key in &keys {
                lines.push(format!(
                    "{} {}",
                    if index.preferred(c.id, c.context) == key.display() {
                        "●"
                    } else {
                        "○"
                    },
                    key.display()
                ));
            }
            let custom = settings(d).unwrap_or_default();
            let edit = custom
                .bindings
                .iter()
                .find(|b| b.command == c.id && b.context == c.context);
            let effective = index
                .bindings
                .iter()
                .find(|b| b.command == c.id && b.context == c.context);
            let declared = index
                .commands
                .iter()
                .find(|original| original.id == c.id)
                .unwrap();
            for text in [
                format!("ID: {}", c.id),
                format!("Default / 默认: {}", declared.defaults.iter().map(Sequence::display).collect::<Vec<_>>().join(", ")),
                format!("Source / 来源: {}", if edit.is_some() { "custom / 自定义差异" } else if c.context != declared.context { "inherited / 继承父区域" } else { "default / 默认" }),
                format!("Status / 状态: {}", if effective.is_some_and(|b| b.disabled) { "disabled / 已停用" } else if keys.is_empty() { "unbound / 未绑定" } else { "enabled / 已启用" }),
                format!("Allowed / 允许区域: {}", command_contexts(declared.id,declared.context).join(", ")),
                "Availability / 可用性: evaluated in the active view; settings does not execute commands / 由运行时当前视图判断，设置页不执行命令".into(),
                format!("Removed / 已删除: {}", edit.map(|b| b.remove.iter().filter_map(|s| Sequence::parse(s, index.prefix).ok()).map(|s| s.display()).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
                format!("Notes / 备注: {:?}", edit.map(|b| &b.notes)),
                "PgUp/PgDn scroll details / 翻页查看详情".into(),
            ] {
                lines.extend(wrap_lines(&text, detail.width as usize));
            }
            for diagnostic in index.diagnostics.iter().filter(|s| s.contains(c.id)) {
                lines.extend(wrap_lines(diagnostic, detail.width as usize));
            }
            self.detail_scroll = self
                .detail_scroll
                .min(lines.len().saturating_sub(detail.height as usize) as u16);
            f.render_widget(
                Paragraph::new(lines.join("\n")).scroll((self.detail_scroll, 0)),
                detail,
            );
            for n in 0..keys.len() {
                let position = 2 + n as u16;
                if position < self.detail_scroll {
                    continue;
                }
                let row = Rect::new(
                    detail.x,
                    detail.y + position - self.detail_scroll,
                    detail.width.min(22),
                    1,
                );
                if row.y < detail.bottom() {
                    hits.push((row, Action::Prefer(n)));
                    let remove = Rect::new(
                        row.right(),
                        row.y,
                        detail.width.saturating_sub(row.width).min(12),
                        1,
                    );
                    f.render_widget(Paragraph::new("[删除/Delete]"), remove);
                    hits.push((remove, Action::Remove(n)));
                }
            }
        }
        let status = if self.candidate.is_some() {
            self.message.clone()
        } else {
            match &self.mode {
            Mode::Record(keys)=>format!("Recording / 录制: {}\nEsc/Enter are recorded too. Use mouse Done/Cancel. Esc/Enter 也作为按键录制，使用鼠标完成/取消。",keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(" → ")),
            Mode::Test{received}=>format!("Key test / 按键测试 [{}]. Commands are never executed; external global hotkeys can still run / 不执行业务命令；外部全局热键仍可能触发。\n{}",CONTEXTS[self.context],received.iter().rev().take(2).cloned().chain(self.raw_events.back().cloned().or_else(|| Some("Physical fields unavailable until a native event arrives / 尚未收到原生物理事件；不能推断外部占用".into()))).collect::<Vec<_>>().join("\n")),
            Mode::Text(_)=>"Text: Ctrl+F3 or Ctrl+G -> N".into(),Mode::Prefix(_)=>"Prefix / 前缀 chord".into(),Mode::Note(_)=>"Deletion note / 删除备注".into(),Mode::Import(_)=>"Import file path / 导入文件路径（只修改草稿）".into(),Mode::Export(_)=>"Export committed bindings; existing destination is replaced / 导出已提交绑定；覆盖目标".into(),_=>self.error.as_ref().unwrap_or(&self.message).clone(),
        }
        };
        let overlay = Rect::new(r.x, r.bottom().saturating_sub(8), r.width, 5.min(r.height));
        if self.exclusive() {
            f.render_widget(Block::default().style(crate::appearance::style()), overlay);
        }
        if self.exclusive() || !status.is_empty() {
            f.render_widget(Paragraph::new(status).wrap(Wrap { trim: false }), overlay);
        }
        if let Mode::Text(a) | Mode::Prefix(a) | Mode::Note(a) | Mode::Import(a) | Mode::Export(a) =
            &self.mode
        {
            f.render_widget(a, Rect::new(overlay.x, overlay.y + 2, overlay.width, 2));
        }
        let mut buttons = if self.exclusive() {
            vec![
                ("完成 Done", Action::Finish),
                ("取消 Cancel", Action::Cancel),
                ("区域 Context", Action::Context),
                ("设为首选 Preferred", Action::PreferTest),
                ("替换冲突 Replace", Action::ReplaceConflict),
            ]
        } else {
            vec![
                ("录制 R", Action::Record),
                ("文本 E", Action::Text),
                ("修改 M", Action::Modify),
                ("前缀 P", Action::Prefix),
                ("测试 T", Action::Test),
                ("停用 D", Action::Disable),
                ("恢复 U", Action::Restore),
                ("备注 N", Action::Note),
                ("导入 I", Action::Import),
                ("导出 X", Action::Export),
                ("作用域 C", Action::Scope),
                ("筛选 F", Action::Filter),
                ("详情 Enter", Action::Details),
            ]
        };
        if self.exclusive() {
            buttons.retain(|(_, a)| match a {
                Action::Context | Action::PreferTest => matches!(self.mode, Mode::Test { .. }),
                Action::ReplaceConflict => self.conflict.is_some(),
                _ => true,
            });
        } else if r.width < 90 {
            buttons = vec![
                ("操作 A", Action::Menu),
                ("详情 Enter", Action::Details),
                ("筛选 F", Action::Filter),
            ];
        }
        if self.detail && !self.exclusive() {
            buttons.push(("详情↑ PgUp", Action::Scroll(-3)));
            buttons.push(("详情↓ PgDn", Action::Scroll(3)));
        }
        if self.export_confirm.is_some() {
            buttons = vec![
                ("取消 Cancel", Action::Cancel),
                ("确认导出已提交绑定 Confirm export", Action::ConfirmExport),
            ];
        }
        if self.candidate.is_some() {
            buttons = vec![
                ("确认草稿 Ctrl+Enter", Action::ConfirmRisk),
                ("取消 Cancel", Action::Cancel),
            ];
        }
        let mut x = r.x;
        if self.note_key.is_some() && !self.exclusive() {
            buttons.push(("清除此备注 Clear note", Action::ClearNote));
        }
        let mut y = r.bottom().saturating_sub(3);
        use unicode_width::UnicodeWidthStr;
        for (label, action) in buttons {
            let w = (label.width() as u16 + 3).min(r.width);
            if x + w > r.right() {
                x = r.x;
                y += 1;
            }
            if y >= r.bottom() {
                break;
            }
            let row = Rect::new(x, y, w, 1);
            f.render_widget(
                Paragraph::new(format!("[{label}]")).style(Style::new().fg(p.accent)),
                row,
            );
            hits.push((row, action));
            x += w;
        }
        hits
    }
}

fn menu_actions() -> Vec<(&'static str, Action)> {
    vec![
        ("录制组合键 Record chord (R)", Action::Record),
        ("录制前缀序列 Record sequence", Action::RecordSequence),
        ("清除当前命令历史备注 Clear notes", Action::ClearNotes),
        ("失效配置 Inactive entries", Action::Inactive),
        (
            "导出完整有效目录 Export effective catalog",
            Action::ExportEffective,
        ),
        ("文本 Text (E)", Action::Text),
        ("修改选中绑定 Modify (M)", Action::Modify),
        ("前缀 Prefix (P)", Action::Prefix),
        ("测试 Test (T)", Action::Test),
        ("停用/启用 Disable (D)", Action::Disable),
        ("恢复本命令 Restore (U)", Action::Restore),
        ("已删除绑定 Restore deleted (V)", Action::Removed),
        ("删除备注 Note (N)", Action::Note),
        ("导入 Import (I)", Action::Import),
        ("导出 Export (X)", Action::Export),
        ("作用域 Scope (C)", Action::Scope),
        ("筛选 Filter (F)", Action::Filter),
    ]
}

fn wrap_lines(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut lines = vec![String::new()];
    let mut used = 0;
    for ch in text.chars() {
        let n = ch.width().unwrap_or(0);
        if ch == '\n' || (used > 0 && used + n > width.max(1)) {
            lines.push(String::new());
            used = 0;
        }
        if ch != '\n' {
            lines.last_mut().unwrap().push(ch);
            used += n;
        }
    }
    lines
}
fn single_line(text: &str, width: usize) -> String {
    wrap_lines(text, width)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
