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
            .is_some_and(|d| !d.loading && d.rendered.as_ref().is_some_and(|r| r.markdown))
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
    let r = s.document.as_ref().unwrap().rendered.as_ref().unwrap();
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
        .rendered
        .as_ref()
        .unwrap()
        .plain
        .clone();
    s.document.as_mut().unwrap().scroll[1].0 = 500;
    let start = Instant::now();
    loop {
        s.tick(true);
        let d = s.document.as_ref().unwrap();
        let r = d.rendered.as_ref().unwrap();
        if r.window.as_ref().is_some_and(|w| w.line(500).is_some()) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    let d = s.document.as_ref().unwrap();
    let r = d.rendered.as_ref().unwrap();
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
        if s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .complete
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    let d = s.document.as_ref().unwrap();
    assert_eq!(d.matches.len(), 1);
    assert_eq!(d.offset().0, d.matches[0].0);
    let full = preview::render(&source(), std::path::Path::new("large.md"), true, 98, false);
    assert_eq!(d.rendered.as_ref().unwrap().plain, full.plain);
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
        let r = d.rendered.as_ref().unwrap();
        if r.complete {
            assert_eq!(r.plain[d.offset().0], "UniqueEndMarker");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
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
        let r = s.document.as_ref().unwrap().rendered.as_ref().unwrap();
        cases.push(serde_json::json!({"iteration":iteration,"open_ms":open_ms,"rows":r.text.lines.len(),"spans":r.text.lines.iter().map(|l|l.spans.len()).sum::<usize>(),"window_spans":r.window.as_ref().map(|w|w.text.lines.iter().map(|l|l.spans.len()).sum::<usize>()),"stats":s.document.as_ref().unwrap().markdown_stats()}));
    }
    fs::write(std::env::var_os("DEVTABS_MARKDOWN_REPORT").expect("report path"),serde_json::to_vec_pretty(&serde_json::json!({"source_bytes":source().len(),"terminal":[100,40],"blocks":1501,"cases":cases})).unwrap()).unwrap();
}
