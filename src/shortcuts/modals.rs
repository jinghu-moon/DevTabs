//! Exclusive business commands. Fixed text/navigation/confirmation stays with widgets.
use super::*;
pub(super) fn extend(out: &mut Vec<Command>, prefix: Chord) {
    for (id, context, keys) in [
        ("options.debug", "dialog.options", vec!["Ctrl+D"]),
        ("options.arguments", "dialog.options", vec!["Ctrl+A"]),
        ("options.sessions.edit", "dialog.options", vec!["Ctrl+E"]),
        ("options.sessions.add", "dialog.options", vec!["Alt+N"]),
        ("options.codex", "dialog.options", vec!["1"]),
        ("options.claude", "dialog.options", vec!["2"]),
        ("options.shell", "dialog.options", vec!["3"]),
        ("options.resume", "dialog.options", vec!["R", "Shift+R"]),
        ("options.instance", "dialog.options", vec!["N", "Shift+N"]),
        ("options.tab", "dialog.options", vec!["T", "Shift+T"]),
        ("options.window", "dialog.options", vec!["W", "Shift+W"]),
        ("options.continue", "dialog.options", vec!["C", "Shift+C"]),
        ("options.stage", "dialog.options", vec!["Ctrl+S"]),
        (
            "options.composition",
            "dialog.options",
            vec!["M", "Shift+M"],
        ),
        ("options.layout", "dialog.options", vec!["L", "Shift+L"]),
        ("options.launch", "dialog.options", vec!["Enter"]),
        ("presets.apply", "dialog.presets", vec!["Enter"]),
        ("presets.delete", "dialog.presets", vec!["Delete"]),
        ("presets.rename", "dialog.presets", vec!["F2"]),
        ("queue.edit", "dialog.queue", vec!["Enter"]),
        ("queue.launch", "dialog.queue", vec!["Ctrl+Enter"]),
        ("queue.previous", "dialog.queue", vec!["Alt+Up"]),
        ("queue.next", "dialog.queue", vec!["Alt+Down"]),
        ("queue.delete", "dialog.queue", vec!["Delete"]),
        ("queue.undo", "dialog.queue", vec!["Ctrl+Z"]),
        ("instances.join", "dialog.instances", vec!["Enter"]),
        ("instances.new", "dialog.instances", vec!["N", "Shift+N"]),
        ("history.stage", "dialog.history", vec!["Enter"]),
        (
            "history.resume",
            "dialog.history",
            vec!["Alt+P", "Alt+Shift+P"],
        ),
        (
            "history.fresh",
            "dialog.history",
            vec!["Alt+N", "Alt+Shift+N"],
        ),
        ("attention.navigate", "dialog.attention", vec!["Enter"]),
        ("palette.execute", "dialog.palette", vec!["Enter"]),
        ("batch.retry", "dialog.batch", vec!["R", "Shift+R"]),
    ] {
        out.push(Command {
            id,
            context,
            label: id,
            defaults: keys
                .iter()
                .map(|s| {
                    Sequence::parse(
                        &crate::settings::model::Sequence::Chord((*s).into()),
                        prefix,
                    )
                    .unwrap()
                })
                .collect(),
            preferred: 0,
            repeat: false,
        });
    }
}
