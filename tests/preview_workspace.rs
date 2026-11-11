use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use devtabs::{preview, sidebar::Sidebar};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-preview-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn open(&self, text: &str) -> Sidebar {
        fs::write(self.0.join("read.md"), text).unwrap();
        let mut s = Sidebar::new(self.0.clone());
        s.visible = false;
        s.open(self.0.join("read.md"), true).unwrap();
        s.update_layout(Rect::new(0, 0, 120, 30), false);
        ready(&mut s);
        s
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn wait(s: &mut Sidebar, f: impl Fn(&Sidebar) -> bool) {
    let at = Instant::now();
    loop {
        s.tick(true);
        if f(s) {
            break;
        }
        assert!(at.elapsed() < Duration::from_secs(15), "{}", s.status());
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn ready(s: &mut Sidebar) {
    wait(s, |s| {
        s.document
            .as_ref()
            .is_some_and(|d| !d.loading && d.rendered().is_some())
    });
}
fn key(s: &mut Sidebar, k: KeyCode) {
    s.key(KeyEvent::new(k, KeyModifiers::NONE)).unwrap();
}
fn draw(s: &mut Sidebar) -> Buffer {
    let a = s.geometry.preview.unwrap();
    let w = a.right().max(120);
    let mut t = Terminal::new(TestBackend::new(w, 30)).unwrap();
    t.draw(|f| s.draw(f)).unwrap();
    t.backend().buffer().clone()
}
fn text(b: &Buffer) -> String {
    b.content.iter().map(|c| c.symbol()).collect()
}
fn mouse(s: &mut Sidebar, x: u16, y: u16, kind: MouseEventKind) {
    s.mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
}

#[test]
fn maximize_restores_layout_and_keeps_reading_position() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..60)
            .map(|i| format!("## Part {i}\n\nbody\n\n"))
            .collect::<String>(),
    );
    s.update_layout(Rect::new(0, 0, 180, 30), true);
    ready(&mut s);
    s.activate_reading(preview::ReadingAction::Link("#part-10".into()))
        .unwrap();
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().rendered().is_some_and(|r| {
            r.plain
                .get(s.document.as_ref().unwrap().offset().0)
                .is_some_and(|v| v == "Part 10")
        })
    });
    let original = s.geometry;
    key(&mut s, KeyCode::Char('z'));
    assert!(s.geometry.overlay);
    assert!(s.geometry.debug.is_none());
    assert_eq!(s.geometry.preview.unwrap().width, 180);
    ready(&mut s);
    key(&mut s, KeyCode::Char('z'));
    assert_eq!(s.geometry, original);
    ready(&mut s);
}

#[test]
fn wrapping_preserves_code_source_copy_colors_and_can_be_disabled() {
    let f = Fixture::new();
    let source = format!("```rust\nlet 中文 = \"{}\";\n```\n", "abcdef".repeat(25));
    let mut s = f.open(&source);
    s.document.as_mut().unwrap().scroll[1].1 = 100;
    key(&mut s, KeyCode::Char('w'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| r.projection.is_some())
    });
    let r = s.document.as_ref().unwrap().rendered().unwrap();
    assert_eq!(s.document.as_ref().unwrap().offset().1, 0);
    assert!(
        r.plain
            .iter()
            .all(|l| unicode_width::UnicodeWidthStr::width(l.as_str()) <= r.width as usize)
    );
    let code = r
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == preview::RowKind::Code)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert!(code.len() > 1);
    let body = source.lines().nth(1).unwrap();
    assert_eq!(
        r.selected_source((code[0], 0), (*code.last().unwrap(), usize::MAX)),
        body
    );
    let d = s.document.as_mut().unwrap();
    d.selection = Some(((code[0], 0), (*code.last().unwrap(), usize::MAX)));
    assert_eq!(d.selected_text().as_deref(), Some(body));
    let r = d.rendered().unwrap();
    let pair = r
        .projection
        .as_ref()
        .unwrap()
        .windows(2)
        .find(|pair| pair[0].original == pair[1].original)
        .unwrap();
    let original = body.chars().collect::<Vec<_>>();
    let split = pair[0].chars.end;
    let across = original[split - 2..split + 2].iter().collect::<String>();
    let (fragments, targets) = r
        .find(&across, preview::tools::SearchOptions::default())
        .unwrap();
    assert!(!targets.is_empty());
    assert!(
        fragments.len() > targets.len(),
        "cross-wrap search needs highlights on both rows"
    );
    let b = draw(&mut s);
    assert!(
        b.content
            .iter()
            .any(|c| c.fg != ratatui::style::Color::Reset)
    );
    key(&mut s, KeyCode::Char('w'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| r.projection.is_none())
    });
}

#[test]
fn source_code_wrapping_keeps_highlights_when_scrolling() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..180)
            .map(|i| format!("let long_{i} = \"{}\";\n", "word".repeat(30)))
            .collect::<String>()
            .to_string(),
    );
    key(&mut s, KeyCode::Char('m'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| !r.markdown)
    });
    key(&mut s, KeyCode::Char('w'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| r.projection.is_some())
    });
    s.document.as_mut().unwrap().scroll[0].0 = 100;
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .unwrap()
            .window
            .as_ref()
            .is_some_and(|w| w.line(100).is_some())
    });
    assert!(s.status().contains("Wrap"));
}

#[test]
fn navigation_accepts_lines_percent_and_reports_invalid_input() {
    let f = Fixture::new();
    let mut s = f.open(
        &(1..100)
            .map(|i| format!("## Section {i}\n\n"))
            .collect::<String>(),
    );
    key(&mut s, KeyCode::Char('g'));
    s.paste("999999").unwrap();
    key(&mut s, KeyCode::Enter);
    assert!(s.modal());
    assert!(text(&draw(&mut s)).contains("outside"));
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('g'));
    s.paste("51").unwrap();
    key(&mut s, KeyCode::Enter);
    assert!(!s.modal());
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > 10);
    assert!(s.status().contains("Ln 51/"));
    key(&mut s, KeyCode::Char('g'));
    s.paste("100%").unwrap();
    key(&mut s, KeyCode::Enter);
    ready(&mut s);
    assert!(!s.modal());
}

#[test]
fn enhanced_search_handles_unicode_overlap_regex_errors_and_options() {
    use preview::tools::{SearchOptions, search};
    let lines = vec!["aaa Cat cat scatter 中文中文".into()];
    assert_eq!(
        search(&lines, "aa", SearchOptions::default()).unwrap(),
        vec![(0, 0, 2), (0, 1, 3)]
    );
    assert_eq!(
        search(
            &lines,
            "cat",
            SearchOptions {
                whole_word: true,
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        2
    );
    assert_eq!(
        search(
            &lines,
            "cat",
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                regex: false
            }
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        search(&lines, "中文", SearchOptions::default())
            .unwrap()
            .len(),
        2
    );
    assert!(
        search(
            &lines,
            "[",
            SearchOptions {
                regex: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    let f = Fixture::new();
    let mut s = f.open("Cat cat scatter\n");
    key(&mut s, KeyCode::Char('/'));
    s.paste("cat").unwrap();
    assert_eq!(s.document.as_ref().unwrap().matches.len(), 3);
    s.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(s.document.as_ref().unwrap().matches.len(), 2);
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(s.document.as_ref().unwrap().matches.len(), 1);
    assert!(text(&draw(&mut s)).contains("Case [x]"));
    key(&mut s, KeyCode::Enter);
    key(&mut s, KeyCode::Esc);
    assert!(s.document.as_ref().unwrap().matches.is_empty());
    let position = s.document.as_ref().unwrap().offset();
    key(&mut s, KeyCode::Char('n'));
    assert_eq!(s.document.as_ref().unwrap().offset(), position);
}

#[test]
fn outline_opens_on_current_section_and_truncates_long_titles() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..40)
            .map(|i| format!("## Section {i}\n\nbody\n\n"))
            .collect::<String>(),
    );
    s.activate_reading(preview::ReadingAction::Link("#section-30".into()))
        .unwrap();
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().rendered().is_some_and(|r| {
            r.plain
                .get(s.document.as_ref().unwrap().offset().0)
                .is_some_and(|l| l == "Section 30")
        })
    });
    key(&mut s, KeyCode::Char('t'));
    assert!(text(&draw(&mut s)).contains(">   Section 30"));
    key(&mut s, KeyCode::Enter);
    assert!(!s.modal());
    let path = f.0.join("long.md");
    fs::write(&path, format!("## {}\n", "长标题".repeat(30))).unwrap();
    s.open(path, true).unwrap();
    ready(&mut s);
    key(&mut s, KeyCode::Char('t'));
    assert!(text(&draw(&mut s)).contains('…'));
}

#[test]
fn tabs_retain_positions_close_independently_and_guard_unsaved_edits() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..30)
            .map(|i| format!("## Part {i}\n\n"))
            .collect::<String>(),
    );
    s.activate_reading(preview::ReadingAction::Link("#part-10".into()))
        .unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > 5);
    fs::write(f.0.join("other.md"), "# Other\n").unwrap();
    s.open(f.0.join("other.md"), true).unwrap();
    ready(&mut s);
    let b = draw(&mut s);
    assert!(text(&b).contains("other.md"));
    s.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::ALT))
        .unwrap();
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    assert!(s.document.as_ref().unwrap().offset().0 > 5);
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.paste("unsaved").unwrap();
    s.open(f.0.join("other.md"), true).unwrap();
    assert!(s.modal());
    key(&mut s, KeyCode::Esc);
    assert!(s.dirty());
    s.close();
    assert!(s.modal());
    key(&mut s, KeyCode::Char('d'));
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().path.ends_with("other.md"));
    s.close();
    assert!(s.document.is_none());
}

#[test]
fn file_diff_shows_external_changes_without_modifying_disk() {
    let f = Fixture::new();
    let mut s = f.open("old line\nkept\n");
    fs::write(f.0.join("read.md"), "new line\nkept\nadded\n").unwrap();
    s.refresh_document().unwrap();
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .loaded()
            .is_some_and(|l| l.source.contains("new line"))
    });
    key(&mut s, KeyCode::Char('d'));
    wait(&mut s, |s| text(&draw_immutable(s)).contains("+2 -1"));
    let b = draw(&mut s);
    let value = text(&b);
    assert!(value.contains("-old line") && value.contains("+new line"));
    key(&mut s, KeyCode::Esc);
    assert!(!s.modal());
    assert_eq!(
        fs::read_to_string(f.0.join("read.md")).unwrap(),
        "new line\nkept\nadded\n"
    );
}

#[test]
fn file_diff_handles_clean_editor_reload_and_unsaved_conflicts() {
    for dirty in [false, true] {
        let f = Fixture::new();
        let mut s = f.open("old line\n");
        key(&mut s, KeyCode::Char('e'));
        wait(&mut s, |s| s.document.as_ref().unwrap().editing);
        if dirty {
            s.paste("LOCAL ").unwrap();
        }
        fs::write(f.0.join("read.md"), "disk line\n").unwrap();
        wait(&mut s, |s| {
            let d = s.document.as_ref().unwrap();
            if dirty {
                d.editor().unwrap().conflict
            } else {
                d.loaded().unwrap().source.contains("disk line")
            }
        });
        let local = s.document.as_ref().unwrap().editor().unwrap().source();
        s.menu();
        let m = s.menu.as_mut().unwrap();
        m.selected = m
            .items
            .iter()
            .position(|(l, _)| *l == "File changes [D]")
            .unwrap();
        key(&mut s, KeyCode::Enter);
        wait(&mut s, |s| text(&draw_immutable(s)).contains("+disk line"));
        let contents = text(&draw_immutable(&s));
        assert!(
            contents.contains(if dirty {
                "-LOCAL old line"
            } else {
                "-old line"
            }),
            "{contents}"
        );
        key(&mut s, KeyCode::Esc);
        assert_eq!(
            s.document.as_ref().unwrap().editor().unwrap().source(),
            local
        );
        assert_eq!(s.dirty(), dirty);
    }
}
fn draw_immutable(s: &Sidebar) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
    t.draw(|f| s.draw_overlay(f)).unwrap();
    t.backend().buffer().clone()
}

#[test]
fn tabs_are_bounded_and_header_click_switches_the_selected_file() {
    let f = Fixture::new();
    let mut s = f.open("initial");
    for i in 0..10 {
        let path = f.0.join(format!("file-{i}.md"));
        fs::write(&path, format!("# File {i}\n")).unwrap();
        s.open(path, true).unwrap();
        ready(&mut s);
    }
    s.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::ALT))
        .unwrap();
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().path.ends_with("file-2.md"));
    let b = draw(&mut s);
    let rect = s.geometry.preview.unwrap();
    let row = (rect.x + 1..rect.right() - 1)
        .map(|x| b[(x, rect.y + 2)].symbol())
        .collect::<String>();
    assert!(!row.contains("file-0") && !row.contains("file-1"));
    assert!(row.contains("file-2"));
    mouse(
        &mut s,
        rect.x + 2,
        rect.y + 2,
        MouseEventKind::Down(MouseButton::Left),
    );
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().path.ends_with("file-2.md"));
    for _ in 0..8 {
        s.close();
        if s.document.is_some() {
            ready(&mut s);
        }
    }
    assert!(s.document.is_none());
}

#[test]
fn file_tabs_have_boundaries_filename_labels_and_distinct_active_state() {
    use ratatui::style::Modifier;
    let f = Fixture::new();
    fs::create_dir(f.0.join("deep-directory")).unwrap();
    let path = f.0.join("deep-directory/文档-preview.md");
    fs::write(&path, "# Body marker\n").unwrap();
    for light in [false, true] {
        let mut s = f.open("first body");
        s.files.light = light;
        s.open(path.clone(), true).unwrap();
        ready(&mut s);
        let b = draw(&mut s);
        let rect = s.geometry.preview.unwrap();
        let mut row = String::new();
        let mut x = rect.x + 1;
        while x < rect.right() - 1 {
            let symbol = b[(x, rect.y + 2)].symbol();
            row.push_str(symbol);
            x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
        }
        assert!(row.contains("文档-preview.md"), "{row}");
        assert!(
            !row.contains("deep-directory"),
            "show the filename before its directory"
        );
        let starts = (rect.x + 1..rect.right() - 1)
            .filter(|&x| b[(x, rect.y + 2)].symbol() == "│")
            .collect::<Vec<_>>();
        assert_eq!(starts.len(), 4, "each tab has two side borders");
        assert_eq!(b[(starts[0], rect.y + 1)].symbol(), "┌");
        assert_eq!(b[(starts[2], rect.y + 1)].symbol(), "┌");
        let inactive = &b[(starts[0] + 1, rect.y + 2)];
        let active = &b[(starts[2] + 1, rect.y + 2)];
        assert_ne!(active.bg, inactive.bg);
        assert!(active.modifier.contains(Modifier::BOLD));
        assert_eq!(b[(starts[0], rect.y + 3)].symbol(), "└");
        assert_eq!(b[(starts[2], rect.y + 3)].symbol(), "└");
        assert_ne!(b[(starts[0], rect.y + 3)].fg, b[(starts[2], rect.y + 3)].fg);
        assert_eq!(
            b[(rect.x + 2, rect.y + 4)].symbol(),
            " ",
            "keep padding beneath tabs"
        );
        // The tab's lower edge is also clickable.
        mouse(
            &mut s,
            starts[0] + 2,
            rect.y + 3,
            MouseEventKind::Down(MouseButton::Left),
        );
        ready(&mut s);
        assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    }
}

#[test]
fn wrapped_markdown_and_source_comparison_keep_large_open_lazy() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..800)
            .map(|i| {
                format!(
                    "## Heading {i}\n\n```rust\nlet value = \"{}\";\n```\n\n",
                    "word".repeat(30)
                )
            })
            .collect::<String>(),
    );
    key(&mut s, KeyCode::Char('w'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| r.projection.is_some())
    });
    let r = s.document.as_ref().unwrap().rendered().unwrap();
    assert!(!r.complete);
    assert!(r.plain.len() < 300);
    key(&mut s, KeyCode::Char('v'));
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("## Heading 0")
    });
    assert!(!s.document.as_ref().unwrap().rendered().unwrap().complete);
}

#[test]
fn markdown_source_comparison_uses_unsaved_buffer_and_scrolls_both_directions() {
    let f = Fixture::new();
    let mut s = f.open(
        &(1..60)
            .map(|i| format!("## Heading {i}\n\nbody {i}\n\n"))
            .collect::<String>(),
    );
    key(&mut s, KeyCode::Char('v'));
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("## Heading 1")
    });
    key(&mut s, KeyCode::Char('g'));
    s.paste("81").unwrap();
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("## Heading 21")
    });
    let before = s.document.as_ref().unwrap().offset().0;
    mouse(&mut s, 90, 10, MouseEventKind::ScrollDown);
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > before);
    key(&mut s, KeyCode::Char('v'));
    ready(&mut s);
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.paste("UNSAVED_MARKER\n").unwrap();
    s.menu();
    let m = s.menu.as_mut().unwrap();
    m.selected = m
        .items
        .iter()
        .position(|(l, _)| *l == "Preview current content")
        .unwrap();
    key(&mut s, KeyCode::Enter);
    ready(&mut s);
    key(&mut s, KeyCode::Home);
    key(&mut s, KeyCode::Char('v'));
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("UNSAVED_MARKER")
    });
    assert!(
        !fs::read_to_string(f.0.join("read.md"))
            .unwrap()
            .contains("UNSAVED_MARKER")
    );
    mouse(&mut s, 3, 2, MouseEventKind::Down(MouseButton::Left));
}

fn menu_action(s: &mut Sidebar, label: &str) {
    s.menu();
    let m = s.menu.as_mut().unwrap();
    m.selected = m.items.iter().position(|(l, _)| *l == label).expect(label);
    key(s, KeyCode::Enter);
}
fn history(s: &mut Sidebar, forward: bool) {
    s.key(KeyEvent::new(
        if forward {
            KeyCode::Right
        } else {
            KeyCode::Left
        },
        KeyModifiers::ALT,
    ))
    .unwrap();
    ready(s);
}
fn row_text(b: &Buffer, y: u16) -> String {
    (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
}
fn tab_x(s: &mut Sidebar, name: &str) -> (u16, u16) {
    let b = draw(s);
    let y = s.geometry.preview.unwrap().y + 2;
    let line = row_text(&b, y);
    let byte = line.find(name).expect(name);
    (line[..byte].chars().count() as u16, y)
}

#[test]
fn toolbar_more_keeps_actions_and_filename_clear_at_narrow_widths() {
    let f = Fixture::new();
    let mut s = f.open("# Heading\n");
    for width in [62, 80, 120, 200] {
        s.update_layout(Rect::new(0, 0, width, 30), false);
        let b = draw(&mut s);
        let r = s.geometry.preview.unwrap();
        let header = row_text(&b, r.y);
        assert!(header.contains("[More]") && header.contains("[X]Close"));
        assert!(header.contains("read.md"));
        assert!(!header.contains("[V]") && !header.contains("[W]") && !header.contains("[D]"));
    }
    menu_action(&mut s, "Toggle code wrapping [W]");
    assert!(s.status().contains("Wrap"));
    s.menu();
    assert!(
        s.menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|(l, _)| *l == "File information [I]")
    );
}

#[test]
fn tabs_hover_show_full_path_and_close_by_button_middle_and_context() {
    let f = Fixture::new();
    let mut s = f.open("# One\n");
    fs::create_dir(f.0.join("child")).unwrap();
    let other = f.0.join("child/read.md");
    fs::write(&other, "# Two\n").unwrap();
    s.open(other.clone(), true).unwrap();
    ready(&mut s);
    let (x, y) = tab_x(&mut s, "read.md");
    mouse(&mut s, x, y, MouseEventKind::Moved);
    let b = draw(&mut s);
    assert!(text(&b).contains("Full path"));
    assert!(text(&b).contains(f.0.to_string_lossy().as_ref()));
    mouse(&mut s, 0, 29, MouseEventKind::Moved);
    assert!(!text(&draw(&mut s)).contains("Full path"));
    mouse(&mut s, x, y, MouseEventKind::Down(MouseButton::Right));
    assert_eq!(s.menu.as_ref().unwrap().target, f.0.join("read.md"));
    let m = s.menu.as_mut().unwrap();
    m.selected = m.items.iter().position(|(l, _)| *l == "Close tab").unwrap();
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.document.as_ref().unwrap().path, other);
    s.open(f.0.join("read.md"), true).unwrap();
    ready(&mut s);
    let (x, y) = tab_x(&mut s, "read.md");
    mouse(&mut s, x, y, MouseEventKind::Down(MouseButton::Middle));
    ready(&mut s);
    assert!(s.document.is_some());
    s.open(other, true).unwrap();
    ready(&mut s);
    let b = draw(&mut s);
    let y = s.geometry.preview.unwrap().y + 2;
    let x = (0..b.area.width)
        .find(|&x| b[(x, y)].symbol() == "×")
        .unwrap();
    mouse(&mut s, x, y, MouseEventKind::Down(MouseButton::Left));
    ready(&mut s);
    assert!(
        !text(&draw(&mut s)).contains('×'),
        "single file has no tab strip"
    );
}

#[test]
fn close_groups_keep_target_and_guard_dirty_active_document() {
    let f = Fixture::new();
    let mut s = f.open("# One\n");
    for name in ["two.md", "three.md"] {
        let p = f.0.join(name);
        fs::write(&p, "# Other\n").unwrap();
        s.open(p, true).unwrap();
        ready(&mut s);
    }
    // Right-click an inactive tab: closing its right side would remove the dirty active tab.
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.paste("LOCAL ").unwrap();
    s.menu();
    let m = s.menu.as_mut().unwrap();
    m.target = f.0.join("read.md");
    m.selected = m
        .items
        .iter()
        .position(|(l, _)| *l == "Close tabs to the right")
        .unwrap();
    key(&mut s, KeyCode::Enter);
    assert!(s.modal());
    key(&mut s, KeyCode::Esc);
    assert!(s.dirty());
    menu_action(&mut s, "Preview current content");
    ready(&mut s);
    menu_action(&mut s, "Close other tabs");
    assert!(s.dirty()); // current dirty document is retained
    assert!(!text(&draw(&mut s)).contains("two.md"));
    s.close();
    key(&mut s, KeyCode::Char('d'));
    assert!(s.document.is_none());
}

#[test]
fn tab_budget_reports_eviction() {
    let f = Fixture::new();
    let mut s = f.open("one\n");
    for i in 0..8 {
        let p = f.0.join(format!("{i}.txt"));
        fs::write(&p, "text\n").unwrap();
        s.open(p, true).unwrap();
        ready(&mut s);
    }
    assert!(s.notice.contains("8-tab limit"));
    assert!(s.notice.contains("read.md"));
}

#[test]
fn reading_history_supports_files_goto_search_forward_and_new_branch() {
    let f = Fixture::new();
    let mut s = f.open(
        &(1..=80)
            .map(|i| format!("line {i:02}\n"))
            .collect::<String>(),
    );
    key(&mut s, KeyCode::Char('m'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| !r.markdown)
    });
    let goto = |s: &mut Sidebar, n: &str| {
        key(s, KeyCode::Char('g'));
        s.paste(n).unwrap();
        key(s, KeyCode::Enter);
        ready(s);
    };
    goto(&mut s, "30");
    let target = s.document.as_ref().unwrap().offset().0;
    assert!(target > 15);
    history(&mut s, false);
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().offset().0, target);
    let other = f.0.join("other.txt");
    fs::write(&other, "other\n").unwrap();
    s.open(other.clone(), true).unwrap();
    ready(&mut s);
    history(&mut s, false);
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    assert_eq!(s.document.as_ref().unwrap().offset().0, target);
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().path, other);
    history(&mut s, false);
    key(&mut s, KeyCode::Char('/'));
    s.paste("line 60").unwrap();
    key(&mut s, KeyCode::Enter);
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().offset().0 > 40);
    history(&mut s, false);
    assert_eq!(s.document.as_ref().unwrap().offset().0, target);
    history(&mut s, true);
    assert!(s.document.as_ref().unwrap().offset().0 > 40);
    history(&mut s, false);
    goto(&mut s, "10");
    let before = s.document.as_ref().unwrap().offset();
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().offset(), before);
}

#[test]
fn heading_history_and_unsaved_cross_file_back_cancel_preserve_forward() {
    let f = Fixture::new();
    let mut s = f.open(
        &(0..30)
            .map(|i| format!("## Part {i}\n\nbody\n\n"))
            .collect::<String>(),
    );
    s.activate_reading(preview::ReadingAction::Link("#part-20".into()))
        .unwrap();
    ready(&mut s);
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > 15);
    let at = s.document.as_ref().unwrap().offset().0;
    history(&mut s, false);
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().offset().0, at);
    let other = f.0.join("other.md");
    fs::write(&other, "other\n").unwrap();
    s.open(other.clone(), true).unwrap();
    ready(&mut s);
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    s.paste("dirty ").unwrap();
    menu_action(&mut s, "Preview current content");
    ready(&mut s);
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    assert!(s.modal());
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.document.as_ref().unwrap().path, other);
    assert!(s.dirty());
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    key(&mut s, KeyCode::Char('d'));
    ready(&mut s);
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().path, other);
}

#[test]
fn diff_hunks_navigate_and_patch_remains_exact() {
    let original = (0..80).map(|i| format!("line {i}\n")).collect::<String>();
    let modified = original
        .replace("line 2\n", "first change\n")
        .replace("line 65\n", "second change\n");
    let diff = preview::tools::diff(&original, &modified);
    assert_eq!(diff.hunks.len(), 2);
    assert!(diff.patch.starts_with("--- previous\n+++ disk\n"));
    assert!(diff.patch.contains("+second change\n"));
    let f = Fixture::new();
    let mut s = f.open(&original);
    fs::write(f.0.join("read.md"), &modified).unwrap();
    s.refresh_document().unwrap();
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .loaded()
            .unwrap()
            .source
            .contains("second change")
    });
    key(&mut s, KeyCode::Char('d'));
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("+first change")
    });
    key(&mut s, KeyCode::Char('n'));
    key(&mut s, KeyCode::Char('n'));
    let b = draw(&mut s);
    assert!(text(&b).contains("+second change"));
    assert!(!text(&b).contains("+first change"));
    key(&mut s, KeyCode::Char('N'));
    assert!(text(&draw(&mut s)).contains("+first change"));
    key(&mut s, KeyCode::Esc);
    assert_eq!(fs::read_to_string(f.0.join("read.md")).unwrap(), modified);
}

#[test]
fn file_information_uses_disk_metadata_and_blocks_preview_input() {
    let f = Fixture::new();
    let mut s = f.open("# Text\r\n");
    key(&mut s, KeyCode::Char('i'));
    let contents = text(&draw(&mut s));
    for expected in [
        "File information",
        "UTF-8",
        "CRLF",
        "8 bytes",
        "Modified:",
        "UTC",
        "Truncated: No",
    ] {
        assert!(contents.contains(expected), "{expected}: {contents}");
    }
    let before = s.document.as_ref().unwrap().offset();
    key(&mut s, KeyCode::Down);
    assert_eq!(s.document.as_ref().unwrap().offset(), before);
    key(&mut s, KeyCode::Esc);
    assert!(!s.modal());
    let p = f.0.join("large.txt");
    fs::write(&p, "x".repeat(preview::MAX_BYTES + 10)).unwrap();
    s.open(p, true).unwrap();
    ready(&mut s);
    key(&mut s, KeyCode::Char('i'));
    assert!(text(&draw(&mut s)).contains("Truncated: Yes"));
    assert_eq!(
        s.document.as_ref().unwrap().loaded().unwrap().disk_bytes,
        (preview::MAX_BYTES + 10) as u64
    );
}

#[test]
fn bookmarks_survive_sidebar_restart_edit_delete_and_source_shifts() {
    if std::env::var_os("DEVTABS_BOOKMARK_CHILD").is_none() {
        let f = Fixture::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bookmarks_survive_sidebar_restart_edit_delete_and_source_shifts",
                "--nocapture",
            ])
            .env("DEVTABS_BOOKMARK_CHILD", "1")
            .env("DEVTABS_HOME", f.0.join("settings"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let f = Fixture::new();
    let source = (0..80).map(|i| format!("line {i}\n")).collect::<String>();
    let mut s = f.open(&source);
    key(&mut s, KeyCode::Char('m'));
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| !r.markdown)
    });
    key(&mut s, KeyCode::Char('g'));
    s.paste("40").unwrap();
    key(&mut s, KeyCode::Enter);
    ready(&mut s);
    key(&mut s, KeyCode::Char('b'));
    s.paste("继续阅读中文").unwrap();
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| text(&draw_immutable(s)).contains("1 bookmarks"));
    let saved = devtabs::bookmarks::update(
        &devtabs::config::home().join("bookmarks.json"),
        &f.0,
        devtabs::bookmarks::Operation::List,
    )
    .unwrap();
    assert_eq!(saved[0].note, "继续阅读中文");
    drop(s);
    fs::write(f.0.join("read.md"), format!("inserted\n{source}")).unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.visible = false;
    s.open(f.0.join("read.md"), true).unwrap();
    s.update_layout(Rect::new(0, 0, 120, 30), false);
    ready(&mut s);
    s.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
        .unwrap();
    wait(&mut s, |s| text(&draw_immutable(s)).contains("1 bookmarks"));
    key(&mut s, KeyCode::Char('e'));
    s.paste(" · updated").unwrap();
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| {
        text(&draw_immutable(s)).contains("updated")
            && text(&draw_immutable(s)).contains("1 bookmarks")
    });
    key(&mut s, KeyCode::Enter);
    ready(&mut s);
    assert_eq!(s.document.as_ref().unwrap().offset().0, 40);
    s.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
        .unwrap();
    wait(&mut s, |s| text(&draw_immutable(s)).contains("1 bookmarks"));
    key(&mut s, KeyCode::Delete);
    wait(&mut s, |s| text(&draw_immutable(s)).contains("0 bookmarks"));
    assert!(
        devtabs::bookmarks::update(
            &devtabs::config::home().join("bookmarks.json"),
            &f.0,
            devtabs::bookmarks::Operation::List
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn menu_popup_border_stays_above_header_at_small_heights() {
    let f = Fixture::new();
    let mut s = f.open("# Heading\n");
    s.update_layout(Rect::new(0, 0, 120, 12), false);
    s.menu();
    let b = draw(&mut s);
    // The large action menu fills the frame; the toolbar must not paint over it.
    assert_eq!(b[(28, 0)].symbol(), "┌");
    assert_eq!(b[(91, 0)].symbol(), "┐");
    let popup_header = (28..92).map(|x| b[(x, 0)].symbol()).collect::<String>();
    assert!(!popup_header.contains("[E]") && !popup_header.contains("[More]"));
}

#[test]
fn cancelled_empty_search_keeps_forward_and_deep_duplicate_names_disambiguate() {
    let f = Fixture::new();
    let mut s = f.open("# First\n\ntext\n\n# Second\n\ntext\n");
    s.activate_reading(preview::ReadingAction::Link("#second".into()))
        .unwrap();
    ready(&mut s);
    history(&mut s, false);
    key(&mut s, KeyCode::Char('/'));
    key(&mut s, KeyCode::Esc);
    history(&mut s, true);
    assert!(s.document.as_ref().unwrap().offset().0 > 0);
    for dir in ["a/src", "b/src"] {
        let parent = f.0.join(dir);
        fs::create_dir_all(&parent).unwrap();
        let p = parent.join("read.md");
        fs::write(&p, "# Other\n").unwrap();
        s.open(p, true).unwrap();
        ready(&mut s);
    }
    s.update_layout(Rect::new(0, 0, 180, 30), false);
    let b = draw(&mut s);
    let labels = row_text(&b, s.geometry.preview.unwrap().y + 2).replace('\\', "/");
    assert!(
        labels.contains("a/src") && labels.contains("b/src"),
        "{labels}"
    );
}

#[test]
fn deferred_markdown_search_records_history_after_search_prompt_closes() {
    let f = Fixture::new();
    let source = format!(
        "{}\nneedle-at-tail\n",
        (0..1200)
            .map(|i| format!("## Section {i}\n\n{}\n\n", "body ".repeat(10)))
            .collect::<String>()
    );
    let mut s = f.open(&source);
    assert!(!s.document.as_ref().unwrap().rendered().unwrap().complete);
    key(&mut s, KeyCode::Char('/'));
    s.paste("needle-at-tail").unwrap();
    key(&mut s, KeyCode::Enter);
    wait(&mut s, |s| s.document.as_ref().unwrap().offset().0 > 1000);
    let at = s.document.as_ref().unwrap().offset().0;
    history(&mut s, false);
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
    history(&mut s, true);
    assert_eq!(s.document.as_ref().unwrap().offset().0, at);
}

#[test]
fn more_and_tab_context_menus_support_mouse_selection() {
    let f = Fixture::new();
    let mut s = f.open("# First\n");
    let b = draw(&mut s);
    let y = s.geometry.preview.unwrap().y;
    let header = row_text(&b, y);
    let x = header[..header.find("[More]").unwrap()].chars().count() as u16;
    mouse(&mut s, x, y, MouseEventKind::Down(MouseButton::Left));
    assert!(s.menu.is_some());
    let index = s
        .menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|(l, _)| *l == "Toggle code wrapping [W]")
        .unwrap() as u16;
    mouse(
        &mut s,
        30,
        index + 1,
        MouseEventKind::Down(MouseButton::Left),
    );
    assert!(s.menu.is_none());
    assert!(s.status().contains("Wrap"));
    let other = f.0.join("other.md");
    fs::write(&other, "# Other\n").unwrap();
    s.open(other.clone(), true).unwrap();
    ready(&mut s);
    let (x, y) = tab_x(&mut s, "read.md");
    mouse(&mut s, x, y, MouseEventKind::Down(MouseButton::Right));
    mouse(&mut s, 30, 13, MouseEventKind::Down(MouseButton::Left));
    assert!(s.menu.is_none());
    assert_eq!(s.document.as_ref().unwrap().path, other);
    assert!(!text(&draw(&mut s)).contains('×'));
}

#[test]
fn clean_active_tab_has_no_dirty_marker_and_progress_stays_on_preview_edge() {
    let f = Fixture::new();
    let mut s = f.open("# First\n\nReading body\n");
    fs::write(f.0.join("other.md"), "# Other\n").unwrap();
    s.open(f.0.join("other.md"), true).unwrap();
    ready(&mut s);
    let b = draw(&mut s);
    let rect = s.geometry.preview.unwrap();
    let row = |y| {
        (rect.x..rect.right())
            .map(|x| b[(x, y)].symbol())
            .collect::<String>()
    };
    assert!(
        !row(rect.y + 2).contains('●'),
        "active selection is not an unsaved change"
    );
    assert!(row(rect.bottom() - 1).contains("Markdown"));
    s.focus = devtabs::sidebar::Focus::Terminal;
    let b = draw(&mut s);
    let header = (rect.x..rect.right())
        .map(|x| b[(x, rect.y)].symbol())
        .collect::<String>();
    assert!(
        !header.contains("[E]") && !header.contains("[Z]"),
        "blurred actions should be hidden: {header}"
    );
    assert!(header.contains("More") || header.contains("更多"));
    assert!(header.contains("other.md"));
}
