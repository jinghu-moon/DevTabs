//! Application-owned commands, exact chords and scoped binding differences.
use anyhow::{Result, bail, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as Mods};
use std::{collections::BTreeSet, fmt};

pub mod ledger;

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
        Self {
            code,
            modifiers: event.modifiers,
        }
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
        Ok(Self { code, modifiers })
    }
    pub fn event_value(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
    pub fn warning(self) -> Option<&'static str> {
        if self.modifiers.contains(Mods::SUPER)
            || (self.modifiers == Mods::CONTROL && self.code == KeyCode::Char(' '))
            || (self.modifiers == Mods::ALT && matches!(self.code, KeyCode::Tab | KeyCode::F(4)))
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
fn reachable(declared: &str, actual: &str) -> bool {
    declared == actual
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
impl Index {
    pub fn build(settings: &crate::settings::model::Shortcuts) -> Result<Self> {
        let prefix = Chord::parse(settings.prefix.as_deref().unwrap_or("Ctrl+G"))?;
        let commands = catalog(prefix);
        let mut bindings = commands
            .iter()
            .map(|c| Binding {
                command: c.id.into(),
                context: c.context.into(),
                keys: c.defaults.clone(),
                preferred: c.defaults.get(c.preferred).cloned(),
                disabled: false,
            })
            .collect::<Vec<_>>();
        let mut identities = BTreeSet::new();
        for edit in &settings.bindings {
            ensure!(
                identities.insert((&edit.command, &edit.context)),
                "Duplicate command/context override"
            );
            let command = commands
                .iter()
                .find(|c| c.id == edit.command)
                .ok_or_else(|| anyhow::anyhow!("Unknown command {}", edit.command))?;
            ensure!(
                reachable(command.context, &edit.context),
                "{} is unavailable in {}",
                edit.command,
                edit.context
            );
            let index = bindings
                .iter()
                .position(|b| b.command == edit.command && b.context == edit.context);
            let i = match index {
                Some(i) => i,
                None => {
                    bindings.push(Binding {
                        command: edit.command.clone(),
                        context: edit.context.clone(),
                        keys: command.defaults.clone(),
                        preferred: command.defaults.get(command.preferred).cloned(),
                        disabled: false,
                    });
                    bindings.len() - 1
                }
            };
            let binding = &mut bindings[i];
            binding.disabled = edit.disabled;
            for remove in &edit.remove {
                let sequence = Sequence::parse(remove, prefix)?;
                binding.keys.retain(|s| s != &sequence);
            }
            for add in &edit.add {
                let sequence = Sequence::parse(add, prefix)?;
                if !binding.keys.contains(&sequence) {
                    binding.keys.push(sequence);
                }
            }
            if let Some(preferred) = &edit.preferred {
                binding.preferred = Some(Sequence::parse(preferred, prefix)?);
            }
            if let Some(p) = &binding.preferred {
                if !binding.keys.contains(p) {
                    binding.preferred = binding.keys.first().cloned();
                }
            }
        }
        let mut diagnostics = vec![];
        // Explicit choices take precedence over a newly introduced default.
        for i in 0..bindings.len() {
            if bindings[i].disabled {
                continue;
            }
            for j in i + 1..bindings.len() {
                if bindings[j].disabled
                    || bindings[i].command == bindings[j].command
                    || !conflict(&bindings[i].context, &bindings[j].context)
                {
                    continue;
                }
                let shared = bindings[i]
                    .keys
                    .iter()
                    .filter(|s| bindings[j].keys.contains(s))
                    .cloned()
                    .collect::<Vec<_>>();
                if shared.is_empty() {
                    continue;
                }
                let explicit = |b: &Binding| {
                    settings
                        .bindings
                        .iter()
                        .any(|e| e.command == b.command && e.context == b.context)
                };
                match (explicit(&bindings[i]), explicit(&bindings[j])) {
                    (true, false) => {
                        bindings[j].keys.retain(|s| !shared.contains(s));
                        diagnostics.push(format!(
                            "Default {} suppressed by {}",
                            bindings[j].command, bindings[i].command
                        ));
                    }
                    (false, true) => {
                        bindings[i].keys.retain(|s| !shared.contains(s));
                        diagnostics.push(format!(
                            "Default {} suppressed by {}",
                            bindings[i].command, bindings[j].command
                        ));
                    }
                    _ => bail!(
                        "Internal conflict: {} / {} on {}",
                        bindings[i].command,
                        bindings[j].command,
                        shared[0].display()
                    ),
                }
            }
        }
        for b in bindings.iter().filter(|b| !b.disabled) {
            for sequence in &b.keys {
                ensure!(
                    !(sequence.0.len() == 1 && sequence.0[0] == prefix),
                    "Prefix cannot also execute a command"
                );
                for other in bindings
                    .iter()
                    .filter(|o| !o.disabled && conflict(&b.context, &o.context))
                {
                    ensure!(
                        !other.keys.iter().any(|s| s.0.len() == 1
                            && sequence.0.len() == 2
                            && s.0[0] == sequence.0[0]),
                        "Sequence opener also executes a standalone command"
                    );
                }
            }
        }
        Ok(Self {
            commands,
            bindings,
            prefix,
            diagnostics,
        })
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
            .and_then(|b| {
                b.preferred
                    .as_ref()
                    .filter(|p| b.keys.contains(p))
                    .or_else(|| b.keys.first())
            })
            .map_or_else(String::new, Sequence::display)
    }
    fn binding(&self, id: &str, context: &str) -> Option<&Binding> {
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
        let mut keys = vec![self.prefix.to_string()];
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
                let mut values = vec![Sequence(vec![prefix, Chord::parse(key).unwrap()])];
                if matches!(key, "+" | "!") {
                    let mut c = Chord::parse(key).unwrap();
                    c.modifiers |= Mods::SHIFT;
                    values.push(Sequence(vec![prefix, c]));
                }
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
    out
}
