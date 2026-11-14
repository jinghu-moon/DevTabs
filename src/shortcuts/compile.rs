//! Compile defaults and context-local differences independently of file order.
use super::*;

pub(super) fn build(settings: &crate::settings::model::Shortcuts) -> Result<Index> {
    let prefix = Chord::parse(settings.prefix.as_deref().unwrap_or("Ctrl+G"))?;
    let commands = catalog(prefix);
    let mut diagnostics = vec![];
    let mut identities = BTreeSet::new();
    for edit in &settings.bindings {
        ensure!(
            identities.insert((&edit.command, &edit.context)),
            "Duplicate command/context override"
        );
        if let Some(c) = commands.iter().find(|c| c.id == edit.command) {
            if CONTEXTS.contains(&edit.context.as_str()) {
                ensure!(
                    command_contexts(c.id, c.context).contains(&edit.context.as_str()),
                    "{} is unavailable in {}",
                    c.id,
                    edit.context
                );
            } else {
                diagnostics.push(format!(
                    "Inactive unknown context {} for {}",
                    edit.context, edit.command
                ));
            }
        } else {
            diagnostics.push(format!(
                "Inactive unknown command {} [{}]",
                edit.command, edit.context
            ));
        }
        let add = edit
            .add
            .iter()
            .map(|s| Sequence::parse(s, prefix))
            .collect::<Result<Vec<_>>>()?;
        let remove = edit
            .remove
            .iter()
            .map(|s| Sequence::parse(s, prefix))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !add.iter().any(|s| remove.contains(s)),
            "{} [{}]: add/remove overlap",
            edit.command,
            edit.context
        );
    }
    let mut bindings: Vec<Binding> = vec![];
    let mut explicit: Vec<Vec<Sequence>> = vec![];
    for command in &commands {
        for context in command_contexts(command.id, command.context) {
            let parent = bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| b.command == command.id && reachable(&b.context, context))
                .max_by_key(|(_, b)| CONTEXTS.iter().position(|c| *c == b.context).unwrap_or(0));
            let (mut b, mut origins) = if let Some((n, b)) = parent {
                (b.clone(), explicit[n].clone())
            } else {
                (
                    Binding {
                        command: command.id.into(),
                        context: context.into(),
                        keys: command.defaults.clone(),
                        preferred: command.defaults.get(command.preferred).cloned(),
                        disabled: false,
                    },
                    vec![],
                )
            };
            b.context = context.into();
            if command.id == "panel.quick_open" && context == "terminal.input" {
                // Extension is opt-in: panel defaults and parent edits never take terminal input.
                b.keys.clear();
                b.preferred = None;
                origins.clear();
                b.disabled = false;
            }
            if let Some(edit) = settings
                .bindings
                .iter()
                .find(|e| e.command == command.id && e.context == context)
            {
                b.disabled |= edit.disabled;
                let remove = edit
                    .remove
                    .iter()
                    .map(|s| Sequence::parse(s, prefix))
                    .collect::<Result<Vec<_>>>()?;
                b.keys.retain(|s| !remove.contains(s));
                origins.retain(|s| !remove.contains(s));
                for add in &edit.add {
                    let key = Sequence::parse(add, prefix)?;
                    if !b.keys.contains(&key) {
                        b.keys.push(key.clone());
                    }
                    if !origins.contains(&key) {
                        origins.push(key);
                    }
                }
                if let Some(p) = &edit.preferred {
                    b.preferred = Some(Sequence::parse(p, prefix)?);
                }
            }
            if b.preferred.as_ref().is_some_and(|p| !b.keys.contains(p)) {
                diagnostics.push(format!(
                    "Preferred binding unavailable for {} [{}]; using an effective key",
                    command.id, context
                ));
                b.preferred = command
                    .defaults
                    .get(command.preferred)
                    .filter(|p| b.keys.contains(p))
                    .cloned()
                    .or_else(|| b.keys.first().cloned());
            }
            bindings.push(b);
            explicit.push(origins);
        }
    }
    // Compare at actual contexts, so a local user choice never removes a default elsewhere.
    for i in 0..bindings.len() {
        if bindings[i].disabled {
            continue;
        }
        for j in i + 1..bindings.len() {
            if bindings[j].disabled
                || bindings[i].context != bindings[j].context
                || bindings[i].command == bindings[j].command
            {
                continue;
            }
            let shared = bindings[i]
                .keys
                .iter()
                .filter(|s| bindings[j].keys.contains(s))
                .cloned()
                .collect::<Vec<_>>();
            for key in shared {
                let (loser, winner) = match (explicit[i].contains(&key), explicit[j].contains(&key))
                {
                    (true, false) => (j, i),
                    (false, true) => (i, j),
                    _ => bail!(
                        "Internal conflict: {} / {} [{}] on {}",
                        bindings[i].command,
                        bindings[j].command,
                        bindings[i].context,
                        key.display()
                    ),
                };
                diagnostics.push(format!(
                    "Default {} [{}] suppressed by {} on {}",
                    bindings[loser].command,
                    bindings[loser].context,
                    bindings[winner].command,
                    key.display()
                ));
                bindings[loser].keys.retain(|s| s != &key);
            }
        }
    }
    for b in bindings.iter().filter(|b| !b.disabled) {
        for sequence in &b.keys {
            ensure!(
                !(sequence.0.len() == 1 && sequence.0[0] == prefix),
                "Prefix cannot also execute a command"
            );
            ensure!(
                !bindings
                    .iter()
                    .filter(|o| !o.disabled && o.context == b.context)
                    .any(|o| o.keys.iter().any(|s| s.0.len() == 1
                        && sequence.0.len() == 2
                        && s.0[0] == sequence.0[0])),
                "Sequence opener also executes a standalone command"
            );
        }
    }
    Ok(Index {
        commands,
        bindings,
        prefix,
        diagnostics,
    })
}
