use devtabs::{preview, sidebar::Sidebar};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("DevTabs-md-viewport-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn source() -> String {
    let mut text = String::new();
    for i in 0..500 {
        text.push_str(&format!("## Section {i}\n\nA **strong** paragraph with 中文 and [link](https://example.com).\n\n```rust\nlet value = 42; // 中文\nlet other = value + 1;\n```\n\n"));
    }
    text.push_str("UniqueEndMarker\n");
    text
}
fn opened(root: &Fixture) -> Sidebar {
    let path = root.0.join("large.md");
    fs::write(&path, source()).unwrap();
    let mut s = Sidebar::new(root.0.clone());
    s.visible = false;
    s.open(path, true).unwrap();
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    ready(&mut s);
    s
}
fn ready(s: &mut Sidebar) {
    let start = Instant::now();
    loop {
        s.tick(true);
        if s.document
            .as_ref()
            .is_some_and(|d| !d.loading && d.rendered().as_ref().is_some_and(|r| r.markdown))
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn first_screen_does_not_layout_the_whole_markdown_document() {
    let root = Fixture::new();
    let s = opened(&root);
    let r = s.document.as_ref().unwrap().rendered().unwrap();
    assert!(
        r.text.lines.len() < 200,
        "first screen laid out {} rows",
        r.text.lines.len()
    );
    assert!(r.plain.join("\n").contains("Section 0"));
}

#[test]
fn indexed_blocks_match_complete_renderer_and_preserve_source_coordinates() {
    let samples = [
        "> - inline $x^2$ and $$\\frac{a}{b}$$\n> - [reference][global]\n\n[global]: https://example.com/$x$\n",
        "",
        "```rust\n```\n",
        "```\n```\n",
        "#\n\ntext\n\n##\n",
        "before\n\n<div>raw</div>\n\nafter\n",
        "# Heading\n\n[reference][global]\n\n> quoted **text**\n>\n> - nested\n> - item\n\n1. one\n2. two\n\n[global]: https://example.com\n",
        "## Math\n\n$$\\frac{a}{b}$$\n\nInline $x^2$ and &amp;.\n\n| A | B |\n|---|---|\n| 中文 | long text |\n\n```rust\n/* comment\ncontinued */\nlet x = 42;\n```\n\n```mermaid\ngraph LR\nA-->B\n```\n",
        "- [x] first\n- [ ] second\n\n---\n\n![alt](image.png)\n\n    indented\n",
    ];
    for sample in samples {
        for width in [12, 40, 98] {
            for light in [false, true] {
                let source: std::sync::Arc<str> = std::sync::Arc::from(sample);
                let full = preview::render(
                    sample,
                    std::path::Path::new("sample.md"),
                    true,
                    width,
                    light,
                );
                let mut cache = preview::MarkdownCache::default();
                let r = cache.render(
                    source,
                    width,
                    light,
                    preview::CodeViewport { top: 0, rows: 1000 },
                    true,
                );
                assert_eq!(r.plain, full.plain, "sample={sample} width={width}");
                assert_eq!(
                    r.rows.iter().map(|r| &r.code_language).collect::<Vec<_>>(),
                    full.rows
                        .iter()
                        .map(|r| &r.code_language)
                        .collect::<Vec<_>>(),
                    "code headers differ sample={sample} width={width}"
                );
                assert_eq!(
                    r.window.as_ref().unwrap().text,
                    *full.text,
                    "styles differ sample={sample} width={width}"
                );
                for (i, line) in r.plain.iter().enumerate() {
                    assert_eq!(
                        r.selected_source((i, 0), (i, line.chars().count())),
                        full.selected_source((i, 0), (i, line.chars().count()))
                    );
                }
            }
        }
    }
}

#[test]
fn scrolling_keeps_known_rows_stable_bounds_cache_and_full_search_reaches_tail() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use devtabs::sidebar::Focus;
    let root = Fixture::new();
    let mut s = opened(&root);
    let initial = s
        .document
        .as_ref()
        .unwrap()
        .rendered()
        .unwrap()
        .plain
        .clone();
    s.document.as_mut().unwrap().scroll[1].0 = 500;
    let start = Instant::now();
    loop {
        s.tick(true);
        let d = s.document.as_ref().unwrap();
        let r = d.rendered().unwrap();
        if r.window.as_ref().is_some_and(|w| w.line(500).is_some()) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    let d = s.document.as_ref().unwrap();
    let r = d.rendered().unwrap();
    assert_eq!(&r.plain[..initial.len()], initial.as_slice());
    assert!(d.markdown_stats().unwrap().cached_blocks <= 64);
    assert!(d.markdown_stats().unwrap().cached_rows <= 512);
    assert!(!r.complete);
    s.focus = Focus::Preview;
    s.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .unwrap();
    s.paste("UniqueEndMarker").unwrap();
    s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let start = Instant::now();
    loop {
        s.tick(true);
        if s.document.as_ref().unwrap().rendered().unwrap().complete {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    let d = s.document.as_ref().unwrap();
    assert_eq!(d.matches.len(), 1);
    assert_eq!(d.offset().0, d.matches[0].0);
    let full = preview::render(&source(), std::path::Path::new("large.md"), true, 98, false);
    assert_eq!(d.rendered().unwrap().plain, full.plain);
}

#[test]
fn end_key_reaches_the_real_tail_of_a_partially_laid_out_document() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let root = Fixture::new();
    let mut s = opened(&root);
    s.focus = devtabs::sidebar::Focus::Preview;
    s.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .unwrap();
    let start = Instant::now();
    loop {
        s.tick(true);
        let d = s.document.as_ref().unwrap();
        let r = d.rendered().unwrap();
        if r.complete {
            assert_eq!(r.plain[d.offset().0], "UniqueEndMarker");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn reading_fixture_keeps_code_diagrams_math_and_quotes_in_the_actual_sidebar() {
    use ratatui::style::Color;
    use std::collections::HashSet;
    let original = include_str!("fixtures/markdown-reading.md").replace("\r\n", "\n");
    let root = Fixture::new();
    for crlf in [false, true] {
        let source = if crlf {
            original.replace('\n', "\r\n")
        } else {
            original.clone()
        };
        let path = root.0.join("reading.md");
        fs::write(&path, &source).unwrap();
        for light in [false, true] {
            let mut s = Sidebar::new(root.0.clone());
            s.visible = false;
            s.files.light = light;
            s.open(path.clone(), true).unwrap();
            s.update_layout(Rect::new(0, 0, 100, 120), false);
            ready(&mut s);
            let r = s.document.as_ref().unwrap().rendered().unwrap();
            let full =
                preview::render(&source, std::path::Path::new("reading.md"), true, 96, light);
            assert!(r.complete, "fixture should fit the viewport plus prefetch");
            assert_eq!(r.plain, full.plain, "crlf={crlf} light={light}");
            let window = r.window.as_ref().unwrap();
            assert_eq!(window.text, *full.text);
            let prose = r.plain.join("\n");
            assert!(
                prose.contains("E=mc²"),
                "inline math fell back to TeX: {prose}"
            );
            assert!(
                r.rows.iter().any(|r| r.kind == preview::RowKind::Diagram),
                "Mermaid fell back to source: {}",
                r.note
            );
            assert!(r.rows.iter().any(|r| r.kind == preview::RowKind::Math));
            assert!(prose.contains("☑") && prose.contains("☐"));
            let code = r
                .plain
                .iter()
                .position(|l| l.contains("let answer = 42;"))
                .unwrap();
            assert!(
                window
                    .line(code)
                    .unwrap()
                    .spans
                    .iter()
                    .filter_map(|s| s.style.fg)
                    .collect::<HashSet<_>>()
                    .len()
                    >= 3
            );
            let plain = r.plain.clone();
            let note = r.note.clone();
            let mut terminal = Terminal::new(TestBackend::new(100, 120)).unwrap();
            terminal.draw(|f| s.draw(f)).unwrap();
            let buffer = terminal.backend().buffer();
            let row = &buffer.content[(code + 2) * 100..(code + 3) * 100];
            assert!(
                row.iter()
                    .filter(|c| !c.symbol().trim().is_empty())
                    .map(|c| c.fg)
                    .collect::<HashSet<_>>()
                    .len()
                    >= 3,
                "Sidebar lost Rust colors"
            );
            assert!(buffer.content.iter().any(|c| c.fg != Color::Reset));
            if let Some(directory) = std::env::var_os("DEVTABS_READING_SNAPSHOT") {
                let directory = PathBuf::from(directory);
                fs::create_dir_all(&directory).unwrap();
                let cells=buffer.content.iter().map(|c|serde_json::json!({"symbol":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),"modifiers":format!("{:?}",c.modifier)})).collect::<Vec<_>>();
                fs::write(directory.join(format!("reading-{}-{}.json",if light{"light"}else{"dark"},if crlf{"crlf"}else{"lf"})),serde_json::to_vec_pretty(&serde_json::json!({"columns":100,"rows":120,"plain":plain,"note":note,"cells":cells})).unwrap()).unwrap();
            }
        }
    }
}

#[test]
fn code_language_is_fixed_at_top_right_without_entering_code_or_source_selection() {
    let root = Fixture::new();
    let path = root.0.join("languages.md");
    for (info, expected) in [
        ("rust,no_run", "rust"),
        ("json title=example", "json"),
        ("unknown-language", "unknown-language"),
        ("", ""),
        ("很长的语言标签".repeat(20).as_str(), ""),
    ] {
        let source = format!("```{info}\nlet value = 42;\n```\n");
        fs::write(&path, &source).unwrap();
        for light in [false, true] {
            for width in [64, 100] {
                let mut s = Sidebar::new(root.0.clone());
                s.visible = false;
                s.files.light = light;
                s.open(path.clone(), true).unwrap();
                s.update_layout(Rect::new(0, 0, width, 12), false);
                ready(&mut s);
                let cached = s
                    .document
                    .as_ref()
                    .unwrap()
                    .rendered()
                    .unwrap()
                    .window
                    .clone();
                for left in [0, 8, 100] {
                    s.document.as_mut().unwrap().scroll[1].1 = left;
                    for fallback in [false, true] {
                        s.document.as_mut().unwrap().rendered_mut().unwrap().window =
                            if fallback { None } else { cached.clone() };
                        let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
                        terminal.draw(|f| s.draw(f)).unwrap();
                        let r = s.document.as_ref().unwrap().rendered().unwrap();
                        let code = r.plain.iter().position(|s| s == "let value = 42;").unwrap();
                        assert!(code > 0, "language header must not overlap code");
                        let header = code - 1;
                        assert_eq!(
                            r.plain[header], "",
                            "label must stay out of searchable text"
                        );
                        let buffer = terminal.backend().buffer();
                        let label: String = (2..width - 2)
                            .map(|x| buffer[(x, 2 + header as u16)].symbol())
                            .collect();
                        // Hide a label that cannot fit; never truncate it into a misleading language.
                        assert_eq!(
                            label
                                .trim()
                                .strip_suffix("[Copy]")
                                .unwrap_or(label.trim())
                                .trim_end(),
                            expected,
                            "info={info} light={light} width={width} left={left} fallback={fallback}"
                        );
                        if !expected.is_empty() {
                            assert!(
                                label.ends_with(&format!("{expected} [Copy]")),
                                "language must be right aligned"
                            );
                        }
                        assert_eq!(
                            r.selected_source((header, 0), (code, 15)),
                            "let value = 42;"
                        );
                        let bg = buffer[(2, 2 + code as u16)].bg;
                        for x in 2..width - 2 {
                            assert_eq!(buffer[(x, 2 + header as u16)].bg, bg);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn empty_fences_keep_their_headers_and_other_block_types_stay_unlabelled() {
    use std::path::Path;
    for info in ["rust", ""] {
        let r = preview::render(
            &format!("```{info}\n```"),
            Path::new("test.md"),
            true,
            80,
            false,
        );
        assert_eq!(r.rows[0].code_language.as_deref(), Some(info));
        assert!(r.rows.iter().any(|r| r.kind == preview::RowKind::Code));
        assert!(r.plain.iter().all(|s| s.is_empty()));
    }
    for (source, markdown) in [
        ("    indented code\n", true),
        ("```mermaid\ngraph LR\nA-->B\n```", true),
        ("```rust\nlet x = 1;\n```", false),
    ] {
        let r = preview::render(source, Path::new("test.md"), markdown, 80, false);
        assert!(r.rows.iter().all(|r| r.code_language.is_none()));
    }
}

#[test]
fn list_markers_have_no_underline_but_links_still_do() {
    use ratatui::style::Modifier;
    let root = Fixture::new();
    let path = root.0.join("lists.md");
    fs::write(&path,"- [x] done\n- [ ] pending\n  1. ordered\n  2. second\n\n- bullet\n\n[link](https://example.com)\n").unwrap();
    for light in [false, true] {
        let mut s = Sidebar::new(root.0.clone());
        s.visible = false;
        s.files.light = light;
        s.open(path.clone(), true).unwrap();
        s.update_layout(Rect::new(0, 0, 80, 30), false);
        ready(&mut s);
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|f| s.draw(f)).unwrap();
        let r = s.document.as_ref().unwrap().rendered().unwrap();
        let buffer = terminal.backend().buffer();
        for marker in ["☑", "☐", "1.", "2.", "•"] {
            let row = r
                .plain
                .iter()
                .position(|s| s.trim_start().starts_with(marker))
                .unwrap();
            let col = r.plain[row]
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            for x in col..col + marker.chars().count() {
                let cell = &buffer[(2 + x as u16, 2 + row as u16)];
                assert!(
                    !cell.modifier.contains(Modifier::UNDERLINED),
                    "marker {marker} light={light} is underlined"
                );
            }
        }
        let row = r.plain.iter().position(|s| s == "link").unwrap();
        assert!(
            buffer[(2, 2 + row as u16)]
                .modifier
                .contains(Modifier::UNDERLINED),
            "link underline was lost"
        );
    }
}

#[test]
fn code_background_fills_sidebar_width_for_short_empty_and_scrolled_rows() {
    use ratatui::{
        style::{Color, Style},
        widgets::Block,
    };
    let root = Fixture::new();
    let path = root.0.join("code.md");
    fs::write(
        &path,
        "```rust\nfn main() {}\n\nlet value = 42;\n```\n\ninline `code` ends here\n",
    )
    .unwrap();
    for light in [false, true] {
        for width in [64, 100] {
            let mut s = Sidebar::new(root.0.clone());
            s.visible = false;
            s.files.light = light;
            s.open(path.clone(), true).unwrap();
            s.update_layout(Rect::new(0, 0, width, 30), false);
            ready(&mut s);
            let cached = s
                .document
                .as_ref()
                .unwrap()
                .rendered()
                .unwrap()
                .window
                .clone();
            for left in [0, 8, 100] {
                s.document.as_mut().unwrap().scroll[1].1 = left;
                for fallback in [false, true] {
                    // The same base style must work before the viewport's colored window arrives.
                    s.document.as_mut().unwrap().rendered_mut().unwrap().window =
                        if fallback { None } else { cached.clone() };
                    let background = if light {
                        Color::White
                    } else {
                        Color::Rgb(46, 52, 64)
                    };
                    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
                    terminal
                        .draw(|f| {
                            f.render_widget(
                                Block::default().style(Style::new().bg(background)),
                                f.area(),
                            );
                            s.draw(f);
                        })
                        .unwrap();
                    let r = s.document.as_ref().unwrap().rendered().unwrap();
                    let buffer = terminal.backend().buffer();
                    let mut checked = 0;
                    for (row, mapping) in r
                        .rows
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| r.kind == preview::RowKind::Code)
                    {
                        let line = r
                            .window
                            .as_ref()
                            .and_then(|w| w.line(row))
                            .unwrap_or(&r.text.lines[row]);
                        let expected = line
                            .style
                            .bg
                            .expect("code row must retain a block background");
                        for x in 2..width - 2 {
                            assert_eq!(
                                buffer[(x, 2 + row as u16)].bg,
                                expected,
                                "code row {row} col {x} width={width} light={light} left={left} fallback={fallback} kind={:?}",
                                mapping.kind
                            );
                        }
                        assert_eq!(
                            buffer[(0, 2 + row as u16)].bg,
                            Color::Reset,
                            "code background leaked into the border"
                        );
                        for x in [1, width - 2] {
                            assert_eq!(
                                buffer[(x, 2 + row as u16)].bg,
                                Color::Reset,
                                "code background leaked into the padding"
                            );
                        }
                        checked += 1;
                    }
                    assert!(checked >= 3, "must cover short code and an empty code row");
                    let row = r
                        .plain
                        .iter()
                        .position(|s| s.starts_with("inline "))
                        .unwrap();
                    assert_eq!(
                        buffer[(width - 3, 2 + row as u16)].bg,
                        Color::Reset,
                        "inline code must not fill the entire row"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "isolated Release Markdown viewport comparison"]
fn markdown_viewport_workload() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let root = Fixture::new();
    std::hint::black_box(preview::highlight(
        "let x = 42;",
        std::path::Path::new("warm.rs"),
        false,
    ));
    std::hint::black_box(preview::render(
        "# warm",
        std::path::Path::new("warm.md"),
        true,
        98,
        false,
    ));
    let mut cases = Vec::new();
    for iteration in 0..5 {
        let start = Instant::now();
        let mut s = opened(&root);
        let open_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        terminal.draw(|f| s.draw(f)).unwrap();
        let colors = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|c| !c.symbol().trim().is_empty())
            .map(|c| c.fg)
            .collect::<std::collections::HashSet<_>>();
        assert!(colors.len() >= 4);
        let r = s.document.as_ref().unwrap().rendered().unwrap();
        cases.push(serde_json::json!({"iteration":iteration,"open_ms":open_ms,"rows":r.text.lines.len(),"spans":r.text.lines.iter().map(|l|l.spans.len()).sum::<usize>(),"window_spans":r.window.as_ref().map(|w|w.text.lines.iter().map(|l|l.spans.len()).sum::<usize>()),"stats":s.document.as_ref().unwrap().markdown_stats()}));
    }
    fs::write(std::env::var_os("DEVTABS_MARKDOWN_REPORT").expect("report path"),serde_json::to_vec_pretty(&serde_json::json!({"source_bytes":source().len(),"terminal":[100,40],"blocks":1501,"cases":cases})).unwrap()).unwrap();
}
