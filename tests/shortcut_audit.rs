//! An exact production-key inventory makes new routing branches reviewable in CI.
use std::{collections::BTreeSet, fs, path::Path};
fn collect(root: &Path, output: &mut BTreeSet<String>) {
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, output);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for line in source.lines().filter(|l| l.contains("KeyCode::")) {
            output.insert(format!(
                "{}\t{}",
                path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/"),
                line.trim()
            ));
        }
    }
}
#[test]
fn every_key_branch_has_an_explicit_review_classification() {
    let mut actual = BTreeSet::new();
    collect(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut actual,
    );
    let ledger = include_str!("../docs/shortcut-key-audit.tsv");
    let expected = ledger
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let mut columns = l.splitn(3, '\t');
            let category = columns.next().unwrap();
            assert!(["protocol", "registry", "control", "widget", "fixture"].contains(&category));
            format!("{}\t{}", columns.next().unwrap(), columns.next().unwrap())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Review and classify each new/changed KeyCode branch in docs/shortcut-key-audit.tsv"
    );
}
