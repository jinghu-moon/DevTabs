//! Inventory production paths through the Rust syntax tree, including aliases and macros.
use std::{collections::BTreeSet, fs, path::Path};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
};
fn test_attributes(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.meta
                    .require_list()
                    .is_ok_and(|l| l.tokens.to_string() == "test"))
    })
}
struct Scanner<'a> {
    lines: Vec<&'a str>,
    aliases: BTreeSet<String>,
    rows: BTreeSet<String>,
}
impl Scanner<'_> {
    fn record(&mut self, line: usize) {
        if let Some(text) = self.lines.get(line.saturating_sub(1)) {
            self.rows.insert(text.trim().to_owned());
        }
    }
    fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
        let values = tokens.into_iter().collect::<Vec<_>>();
        for (i, t) in values.iter().enumerate() {
            match t {
                proc_macro2::TokenTree::Group(g) => self.tokens(g.stream()),
                proc_macro2::TokenTree::Ident(id)
                    if self.aliases.contains(&id.to_string())
                        && matches!(values.get(i+1),Some(proc_macro2::TokenTree::Punct(p)) if p.as_char()==':') =>
                {
                    self.record(id.span().start().line)
                }
                _ => {}
            }
        }
    }
}
impl<'ast> Visit<'ast> for Scanner<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !test_attributes(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !test_attributes(&node.attrs) {
            visit::visit_item_fn(self, node);
        }
    }
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if !test_attributes(&node.attrs) {
            visit::visit_item_impl(self, node);
        }
    }
    fn visit_path(&mut self, node: &'ast syn::Path) {
        if node.segments.len() > 1
            && node
                .segments
                .iter()
                .any(|p| self.aliases.contains(&p.ident.to_string()))
        {
            self.record(node.span().start().line);
        }
        visit::visit_path(self, node);
    }
    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        // Track a modifier-only edit to a guard even when the key pattern is
        // unchanged on an earlier line. Keep source fingerprints, not offsets.
        let mut pattern = Scanner {
            lines: self.lines.clone(),
            aliases: self.aliases.clone(),
            rows: Default::default(),
        };
        pattern.visit_pat(&node.pat);
        if !pattern.rows.is_empty()
            && let Some((_, guard)) = &node.guard
        {
            let first = node.pat.span().start().line.saturating_sub(1);
            let last = guard.span().end().line.min(self.lines.len());
            self.rows.insert(format!(
                "guard: {}",
                self.lines[first..last]
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        visit::visit_arm(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.tokens(node.tokens.clone());
    }
}
fn inventory(source: &str) -> BTreeSet<String> {
    let syntax = syn::parse_file(source).expect("valid Rust source");
    struct Aliases(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Aliases {
        fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
            if matches!(
                node.ident.to_string().as_str(),
                "KeyCode" | "KeyModifiers" | "KeyEvent"
            ) {
                self.0.insert(node.rename.to_string());
            }
            visit::visit_use_rename(self, node);
        }
    }
    let mut aliases = Aliases(BTreeSet::from([
        "KeyCode".into(),
        "KeyModifiers".into(),
        "KeyEvent".into(),
    ]));
    aliases.visit_file(&syntax);
    let mut scanner = Scanner {
        lines: source.lines().collect(),
        aliases: aliases.0,
        rows: Default::default(),
    };
    scanner.visit_file(&syntax);
    scanner.rows
}
fn rust_files(root: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
fn test_modules(items: &[syn::Item], dir: &Path, excluded: &mut BTreeSet<std::path::PathBuf>) {
    for item in items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let name = module.ident.to_string();
        let base = dir.join(&name);
        if let Some((_, content)) = &module.content {
            if !test_attributes(&module.attrs) {
                test_modules(content, &base, excluded);
            }
        } else if test_attributes(&module.attrs) {
            let explicit = module.attrs.iter().find_map(|a| {
                if a.path().is_ident("path") {
                    match &a.meta {
                        syn::Meta::NameValue(v) => match &v.value {
                            syn::Expr::Lit(v) => match &v.lit {
                                syn::Lit::Str(s) => Some(dir.join(s.value())),
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    }
                } else {
                    None
                }
            });
            excluded.insert(explicit.unwrap_or_else(|| {
                if base.with_extension("rs").is_file() {
                    base.with_extension("rs")
                } else {
                    base.join("mod.rs")
                }
            }));
            // Child files of a test-only module inherit cfg(test).
            excluded.insert(base);
        }
    }
}
fn collect(root: &Path, output: &mut BTreeSet<String>) {
    let mut files = vec![];
    rust_files(root, &mut files);
    let mut excluded = BTreeSet::new();
    for path in &files {
        let source = fs::read_to_string(path).unwrap();
        let syntax = syn::parse_file(&source).expect("valid Rust source");
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let dir = if matches!(stem, "lib" | "main" | "mod") {
            path.parent().unwrap().to_path_buf()
        } else {
            path.with_extension("")
        };
        test_modules(&syntax.items, &dir, &mut excluded);
    }
    for path in files {
        if excluded.iter().any(|p| path == *p || path.starts_with(p)) {
            continue;
        }
        let relative = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        for row in inventory(&fs::read_to_string(&path).unwrap()) {
            output.insert(format!("{relative}\t{row}"));
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
    if let Some(path) = std::env::var_os("DEVTABS_AUDIT_EXPORT") {
        fs::write(path, actual.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();
        return;
    }
    let expected = include_str!("../docs/shortcut-key-audit.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let mut columns = l.splitn(3, '\t');
            let category = columns.next().unwrap();
            assert!(["protocol", "registry", "control", "widget", "fixture"].contains(&category));
            let path = columns.next().unwrap();
            let source = columns.next().unwrap();
            (category != "fixture").then(|| format!("{path}\t{source}"))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Review each new/changed production branch in docs/shortcut-key-audit.tsv"
    );
}
#[test]
fn guard_only_modifier_changes_and_qualified_paths_cannot_escape_audit() {
    let source =
        "fn live() { match key { crossterm::event::KeyCode::F(12)\n if ctrl => run(), _ => () } }";
    assert_ne!(
        inventory(source),
        inventory(&source.replace("if ctrl", "if !ctrl"))
    );
    assert!(inventory(source).iter().any(|s| s.starts_with("guard:")));
    let mods =
        "use crossterm::event::KeyModifiers as M; fn live() { if key.modifiers == M::CONTROL {} }";
    assert!(inventory(mods).iter().any(|s| s.contains("M::CONTROL")));
}
#[test]
fn only_declared_test_modules_are_excluded_regardless_of_filename() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("key-audit-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("fixture")).unwrap();
    fs::write(root.join("main.rs"), "mod tests; #[cfg(test)] mod fixture;").unwrap();
    fs::write(
        root.join("tests.rs"),
        "fn live() { let _ = KeyCode::F(21); }",
    )
    .unwrap();
    fs::write(
        root.join("fixture.rs"),
        "fn fixture() { let _ = KeyCode::F(22); }",
    )
    .unwrap();
    fs::write(
        root.join("fixture/child.rs"),
        "fn fixture_child() { let _ = KeyCode::F(23); }",
    )
    .unwrap();
    let mut rows = BTreeSet::new();
    collect(&root, &mut rows);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows.iter().next().unwrap().contains("KeyCode::F(21)"));
}
#[test]
fn ast_audit_covers_aliases_macros_and_production_after_test_modules() {
    let source = r#"
use crossterm::event::KeyCode as Code;
fn production() { let _ = Code::F(12); }
#[cfg(test)] mod tests { fn fixture() { let _ = KeyCode::F(23); } }
#[test] fn test_only() { let _ = KeyCode::F(22); }
fn after_tests() { let _ = KeyCode::F(24); }
fn macro_use() { matches!(key, Code::F(1)); }
// KeyCode::F(19) is a comment, not a branch.
fn text() { let _ = "KeyCode::F(18)"; }
"#;
    let rows = inventory(source);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| r.contains("after_tests")));
    assert!(rows.iter().any(|r| r.contains("Code::F(12)")));
    assert!(rows.iter().any(|r| r.contains("macro_use")));
    assert!(
        !rows
            .iter()
            .any(|r| r.contains("fixture") || r.contains("test_only"))
    );
    let mut registered = rows.clone();
    registered.retain(|r| !r.contains("after_tests"));
    assert_ne!(registered, rows, "new branch must be detected");
    registered = rows.clone();
    registered.insert("removed production branch".into());
    assert_ne!(registered, rows, "stale inventory must be detected");
}
