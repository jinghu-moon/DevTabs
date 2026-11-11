use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devtabs::{
    editor::{Editor, Save, encode, save},
    preview::{self, Encoding},
    sidebar::{Command, Focus, Sidebar},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use ratatui_textarea::CursorMove;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-editor-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn until(label: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < Duration::from_secs(10), "{label}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn request(e: &Editor) -> Save {
    Save {
        snapshot: e.loaded.clone(),
        source: e.source(),
        revision: e.revision,
    }
}

#[test]
fn byte_exact_round_trip_all_encodings_newlines_and_endings() {
    let f = Fixture::new();
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf8Bom,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for newline in ["\n", "\r\n"] {
            for ending in [false, true] {
                let p = f.file("file.txt");
                let text = format!("中文😀{newline}last{}", if ending { newline } else { "" });
                let bytes = encode(&text, encoding);
                fs::write(&p, &bytes).unwrap();
                let mut e = Editor::new(preview::read(&p).unwrap()).unwrap();
                assert_eq!(encode(&e.source(), encoding), bytes);
                let result = save(&request(&e)).unwrap();
                assert_eq!(result.bytes, bytes);
                e.mark_saved(result, e.text());
                assert!(!e.dirty());
                e.area.move_cursor(CursorMove::End);
                e.paste(" +编辑").unwrap();
                let expected = encode(&e.source(), encoding);
                let source = e.text();
                let result = save(&request(&e)).unwrap();
                e.mark_saved(result, source);
                assert_eq!(fs::read(&p).unwrap(), expected);
                assert!(!e.dirty());
                e.undo().unwrap();
                assert!(e.dirty());
                e.redo().unwrap();
                assert!(!e.dirty());
            }
        }
    }
    let p = f.file("empty");
    fs::write(&p, []).unwrap();
    let e = Editor::new(preview::read(&p).unwrap()).unwrap();
    assert_eq!(encode(&e.source(), e.loaded.encoding), Vec::<u8>::new());
}

#[test]
fn edits_preserve_history_selection_and_reject_oversized_paste_atomically() {
    let f = Fixture::new();
    let p = f.file("a.txt");
    fs::write(&p, "原文\nnext").unwrap();
    let mut e = Editor::new(preview::read(&p).unwrap()).unwrap();
    e.area.move_cursor(CursorMove::Jump(0, 2));
    e.paste("中文😀").unwrap();
    assert_eq!(e.text(), "原文中文😀\nnext");
    e.undo().unwrap();
    assert!(!e.dirty());
    e.redo().unwrap();
    assert!(e.dirty());
    let before = e.text();
    assert!(e.paste(&"x".repeat(preview::MAX_BYTES + 1)).is_err());
    assert_eq!(e.text(), before);
    e.undo().unwrap();
    assert!(!e.dirty());
    e.area.move_cursor(CursorMove::Jump(0, 0));
    e.area.start_selection();
    e.area.move_cursor(CursorMove::Jump(1, 0));
    e.indent(false).unwrap();
    assert_eq!(e.text(), "\t原文\nnext");
    e.indent(true).unwrap();
    assert_eq!(e.text(), "原文\nnext");
    for c in ['m', 'o', 'i', 'e', '/', '.'] {
        e.key(key(KeyCode::Char(c))).unwrap();
    }
    assert!(e.text().contains("moie/."));
}

#[test]
fn external_same_length_changes_and_atomic_replacement_are_conflicts() {
    let f = Fixture::new();
    let p = f.file("x.txt");
    fs::write(&p, "original").unwrap();
    let mut e = Editor::new(preview::read(&p).unwrap()).unwrap();
    e.paste("mine").unwrap();
    fs::write(&p, "external").unwrap();
    assert!(save(&request(&e)).is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), "external");
    assert!(e.dirty());
    fs::write(&p, "original").unwrap();
    let r = request(&e);
    let replacement = f.file("new");
    fs::write(&replacement, "original").unwrap();
    fs::remove_file(&p).unwrap();
    fs::rename(&replacement, &p).unwrap();
    assert!(
        save(&r).is_err(),
        "same bytes with a new identity must conflict"
    );
    assert_eq!(fs::read_to_string(&p).unwrap(), "original");
    fs::remove_file(&p).unwrap();
    assert!(save(&r).is_err());
    assert!(!p.exists());
    assert!(e.dirty());
}

#[test]
fn readonly_and_hardlinked_files_cannot_be_saved() {
    let f = Fixture::new();
    let p = f.file("x.txt");
    fs::write(&p, "original").unwrap();
    let loaded = preview::read(&p).unwrap();
    let mut e = Editor::new(loaded).unwrap();
    e.paste("mine").unwrap();
    let original = fs::metadata(&p).unwrap().permissions();
    let mut perm = original.clone();
    perm.set_readonly(true);
    fs::set_permissions(&p, perm).unwrap();
    assert!(save(&request(&e)).is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), "original");
    fs::set_permissions(&p, original).unwrap();
    let link = f.file("link.txt");
    fs::hard_link(&p, &link).unwrap();
    assert!(Editor::new(preview::read(&p).unwrap()).is_err());
    assert!(save(&request(&e)).is_err());
}

#[test]
fn concurrent_devtabs_saves_allow_exactly_one_writer() {
    let f = Fixture::new();
    let p = f.file("x.txt");
    fs::write(&p, "original").unwrap();
    let loaded = preview::read(&p).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut handles = vec![];
    for text in ["writer A", "writer B"] {
        let mut e = Editor::new(loaded.clone()).unwrap();
        e.paste(text).unwrap();
        let r = request(&e);
        let b = barrier.clone();
        handles.push(thread::spawn(move || {
            b.wait();
            save(&r).is_ok()
        }));
    }
    barrier.wait();
    let wins = handles
        .into_iter()
        .map(|h| usize::from(h.join().unwrap()))
        .sum::<usize>();
    assert_eq!(wins, 1);
    assert!(fs::read_to_string(&p).unwrap().starts_with("writer "));
}

#[test]
fn concurrent_readers_observe_complete_old_or_new_file_during_saves() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let f = Fixture::new();
    let path = f.file("atomic.txt");
    let a = format!("{}\n", "a".repeat(126)).repeat(1000);
    let b = format!("{}\n", "b".repeat(126)).repeat(1000);
    fs::write(&path, &a).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = {
        let path = path.clone();
        let stop = stop.clone();
        let reads = reads.clone();
        let a = a.clone();
        let b = b.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // Test the supported DevTabs reader contract; raw opens may
                // briefly return NotFound on this Windows replacement API.
                let data = preview::read(&path)
                    .expect("DevTabs bounded reader must survive replace window")
                    .bytes;
                assert!(
                    data == a.as_bytes() || data == b.as_bytes(),
                    "atomic replacement exposed incomplete file"
                );
                reads.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        })
    };
    let mut e = Editor::new(preview::read(&path).unwrap()).unwrap();
    for n in 0..10 {
        e.area.select_all();
        e.paste(if n % 2 == 0 { &b } else { &a }).unwrap();
        let loaded = save(&request(&e)).unwrap();
        e.mark_saved(loaded, e.text());
    }
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();
    assert!(reads.load(Ordering::Relaxed) > 10);
    assert_eq!(fs::read(path).unwrap(), a.as_bytes());
}

#[test]
fn real_file_share_blocks_direct_writer_and_save_failure_preserves_original() {
    #[cfg(windows)]
    {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        let f = Fixture::new();
        let p = f.file("x.txt");
        fs::write(&p, "original").unwrap();
        let mut e = Editor::new(preview::read(&p).unwrap()).unwrap();
        e.paste("mine").unwrap();
        let _reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&p)
            .unwrap();
        assert!(save(&request(&e)).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "original");
        assert!(e.dirty());
        assert!(fs::read_dir(&f.0).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".devtabs-")
        }));
    }
}

#[test]
fn panel_edit_projection_unsaved_cancel_save_and_external_conflict() {
    let f = Fixture::new();
    let p = f.file("a.md");
    let other = f.file("b.txt");
    fs::write(&p, "# 原文\n").unwrap();
    fs::write(&other, "other").unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    s.open(p.clone(), true).unwrap();
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    until("preview ready", || {
        s.tick(true);
        s.document.as_ref().is_some_and(|d| d.rendered().is_some())
    });
    s.key(key(KeyCode::Char('e'))).unwrap();
    until("editor ready", || {
        s.tick(true);
        s.document.as_ref().is_some_and(|d| d.editor().is_some())
    });
    assert_eq!(s.focus, Focus::Editor);
    s.paste("新内容").unwrap();
    assert!(s.dirty());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal.draw(|f| s.draw(f)).unwrap();
    s.open(other.clone(), true).unwrap();
    assert!(s.modal());
    s.key(key(KeyCode::Esc)).unwrap();
    assert_eq!(s.document.as_ref().unwrap().path, p);
    assert!(s.dirty());
    s.key(ctrl('s')).unwrap();
    until("saved", || {
        s.tick(true);
        !s.dirty()
    });
    assert!(fs::read_to_string(&p).unwrap().starts_with("新内容"));
    s.paste("local").unwrap();
    fs::write(&p, "external\n").unwrap();
    until("conflict marked", || {
        s.tick(true);
        s.document.as_ref().unwrap().editor().unwrap().conflict
    });
    assert!(
        s.document
            .as_ref()
            .unwrap()
            .editor()
            .unwrap()
            .text()
            .contains("local")
    );
    s.key(ctrl('s')).unwrap();
    until("save refused", || {
        s.tick(true);
        !s.document.as_ref().unwrap().editor().unwrap().saving
    });
    assert_eq!(fs::read_to_string(&p).unwrap(), "external\n");
    s.open(other.clone(), true).unwrap();
    s.key(key(KeyCode::Char('d'))).unwrap();
    until("other loaded", || {
        s.tick(true);
        s.document
            .as_ref()
            .is_some_and(|d| d.path == other && d.loaded().is_some())
    });
    assert!(!s.dirty());
}

#[test]
fn newline_and_control_boundaries_remain_preview_only() {
    let f = Fixture::new();
    for (name, text) in [
        ("mixed", "a\r\nb\nc"),
        ("cr", "a\rb"),
        ("control", "a\x1bb"),
    ] {
        let p = f.file(name);
        fs::write(&p, text).unwrap();
        assert!(Editor::new(preview::read(&p).unwrap()).is_err());
    }
    assert!(devtabs::editor::eligible(Path::new("\\\\server\\share\\file")).is_err());
}
#[test]
fn preview_budget_does_not_imply_edit_budget_and_large_paste_is_atomic() {
    let f = Fixture::new();
    let p = f.file("budget.txt");
    fs::write(&p, format!("{}\n", "x".repeat(8192)).repeat(65)).unwrap();
    let loaded = preview::read(&p).unwrap();
    assert!(!loaded.editable && !loaded.truncated);
    assert!(loaded.note.contains("512 KiB"));
    assert!(Editor::new(loaded).is_err());
    fs::write(&p, "original").unwrap();
    let mut e = Editor::new(preview::read(&p).unwrap()).unwrap();
    let text = format!("{}\n", "x".repeat(126)).repeat(4200);
    assert!(e.paste(&text).is_err());
    assert_eq!(e.text(), "original");
    assert!(!e.dirty());
}

fn menu(s: &mut Sidebar, select: impl Fn(&Command) -> bool) {
    s.key(key(KeyCode::F(2))).unwrap();
    let m = s.menu.as_mut().unwrap();
    m.selected = m
        .items
        .iter()
        .position(|(_, c)| select(c))
        .expect("required menu operation");
    s.key(key(KeyCode::Enter)).unwrap();
}
#[test]
fn markdown_projection_menus_and_save_revision_keep_single_buffer() {
    let f = Fixture::new();
    let p = f.file("buffer.md");
    let other = f.file("other.txt");
    fs::write(&p, "# original\n").unwrap();
    fs::write(&other, "other").unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    s.open(p.clone(), true).unwrap();
    until("loaded", || {
        s.tick(true);
        s.document.as_ref().unwrap().loaded().is_some()
    });
    menu(&mut s, |c| matches!(c, Command::Edit));
    until("editing", || {
        s.tick(true);
        s.document.as_ref().unwrap().editor().is_some()
    });
    s.paste("# UNSAVED\n").unwrap();
    let cursor = s.document.as_ref().unwrap().editor().unwrap().area.cursor();
    menu(&mut s, |c| matches!(c, Command::Project));
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    until("buffer projection", || {
        s.tick(true);
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| r.markdown && r.plain.iter().any(|l| l.contains("UNSAVED")))
    });
    assert_eq!(fs::read_to_string(&p).unwrap(), "# original\n");
    s.key(key(KeyCode::Char('e'))).unwrap();
    assert_eq!(
        s.document.as_ref().unwrap().editor().unwrap().area.cursor(),
        cursor
    );
    s.open(other.clone(), true).unwrap();
    s.key(key(KeyCode::Char('s'))).unwrap(); // save then navigate
    s.save().unwrap(); // coalesce while already saving
    s.paste("newer").unwrap(); // mutate after the saved snapshot, before completion
    until("save completes even when sidebar inactive", || {
        s.tick(false);
        !s.document.as_ref().unwrap().editor().unwrap().saving
    });
    assert_eq!(fs::read_to_string(&p).unwrap(), "# UNSAVED\n# original\n");
    assert!(s.dirty() && s.modal());
    assert_eq!(s.document.as_ref().unwrap().path, p);
    s.key(key(KeyCode::Esc)).unwrap();
    s.key(ctrl('f')).unwrap();
    s.paste("newer").unwrap();
    assert!(
        s.document
            .as_ref()
            .unwrap()
            .editor()
            .unwrap()
            .area
            .search_pattern()
            .is_some()
    );
    s.key(key(KeyCode::Esc)).unwrap();
    s.open(other.clone(), true).unwrap();
    s.key(key(KeyCode::Char('d'))).unwrap();
    until("navigated", || {
        s.tick(true);
        s.document.as_ref().unwrap().path == other
            && s.document.as_ref().unwrap().loaded().is_some()
    });
}

#[test]
fn visible_panel_controls_toggle_restore_edit_and_guard_close() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    fn click(s: &mut Sidebar, t: &mut Terminal<TestBackend>, label: &str) {
        t.draw(|f| s.draw(f)).unwrap();
        let b = t.backend().buffer();
        let chars = label.chars().map(|c| c.to_string()).collect::<Vec<_>>();
        let mut found = None;
        for y in 0..b.area.height {
            for x in 0..b.area.width.saturating_sub(chars.len() as u16) {
                if chars
                    .iter()
                    .enumerate()
                    .all(|(i, c)| b[(x + i as u16, y)].symbol() == c)
                {
                    found = Some((x, y));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (column, row) = found.expect("visible control");
        assert!(
            s.mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE
            })
            .unwrap()
            .0
        );
    }
    let f = Fixture::new();
    let p = f.file("button.md");
    fs::write(&p, "# original\n").unwrap();
    let mut s = Sidebar::new(f.0.clone());
    let area = Rect::new(0, 0, 160, 30);
    s.update_layout(area, false);
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    click(&mut s, &mut terminal, "[Files]");
    s.update_layout(area, false);
    assert!(s.geometry.tree.is_none());
    click(&mut s, &mut terminal, "[Files]");
    s.update_layout(area, false);
    assert!(s.geometry.tree.is_some());
    s.open(p.clone(), true).unwrap();
    s.update_layout(area, false);
    until("preview ready", || {
        s.tick(true);
        s.document.as_ref().unwrap().loaded().is_some()
    });
    click(&mut s, &mut terminal, "[E]");
    until("edit button", || {
        s.tick(true);
        s.document.as_ref().unwrap().editor().is_some()
    });
    s.paste("local").unwrap();
    click(&mut s, &mut terminal, "[X]");
    assert!(s.modal());
    s.key(key(KeyCode::Esc)).unwrap();
    assert!(s.dirty());
    assert_eq!(fs::read_to_string(p).unwrap(), "# original\n");
}

#[test]
fn navigation_changes_cursor_and_selection_without_mutating_history_or_revision() {
    let f = Fixture::new();
    let path = f.file("navigation.txt");
    fs::write(&path, "中文😀\nsecond\nthird").unwrap();
    let mut e = Editor::new(preview::read(&path).unwrap()).unwrap();
    let revision = e.revision;
    e.key(key(KeyCode::Down)).unwrap();
    assert_eq!(e.area.cursor().0, 1);
    e.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(e.selected().as_deref(), Some("s"));
    e.key(key(KeyCode::End)).unwrap();
    e.key(key(KeyCode::Up)).unwrap();
    assert_eq!(e.revision, revision);
    assert!(!e.dirty());
    e.paste("insert").unwrap();
    e.key(key(KeyCode::Left)).unwrap();
    e.undo().unwrap();
    assert!(!e.dirty());
    assert_eq!(e.text(), "中文😀\nsecond\nthird");
    e.redo().unwrap();
    assert!(e.text().contains("insert"));
}
