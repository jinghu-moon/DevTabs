//! Fixed, source-reviewed baseline cases from 61fecb7; never ship the old router.
//! These cover common key/focus states, not every possible UI state.
use devtabs::shortcuts::{Chord, Index, Resolution};
use std::fs;

fn result(value: Resolution) -> String {
    match value {
        Resolution::Execute(id) => format!("Execute:{id}"),
        other => format!("{other:?}"),
    }
}

#[test]
fn reviewed_before_after_cases_generate_reproducible_behavior_and_default_inventory_reports() {
    let index = Index::build(&Default::default()).unwrap();
    let mut cases = vec![];
    // Baseline overview.rs matched these Fn codes without modifier guards.
    for (key, command) in [
        ("F3", "overview.note.edit"),
        ("F4", "overview.project.settings"),
    ] {
        for mask in 0..8 {
            let chord = format!(
                "{}{}{}{key}",
                if mask & 1 != 0 { "Ctrl+" } else { "" },
                if mask & 2 != 0 { "Alt+" } else { "" },
                if mask & 4 != 0 { "Shift+" } else { "" }
            );
            let expected = if mask == 0 || (key == "F3" && mask == 1) {
                format!("Execute:{command}")
            } else {
                "TextInput".into()
            };
            cases.push((
                "overview.projects",
                chord,
                format!("Execute:{command}"),
                expected,
                "Fn 修饰键精确匹配；F3 保留显式 Ctrl 别名",
            ));
        }
    }
    // Baseline App::key used contains(CONTROL) for the prefix, and a sidebar
    // Ctrl+P exception before the terminal route (sidebar present, no modal).
    for (chord, before, expected, reason) in [
        ("Ctrl+G", "AwaitSequence", "AwaitSequence", "保留精确前缀"),
        (
            "Ctrl+Alt+G",
            "AwaitSequence",
            "ForwardTerminal",
            "额外 Alt 不再意外进入前缀",
        ),
        (
            "Ctrl+P",
            "Execute:panel.quick_open",
            "ForwardTerminal",
            "普通终端保留 CLI 历史导航；文件区域仍可快速打开",
        ),
    ] {
        cases.push((
            "terminal.input",
            chord.into(),
            before.into(),
            expected.into(),
            reason,
        ));
    }
    let mut before_rows = vec![];
    let mut after_rows = vec![];
    let mut changes = String::from(
        "# 快捷键固定场景行为差异\n\n旧结果由 61fecb7 的 overview.rs / app.rs 分支核对后固定；本报告自动运行当前正式 resolver，不声称本轮重跑旧构建或穷尽所有模态。\n\n| 区域 | 键 | 旧结果 | 新结果 | 原因/替代操作 |\n| --- | --- | --- | --- | --- |\n",
    );
    for (context, chord, before, expected, reason) in cases {
        let after = result(index.resolve(context, &[Chord::parse(&chord).unwrap()]));
        assert_eq!(after, expected, "{context}: {chord}");
        before_rows.push(serde_json::json!({"context":context,"key":chord,"result":before,"evidence":"61fecb7, source-reviewed"}));
        after_rows.push(serde_json::json!({"context":context,"key":chord,"result":after}));
        if before != after {
            changes.push_str(&format!(
                "| {context} | {chord} | {before} | {after} | {reason} |\n"
            ));
        }
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/shortcuts");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("keymap-before.json"),
        serde_json::to_vec_pretty(&before_rows).unwrap(),
    )
    .unwrap();
    fs::write(
        output.join("keymap-after.json"),
        serde_json::to_vec_pretty(&after_rows).unwrap(),
    )
    .unwrap();
    fs::write(output.join("behavior-changes.md"), changes).unwrap();
    let mut defaults = String::from("command\tcontext\tkeys\tpreferred\n");
    for command in &index.commands {
        defaults.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            command.id,
            command.context,
            command
                .defaults
                .iter()
                .map(|s| s.display())
                .collect::<Vec<_>>()
                .join(" / "),
            index.preferred(command.id, command.context)
        ));
    }
    fs::write(output.join("default-bindings.tsv"), defaults).unwrap();
    assert_eq!(index.terminal_owned(), ["Ctrl+G"]);
    fs::write(
        output.join("terminal-owned.json"),
        serde_json::to_vec_pretty(&index.terminal_owned()).unwrap(),
    )
    .unwrap();
}
