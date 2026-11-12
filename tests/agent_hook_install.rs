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

#[test]
fn installed_hooks_have_provider_and_event_display_names() {
    for (provider, title) in [("codex", "Codex"), ("claude", "Claude")] {
        let f = support::Fixture::new();
        let path = f.0.join("hooks.json");
        let exe = f.0.join("devtabs.exe");
        devtabs::hooks::install(&path, provider, &exe, false).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (event, groups) in value["hooks"].as_object().unwrap() {
            assert_eq!(
                groups[0]["hooks"][0]["statusMessage"],
                format!("DevTabs · {title} · {event}")
            );
        }
    }
}
#[test]
fn reinstall_upgrades_owned_unnamed_hooks_without_touching_commands_peers_or_trust() {
    let f = support::Fixture::new();
    let path = f.0.join("hooks.json");
    let owner_path = path.with_extension("devtabs-owner.json");
    let exe = f.0.join("devtabs.exe");
    let trust_path = f.0.join("config.toml");
    let trust = b"[hooks.state.example]\ntrusted_hash = 'keep-exact'\n";
    fs::write(&trust_path, trust).unwrap();
    let other = serde_json::json!({"hooks":[{"type":"command","command":"pebrel","statusMessage":"Pebrel"}]});
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"custom":42,"hooks":{"Stop":[other.clone()]}}))
            .unwrap(),
    )
    .unwrap();
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut owner: serde_json::Value =
        serde_json::from_slice(&fs::read(&owner_path).unwrap()).unwrap();
    for groups in value["hooks"].as_object_mut().unwrap().values_mut() {
        for group in groups.as_array_mut().unwrap() {
            if group != &other {
                group["hooks"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("statusMessage");
            }
        }
    }
    for group in owner["groups"].as_object_mut().unwrap().values_mut() {
        group["hooks"][0]
            .as_object_mut()
            .unwrap()
            .remove("statusMessage");
    }
    let command = value["hooks"]["Stop"][1]["hooks"][0]["command"].clone();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(&owner_path, serde_json::to_vec(&owner).unwrap()).unwrap();
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    let first = fs::read(&path).unwrap();
    let updated: serde_json::Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(updated["custom"], 42);
    assert_eq!(updated["hooks"]["Stop"][0], other);
    assert_eq!(updated["hooks"]["Stop"][1]["hooks"][0]["command"], command);
    assert_eq!(
        updated["hooks"]["Stop"][1]["hooks"][0]["statusMessage"],
        "DevTabs · Codex · Stop"
    );
    assert_eq!(fs::read(&trust_path).unwrap(), trust);
    devtabs::hooks::install(&path, "codex", &exe, false).unwrap();
    assert_eq!(fs::read(&path).unwrap(), first);
    devtabs::hooks::install(&path, "codex", &exe, true).unwrap();
    let removed: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(removed["hooks"]["Stop"], serde_json::json!([other]));
}
