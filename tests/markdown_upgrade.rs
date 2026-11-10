use devtabs::preview;
use ratatui::style::Color;
use std::{collections::HashSet, path::Path};
use unicode_width::UnicodeWidthStr;

fn md(source: &str, width: u16) -> preview::Rendered {
    preview::render(source, Path::new("test.md"), true, width, false)
}

#[test]
fn prose_wraps_at_words_and_list_continuations_align() {
    let r = md("alpha beta gamma delta\n\n- alpha beta gamma delta", 12);
    assert_eq!(r.plain[0], "alpha beta");
    assert!(
        r.plain
            .iter()
            .any(|s| s == "  gamma" || s == "  gamma delta")
    );
}

#[test]
fn narrow_table_changes_to_readable_records() {
    let r = md(
        "| File | Purpose | Status |\n|---|---|---|\n| Cargo.toml | Dependencies and build | Changed |",
        20,
    );
    assert!(r.plain.iter().any(|s| s.contains("File:")), "{:?}", r.plain);
    assert!(r.plain.iter().any(|s| s.contains("Status:")));
    assert!(r.plain.iter().all(|s| s.width() <= 20));
}

#[test]
fn fence_metadata_uses_shared_rust_highlighter() {
    let r = md("```rust,no_run\nfn main() { let value = 42; }\n```", 80);
    let colors: HashSet<_> = r
        .text
        .lines
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| s.style.fg)
        .collect();
    assert!(colors.len() >= 3, "{colors:?}");
}

#[test]
fn mermaid_and_math_render_with_readable_source_fallbacks() {
    let r = md(
        "```mermaid\nflowchart TD\nA[Start] --> B[Done]\n```\n\n$$\\frac{1}{2}$$\n\n$E=mc^2$",
        100,
    );
    let text = r.plain.join("\n");
    assert!(
        text.contains('┌') && !text.contains("flowchart TD"),
        "{text}"
    );
    assert!(text.contains("E=mc²"));
    assert!(text.lines().any(|s| s.trim() == "─"));
    let fallback = md("```mermaid\npie\n\"A\": 1\n```\n\n$\\unknown{x}$", 40)
        .plain
        .join("\n");
    assert!(fallback.contains("pie") && fallback.contains("\\unknown{x}"));
}

#[test]
fn oversized_code_line_skips_expensive_highlighting() {
    let source = format!("let value = \"{}\";", "x".repeat(5000));
    let r = preview::render(&source, Path::new("test.rs"), false, 80, false);
    assert!(
        r.text
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset))
    );
    assert!(r.note.contains("budget"));
}

#[test]
fn source_ranges_survive_wrapping_entities_code_and_visual_transforms() {
    let source = "alpha beta gamma delta\n\n- item with &amp; entity\n\n```rust\nfn main() {}\n```\n\n$E=mc^2$\n\n```mermaid\nflowchart TD\nA[Start] --> B[Done]\n```";
    let r = md(source, 40);
    assert_eq!(r.rows.len(), r.plain.len());
    for (i, row) in r.rows.iter().enumerate() {
        for segment in &row.segments {
            assert!(source.get(segment.source.clone()).is_some());
            assert!(segment.display.end <= r.plain[i].chars().count());
        }
    }
    let entity = r.plain.iter().position(|l| l.contains('&')).unwrap();
    let col = r.plain[entity].chars().position(|c| c == '&').unwrap();
    assert_eq!(r.selected_source((entity, col), (entity, col + 1)), "&amp;");
    let math = r.plain.iter().position(|l| l.contains("mc²")).unwrap();
    assert_eq!(
        r.selected_source((math, 0), (math, r.plain[math].chars().count())),
        "$E=mc^2$"
    );
    let diagram = r
        .rows
        .iter()
        .position(|row| row.kind == preview::RowKind::Diagram)
        .unwrap();
    assert!(
        r.selected_source((diagram, 0), (diagram, 40))
            .contains("```mermaid")
    );
    let wrapped = md("alpha beta gamma delta", 12);
    assert_eq!(
        wrapped.selected_source((0, 0), (1, 11)),
        "alpha beta gamma delta"
    );
    let list = md("- alpha beta gamma delta", 12);
    assert_eq!(
        list.selected_source((0, 0), (list.plain.len() - 1, 40)),
        "alpha beta gamma delta"
    );
}

#[test]
fn unicode_layout_and_source_coordinates_stay_valid_at_all_widths() {
    let sources = [
        "## 中文标题👩‍💻e\u{301}\n\n> 中文引文 **加粗** 与英文 words 👩‍💻👩‍💻\n\n- [x] 中文任务和后续说明\n  - nested 中文 task",
        "| 文件 | 说明 | 状态 |\n|---|:---:|---:|\n| src/main.rs | 中文👩‍💻长说明 | 已改 |",
        "[label](https://example.com) https://example.com/a/very/long/path\n\n$E=mc^2$\n\n$$\\frac{longnumerator}{denominator}$$",
    ];
    for width in [1, 2, 3, 5, 20, 40, 80, 120] {
        for source in sources {
            let r = md(source, width);
            assert_eq!(r.rows.len(), r.plain.len());
            for (i, text) in r.plain.iter().enumerate() {
                assert!(text.width() <= usize::from(width), "width={width} {text:?}");
                for segment in &r.rows[i].segments {
                    assert!(source.get(segment.source.clone()).is_some());
                    assert!(
                        segment.display.end <= text.chars().count(),
                        "{segment:?} {text:?}"
                    );
                }
            }
        }
    }
    let emoji = md("👩‍💻👩‍💻👩‍💻", 3);
    assert!(emoji.plain.iter().all(|s| s == "👩‍💻"), "{:?}", emoji.plain);
}

#[test]
fn math_recognition_protects_code_money_links_and_unknown_commands() {
    let r = md(
        "Cost $20 and $30. `E=mc^2 $x$`\n\n[price $x$](https://example.com/$x$)\n\n\\(E=mc^2\\)\n\n$\\unknown{x}$\n\n```text\n$E=mc^2$\n```",
        120,
    );
    let text = r.plain.join("\n");
    for expected in ["$20", "$30", "$x$", "E=mc²", "$\\unknown{x}$", "$E=mc^2$"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let raw = "$$\\frac{1111111111}{2222222222}$$";
    let narrow = md(raw, 8);
    assert!(narrow.plain.join("").contains("\\frac"));
    assert_eq!(
        narrow.selected_source((0, 0), (narrow.plain.len() - 1, 80)),
        raw
    );
}

#[test]
fn incomplete_unsupported_and_overwide_mermaid_retain_source() {
    for source in [
        "```mermaid\nflowchart TD\nA-->B",
        "```mermaid\npie\n\"a\": 1\n```",
    ] {
        let r = md(source, 80);
        assert!(!r.rows.iter().any(|r| r.kind == preview::RowKind::Diagram));
        assert!(r.plain.join("\n").contains(if source.contains("pie") {
            "pie"
        } else {
            "flowchart TD"
        }));
    }
    let r = md("```mermaid\nflowchart TD\nA[Start] --> B[Done]\n```", 3);
    assert!(!r.rows.iter().any(|r| r.kind == preview::RowKind::Diagram));
    assert!(r.note.contains("source"));
    let literal = md(
        "````markdown\n```mermaid\nflowchart TD\nA-->B\n```\n````",
        80,
    );
    assert!(literal.plain.join("\n").contains("```mermaid"));
}

#[test]
fn table_cells_preserve_styles_alignment_and_source_selection() {
    for width in [20, 80] {
        let source = "| Name | Value |\n|:---|---:|\n| **Alpha** | `42` |\n| Beta | 7 |";
        let r = md(source, width);
        let row = r.plain.iter().position(|s| s.contains("Alpha")).unwrap();
        let col = r.plain[row].chars().position(|c| c == 'A').unwrap();
        assert_eq!(r.selected_source((row, col), (row, col + 5)), "Alpha");
        assert!(r.text.lines[row].spans.iter().any(|s| {
            s.content.contains("Alpha")
                && s.style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
        }));
    }
}

#[test]
fn code_keeps_long_lines_multiline_syntax_and_shared_light_dark_theme() {
    let code = "/* comment\ncontinued */\nfn main() { let x = 42; }";
    for light in [false, true] {
        let r = preview::render(
            &format!("```rust\n{code}\n```"),
            Path::new("t.md"),
            true,
            10,
            light,
        );
        assert!(r.plain.iter().any(|s| s == "fn main() { let x = 42; }"));
        let direct = preview::highlight(code, Path::new("main.rs"), light);
        let rendered = r
            .text
            .lines
            .iter()
            .filter(|l| !l.to_string().is_empty())
            .collect::<Vec<_>>();
        assert_eq!(rendered.len(), direct.lines.len());
        for (a, b) in rendered.iter().zip(&direct.lines) {
            assert_eq!(
                a.spans
                    .iter()
                    .map(|s| (&s.content, s.style.fg))
                    .collect::<Vec<_>>(),
                b.spans
                    .iter()
                    .map(|s| (&s.content, s.style.fg))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
#[ignore = "release rendering workload; set DEVTABS_MARKDOWN_BENCH to output JSON"]
fn markdown_render_workload() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let prose = "## 中文标题\n\n- Read **important** details with [documentation](https://example.com/docs).\n\n> A paragraph with 中文 and emoji 👩‍💻 that wraps across rows.\n\n".repeat(100);
    let table = format!(
        "| File | Purpose | Status |\n|---|---|---|\n{}",
        "| src/main.rs | A longer explanation containing 中文文字 | Changed |\n".repeat(100)
    );
    let code = format!(
        "```rust\n{}\n```",
        "fn main() { let value = 42; println!(\"hello\"); }\n".repeat(200)
    );
    let mut cases = Vec::new();
    for (name, source) in [("prose", prose), ("table", table), ("code", code)] {
        for width in [20, 40, 80, 120] {
            let mut timings = Vec::new();
            let mut rows = 0;
            for _ in 0..12 {
                let started = std::time::Instant::now();
                let r = md(&source, width);
                rows = r.plain.len();
                std::hint::black_box(r);
                timings.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            cases.push(serde_json::json!({"name":name,"width":width,"rows":rows,"cold_ms":timings[0],"warm_mean_ms":timings[1..].iter().sum::<f64>()/11.0,"max_ms":timings.iter().copied().fold(0.0,f64::max)}));
        }
    }
    let path = std::env::var_os("DEVTABS_MARKDOWN_BENCH").expect("output path");
    std::fs::write(path, serde_json::to_vec_pretty(&cases).unwrap()).unwrap();
    devtabs::perf::flush().unwrap();
}

#[test]
fn sidebar_resize_theme_and_unsaved_preview_use_current_render_state() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use devtabs::sidebar::{Command, Menu, Sidebar};
    use ratatui::layout::Rect;
    let root = std::env::temp_dir().join(format!("DevTabs-markdown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("preview.md");
    std::fs::write(
        &path,
        "## Preview heading\n\n- alpha beta gamma delta\n\n$E=mc^2$",
    )
    .unwrap();
    let mut s = Sidebar::new(root.clone());
    s.visible = false;
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    s.open(path.clone(), true).unwrap();
    fn ready(s: &mut Sidebar) {
        let start = std::time::Instant::now();
        loop {
            s.tick(true);
            let width = s
                .geometry
                .preview
                .map(devtabs::ui::content)
                .map_or(80, |r| r.width);
            if s.document.as_ref().is_some_and(|d| {
                !d.loading
                    && d.rendered.as_ref().is_some_and(|r| {
                        r.width == width
                            && r.light == s.files.light
                            && r.markdown == (d.markdown && !d.editing)
                    })
            }) {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(15));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    ready(&mut s);
    {
        let d = s.document.as_ref().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(
                &d.loaded.as_ref().unwrap().source,
                &d.rendered.as_ref().unwrap().source
            ),
            "read-only render must share the loaded source allocation"
        );
    }
    s.document.as_mut().unwrap().selection = Some(((0, 0), (0, 4)));
    s.update_layout(Rect::new(0, 0, 160, 40), false);
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().selection.is_none());
    s.menu = Some(Menu {
        items: vec![("Theme", Command::Theme)],
        selected: 0,
        target: path.clone(),
    });
    s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    ready(&mut s);
    assert!(
        s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .light
    );
    s.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    let start = std::time::Instant::now();
    while s.document.as_ref().unwrap().editor.is_none() {
        s.tick(true);
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    s.document
        .as_mut()
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .paste("UNSAVED ")
        .unwrap();
    s.menu = Some(Menu {
        items: vec![("Preview", Command::Project)],
        selected: 0,
        target: path.clone(),
    });
    s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    ready(&mut s);
    let d = s.document.as_ref().unwrap();
    let r = d.rendered.as_ref().unwrap();
    assert!(r.source.contains("UNSAVED"));
    assert!(r.plain.join("\n").contains("UNSAVED"));
    assert!(
        d.editor.as_ref().unwrap().highlight.is_none(),
        "Markdown display rows must never become editor source colors"
    );
    assert!(!std::fs::read_to_string(path).unwrap().contains("UNSAVED"));
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn render_fixture_to_real_ratatui_buffers() {
    use ratatui::{
        Terminal,
        backend::TestBackend,
        style::{Color, Style},
        widgets::{Block, Paragraph},
    };
    let source = include_str!("fixtures/markdown-reading.md");
    let mut snapshots = Vec::new();
    for light in [false, true] {
        for width in [20, 40, 80, 120] {
            let r = preview::render(source, Path::new("reading.md"), true, width, light);
            let height = r.plain.len().min(240) as u16;
            let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
            t.draw(|f| {
                f.render_widget(
                    Block::default().style(
                        Style::new()
                            .fg(if light { Color::Black } else { Color::White })
                            .bg(if light {
                                Color::White
                            } else {
                                Color::Rgb(20, 22, 28)
                            }),
                    ),
                    f.area(),
                );
                for (y, line) in r.text.lines.iter().take(usize::from(height)).enumerate() {
                    f.render_widget(
                        Paragraph::new(line.clone()),
                        ratatui::layout::Rect::new(0, y as u16, width, 1),
                    );
                }
            })
            .unwrap();
            let b = t.backend().buffer();
            assert!(
                b.content
                    .iter()
                    .any(|c| c.modifier.contains(ratatui::style::Modifier::BOLD))
            );
            snapshots.push(serde_json::json!({"light":light,"width":width,"rows":r.plain,"cells":b.content.iter().map(|c|serde_json::json!({"symbol":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),"modifiers":format!("{:?}",c.modifier)})).collect::<Vec<_>>()}));
        }
    }
    if let Some(path) = std::env::var_os("DEVTABS_MARKDOWN_SNAPSHOTS") {
        std::fs::write(path, serde_json::to_vec(&snapshots).unwrap()).unwrap();
    }
}

#[test]
fn layout_budget_bounds_expansion_and_sanitizes_terminal_controls() {
    let source = "中".repeat(40_000);
    let r = md(&source, 2);
    assert!(r.plain.len() <= 30_000);
    assert!(r.note.contains("budget"));
    let r = md(
        "# title\n\ntext \x1b]52;c;payload\x07\n\n```text\n\x1b[31m control\n```",
        80,
    );
    assert!(!r.plain.join("\n").contains(['\x1b', '\x07']));
    let huge = "x".repeat(513 * 1024);
    let r = preview::render(&huge, Path::new("test.rs"), false, 80, false);
    assert!(r.note.contains("budget"));
}

#[test]
fn tab_expansion_retains_precise_code_token_source_ranges() {
    let r = md("```rust\n\tlet value = 42;\n```", 80);
    let row = r.plain.iter().position(|s| s.contains("let")).unwrap();
    assert_eq!(r.selected_source((row, 4), (row, 7)), "let");
    assert_eq!(r.selected_source((row, 0), (row, 4)), "\t");
    let code = preview::render("\tlet value = 42;", Path::new("code.rs"), false, 80, false);
    assert_eq!(code.selected_source((0, 4), (0, 7)), "let");
}
