use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use devtabs::{
    preview::{
        self, ReadingAction,
        reading::{Destination, ReadingOptions},
    },
    sidebar::Sidebar,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-reading-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn open(&self, text: &str, width: u16, height: u16) -> Sidebar {
        let path = self.0.join("read.md");
        fs::write(&path, text).unwrap();
        let mut s = Sidebar::new(self.0.clone());
        s.visible = false;
        s.open(path, true).unwrap();
        s.update_layout(Rect::new(0, 0, width, height), false);
        wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
        s
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn wait(s: &mut Sidebar, f: impl Fn(&Sidebar) -> bool) {
    let start = Instant::now();
    loop {
        s.tick(true);
        if f(s) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "{}", s.status());
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn draw(s: &mut Sidebar, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| s.draw(f)).unwrap();
    t.backend().buffer().clone()
}
fn key(s: &mut Sidebar, k: KeyCode) {
    s.key(KeyEvent::new(k, KeyModifiers::NONE)).unwrap();
}
fn mouse(s: &mut Sidebar, kind: MouseEventKind, x: u16, y: u16) {
    s.mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
}
fn md(text: &str) -> preview::Rendered {
    preview::render(text, Path::new("test.md"), true, 80, false)
}

#[test]
fn code_copy_preserves_full_body_tabs_empty_lines_and_excludes_labels() {
    let r = md("```rust,no_run\n\tlet x = 1;\n\nlet y = 2;\n```\n");
    assert_eq!(
        r.rows[0].code_copy.as_deref(),
        Some("\tlet x = 1;\n\nlet y = 2;\n")
    );
    assert!(!r.plain.join("\n").contains("Copy"));
}
#[test]
fn headings_have_unicode_unique_anchors_and_explicit_ids() {
    let r = md("# 标题 **one**\n\n## 标题 one\n\n### Explicit {#my-id}\n");
    let anchors: Vec<_> = r
        .reading
        .headings
        .iter()
        .map(|h| h.anchor.as_str())
        .collect();
    assert_eq!(anchors, ["标题-one", "标题-one-1", "my-id"]);
    assert_eq!(r.reading.headings[0].title, "标题 one");
}
#[test]
fn links_images_and_footnotes_hit_correct_wrapped_source_ranges() {
    let r = preview::render(
        "- long [click 中文](other.md#part) and ![picture](image.png) and note[^n]\n\n[^n]: footnote text\n",
        Path::new("test.md"),
        true,
        22,
        false,
    );
    for (needle, expected) in [
        ("click", ReadingAction::Link("other.md#part".into())),
        ("picture", ReadingAction::Image("image.png".into())),
        ("note[^n]", ReadingAction::Footnote("n".into())),
    ] {
        let row = r.plain.iter().position(|s| s.contains(needle)).unwrap();
        let col = r.plain[row][..r.plain[row].find(needle).unwrap()]
            .chars()
            .count()
            + if needle == "note[^n]" { 4 } else { 0 };
        assert_eq!(
            r.action_at(row, col),
            Some(expected),
            "{needle}: {:?}",
            r.plain
        );
    }
    let row = r.plain.iter().position(|s| s.contains("↩")).unwrap();
    assert_eq!(r.action_at(row, 0), Some(ReadingAction::Back));
}
#[test]
fn relative_encoded_links_anchors_and_unsupported_schemes() {
    let base = Path::new("D:/project/docs/read.md");
    assert_eq!(
        preview::reading::destination(base, "../a%20b.md#%E4%B8%AD%E6%96%87").unwrap(),
        Destination::File(
            PathBuf::from("D:/project/docs/../a b.md"),
            Some("中文".into())
        )
    );
    assert_eq!(
        preview::reading::destination(base, "#part").unwrap(),
        Destination::Anchor("part".into())
    );
    assert_eq!(
        preview::reading::destination(base, "https://example.com/a#b").unwrap(),
        Destination::Web("https://example.com/a#b".into())
    );
    for url in [
        "javascript:alert(1)",
        "file://host/a",
        "cmd:run",
        "x%ZZ",
        "x%",
        "%中",
        "x%💻",
        "x%00",
        "x\n",
    ] {
        assert!(preview::reading::destination(base, url).is_err(), "{url}");
    }
}
#[test]
fn details_toggle_nested_content_and_preserve_original_source() {
    let text = "# Before\n\n<details>\n<summary>More **info**</summary>\n\n## Hidden\n\nsecret $x^2$\n\n<details open>\n<summary>Inner</summary>\n\ninner body\n\n</details>\n</details>\n\nAfter\n";
    let closed = md(text);
    assert!(!closed.plain.join("\n").contains("secret"));
    assert!(closed.plain.join("\n").contains("▸ More info"));
    assert_eq!(closed.reading.headings.len(), 2);
    let id = closed.reading.details[0].source.start;
    let mut options = ReadingOptions::default();
    options.expanded.insert(id);
    let mut cache = preview::MarkdownCache::default();
    let open = cache.render_reading(
        Arc::from(text),
        80,
        false,
        preview::CodeViewport { top: 0, rows: 100 },
        true,
        &options,
    );
    assert!(
        open.plain.join("\n").contains("secret x²"),
        "{:?}",
        open.plain
    );
    assert!(open.plain.join("\n").contains("inner body"));
    assert_eq!(open.reading.headings.len(), 2);
    assert_eq!(open.source.as_ref(), text);
    let inner = open.reading.details[1].source.start;
    options.expanded.insert(inner);
    let inner_closed = cache.render_reading(
        Arc::from(text),
        80,
        false,
        preview::CodeViewport { top: 0, rows: 100 },
        true,
        &options,
    );
    assert!(!inner_closed.plain.join("\n").contains("inner body"));
    let literal = md("```html\n<details>\n<summary>literal</summary>\nbody\n</details>\n```");
    assert!(literal.reading.details.is_empty());
    assert!(
        literal
            .plain
            .join("\n")
            .contains("<summary>literal</summary>")
    );
}
#[test]
fn outline_filters_and_jumps_beyond_first_viewport_without_eager_open() {
    let f = Fixture::new();
    let text = (0..200)
        .map(|i| format!("## Section {i}\n\nparagraph {i}\n\n"))
        .collect::<String>();
    let mut s = f.open(&text, 80, 20);
    assert!(
        s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .plain
            .len()
            < 100
    );
    key(&mut s, KeyCode::Char('t'));
    assert!(s.modal());
    s.paste("Section 180").unwrap();
    let b = draw(&mut s, 80, 20);
    assert!(b.content.iter().any(|c| c.symbol() == "O"));
    key(&mut s, KeyCode::Enter);
    assert!(!s.modal());
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered.as_ref().is_some_and(|r| {
            r.plain
                .get(d.offset().0)
                .is_some_and(|s| s == "Section 180")
        })
    });
}
#[test]
fn clicking_local_links_jumps_to_fragment_and_back_restores_position() {
    let f = Fixture::new();
    fs::write(
        f.0.join("other.md"),
        "# Intro\n\ntext\n\n## Destination\n\nbody\n",
    )
    .unwrap();
    let mut s = f.open("# Start\n\n[Open](other.md#destination)\n", 80, 20);
    let row = s
        .document
        .as_ref()
        .unwrap()
        .rendered
        .as_ref()
        .unwrap()
        .plain
        .iter()
        .position(|s| s == "Open")
        .unwrap() as u16;
    mouse(&mut s, MouseEventKind::Down(MouseButton::Left), 2, row + 1);
    mouse(&mut s, MouseEventKind::Up(MouseButton::Left), 2, row + 1);
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.path.ends_with("other.md")
            && d.rendered.as_ref().is_some_and(|r| {
                r.plain
                    .get(d.offset().0)
                    .is_some_and(|s| s == "Destination")
            })
    });
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().path.ends_with("read.md")
            && s.document.as_ref().unwrap().rendered.is_some()
    });
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
}
#[test]
fn resize_mode_switch_and_file_switch_preserve_logical_reading_position() {
    let f = Fixture::new();
    let text=(0..70).map(|i|format!("## Heading {i}\n\nLong paragraph with enough words to wrap when resized number {i}.\n\n")).collect::<String>();
    let mut s = f.open(&text, 80, 20);
    s.activate_reading(ReadingAction::Link("#heading-12".into()))
        .unwrap();
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered
            .as_ref()
            .is_some_and(|r| r.plain.get(d.offset().0).is_some_and(|s| s == "Heading 12"))
    });
    s.update_layout(Rect::new(0, 0, 64, 20), false);
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .width
            == 62
    });
    let d = s.document.as_ref().unwrap();
    assert_eq!(
        d.rendered.as_ref().unwrap().plain[d.offset().0],
        "Heading 12"
    );
    key(&mut s, KeyCode::Char('m'));
    wait(&mut s, |s| {
        !s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .markdown
    });
    let d = s.document.as_ref().unwrap();
    assert_eq!(
        d.rendered.as_ref().unwrap().plain[d.offset().0],
        "## Heading 12"
    );
    fs::write(f.0.join("other.md"), "other").unwrap();
    s.open(f.0.join("other.md"), true).unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
    s.open(f.0.join("read.md"), true).unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
    let d = s.document.as_ref().unwrap();
    assert!(!d.markdown);
    assert_eq!(
        d.rendered.as_ref().unwrap().plain[d.offset().0],
        "## Heading 12"
    );
}
#[test]
fn table_header_pins_and_horizontal_scrollbar_drags() {
    let f = Fixture::new();
    let mut text = "| Name | Description |\n|---|---|\n".to_owned();
    for i in 0..35 {
        text.push_str(&format!("| item {i} | {} |\n", "long field ".repeat(9)));
    }
    let mut s = f.open(&text, 64, 20);
    s.document.as_mut().unwrap().scroll[1].0 = 10;
    let b = draw(&mut s, 64, 20);
    let top: String = (1..63).map(|x| b[(x, 1)].symbol()).collect();
    assert!(top.contains("Name"), "{top}");
    let rect = s.geometry.preview.unwrap();
    let y = rect.bottom() - 1;
    assert!(b.content.iter().any(|c| c.symbol() == "█"));
    mouse(&mut s, MouseEventKind::Down(MouseButton::Left), 45, y);
    mouse(&mut s, MouseEventKind::Drag(MouseButton::Left), 61, y);
    mouse(&mut s, MouseEventKind::Up(MouseButton::Left), 61, y);
    assert!(s.document.as_ref().unwrap().offset().1 > 0);
}
#[test]
fn vertical_scrollbar_drags_to_unlaid_tail_and_short_documents_have_no_thumb() {
    let f = Fixture::new();
    let text = (0..500)
        .map(|i| format!("## Section {i}\n\ntext {i}\n\n"))
        .collect::<String>();
    let mut s = f.open(&text, 80, 20);
    let b = draw(&mut s, 80, 20);
    assert_eq!(b[(79, 1)].symbol(), "█");
    mouse(&mut s, MouseEventKind::Down(MouseButton::Left), 79, 14);
    mouse(&mut s, MouseEventKind::Drag(MouseButton::Left), 79, 16);
    mouse(&mut s, MouseEventKind::Up(MouseButton::Left), 79, 16);
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > 1000);
    let short = f.open("short", 80, 20);
    let mut short = short;
    let b = draw(&mut short, 80, 20);
    assert!(!b.content.iter().any(|c| c.symbol() == "█"));
}
#[test]
fn footnote_round_trip_and_clicking_summary_use_source_targets() {
    let f = Fixture::new();
    let mut text = "note[^n]\n\n".to_owned();
    text.push_str(&"body\n\n".repeat(30));
    text.push_str(
        "[^n]: explanation\n\n<details>\n<summary>Expand me</summary>\n\nsecret\n</details>\n",
    );
    let mut s = f.open(&text, 80, 20);
    s.activate_reading(ReadingAction::Footnote("n".into()))
        .unwrap();
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered.as_ref().is_some_and(|r| {
            r.plain
                .get(d.offset().0)
                .is_some_and(|s| s.contains("explanation"))
        })
    });
    s.activate_reading(ReadingAction::Back).unwrap();
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
    let id = s
        .document
        .as_ref()
        .unwrap()
        .rendered
        .as_ref()
        .unwrap()
        .reading
        .details[0]
        .source
        .start;
    s.activate_reading(ReadingAction::Toggle(id)).unwrap();
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .is_some_and(|r| r.plain.iter().any(|s| s == "secret"))
    });
}
#[test]
fn nested_quotes_keep_multiple_rails_on_wrapped_and_blank_rows() {
    let r = preview::render(
        "> outer\n>\n> > nested long text that wraps onto another line\n> >\n> > next\n",
        Path::new("test.md"),
        true,
        22,
        false,
    );
    assert!(
        r.plain.iter().filter(|s| s.starts_with("│ │ ")).count() >= 3,
        "{:?}",
        r.plain
    );
    let line = r
        .text
        .lines
        .iter()
        .find(|l| l.to_string().contains("nested"))
        .unwrap();
    assert_ne!(line.spans[0].style.fg, line.spans[1].style.fg);
}

#[test]
fn hidden_heading_links_open_ancestor_details_and_back_closes_them() {
    let f = Fixture::new();
    let text = "[Hidden](#hidden)\n\n<details>\n<summary>More</summary>\n\n## Hidden\n\nsecret\n</details>\n";
    let mut s = f.open(text, 80, 20);
    s.activate_reading(ReadingAction::Link("#hidden".into()))
        .unwrap();
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered
            .as_ref()
            .is_some_and(|r| r.plain.get(d.offset().0).is_some_and(|s| s == "Hidden"))
    });
    s.activate_reading(ReadingAction::Back).unwrap();
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .is_some_and(|r| !r.plain.iter().any(|s| s == "secret"))
    });
}

#[test]
fn tree_and_editor_scrollbars_drag_without_editing_text() {
    let f = Fixture::new();
    for i in 0..100 {
        fs::write(f.0.join(format!("file{i:03}.txt")), "text").unwrap();
    }
    let text = (0..100)
        .map(|i| format!("line{i} {}\n", "x".repeat(200)))
        .collect::<String>();
    let mut s = f.open(&text, 120, 24);
    s.visible = true;
    s.update_layout(Rect::new(0, 0, 120, 24), false);
    wait(&mut s, |s| s.files.nodes.len() > 100);
    let b = draw(&mut s, 120, 24);
    let tree = s.geometry.tree.unwrap();
    assert_eq!(b[(tree.right() - 1, 1)].symbol(), "█");
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        tree.right() - 1,
        12,
    );
    draw(&mut s, 120, 24);
    mouse(
        &mut s,
        MouseEventKind::Drag(MouseButton::Left),
        tree.right() - 1,
        19,
    );
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        tree.right() - 1,
        19,
    );
    assert!(s.files.state.get_offset() > 20);
    s.focus = devtabs::sidebar::Focus::Preview;
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    draw(&mut s, 120, 24);
    let rect = s.geometry.preview.unwrap();
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        rect.right() - 1,
        14,
    );
    mouse(
        &mut s,
        MouseEventKind::Drag(MouseButton::Left),
        rect.right() - 1,
        12,
    );
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        rect.right() - 1,
        12,
    );
    let e = s.document.as_ref().unwrap().editor.as_ref().unwrap();
    assert!(e.area.scroll_offset().0 > 20);
    assert!(!e.dirty());
    draw(&mut s, 120, 24);
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        rect.right() - 12,
        rect.bottom() - 1,
    );
    mouse(
        &mut s,
        MouseEventKind::Drag(MouseButton::Left),
        rect.right() - 3,
        rect.bottom() - 1,
    );
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        rect.right() - 3,
        rect.bottom() - 1,
    );
    let e = s.document.as_ref().unwrap().editor.as_ref().unwrap();
    assert!(e.area.scroll_offset().1 > 0);
    assert!(!e.dirty());
}

#[test]
fn dragging_links_and_missing_targets_keep_the_current_document() {
    let f = Fixture::new();
    fs::write(f.0.join("other.md"), "other").unwrap();
    let mut s = f.open("[Open](other.md)\n", 80, 20);
    mouse(&mut s, MouseEventKind::Down(MouseButton::Left), 1, 1);
    mouse(&mut s, MouseEventKind::Drag(MouseButton::Left), 3, 1);
    mouse(&mut s, MouseEventKind::Drag(MouseButton::Left), 1, 1);
    mouse(&mut s, MouseEventKind::Up(MouseButton::Left), 1, 1);
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    assert!(
        s.activate_reading(ReadingAction::Link("missing.md".into()))
            .is_err()
    );
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
}

#[test]
fn file_links_keep_the_existing_unsaved_cancel_and_discard_guard() {
    let f = Fixture::new();
    fs::write(f.0.join("other.md"), "other").unwrap();
    let mut s = f.open("# original\n", 80, 20);
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.document
        .as_mut()
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .paste("unsaved ")
        .unwrap();
    s.menu();
    let menu = s.menu.as_mut().unwrap();
    menu.selected = menu
        .items
        .iter()
        .position(|(label, _)| *label == "Preview current content")
        .unwrap();
    key(&mut s, KeyCode::Enter);
    s.activate_reading(ReadingAction::Link("other.md".into()))
        .unwrap();
    assert!(s.modal());
    key(&mut s, KeyCode::Esc);
    assert!(s.dirty());
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    s.activate_reading(ReadingAction::Link("other.md".into()))
        .unwrap();
    key(&mut s, KeyCode::Char('d'));
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().path.ends_with("other.md")
            && s.document.as_ref().unwrap().rendered.is_some()
    });
    assert_eq!(
        fs::read_to_string(f.0.join("read.md")).unwrap(),
        "# original\n"
    );
}

#[test]
fn same_file_navigation_and_back_keep_unsaved_edits_without_a_prompt() {
    let f = Fixture::new();
    let mut s = f.open("# First\n\n## Destination\n", 80, 20);
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.document
        .as_mut()
        .unwrap()
        .editor
        .as_mut()
        .unwrap()
        .paste("prefix\n\n")
        .unwrap();
    s.menu();
    let menu = s.menu.as_mut().unwrap();
    menu.selected = menu
        .items
        .iter()
        .position(|(label, _)| *label == "Preview current content")
        .unwrap();
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
    s.activate_reading(ReadingAction::Link("#destination".into()))
        .unwrap();
    s.activate_reading(ReadingAction::Back).unwrap();
    assert!(!s.modal());
    assert!(s.dirty());
    s.activate_reading(ReadingAction::Link("read.md#destination".into()))
        .unwrap();
    assert!(!s.modal());
    assert!(s.dirty());
}

#[test]
fn only_a_real_open_attribute_expands_details_by_default() {
    for (attributes, open) in [
        ("title=\"open\"", false),
        ("title=\"x open y\"", false),
        ("open", true),
        ("open=\"false\"", true),
    ] {
        let r = md(&format!(
            "<details {attributes}>\n<summary>More</summary>\n\nbody\n</details>"
        ));
        assert_eq!(r.plain.iter().any(|s| s == "body"), open, "{attributes}");
    }
}

#[test]
fn cross_file_fragment_works_when_the_target_was_remembered_in_source_mode() {
    let f = Fixture::new();
    let other = f.0.join("other.md");
    fs::write(&other, "# First\n\n## Destination\n").unwrap();
    let mut s = f.open("start", 80, 20);
    s.open(other, true).unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
    key(&mut s, KeyCode::Char('m'));
    wait(&mut s, |s| {
        !s.document
            .as_ref()
            .unwrap()
            .rendered
            .as_ref()
            .unwrap()
            .markdown
    });
    s.open(f.0.join("read.md"), true).unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().rendered.is_some());
    s.activate_reading(ReadingAction::Link("other.md#destination".into()))
        .unwrap();
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered.as_ref().is_some_and(|r| {
            r.markdown
                && r.plain
                    .get(d.offset().0)
                    .is_some_and(|s| s == "Destination")
        })
    });
}

fn outline_button(buffer: &Buffer, preview: Rect) -> u16 {
    (preview.x..preview.right().saturating_sub(2))
        .find(|&x| {
            buffer[(x, preview.y)].symbol() == "["
                && buffer[(x + 1, preview.y)].symbol() == "T"
                && buffer[(x + 2, preview.y)].symbol() == "]"
        })
        .expect("preview needs an outline button")
}
fn outline_bounds(buffer: &Buffer) -> Rect {
    let (x, y) = (buffer.area.y..buffer.area.bottom())
        .flat_map(|y| (buffer.area.x..buffer.area.right()).map(move |x| (x, y)))
        .find(|&(x, y)| {
            buffer[(x, y)].symbol() == "O"
                && x + 6 < buffer.area.right()
                && (0..7)
                    .map(|i| buffer[(x + i, y)].symbol())
                    .collect::<String>()
                    == "Outline"
        })
        .expect("outline title");
    let left = (buffer.area.x..x)
        .rev()
        .find(|&x| buffer[(x, y)].symbol() == "┌")
        .unwrap();
    let right = (x..buffer.area.right())
        .find(|&x| buffer[(x, y)].symbol() == "┐")
        .unwrap();
    let bottom = (y + 1..buffer.area.bottom())
        .find(|&y| buffer[(left, y)].symbol() == "└")
        .unwrap();
    Rect::new(left, y, right - left + 1, bottom - y + 1)
}

#[test]
fn outline_button_opens_a_small_dropdown_anchored_inside_the_preview() {
    let f = Fixture::new();
    let mut s = f.open("# One\n\n## Two\n\n## Three\n", 180, 40);
    let b = draw(&mut s, 180, 40);
    let preview = s.geometry.preview.unwrap();
    let x = outline_button(&b, preview);
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        x + 1,
        preview.y,
    );
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        x + 1,
        preview.y,
    );
    assert!(s.modal());
    let b = draw(&mut s, 180, 40);
    let popup = outline_bounds(&b);
    assert_eq!(popup.y, preview.y + 1);
    assert!(popup.x > preview.x);
    assert!(popup.right() <= preview.right());
    assert!(
        popup.width <= 48 && popup.height <= 8,
        "dropdown must fit its short list: {popup:?}"
    );
    assert!(
        popup.x <= x && popup.right() > x,
        "dropdown must anchor below its button"
    );
}

#[test]
fn outline_dropdown_click_selects_and_outside_click_dismisses_without_clickthrough() {
    let f = Fixture::new();
    let mut s = f.open("# One\n\n## Two\n\n## Three\n", 80, 24);
    key(&mut s, KeyCode::Char('t'));
    let b = draw(&mut s, 80, 24);
    let popup = outline_bounds(&b);
    let row = popup.y + 4;
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        popup.x + 3,
        row,
    );
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        popup.x + 3,
        row,
    );
    assert!(!s.modal());
    let d = s.document.as_ref().unwrap();
    assert_eq!(d.rendered.as_ref().unwrap().plain[d.offset().0], "Three");
    key(&mut s, KeyCode::Char('t'));
    let old = s.document.as_ref().unwrap().offset();
    mouse(&mut s, MouseEventKind::Down(MouseButton::Left), 1, 12);
    mouse(&mut s, MouseEventKind::Up(MouseButton::Left), 1, 12);
    assert!(!s.modal());
    assert_eq!(s.document.as_ref().unwrap().offset(), old);
    assert!(s.document.as_ref().unwrap().selection.is_none());
}

#[test]
fn outline_dropdown_scrolls_filters_toggles_and_stays_in_bounds_after_resize() {
    let f = Fixture::new();
    let text = (0..40)
        .map(|i| format!("## Heading {i}\n\n"))
        .collect::<String>();
    let mut s = f.open(&text, 180, 40);
    let b = draw(&mut s, 180, 40);
    let preview = s.geometry.preview.unwrap();
    let x = outline_button(&b, preview);
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        x + 1,
        preview.y,
    );
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        x + 1,
        preview.y,
    );
    assert!(!s.modal());
    key(&mut s, KeyCode::Char('t'));
    for _ in 0..8 {
        key(&mut s, KeyCode::Down);
    }
    let b = draw(&mut s, 180, 40);
    let popup = outline_bounds(&b);
    assert!(popup.height <= 14);
    mouse(&mut s, MouseEventKind::ScrollDown, popup.x + 3, popup.y + 3);
    s.paste("Heading 39").unwrap();
    let b = draw(&mut s, 180, 40);
    assert!(outline_bounds(&b).height <= 6);
    s.update_layout(Rect::new(0, 0, 64, 12), false);
    let b = draw(&mut s, 64, 12);
    let popup = outline_bounds(&b);
    let preview = s.geometry.preview.unwrap();
    assert!(
        popup.x >= preview.x
            && popup.right() <= preview.right()
            && popup.bottom() <= preview.bottom()
    );
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered
            .as_ref()
            .is_some_and(|r| r.plain.get(d.offset().0).is_some_and(|s| s == "Heading 39"))
    });
}

#[test]
fn clicking_a_scrolled_outline_row_does_not_reposition_it_before_release() {
    let f = Fixture::new();
    let text = (0..50)
        .map(|i| format!("## Heading {i}\n\n"))
        .collect::<String>();
    let mut s = f.open(&text, 80, 30);
    key(&mut s, KeyCode::Char('t'));
    for _ in 0..20 {
        key(&mut s, KeyCode::Down);
    }
    let b = draw(&mut s, 80, 30);
    let popup = outline_bounds(&b);
    let row = popup.y + 2;
    let displayed: String = (popup.x + 1..popup.right() - 1)
        .map(|x| b[(x, row)].symbol())
        .collect();
    assert!(displayed.contains("Heading 11"), "{displayed}");
    mouse(
        &mut s,
        MouseEventKind::Down(MouseButton::Left),
        popup.x + 3,
        row,
    );
    let b = draw(&mut s, 80, 30);
    let after: String = (popup.x + 1..popup.right() - 1)
        .map(|x| b[(x, row)].symbol())
        .collect();
    assert!(after.contains("Heading 11"), "{after}");
    mouse(
        &mut s,
        MouseEventKind::Up(MouseButton::Left),
        popup.x + 3,
        row,
    );
    assert!(!s.modal());
    wait(&mut s, |s| {
        let d = s.document.as_ref().unwrap();
        d.rendered
            .as_ref()
            .is_some_and(|r| r.plain.get(d.offset().0).is_some_and(|s| s == "Heading 11"))
    });
}
