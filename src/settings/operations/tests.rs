use super::super::data::UserData;
use super::*;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let f = Self(std::env::temp_dir().join(format!(
            "DevTabs-settings-operations-{}",
            uuid::Uuid::new_v4()
        )));
        fs::create_dir_all(&f.0).unwrap();
        f
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn exports_and_cleanup_respect_storage_owners_and_committed_values() {
    let f = Fixture::new();
    let config = f.0.join("config.toml");
    fs::write(
        &config,
        "# committed\nschema=1\n[appearance]\ntheme='light'",
    )
    .unwrap();
    let mut data = UserData::default();
    data.notes
        .insert(crate::config::key(Path::new("D:/p")), "中文正文".into());
    data.save(&f.0.join("user-data.json")).unwrap();
    fs::create_dir_all(f.0.join("runtime")).unwrap();
    fs::write(f.0.join("runtime/tasks.json"), b"task").unwrap();
    fs::create_dir_all(f.0.join("cache/stats")).unwrap();
    fs::write(f.0.join("cache/stats/rebuildable.json"), b"cache").unwrap();
    fs::write(f.0.join("cache/stats/unknown.bin"), b"retain").unwrap();
    fs::write(f.0.join("ui-state.json"), b"{\"projects\":{}}").unwrap();
    let source = fs::read(&config).unwrap();
    let content = fs::read(f.0.join("user-data.json")).unwrap();
    let export = f.0.join("backups/config.toml");
    execute_at(&f.0, &Action::ExportConfig, &export.to_string_lossy()).unwrap();
    assert_eq!(fs::read(export).unwrap(), source);
    execute_at(&f.0, &Action::ClearStats, "").unwrap();
    execute_at(&f.0, &Action::ResetMemory, "").unwrap();
    assert!(!f.0.join("cache/stats/rebuildable.json").exists());
    assert!(f.0.join("cache/stats/unknown.bin").exists());
    assert_eq!(fs::read(&config).unwrap(), source);
    assert_eq!(fs::read(f.0.join("user-data.json")).unwrap(), content);
    assert_eq!(fs::read(f.0.join("runtime/tasks.json")).unwrap(), b"task");
    for name in [
        "config.toml",
        "CONFIG.TOML",
        "user-data.json",
        "ui-state.json",
        "runtime/tasks.json",
        "cache/stats/file.json",
    ] {
        assert!(
            execute_at(
                &f.0,
                &Action::ExportConfig,
                &f.0.join(name).to_string_lossy()
            )
            .is_err(),
            "{name}"
        );
    }
}
#[test]
fn restore_preview_is_read_only_default_keeps_conflicts_and_commit_checks_its_base() {
    let f = Fixture::new();
    let project = crate::config::key(Path::new("D:/p"));
    let other = crate::config::key(Path::new("D:/q"));
    let mut current = UserData::default();
    current.notes.insert(project.clone(), "current".into());
    current.save(&f.0.join("user-data.json")).unwrap();
    let mut incoming = UserData::default();
    incoming.notes.insert(project.clone(), "archive".into());
    incoming.notes.insert(other.clone(), "new".into());
    let archive = f.0.join("backup.json");
    incoming.save(&archive).unwrap();
    let before = fs::read(f.0.join("user-data.json")).unwrap();
    let plan = inspect_restore(&f.0, &archive.to_string_lossy()).unwrap();
    assert_eq!(plan.added, 1);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(fs::read(f.0.join("user-data.json")).unwrap(), before);
    commit_restore(&f.0, &plan).unwrap();
    let after = UserData::read(&f.0.join("user-data.json")).unwrap();
    assert_eq!(after.notes[&project], "current");
    assert_eq!(after.notes[&other], "new");
    let mut plan = inspect_restore(&f.0, &archive.to_string_lossy()).unwrap();
    plan.use_archive.insert(format!("note:{project}"));
    UserData::note(&f.0, Path::new("D:/p"), "current", "external edit").unwrap();
    let bytes = fs::read(f.0.join("user-data.json")).unwrap();
    assert!(commit_restore(&f.0, &plan).is_err());
    assert_eq!(fs::read(f.0.join("user-data.json")).unwrap(), bytes);
    let mut plan = inspect_restore(&f.0, &archive.to_string_lossy()).unwrap();
    plan.use_archive.insert(format!("note:{project}"));
    commit_restore(&f.0, &plan).unwrap();
    assert_eq!(
        UserData::read(&f.0.join("user-data.json")).unwrap().notes[&project],
        "archive"
    );
}
#[test]
fn corrupt_and_over_capacity_archives_never_change_user_content() {
    let f = Fixture::new();
    let archive = f.0.join("bad.json");
    fs::write(&archive, b"{bad").unwrap();
    assert!(inspect_restore(&f.0, &archive.to_string_lossy()).is_err());
    let value =
        serde_json::json!({"notes":{crate::config::key(Path::new("D:/p")):"x".repeat(2049)}});
    fs::write(&archive, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(inspect_restore(&f.0, &archive.to_string_lossy()).is_err());
    assert!(!f.0.join("user-data.json").exists());
}

#[test]
fn exporting_a_missing_config_is_a_valid_non_mutating_default_document() {
    let f = Fixture::new();
    let dest = f.0.join("export.toml");
    execute_at(&f.0, &Action::ExportConfig, &dest.to_string_lossy()).unwrap();
    assert_eq!(
        super::super::store::Snapshot::parse(fs::read_to_string(dest).unwrap())
            .unwrap()
            .effective
            .schema,
        1
    );
    assert!(!f.0.join("config.toml").exists());
}
