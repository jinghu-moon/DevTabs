//! Application-owned commands, exact chords and scoped binding differences.
use anyhow::{Result, bail, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as Mods};
use std::{collections::BTreeSet, fmt};

mod compile;
#[cfg(test)]
mod contract_tests;
pub mod ledger;
mod modals;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub code: KeyCode,
    pub modifiers: Mods,
}
impl Chord {
    pub fn event(event: KeyEvent) -> Self {
        let mut code = event.code;
        if code == KeyCode::BackTab {
            return Self {
                code: KeyCode::Tab,
                modifiers: event.modifiers | Mods::SHIFT,
            };
        }
        if let KeyCode::Char(c) = code {
            // Case is represented by Shift, never discarded for command matching.
            if c.is_ascii_alphabetic() {
                code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        let mut modifiers = event.modifiers;
        if matches!(code, KeyCode::Char(c) if c.is_ascii_punctuation()) {
            modifiers.remove(Mods::SHIFT);
        }
        Self { code, modifiers }
    }
    pub fn parse(text: &str) -> Result<Self> {
        let mut modifiers = Mods::empty();
        let mut rest = text;
        loop {
            let Some((head, tail)) = rest.split_once('+') else {
                break;
            };
            let modifier = match head.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Mods::CONTROL,
                "alt" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "super" | "win" => Mods::SUPER,
                _ => break,
            };
            ensure!(!modifiers.contains(modifier), "Repeated modifier");
            modifiers |= modifier;
            rest = tail;
        }
        ensure!(
            !rest.is_empty(),
            "Modifier-only system/input-method chords are unsupported; include a main key"
        );
        let lower = rest.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => {
                modifiers |= Mods::SHIFT;
                KeyCode::Tab
            }
            "space" => KeyCode::Char(' '),
            "plus" => KeyCode::Char('+'),
            "period" => KeyCode::Char('.'),
            "backspace" => KeyCode::Backspace,
            "delete" => KeyCode::Delete,
            "insert" => KeyCode::Insert,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            _ if lower.starts_with('f') && lower.len() > 1 => {
                let n = lower[1..].parse::<u8>()?;
                ensure!((1..=24).contains(&n), "Function key must be F1..F24");
                KeyCode::F(n)
            }
            _ => {
                let mut chars = rest.chars();
                let c = chars.next().ok_or_else(|| anyhow::anyhow!("Missing key"))?;
                ensure!(
                    chars.next().is_none() && !c.is_control(),
                    "Expected one key"
                );
                KeyCode::Char(c.to_ascii_lowercase())
            }
        };
        ensure!(
            !(code == KeyCode::Delete && modifiers.contains(Mods::CONTROL | Mods::ALT)),
            "Ctrl+Alt+Delete is a system security chord and cannot be bound"
        );
        ensure!(
            !matches!(code, KeyCode::Char(c) if !c.is_ascii()),
            "IME and composed text cannot be shortcut keys"
        );
        Ok(Self::event(KeyEvent::new(code, modifiers)))
    }
    pub fn event_value(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
    pub fn warning(self) -> Option<&'static str> {
        if self.modifiers.contains(Mods::SUPER)
            || (self.code == KeyCode::Char(' ')
                && self.modifiers.intersects(Mods::CONTROL | Mods::SHIFT))
            || (self.modifiers == Mods::CONTROL
                && matches!(self.code, KeyCode::Char('.') | KeyCode::Esc | KeyCode::F(4)))
            || (self.modifiers == Mods::ALT
                && matches!(self.code, KeyCode::Tab | KeyCode::Esc | KeyCode::F(4)))
        {
            Some("System or input-method shortcut; DevTabs may never receive it")
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn edit(command: &str, context: &str, add: &str) -> crate::settings::model::Binding {
        use crate::settings::model::{Binding, Sequence as S};
        Binding {
            command: command.into(),
            context: context.into(),
            add: vec![S::Chord(add.into())],
            remove: vec![],
            disabled: false,
            preferred: None,
            notes: Default::default(),
        }
    }
    #[test]
    fn defaults_are_valid_exact_and_scoped_with_one_terminal_prefix() {
        let i = Index::build(&Default::default()).unwrap();
        for text in ["F3", "Ctrl+F3"] {
            assert_eq!(
                i.resolve("overview.projects", &[Chord::parse(text).unwrap()]),
                Resolution::Execute("overview.note.edit")
            );
        }
        assert_eq!(
            i.preferred("overview.note.edit", "overview.projects"),
            "Ctrl+F3"
        );
        assert_eq!(
            i.resolve("overview.projects", &[Chord::parse("Alt+F3").unwrap()]),
            Resolution::TextInput
        );
        assert_eq!(
            i.resolve("editor", &[Chord::parse("F3").unwrap()]),
            Resolution::Execute("editor.match.next")
        );
        assert_eq!(i.terminal_owned(), ["Ctrl+G"]);
        for text in [
            "F3",
            "Ctrl+F3",
            "Ctrl+C",
            "Ctrl+Space",
            "Alt+Enter",
            "Ctrl+P",
        ] {
            assert_eq!(
                i.resolve("terminal.input", &[Chord::parse(text).unwrap()]),
                Resolution::ForwardTerminal
            );
        }
        let mut router = Router::default();
        assert_eq!(
            router.route(
                &i,
                "terminal.input",
                Chord::parse("Ctrl+G").unwrap().event_value()
            ),
            Resolution::AwaitSequence
        );
        assert_eq!(
            router.route(
                &i,
                "terminal.input",
                Chord::parse("Ctrl+G").unwrap().event_value()
            ),
            Resolution::Execute("terminal.send_prefix")
        );
    }
    #[test]
    fn diff_overrides_keep_new_defaults_and_reject_real_conflicts_or_prefix_commands() {
        use crate::settings::model::Shortcuts;
        let mut p = Shortcuts {
            prefix: None,
            bindings: vec![edit("overview.note.edit", "overview", "Ctrl+J")],
        };
        let i = Index::build(&p).unwrap();
        assert_eq!(
            i.resolve("overview", &[Chord::parse("Ctrl+J").unwrap()]),
            Resolution::Execute("overview.note.edit")
        );
        assert_eq!(
            i.resolve("overview", &[Chord::parse("F3").unwrap()]),
            Resolution::Execute("overview.note.edit")
        );
        p.bindings
            .push(edit("overview.favorite", "overview", "Ctrl+J"));
        assert!(Index::build(&p).is_err());
        p.bindings = vec![edit("overview.note.edit", "terminal.input", "F3")];
        assert!(Index::build(&p).is_err());
        p.bindings = vec![edit("overview.note.edit", "overview", "Ctrl+G")];
        assert!(Index::build(&p).is_err());
    }
    #[test]
    fn terminal_inventory_is_exhaustive_and_unregistered_chords_forward() {
        let i = Index::build(&Default::default()).unwrap();
        for code in (1..=24)
            .map(KeyCode::F)
            .chain(('a'..='z').map(KeyCode::Char))
        {
            for modifiers in [
                Mods::NONE,
                Mods::CONTROL,
                Mods::ALT,
                Mods::SHIFT,
                Mods::CONTROL | Mods::SHIFT,
            ] {
                let chord = Chord { code, modifiers };
                let expected = if chord == i.prefix {
                    Resolution::AwaitSequence
                } else {
                    Resolution::ForwardTerminal
                };
                assert_eq!(i.resolve("terminal.input", &[chord]), expected, "{chord}");
            }
        }
    }
    #[test]
    fn inherited_contexts_detect_intersection_and_child_disable_stops_fallback() {
        let mut p = crate::settings::model::Shortcuts::default();
        p.bindings = vec![
            edit("panel.quick_open", "panel", "Ctrl+J"),
            edit("workspace.zoom", "workspace", "Ctrl+J"),
        ];
        assert!(Index::build(&p).is_err());
        p.bindings = vec![edit("workspace.zoom", "tree", "Ctrl+J")];
        p.bindings[0].disabled = true;
        let i = Index::build(&p).unwrap();
        assert_eq!(
            i.resolve("tree", &[i.prefix, Chord::parse("Z").unwrap()]),
            Resolution::TextInput
        );
        assert_eq!(
            i.resolve("terminal.input", &[i.prefix, Chord::parse("Z").unwrap()]),
            Resolution::Execute("workspace.zoom")
        );
    }
    #[test]
    fn sequence_is_canceled_by_context_change_and_never_leaks_unknown_suffix() {
        let i = Index::load(&Default::default());
        let mut r = Router::default();
        assert_eq!(
            r.route(
                &i,
                "terminal.input",
                Chord::parse("Ctrl+G").unwrap().event_value()
            ),
            Resolution::AwaitSequence
        );
        assert_eq!(
            r.route(&i, "tree", Chord::parse("P").unwrap().event_value()),
            Resolution::Handled
        );
        assert!(!r.waiting());
        assert_eq!(
            Chord::parse("Ctrl+Space").unwrap().warning(),
            Some("System or input-method shortcut; DevTabs may never receive it")
        );
    }
}
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (modifier, name) in [
            (Mods::CONTROL, "Ctrl"),
            (Mods::ALT, "Alt"),
            (Mods::SHIFT, "Shift"),
            (Mods::SUPER, "Win"),
        ] {
            if self.modifiers.contains(modifier) {
                write!(f, "{name}+")?;
            }
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "Space"),
            KeyCode::Char('+') => write!(f, "Plus"),
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            KeyCode::F(n) => write!(f, "F{n}"),
            key => write!(f, "{key:?}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence(pub Vec<Chord>);
impl Sequence {
    pub fn parse(value: &crate::settings::model::Sequence, prefix: Chord) -> Result<Self> {
        let keys = match value {
            crate::settings::model::Sequence::Chord(c) => vec![c.as_str()],
            crate::settings::model::Sequence::Steps(v) => v.iter().map(String::as_str).collect(),
        };
        ensure!(
            (1..=2).contains(&keys.len()),
            "One chord or prefix plus one chord is supported"
        );
        let result = Self(
            keys.into_iter()
                .map(|s| {
                    if s == "Prefix" {
                        Ok(prefix)
                    } else {
                        Chord::parse(s)
                    }
                })
                .collect::<Result<_>>()?,
        );
        ensure!(
            result.0.len() == 1 || result.0[0] == prefix,
            "A sequence must start with the managed prefix"
        );
        Ok(result)
    }
    pub fn display(&self) -> String {
        self.0
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" → ")
    }
    pub fn model(&self) -> crate::settings::model::Sequence {
        let keys = self.0.iter().map(ToString::to_string).collect::<Vec<_>>();
        if keys.len() == 1 {
            crate::settings::model::Sequence::Chord(keys[0].clone())
        } else {
            crate::settings::model::Sequence::Steps(keys)
        }
    }
}
#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub context: &'static str,
    pub label: &'static str,
    pub defaults: Vec<Sequence>,
    pub preferred: usize,
    pub repeat: bool,
}
#[derive(Clone, Debug)]
pub struct Binding {
    pub command: String,
    pub context: String,
    pub keys: Vec<Sequence>,
    pub preferred: Option<Sequence>,
    pub disabled: bool,
}
#[derive(Clone, Debug)]
pub struct Index {
    pub commands: Vec<Command>,
    pub bindings: Vec<Binding>,
    pub prefix: Chord,
    pub diagnostics: Vec<String>,
}
/// Closed, ordered parent-before-child context set. Modals have no parent fallback.
pub const CONTEXTS: &[&str] = &[
    "app",
    "overview",
    "overview.projects",
    "overview.tasks",
    "overview.statistics",
    "overview.detail",
    "overview.queue",
    "overview.search",
    "workspace",
    "panel",
    "terminal.input",
    "tree",
    "editor",
    "editor.search",
    "preview",
    "preview.text",
    "preview.markdown",
    "preview.image",
    "preview.diff",
    "menu.terminals",
    "dialog.resources",
    "dialog.notifications",
    "dialog.review",
    "dialog.bookmarks",
    "dialog.info",
    "dialog.diff",
    "dialog.overview",
    "dialog.options",
    "dialog.presets",
    "dialog.queue",
    "dialog.instances",
    "dialog.history",
    "dialog.attention",
    "dialog.palette",
    "dialog.batch",
    "dialog.form",
    "dialog.help",
    "gesture.move",
    "gesture.resize",
    "terminal.selection",
    "shortcuts.manage",
    "shortcuts.record",
    "shortcuts.test",
];
fn reachable(declared: &str, actual: &str) -> bool {
    declared == actual
        || (declared == "app"
            && (actual == "overview"
                || actual.starts_with("overview.")
                || reachable("workspace", actual)))
        || (declared == "overview" && actual.starts_with("overview."))
        || (declared == "panel"
            && (matches!(actual, "tree" | "editor" | "preview") || actual.starts_with("preview.")))
        || (declared == "workspace"
            && matches!(
                actual,
                "terminal.input"
                    | "tree"
                    | "editor"
                    | "preview"
                    | "preview.text"
                    | "preview.markdown"
                    | "preview.image"
                    | "preview.diff"
            ))
        || (declared == "preview" && actual.starts_with("preview."))
}
fn conflict(a: &str, b: &str) -> bool {
    a == b
        || reachable(a, b)
        || reachable(b, a)
        || [
            "overview",
            "overview.projects",
            "terminal.input",
            "tree",
            "editor",
            "preview",
            "preview.image",
            "preview.markdown",
            "preview.diff",
        ]
        .iter()
        .any(|c| reachable(a, c) && reachable(b, c))
}
pub fn contexts_overlap(a: &str, b: &str) -> bool {
    conflict(a, b)
}
pub fn allowed_contexts(declared: &'static str) -> Vec<&'static str> {
    let mut out = vec![declared];
    for &context in CONTEXTS {
        if context != declared && reachable(declared, context) {
            out.push(context);
        }
    }
    out
}
pub fn command_contexts(id: &str, declared: &'static str) -> Vec<&'static str> {
    let mut contexts = allowed_contexts(declared);
    if id == "panel.quick_open" {
        contexts.push("terminal.input");
    }
    contexts
}
impl Index {
    pub fn build(settings: &crate::settings::model::Shortcuts) -> Result<Self> {
        compile::build(settings)
    }
    pub fn current() -> Self {
        crate::settings::store::SettingsStore::current()
            .read()
            .map(|s| Self::load(&s.effective.shortcuts))
            .unwrap_or_else(|e| {
                let mut i = Self::load(&Default::default());
                i.diagnostics.push(e.to_string());
                i
            })
    }
    pub fn load(settings: &crate::settings::model::Shortcuts) -> Self {
        Self::build(settings).unwrap_or_else(|e| {
            let mut index = Self::build(&Default::default()).expect("valid built-in shortcuts");
            index.diagnostics.push(format!("{e:#}"));
            index
        })
    }
    pub fn preferred(&self, id: &str, context: &str) -> String {
        self.binding(id, context)
            .filter(|b| !b.disabled)
            .and_then(|b| {
                b.preferred
                    .as_ref()
                    .filter(|p| b.keys.contains(p))
                    .or_else(|| b.keys.first())
            })
            .map_or_else(String::new, Sequence::display)
    }
    pub fn binding(&self, id: &str, context: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .filter(|b| b.command == id && reachable(&b.context, context))
            .max_by_key(|b| {
                if b.context == context {
                    4
                } else {
                    match b.context.as_str() {
                        "workspace" => 0,
                        "panel" => 1,
                        _ => 2,
                    }
                }
            })
            .filter(|b| !b.disabled)
    }
    pub fn resolve(&self, context: &str, keys: &[Chord]) -> Resolution {
        if !CONTEXTS.contains(&context) {
            return Resolution::TextInput;
        }
        for c in &self.commands {
            if let Some(b) = self.binding(c.id, context) {
                if b.keys.iter().any(|s| s.0 == keys) {
                    return Resolution::Execute(c.id);
                }
            }
        }
        if self.commands.iter().any(|c| {
            self.binding(c.id, context).is_some_and(|b| {
                b.keys
                    .iter()
                    .any(|s| s.0.starts_with(keys) && s.0.len() > keys.len())
            })
        }) {
            return Resolution::AwaitSequence;
        }
        if context == "terminal.input" {
            Resolution::ForwardTerminal
        } else {
            Resolution::TextInput
        }
    }
    pub fn terminal_owned(&self) -> Vec<String> {
        let mut keys = vec![];
        for c in &self.commands {
            if let Some(b) = self.binding(c.id, "terminal.input") {
                for s in &b.keys {
                    let key = s.0[0].to_string();
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
        }
        keys
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Execute(&'static str),
    AwaitSequence,
    Handled,
    TextInput,
    ForwardTerminal,
}
#[derive(Default)]
pub struct Router {
    waiting: Option<(String, Chord)>,
}
impl Router {
    pub fn cancel(&mut self) {
        self.waiting = None;
    }
    pub fn route(&mut self, index: &Index, context: &str, key: KeyEvent) -> Resolution {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return Resolution::Handled;
        }
        if key.kind == crossterm::event::KeyEventKind::Repeat {
            return Resolution::Handled;
        }
        if matches!(key.code, KeyCode::Modifier(_)) {
            return Resolution::Handled;
        }
        let chord = Chord::event(key);
        if let Some((owner, first)) = self.waiting.take() {
            if owner != context || chord == Chord::parse("Esc").unwrap() {
                return Resolution::Handled;
            }
            return match index.resolve(context, &[first, chord]) {
                Resolution::Execute(id) => Resolution::Execute(id),
                _ => Resolution::Handled,
            };
        }
        let result = index.resolve(context, &[chord]);
        if result == Resolution::AwaitSequence {
            self.waiting = Some((context.into(), chord));
        }
        result
    }
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }
}

pub fn workspace_menu_command(key: char) -> Option<&'static str> {
    match key {
        '.' => Some("workspace.files.toggle"),
        'z' => Some("workspace.zoom"),
        'w' => Some("workspace.terminal.menu"),
        '+' => Some("workspace.task.new"),
        'g' => Some("workspace.review"),
        '!' => Some("workspace.attention"),
        'm' => Some("workspace.resources"),
        'c' => Some("workspace.task.pin"),
        'e' => Some("workspace.task.replace"),
        '[' => Some("workspace.task.previous"),
        ']' => Some("workspace.task.next"),
        'v' => Some("workspace.axis"),
        'a' => Some("workspace.agent.focus"),
        'u' => Some("workspace.language.next"),
        'd' => Some("workspace.debug.focus"),
        'j' => Some("workspace.agent.restart"),
        'h' => Some("workspace.agent.fresh"),
        'l' => Some("workspace.layout.next"),
        'x' => Some("workspace.detach"),
        'p' => Some("workspace.overview"),
        't' => Some("workspace.popup.move"),
        'n' => Some("workspace.session.next"),
        'k' => Some("workspace.interrupt"),
        'F' => Some("workspace.force"),
        'b' => Some("workspace.tree.toggle"),
        'f' => Some("workspace.tree.focus"),
        'o' => Some("workspace.preview.focus"),
        'i' => Some("workspace.path.insert"),
        'r' => Some("workspace.debug.restart"),
        's' => Some("terminal.selection.toggle"),
        'y' => Some("terminal.selection.copy"),
        'q' => Some("workspace.quit"),
        ',' => Some("settings.workspace.open"),
        '*' => Some("shortcuts.open"),
        '~' => Some("terminal.send_prefix"),
        '`' => Some("terminal.send_quick_open_key"),
        _ => None,
    }
}
fn catalog(prefix: Chord) -> Vec<Command> {
    let mut out = vec![];
    let mut add = |id: &'static str, context: &'static str, keys: &[&str], repeat: bool| {
        let defaults = keys
            .iter()
            .map(|s| {
                Sequence::parse(
                    &crate::settings::model::Sequence::Chord((*s).into()),
                    prefix,
                )
                .unwrap()
            })
            .collect();
        out.push(Command {
            id,
            context,
            label: id,
            defaults,
            preferred: 0,
            repeat,
        });
    };
    for (id, context, keys) in [
        ("overview.activate", "overview.projects", vec!["Enter"]),
        ("overview.quit", "overview", vec!["Esc"]),
        ("overview.search", "overview", vec!["/"]),
        ("overview.queue.toggle", "overview.projects", vec!["Space"]),
        ("overview.queue.open", "overview.queue", vec!["Enter"]),
        ("overview.queue.delete", "overview.queue", vec!["Delete"]),
        ("overview.queue.undo", "overview", vec!["Ctrl+Z"]),
        ("overview.agent.next", "overview.projects", vec!["Right"]),
        ("overview.agent.previous", "overview.projects", vec!["Left"]),
        (
            "overview.project.expand",
            "overview.projects",
            vec!["Alt+Right"],
        ),
        (
            "overview.project.collapse",
            "overview.projects",
            vec!["Alt+Left"],
        ),
        (
            "overview.statistics.period",
            "overview.statistics",
            vec!["Y", "Shift+Y"],
        ),
        (
            "overview.statistics.expand",
            "overview.statistics",
            vec!["Z", "Shift+Z"],
        ),
        (
            "overview.detail.expand",
            "overview.detail",
            vec!["E", "Shift+E"],
        ),
        ("overview.detail.activate", "overview.detail", vec!["Enter"]),
        (
            "overview.statistics.activate",
            "overview.statistics",
            vec!["Enter"],
        ),
    ] {
        add(id, context, &keys, false);
    }
    for (id, key) in [
        ("overview.help", "F1"),
        ("overview.favorite", "F2"),
        ("overview.project.settings", "F4"),
        ("overview.refresh", "F5"),
        ("overview.filter", "F6"),
        ("overview.sort", "F7"),
        ("overview.statistics", "F8"),
        ("overview.history", "F9"),
        ("overview.details", "F10"),
        ("overview.launch", "Ctrl+Enter"),
        ("overview.options", "Alt+Enter"),
        ("overview.new_project", "Ctrl+N"),
        ("overview.presets", "Ctrl+B"),
        ("overview.root.add", "Ctrl+R"),
        ("overview.project.add", "Ctrl+A"),
        ("overview.project.exclude", "Ctrl+E"),
        ("overview.preset.save", "Ctrl+S"),
        ("overview.language.next", "Ctrl+L"),
        ("overview.theme.next", "Ctrl+T"),
        ("overview.copy_path", "Ctrl+C"),
        ("overview.palette", "Ctrl+P"),
        ("overview.select_all", "Ctrl+Shift+A"),
        ("overview.attention", "!"),
    ] {
        add(id, "overview", &[key], false);
    }
    add("overview.note.edit", "overview", &["Ctrl+F3", "F3"], false);
    add("settings.open", "overview", &["Ctrl+,"], false);
    for (id, keys) in [
        ("editor.copy", vec!["Ctrl+C"]),
        ("editor.cut", vec!["Ctrl+X"]),
        ("editor.paste", vec!["Ctrl+V"]),
        ("editor.undo", vec!["Ctrl+Z"]),
        ("editor.redo", vec!["Ctrl+Y"]),
        ("editor.select_all", vec!["Ctrl+A"]),
        ("editor.search", vec!["Ctrl+F"]),
        ("editor.save", vec!["Ctrl+S"]),
        ("editor.match.next", vec!["Ctrl+F3", "F3"]),
        ("editor.match.previous", vec!["F4"]),
    ] {
        add(id, "editor", &keys, false);
    }
    for context in ["tree", "preview", "editor"] {
        add(
            match context {
                "tree" => "tree.menu",
                "editor" => "editor.menu",
                _ => "preview.menu",
            },
            context,
            &["F2"],
            false,
        );
        add(
            match context {
                "tree" => "tree.refresh",
                "editor" => "editor.refresh",
                _ => "preview.refresh",
            },
            context,
            &["F5"],
            false,
        );
    }
    add("panel.quick_open", "panel", &["Ctrl+P"], false);
    add("terminal.send_quick_open_key", "terminal.input", &[], false);
    add("panel.follow", "panel", &[], false);
    add("panel.compact", "panel", &[], false);
    add("panel.modified", "tree", &["G"], false);
    add("panel.new_file", "tree", &["Ctrl+N"], false);
    add("panel.new_directory", "tree", &["Ctrl+Shift+N"], false);
    add("panel.rename", "tree", &["F4"], false);
    add("panel.trash", "tree", &["Delete"], false);
    add("panel.preview_selected", "panel", &[], false);
    add("panel.image_fit", "preview.image", &["0"], false);
    add("panel.image_actual", "preview.image", &["1"], false);
    add("panel.image_zoom_in", "preview.image", &["+", "="], false);
    add("panel.image_zoom_out", "preview.image", &["-"], false);
    add("panel.image_profile", "preview.image", &["P"], false);
    add("panel.image_protocol", "panel", &[], false);
    add("panel.image_refresh", "panel", &[], false);
    add("panel.maximize", "preview", &["Z"], false);
    add("panel.wrap", "preview", &["W"], false);
    add("panel.compare", "preview", &["V"], false);
    add("panel.diff", "preview", &["D"], false);
    add("panel.goto", "preview", &["G"], false);
    add("panel.search", "preview", &["/"], false);
    add("panel.next_tab", "preview", &["Alt+PageDown"], false);
    add("panel.previous_tab", "preview", &["Alt+PageUp"], false);
    add("panel.insert", "panel", &[], false);
    add("panel.copy_absolute", "tree", &["Ctrl+C"], false);
    add("panel.copy_relative", "panel", &[], false);
    add("panel.reveal", "panel", &[], false);
    add("panel.external", "preview", &["O"], false);
    add("panel.collapse", "panel", &[], false);
    add("panel.release", "panel", &[], false);
    add("panel.icons", "panel", &[], false);
    add("panel.theme", "panel", &[], false);
    add("panel.language", "panel", &[], false);
    add("panel.locate", "panel", &[], false);
    add("panel.copy_content", "panel", &[], false);
    add("panel.copy_source", "panel", &[], false);
    add("panel.close", "panel", &[], false);
    add("panel.edit", "preview", &["E"], false);
    add("panel.save", "panel", &[], false);
    add("panel.undo", "panel", &[], false);
    add("panel.redo", "panel", &[], false);
    add("panel.cut", "panel", &[], false);
    add("panel.paste", "panel", &[], false);
    add("panel.find", "panel", &[], false);
    add("panel.project", "panel", &[], false);
    add("panel.end_edit", "panel", &[], false);
    add("panel.mode", "preview", &["M"], false);
    add("panel.outline", "preview", &["T"], false);
    add("panel.back", "preview", &["Alt+Left"], false);
    add("panel.forward", "preview", &["Alt+Right"], false);
    add("panel.more", "panel", &[], false);
    add("panel.close_others", "panel", &[], false);
    add("panel.close_right", "panel", &[], false);
    add("panel.bookmark", "preview", &["B"], false);
    add("panel.bookmarks", "preview", &["Ctrl+B"], false);
    add("panel.info", "preview", &["I"], false);
    add("tree.search", "tree", &["/"], false);
    add("tree.hidden", "tree", &["."], false);
    add("tree.ignored", "tree", &["I"], false);
    add("preview.match.next", "preview", &["N"], false);
    add("preview.match.previous", "preview", &["Shift+N"], false);
    add("preview.copy_selection", "preview", &["Ctrl+C"], false);
    add("preview.copy_path", "preview", &["Alt+C"], false);
    drop(add);
    for (id, key) in [
        ("workspace.files.toggle", "."),
        ("workspace.zoom", "Z"),
        ("workspace.terminal.menu", "W"),
        ("workspace.task.new", "+"),
        ("workspace.review", "G"),
        ("workspace.attention", "!"),
        ("workspace.resources", "M"),
        ("workspace.task.pin", "C"),
        ("workspace.task.replace", "E"),
        ("workspace.task.previous", "["),
        ("workspace.task.next", "]"),
        ("workspace.axis", "V"),
        ("workspace.agent.focus", "A"),
        ("workspace.language.next", "U"),
        ("workspace.debug.focus", "D"),
        ("workspace.agent.restart", "J"),
        ("workspace.agent.fresh", "H"),
        ("workspace.layout.next", "L"),
        ("workspace.detach", "X"),
        ("workspace.overview", "P"),
        ("workspace.popup.move", "T"),
        ("workspace.session.next", "N"),
        ("workspace.interrupt", "K"),
        ("workspace.force", "Shift+F"),
        ("workspace.tree.toggle", "B"),
        ("workspace.tree.focus", "F"),
        ("workspace.preview.focus", "O"),
        ("workspace.path.insert", "I"),
        ("workspace.debug.restart", "R"),
        ("terminal.selection.toggle", "S"),
        ("terminal.selection.copy", "Y"),
        ("workspace.quit", "Q"),
        ("settings.workspace.open", ","),
    ] {
        out.push(Command {
            id,
            context: "workspace",
            label: id,
            defaults: {
                let values = vec![Sequence(vec![prefix, Chord::parse(key).unwrap()])];
                values
            },
            preferred: 0,
            repeat: false,
        });
    }
    for n in 1..=9 {
        let id = [
            "workspace.agent.1",
            "workspace.agent.2",
            "workspace.agent.3",
            "workspace.agent.4",
            "workspace.agent.5",
            "workspace.agent.6",
            "workspace.agent.7",
            "workspace.agent.8",
            "workspace.agent.9",
        ][n - 1];
        out.push(Command {
            id,
            context: "workspace",
            label: id,
            defaults: vec![Sequence(vec![
                prefix,
                Chord::parse(&n.to_string()).unwrap(),
            ])],
            preferred: 0,
            repeat: false,
        });
    }
    out.push(Command {
        id: "terminal.send_prefix",
        context: "workspace",
        label: "Send original prefix to terminal",
        defaults: vec![Sequence(vec![prefix, prefix])],
        preferred: 0,
        repeat: false,
    });
    out.push(Command {
        id: "shortcuts.open",
        context: "app",
        label: "Manage shortcuts",
        defaults: vec![],
        preferred: 0,
        repeat: false,
    });
    modals::extend(&mut out, prefix);
    out
}

pub fn command_label(id: &str) -> String {
    if crate::i18n::language() != crate::i18n::Language::ZhCn {
        return id.replace('.', " / ").replace('_', " ");
    }
    id.split('.')
        .map(|word| match word {
            "app" => "应用",
            "overview" => "总览",
            "activate" => "打开项目",
            "axis" => "排列方向",
            "back" => "后退",
            "close_others" => "关闭其他标签",
            "close_right" => "关闭右侧标签",
            "collapse" => "收起",
            "compact" => "紧凑目录",
            "compare" => "源码对照",
            "copy_absolute" => "复制绝对路径",
            "copy_content" => "复制内容",
            "copy_relative" => "复制相对路径",
            "copy_selection" => "复制选区",
            "copy_source" => "复制源码",
            "delete" => "删除",
            "detail" => "详情",
            "diff" => "文件差异",
            "end_edit" => "结束编辑",
            "expand" => "展开",
            "external" => "使用外部应用打开",
            "follow" => "跟随当前文件",
            "forward" => "前进",
            "goto" => "跳转位置",
            "hidden" => "隐藏文件",
            "icons" => "文件图标",
            "ignored" => "忽略文件",
            "image" => "图片",
            "image_actual" => "图片原始大小",
            "image_fit" => "适应图片",
            "image_profile" => "图片质量",
            "image_protocol" => "图片协议",
            "image_refresh" => "刷新图片",
            "image_zoom_in" => "放大图片",
            "image_zoom_out" => "缩小图片",
            "info" => "文件信息",
            "input" => "输入区",
            "locate" => "定位文件",
            "markdown" => "Markdown",
            "maximize" => "最大化",
            "modified" => "仅修改文件",
            "more" => "更多操作",
            "new_directory" => "新建目录",
            "new_file" => "新建文件",
            "next_tab" => "下一标签",
            "outline" => "标题大纲",
            "path" => "路径",
            "period" => "统计周期",
            "preview_selected" => "预览选中文件",
            "previous_tab" => "上一标签",
            "projects" => "项目列表",
            "queue" => "启动队列",
            "quick_open" => "快速打开",
            "release" => "释放目录缓存",
            "rename" => "重命名",
            "reveal" => "在资源管理器中显示",
            "search" => "搜索",
            "tasks" => "任务列表",
            "text" => "文本",
            "trash" => "移入回收站",
            "workspace" => "工作区",
            "terminal" => "终端",
            "panel" => "文件面板",
            "editor" => "编辑器",
            "preview" => "预览",
            "tree" => "文件树",
            "settings" => "设置",
            "shortcuts" => "快捷键",
            "note" => "便签",
            "edit" => "编辑",
            "help" => "帮助",
            "favorite" => "收藏",
            "refresh" => "刷新",
            "filter" => "筛选",
            "sort" => "排序",
            "statistics" => "统计",
            "history" => "会话历史",
            "details" => "详情",
            "launch" => "启动",
            "options" => "本次选项",
            "new_project" => "新建项目",
            "presets" => "启动预设",
            "preset" => "预设",
            "project" => "项目",
            "root" => "根目录",
            "add" => "添加",
            "exclude" => "排除",
            "save" => "保存",
            "language" => "语言",
            "theme" => "主题",
            "next" => "下一个",
            "previous" => "上一个",
            "copy_path" => "复制路径",
            "palette" => "命令面板",
            "select_all" => "全选",
            "attention" => "需处理",
            "files" => "文件",
            "toggle" => "切换",
            "zoom" => "放大",
            "menu" => "菜单",
            "task" => "任务",
            "new" => "新建",
            "review" => "审阅",
            "resources" => "资源",
            "pin" => "固定",
            "replace" => "替换",
            "agent" => "Agent",
            "debug" => "Debug",
            "focus" => "聚焦",
            "insert" => "插入路径",
            "restart" => "重启",
            "fresh" => "新会话",
            "layout" => "布局",
            "detach" => "脱离",
            "popup" => "浮层",
            "move" => "移动",
            "session" => "会话",
            "interrupt" => "中断",
            "force" => "强制停止",
            "selection" => "选区",
            "copy" => "复制",
            "quit" => "退出",
            "open" => "打开",
            "send_prefix" => "发送前缀原键",
            "send_quick_open_key" => "发送 Ctrl+P 原键",
            "match" => "搜索匹配",
            "find" => "查找",
            "undo" => "撤销",
            "redo" => "重做",
            "cut" => "剪切",
            "paste" => "粘贴",
            "close" => "关闭",
            "wrap" => "自动换行",
            "mode" => "模式",
            "bookmark" => "书签",
            "bookmarks" => "书签列表",
            _ => word,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Explicit terminal-send commands form one balanced cycle, never a router replay.
pub fn send_chord(chord: Chord, wire: ledger::WireMode) -> Vec<u8> {
    let key = chord.event_value();
    let win32 = wire == ledger::WireMode::Win32;
    let mut bytes = crate::input::key(
        key,
        matches!(
            wire,
            ledger::WireMode::Vt {
                application_cursor: true
            }
        ),
        win32,
    );
    if win32 {
        bytes.extend(crate::input::key(
            KeyEvent {
                kind: crossterm::event::KeyEventKind::Release,
                ..key
            },
            false,
            true,
        ));
    }
    bytes
}

pub fn configuration_risks(p: &crate::settings::model::Shortcuts) -> Vec<String> {
    let prefix = Chord::parse(p.prefix.as_deref().unwrap_or("Ctrl+G"))
        .unwrap_or_else(|_| Chord::parse("Ctrl+G").unwrap());
    let mut risks = vec![];
    if prefix.warning().is_some() {
        risks.push(format!("Prefix: {prefix}"));
    }
    for b in &p.bindings {
        for s in &b.add {
            if let Ok(s) = Sequence::parse(s, prefix) {
                for c in s.0.iter().filter(|c| c.warning().is_some()) {
                    risks.push(format!("{} [{}]: {}", b.command, b.context, c));
                }
            }
        }
    }
    risks.sort();
    risks.dedup();
    risks
}
