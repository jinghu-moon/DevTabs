#[allow(dead_code)]
mod support;
use std::fs;
#[test]
fn hooks_install_and_remove_are_idempotent_preserve_others_and_protect_external_edits() {
    let f = support::Fixture::new();
    let path = f.0.join("hooks.json");
    let exe = f.0.join("含 空格 '$` devtabs.exe");
    fs::write(
        &path,
        r#"{"custom":42,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#,
    )
    .unwrap();
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    let first = fs::read(&path).unwrap();
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    assert_eq!(fs::read(&path).unwrap(), first);
    let v: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(v["custom"], 42);
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert!(
        v["hooks"]["Stop"][1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .starts_with("pwsh.exe -NoLogo -NoProfile -EncodedCommand ")
    );
    devtabs::hooks::install(&path, "codex", &exe, true).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "other");
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["hooks"]["Stop"][1]["hooks"][0]["command"] = "externally changed".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(devtabs::hooks::install(&path, "codex", &exe, true).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}
