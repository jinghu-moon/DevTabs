//! Shortcut management never writes preferences independently of the outer draft.
mod view;
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
    FilterTo(usize),
    ModifyKey(usize),
    RemoveNote(usize),
    SetContext(usize),
    Transfers,
    BackToList,
    ClearTest,
}
enum Transfer {
    Import(model::Shortcuts),
    Export(String),
}
enum Mode {
    Browse,
    Menu(usize),
    Transfers(usize),
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
    list_area: Rect,
    detail_area: Rect,
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
            list_area: Rect::default(),
            detail_area: Rect::default(),
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
        let mut commands = self.filtered_commands(&settings, &index, self.filter, &self.search);
        const COMMON: &[&str] = &[
            "overview.note.edit",
            "overview.favorite",
            "overview.settings",
            "overview.refresh",
            "overview.history",
            "overview.queue.launch",
            "overview.palette",
            "shortcuts.open",
            "editor.match.next",
            "editor.match.previous",
            "editor.save",
            "workspace.layout.next",
        ];
        commands.sort_by_key(|c| {
            COMMON
                .iter()
                .position(|id| *id == c.id)
                .unwrap_or(COMMON.len())
        });
        commands
    }
    fn filtered_commands(
        &self,
        settings: &model::Shortcuts,
        index: &Index,
        filter: usize,
        query: &str,
    ) -> Vec<crate::shortcuts::Command> {
        let query = query.to_lowercase();
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
                    "{} {} {} {} {} {}",
                    c.id,
                    view::label(c.id),
                    crate::shortcuts::command_label(c.id),
                    c.context,
                    view::context_label(c.context),
                    bindings
                        .iter()
                        .flat_map(|b| b.keys.iter().map(Sequence::display))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .to_lowercase()
                .contains(&query);
                search
                    && match filter {
                        1 => crate::shortcuts::contexts_overlap(c.context, CONTEXTS[self.context]),
                        2 => !edits.is_empty(),
                        3 => bindings.iter().all(|b| b.disabled || b.keys.is_empty()),
                        4 => index
                            .diagnostics
                            .iter()
                            .any(|e| e.starts_with("Internal conflict:") && e.contains(c.id)),
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
        if self.candidate.is_some()
            && !matches!(
                action,
                Action::ConfirmRisk | Action::Cancel | Action::Scroll(_)
            )
        {
            anyhow::bail!("Confirm or cancel the pending candidate first / 请先确认或取消候选");
        }
        match action {
            Action::ClearTest => {
                self.raw_events.clear();
                self.last_test = None;
                self.test_keys.clear();
                self.committed_router.cancel();
                self.proposed_router.cancel();
                if let Mode::Test { received } = &mut self.mode {
                    received.clear();
                }
            }
            Action::BackToList => {
                self.detail = false;
                self.detail_scroll = 0;
            }
            Action::FilterTo(filter) => {
                self.filter = filter.min(5);
                self.selected = 0;
                self.binding = 0;
                self.scope = 0;
                self.detail_scroll = 0;
                self.note_key = None;
                self.mode = Mode::Browse;
            }
            Action::ModifyKey(i) => {
                self.binding = i;
                self.action(Action::Modify, d)?;
            }
            Action::RemoveNote(i) => {
                self.action(Action::Remove(i), d)?;
                self.action(Action::Note, d)?;
            }
            Action::SetContext(i) => {
                self.context = i.min(CONTEXTS.len() - 1);
                self.last_test = None;
                self.committed_router.cancel();
                self.proposed_router.cancel();
                self.test_keys.clear();
            }
            Action::Transfers => self.mode = Mode::Transfers(0),
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
                self.detail_scroll = 0;
                self.record_sequence = matches!(action, Action::RecordSequence);
                self.record_pending = None;
                self.native_down = None;
                self.replacing = None;
                self.mode = Mode::Record(vec![]);
            }
            Action::Text => {
                self.detail_scroll = 0;
                self.replacing = None;
                self.mode = Mode::Text(TextArea::default());
            }
            Action::Modify => {
                self.detail_scroll = 0;
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
                self.detail_scroll = 0;
                self.mode = Mode::Test { received: vec![] };
                self.committed_router.cancel();
                self.proposed_router.cancel();
                self.test_keys.clear();
                self.last_test = None;
                self.raw_events.clear();
            }
            Action::Context => {
                self.action(Action::SetContext((self.context + 1) % CONTEXTS.len()), d)?;
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
        let transfer_menu = matches!(self.mode, Mode::Transfers(_));
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
            Mode::Menu(selected) | Mode::Transfers(selected) => {
                let actions = if transfer_menu {
                    view::transfer_actions()
                } else {
                    menu_actions()
                };
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
            | Mode::Transfers(_)
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
        self.draw_view(f, r, d)
    }
}

fn menu_actions() -> Vec<(&'static str, Action)> {
    use view::text;
    vec![
        (text("录制组合键 R", "Record chord R"), Action::Record),
        (
            text("录制前缀序列", "Record sequence"),
            Action::RecordSequence,
        ),
        (
            text("清除当前命令历史备注", "Clear command notes"),
            Action::ClearNotes,
        ),
        (text("失效配置", "Inactive entries"), Action::Inactive),
        (
            text("导出完整有效目录", "Export effective catalog"),
            Action::ExportEffective,
        ),
        (text("文本输入 E", "Enter text E"), Action::Text),
        (
            text("修改选中绑定 M", "Edit selected binding M"),
            Action::Modify,
        ),
        (text("修改前缀 P", "Change prefix P"), Action::Prefix),
        (text("按键测试 T", "Key test T"), Action::Test),
        (text("停用 / 启用 D", "Disable / enable D"), Action::Disable),
        (text("恢复此命令 U", "Restore command U"), Action::Restore),
        (
            text("恢复已删除绑定 V", "Restore removed bindings V"),
            Action::Removed,
        ),
        (text("删除备注 N", "Deletion note N"), Action::Note),
        (text("导入 I", "Import I"), Action::Import),
        (text("导出 X", "Export X"), Action::Export),
        (text("切换作用域 C", "Change scope C"), Action::Scope),
        (text("筛选 F", "Filter F"), Action::Filter),
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
