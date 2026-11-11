//! Resource contracts and Release-only cost measurements, without interactive UI.
use devtabs::{
    config::Config,
    i18n::{self, Language},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    time::Instant,
};
fn catalog(lang: Language) -> BTreeMap<String, String> {
    serde_json::from_str(match lang {
        Language::ZhCn => include_str!("../src/i18n/zh-CN.json"),
        _ => include_str!("../src/i18n/en-US.json"),
    })
    .unwrap()
}
fn placeholders(text: &str) -> BTreeSet<String> {
    regex::Regex::new(r"\{([a-z_]+)\}")
        .unwrap()
        .captures_iter(text)
        .map(|c| c[1].to_owned())
        .collect()
}
#[test]
fn catalogs_have_identical_keys_placeholders_and_no_duplicate_json_keys() {
    let en = catalog(Language::EnUs);
    let zh = catalog(Language::ZhCn);
    assert_eq!(en.keys().collect::<Vec<_>>(), zh.keys().collect::<Vec<_>>());
    for (key, text) in &en {
        assert!(!text.is_empty(), "empty English {key}");
        assert!(!zh[key].is_empty(), "empty Chinese {key}");
        assert_eq!(placeholders(text), placeholders(&zh[key]), "{key}");
    }
    // A map silently replaces duplicate JSON keys: count the source properties too.
    for (source, catalog) in [
        (include_str!("../src/i18n/en-US.json"), &en),
        (include_str!("../src/i18n/zh-CN.json"), &zh),
    ] {
        assert_eq!(
            source.lines().filter(|l| l.starts_with("  \"")).count(),
            catalog.len()
        );
    }
}
#[test]
fn named_interpolation_keeps_arguments_literal_and_has_explicit_fallback() {
    for lang in [Language::EnUs, Language::ZhCn] {
        let text = i18n::format_in(
            lang,
            "launch_result",
            &[
                ("done", "%{failed}中文".into()),
                ("total", "10".into()),
                ("failed", "2".into()),
            ],
        );
        assert!(text.contains("%{failed}中文/10"));
        assert!(text.contains('2'));
        assert_eq!(
            i18n::in_language(lang, "missing.future.key"),
            "missing.future.key"
        );
    }
    assert_eq!(
        i18n::format_in(Language::EnUs, "counts", &[("files", "1".into())]),
        "1 files · %{bytes} bytes"
    );
}
#[test]
fn library_catalogue_matches_every_resource_and_borrows_static_messages() {
    for lang in [Language::EnUs, Language::ZhCn] {
        for (key, value) in catalog(lang) {
            let translated = i18n::in_language(lang, &key);
            assert_eq!(translated, value, "{key}");
            assert!(
                matches!(translated, std::borrow::Cow::Borrowed(_)),
                "static resource lookup allocated: {key}"
            );
        }
    }
}
#[test]
fn configuration_roundtrip_preserves_language_and_existing_project_fields() {
    let old = r#"default_agent = "codex"
[projects.'d:\project']
favorite = true
last_opened = 123
"#;
    let mut config: Config = toml::from_str(old).unwrap();
    assert_eq!(config.language, Language::Auto);
    config.language = Language::ZhCn;
    config.projects.get_mut(r"d:\project").unwrap().note = "继续实现 {bytes}".into();
    let encoded = toml::to_string_pretty(&config).unwrap();
    let decoded: Config = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.language, Language::ZhCn);
    assert!(decoded.projects[r"d:\project"].favorite);
    assert_eq!(decoded.projects[r"d:\project"].note, "继续实现 {bytes}");
}
#[test]
fn runtime_switch_uses_the_library_locale_and_keeps_auto_preference() {
    struct Restore(Language);
    impl Drop for Restore {
        fn drop(&mut self) {
            i18n::set(self.0);
        }
    }
    let _restore = Restore(i18n::preference());
    for (lang, expected) in [(Language::ZhCn, "项目"), (Language::EnUs, "Projects")] {
        i18n::set(lang);
        assert_eq!(i18n::preference(), lang);
        assert_eq!(i18n::language(), lang);
        assert_eq!(&*rust_i18n::locale(), lang.locale());
        assert_eq!(i18n::t("projects"), expected);
    }
    i18n::set(Language::Auto);
    assert_eq!(i18n::preference(), Language::Auto);
    assert_eq!(i18n::language(), Language::Auto.resolved());
}
#[test]
fn every_explicit_ui_key_is_catalogued() {
    let en = catalog(Language::EnUs);
    let pattern = regex::Regex::new(
        r#"(?:crate::i18n::(?:t|format)|(?-u:\b(?:t|tf)))\(\s*("(?:\\.|[^"\\])*")"#,
    )
    .unwrap();
    let files = [
        include_str!("../src/ui.rs"),
        include_str!("../src/app.rs"),
        include_str!("../src/sidebar.rs"),
        include_str!("../src/sidebar/controls.rs"),
        include_str!("../src/sidebar/features.rs"),
        include_str!("../src/sidebar/reading.rs"),
        include_str!("../src/sidebar/image.rs"),
        include_str!("../src/sidebar/explorer.rs"),
        include_str!("../src/files.rs"),
        include_str!("../src/explorer.rs"),
        include_str!("../src/overview.rs"),
        include_str!("../src/overview/view.rs"),
        include_str!("../src/appearance.rs"),
        include_str!("../src/editor.rs"),
    ];
    for source in files {
        for found in pattern.captures_iter(source.split("#[cfg(test)]").next().unwrap()) {
            let key: String = serde_json::from_str(&found[1]).unwrap();
            assert!(en.contains_key(&key), "uncatalogued UI key: {key:?}");
        }
    }
}
#[test]
#[ignore = "Release-only I18N lookup/format costs; no terminal or user data is accessed"]
fn i18n_lookup_cost() {
    if cfg!(debug_assertions) {
        panic!("Run this benchmark with --release");
    }
    let en = catalog(Language::EnUs);
    let zh = catalog(Language::ZhCn);
    let start = Instant::now();
    black_box(i18n::in_language(Language::EnUs, "projects"));
    black_box(i18n::in_language(Language::ZhCn, "projects"));
    let init = start.elapsed().as_secs_f64() * 1e6;
    let keys = [
        "projects",
        "settings",
        "history",
        "statistics",
        "language",
        "counts",
        "launch_result",
        "note",
    ];
    let runs = 500_000usize;
    let lookup = |lang| {
        let mut times = vec![];
        for _ in 0..5 {
            let start = Instant::now();
            for n in 0..runs {
                black_box(i18n::in_language(
                    black_box(lang),
                    black_box(keys[n % keys.len()]),
                ));
            }
            times.push(start.elapsed().as_nanos() as f64 / runs as f64)
        }
        times.sort_by(f64::total_cmp);
        times[2]
    };
    let args = [("files", "1234".into()), ("bytes", "9876543".into())];
    i18n::set(Language::ZhCn);
    let start = Instant::now();
    for n in 0..runs {
        black_box(i18n::t(black_box(keys[n % keys.len()])));
    }
    let current_locale = start.elapsed().as_nanos() as f64 / runs as f64;
    let start = Instant::now();
    for _ in 0..100_000 {
        black_box(i18n::format_in(
            Language::ZhCn,
            black_box("counts"),
            black_box(&args),
        ));
    }
    let formatting = start.elapsed().as_nanos() as f64 / 100_000.;
    let start = Instant::now();
    for _ in 0..100_000 {
        black_box(format!(
            "{} files · {} bytes",
            black_box(1234),
            black_box(9876543)
        ));
    }
    let baseline = start.elapsed().as_nanos() as f64 / 100_000.;
    let report = serde_json::json!({"catalog_entries":en.len(),"chinese_entries":zh.len(),"library_first_lookup_us":init,"english_lookup_ns":lookup(Language::EnUs),"chinese_lookup_ns":lookup(Language::ZhCn),"current_locale_lookup_ns":current_locale,"named_format_ns":formatting,"rust_format_baseline_ns":baseline,"resource_bytes":include_bytes!("../src/i18n/en-US.json").len()+include_bytes!("../src/i18n/zh-CN.json").len()});
    println!("{report}");
    if let Some(path) = std::env::var_os("DEVTABS_I18N_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap()
    }
    // A small per-frame budget: 500 catalog lookups should fit well within 1 ms.
    assert!(report["chinese_lookup_ns"].as_f64().unwrap() < 2000.0);
}
