use devtabs::preview::{self, CodeHighlighter, CodeViewport, MarkdownCache};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicU64},
    time::Instant,
};
fn code() -> String {
    "let value = 42; // 中文\n".repeat(7000)
}
fn changed_code(original: &str) -> String {
    let mut lines: Vec<_> = original.lines().map(str::to_owned).collect();
    lines[6500] = "let edited = 123;".into();
    lines.join("\n") + "\n"
}
fn markdown() -> String {
    (0..500).map(|i|format!("## Section {i}\n\nA **strong** paragraph with [link](https://example.com).\n\n```rust\nlet value = 42;\n```\n\n")).collect()
}
#[test]
fn local_edit_near_a_deep_viewport_does_not_reparse_the_whole_prefix() {
    let mut cache = CodeHighlighter::default();
    let generation = AtomicU64::new(1);
    let original = code();
    let path = Path::new("deep.rs");
    let view = CodeViewport {
        top: 6500,
        rows: 30,
    };
    cache
        .render(
            Arc::from(original.as_str()),
            path,
            view,
            false,
            &generation,
            1,
        )
        .unwrap();
    let before = cache.stats();
    let changed = changed_code(&original);
    let window = cache
        .render(
            Arc::from(changed.as_str()),
            path,
            view,
            false,
            &generation,
            1,
        )
        .unwrap();
    let parsed = cache.stats().parsed_lines - before.parsed_lines;
    assert!(parsed < 300, "local edit reparsed {parsed} lines");
    let reference = preview::highlight(&changed, path, false);
    for (i, line) in window.text.lines.iter().enumerate() {
        assert_eq!(line, &reference.lines[window.first + i]);
    }
}
#[test]
fn markdown_edit_reuses_unchanged_resolved_blocks() {
    let original = markdown();
    let view = CodeViewport { top: 0, rows: 36 };
    let mut cache = MarkdownCache::default();
    cache.render(Arc::from(original.as_str()), 98, false, view, false);
    let before = cache.stats();
    let changed = original.replacen("Section 0", "Changed 0", 1);
    let result = cache.render(Arc::from(changed.as_str()), 98, false, view, false);
    assert!(
        cache.stats().laid_out_blocks < before.laid_out_blocks,
        "unchanged blocks were laid out again"
    );
    let full = preview::render(&changed, Path::new("a.md"), true, 98, false);
    assert_eq!(result.plain.as_slice(), &full.plain[..result.plain.len()]);
    let w = result.window.unwrap();
    for (i, line) in w.text.lines.iter().enumerate() {
        assert_eq!(line, &full.text.lines[w.first + i]);
    }
}

#[test]
fn edits_that_change_lexical_state_or_line_counts_match_full_highlighting() {
    let generation = AtomicU64::new(1);
    let path = Path::new("changes.rs");
    let original = "let value = 42;\n".repeat(1200);
    let mut cache = CodeHighlighter::default();
    cache
        .render(
            Arc::from(original.as_str()),
            path,
            CodeViewport {
                top: 1100,
                rows: 30,
            },
            false,
            &generation,
            1,
        )
        .unwrap();
    let mut versions = vec![original.replacen("let value = 42;", "/* opened", 1)];
    versions.push(format!("/* comment\n\n*/\n{original}"));
    versions.push(original.replacen("let value = 42;\n", "\n", 1));
    versions.push(original.clone());
    for source in versions {
        for top in [0, 450, 1100, 100] {
            let w = cache
                .render(
                    Arc::from(source.as_str()),
                    path,
                    CodeViewport { top, rows: 30 },
                    false,
                    &generation,
                    1,
                )
                .unwrap();
            let full = preview::highlight(&source, path, false);
            for (i, line) in w.text.lines.iter().enumerate() {
                assert_eq!(line, &full.lines[w.first + i], "top={top}");
            }
        }
    }
}

#[test]
fn converged_states_restore_downstream_checkpoints() {
    let generation = AtomicU64::new(1);
    let path = Path::new("stable.rs");
    let original = code();
    let mut cache = CodeHighlighter::default();
    cache
        .render(
            Arc::from(original.as_str()),
            path,
            CodeViewport {
                top: 6800,
                rows: 30,
            },
            false,
            &generation,
            1,
        )
        .unwrap();
    let changed = original.replacen("value", "other", 1);
    let changed: Arc<str> = Arc::from(changed);
    cache
        .render(
            changed.clone(),
            path,
            CodeViewport { top: 0, rows: 150 },
            false,
            &generation,
            1,
        )
        .unwrap();
    assert!(cache.stats().reused_checkpoints > 40);
    let before = cache.stats();
    cache
        .render(
            changed,
            path,
            CodeViewport {
                top: 6800,
                rows: 30,
            },
            false,
            &generation,
            1,
        )
        .unwrap();
    assert!(cache.stats().parsed_lines - before.parsed_lines < 300);
}

#[test]
fn markdown_reference_changes_and_shifted_ranges_invalidate_stale_blocks() {
    let original = "# Title\n\n[link][ref]\n\n$x^2$ &amp;\n\n[ref]: https://old.example\n";
    let mut cache = MarkdownCache::default();
    let view = CodeViewport { top: 0, rows: 1000 };
    for source in [
        original.to_owned(),
        original.replace("old.example", "new.example"),
        format!("new paragraph\n\n{original}"),
        original.replace("x^2", "x^3"),
    ] {
        let r = cache.render(Arc::from(source.as_str()), 40, false, view, true);
        let full = preview::render(&source, Path::new("a.md"), true, 40, false);
        assert_eq!(r.plain, full.plain);
        assert_eq!(r.window.unwrap().text, *full.text);
    }
}

#[test]
fn sidebar_editor_applies_incremental_colors_to_the_current_revision() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use devtabs::sidebar::{Focus, Sidebar};
    use ratatui::layout::Rect;
    let root = std::env::temp_dir().join(format!("DevTabs-incremental-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("editor.rs");
    std::fs::write(&path, code()).unwrap();
    let mut s = Sidebar::new(root.clone());
    s.visible = false;
    s.open(path, true).unwrap();
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    let wait = |s: &mut Sidebar, editor: bool| {
        let start = Instant::now();
        loop {
            s.tick(true);
            if s.document.as_ref().is_some_and(|d| {
                if editor {
                    d.editor.as_ref().is_some_and(|e| {
                        e.highlight.as_ref().is_some_and(|(r, _)| *r == e.revision)
                    })
                } else {
                    d.rendered.is_some()
                }
            }) {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(20));
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    wait(&mut s, false);
    s.focus = Focus::Preview;
    s.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    wait(&mut s, true);
    s.document
        .as_mut()
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .area
        .move_cursor(ratatui_textarea::CursorMove::Jump(6500, 0));
    // TextArea establishes its final viewport during drawing.
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
    terminal.draw(|f| s.draw(f)).unwrap();
    let start = Instant::now();
    loop {
        s.tick(true);
        if s.document
            .as_ref()
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .highlight
            .as_ref()
            .unwrap()
            .1
            .line(6500)
            .is_some()
        {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(20));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    s.paste("/*").unwrap();
    wait(&mut s, true);
    let e = s.document.as_ref().unwrap().editor.as_ref().unwrap();
    let (revision, w) = e.highlight.as_ref().unwrap();
    assert_eq!(*revision, e.revision);
    let full = preview::highlight(&e.source(), Path::new("editor.rs"), false);
    for (i, line) in w.text.lines.iter().enumerate() {
        assert_eq!(line, &full.lines[w.first + i]);
    }
    drop(s);
    let _ = std::fs::remove_dir_all(root);
}
#[test]
#[ignore = "isolated Release incremental edit comparison"]
fn incremental_edit_workload() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    std::hint::black_box(preview::highlight(
        "let x = 42;",
        Path::new("warm.rs"),
        false,
    ));
    let original = code();
    let changed = changed_code(&original);
    let generation = AtomicU64::new(1);
    let path = Path::new("deep.rs");
    let view = CodeViewport {
        top: 6500,
        rows: 30,
    };
    let md = markdown();
    let edited_md = md.replacen("Section 0", "Changed 0", 1);
    let mut cases = Vec::new();
    for _ in 0..5 {
        let mut cache = CodeHighlighter::default();
        cache
            .render(
                Arc::from(original.as_str()),
                path,
                view,
                false,
                &generation,
                1,
            )
            .unwrap();
        let before = cache.stats();
        let start = Instant::now();
        std::hint::black_box(
            cache
                .render(
                    Arc::from(changed.as_str()),
                    path,
                    view,
                    false,
                    &generation,
                    1,
                )
                .unwrap(),
        );
        let code_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut markdown = MarkdownCache::default();
        markdown.render(
            Arc::from(md.as_str()),
            98,
            false,
            CodeViewport { top: 0, rows: 36 },
            false,
        );
        let start = Instant::now();
        std::hint::black_box(markdown.render(
            Arc::from(edited_md.as_str()),
            98,
            false,
            CodeViewport { top: 0, rows: 36 },
            false,
        ));
        let markdown_ms = start.elapsed().as_secs_f64() * 1000.0;
        cases.push(serde_json::json!({"code_ms":code_ms,"code_parsed_lines":cache.stats().parsed_lines-before.parsed_lines,"code_stats":cache.stats(),"markdown_ms":markdown_ms,"markdown_stats":markdown.stats()}));
    }
    std::fs::write(std::env::var_os("DEVTABS_INCREMENTAL_REPORT").expect("report path"),serde_json::to_vec_pretty(&serde_json::json!({"code_bytes":original.len(),"code_lines":7000,"edited_row":6500,"viewport_rows":30,"markdown_bytes":md.len(),"cases":cases,"scope":"Direct production worker renderer calls; excludes UI event/frame/debounce and source snapshot construction"})).unwrap()).unwrap();
}
