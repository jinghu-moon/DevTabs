use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use devtabs::{
    files::{self, Completed, Files, Key, Workers},
    preview,
    sidebar::{self, Action, Focus, Gesture, Sidebar},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-sidebar-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
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
fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}
fn until(label: &str, mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < Duration::from_secs(15), "{label}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn pump(s: &mut Sidebar) {
    s.tick(true);
}
fn load_tree(f: &mut Files, io: &Workers, p: &Path) {
    f.request(p, false, io).unwrap();
    let output = io.rx.recv_timeout(Duration::from_secs(10)).unwrap();
    if let Completed::Directory {
        path,
        generation,
        batch,
    } = output
    {
        f.apply(path, generation, batch);
    } else {
        panic!("wrong completion");
    }
}
fn rendered(s: &mut Sidebar, path: &Path) {
    until("document rendered", || {
        pump(s);
        s.document
            .as_ref()
            .is_some_and(|d| d.path == path && d.rendered().is_some())
    });
}

#[test]
fn layout_examples_and_all_boundary_combinations() {
    for (w, split, want) in [
        (200, false, vec![28, 86, 86, 0]),
        (200, true, vec![28, 62, 59, 51]),
        (160, false, vec![28, 66, 66, 0]),
        (160, true, vec![28, 93, 93, 39]),
        (120, false, vec![28, 92, 92, 0]),
        (120, true, vec![26, 62, 62, 32]),
        (80, false, vec![0, 80, 80, 0]),
    ] {
        let p = sidebar::panels(Rect::new(0, 0, w, 30), split, true, true);
        assert_eq!(
            vec![
                p.tree.map_or(0, |r| r.width),
                p.agent.unwrap().width,
                p.preview.unwrap().width,
                p.debug.map_or(0, |r| r.width)
            ],
            want
        );
    }
    assert!(
        sidebar::panels(Rect::new(0, 0, 80, 30), true, true, true)
            .agent
            .is_none()
    );
    for w in 0..400 {
        for h in [0, 10, 11, 12, 30] {
            for split in [false, true] {
                for side in [false, true] {
                    for preview in [false, true] {
                        let p = sidebar::panels(Rect::new(0, 0, w, h), split, side, preview);
                        if let Some(a) = p.agent {
                            assert!(a.width >= 62);
                            assert_eq!(a.bottom(), h - 1, "exactly one global footer row");
                            let sum = p.tree.map_or(0, |r| r.width)
                                + a.width
                                + p.debug.map_or(0, |r| r.width)
                                + if p.overlay {
                                    0
                                } else {
                                    p.preview.map_or(0, |r| r.width)
                                };
                            assert_eq!(sum, w);
                            if let Some(r) = p.preview
                                && !p.overlay
                            {
                                assert!(r.width >= 42);
                                assert!(r.x >= a.right());
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn lazy_tree_respects_ignore_and_collapse_changes_visible_selection() {
    let f = Fixture::new();
    fs::create_dir(f.path(".git")).unwrap();
    fs::create_dir(f.path("src")).unwrap();
    fs::write(f.path(".gitignore"), "*.tmp\n!keep.tmp\n").unwrap();
    fs::write(f.path("discard.tmp"), "").unwrap();
    fs::write(f.path("keep.tmp"), "").unwrap();
    fs::write(f.path(".env"), "x").unwrap();
    fs::write(f.path("src/中文.rs"), "fn main() {}\n").unwrap();
    let mut files = Files::new(f.0.clone());
    let io = Workers::new();
    load_tree(&mut files, &io, &f.0);
    assert!(files.nodes.contains_key(&f.path(".env")));
    assert!(files.nodes.contains_key(&f.path("keep.tmp")));
    assert!(!files.nodes.contains_key(&f.path("discard.tmp")));
    assert!(!files.nodes.contains_key(&f.path(".git")));
    assert!(!files.nodes.contains_key(&f.path("src/中文.rs")));
    let src = f.path("src");
    files.state.open(files.identifier(&src));
    load_tree(&mut files, &io, &src);
    files.state.select(files.identifier(&f.path("src/中文.rs")));
    files.collapse(&io);
    assert_eq!(files.selected_path(), Some(src));
    assert!(files.selected_file().is_none());
}

#[test]
fn directory_batches_are_bounded_sorted_and_stale_results_cannot_return() {
    let f = Fixture::new();
    for n in 0..1100 {
        fs::write(f.path(&format!("file{n:04}.rs")), "").unwrap();
    }
    let mut files = Files::new(f.0.clone());
    let io = Workers::new();
    load_tree(&mut files, &io, &f.0);
    assert!(files.nodes.len() <= 501);
    assert!(!files.dirs[&f.0].complete);
    while !files.dirs[&f.0].complete {
        load_tree(&mut files, &io, &f.0);
    }
    assert_eq!(files.nodes.len(), 1101);
    assert_eq!(
        files.dirs[&f.0].children.first(),
        Some(&f.path("file0000.rs"))
    );
    let old = files.dirs[&f.0].generation;
    files.refresh(&io).unwrap();
    files.apply(f.0.clone(), old, Ok((vec![], true, 0)));
    assert!(!files.dirs[&f.0].complete);
}

#[test]
fn bounded_decoding_encoding_binary_and_control_characters() {
    let f = Fixture::new();
    let text = "中文😀\r\nlast";
    for (name, bom, be) in [
        ("le.txt", vec![255, 254], false),
        ("be.txt", vec![254, 255], true),
    ] {
        let mut bytes = bom;
        for u in text.encode_utf16() {
            bytes.extend(if be { u.to_be_bytes() } else { u.to_le_bytes() });
        }
        fs::write(f.path(name), bytes).unwrap();
        let d = preview::read(&f.path(name)).unwrap();
        assert_eq!(d.source.as_ref(), text);
        assert!(d.editable);
        assert_eq!(d.newline, "\r\n");
    }
    fs::write(f.path("binary"), [0, 1, 2]).unwrap();
    assert!(preview::read(&f.path("binary")).is_err());
    fs::write(f.path("long"), "a".repeat(preview::MAX_LINE + 1)).unwrap();
    let d = preview::read(&f.path("long")).unwrap();
    assert!(d.truncated && !d.editable);
    fs::write(f.path("huge"), "中".repeat(preview::MAX_BYTES / 3 + 1)).unwrap();
    assert!(preview::read(&f.path("huge")).unwrap().truncated);
    let safe = preview::safe_text("\x1b]52;c;abc\x07\t中文");
    assert!(!safe.contains('\x1b') && !safe.contains('\x07'));
    assert_eq!(preview::copy_text("a\tb\n"), "a\tb\n");
    assert!(files::path_input(Path::new("relative.md")).is_err());
    assert_eq!(
        files::path_input(&f.path("中文 & $.md")).unwrap(),
        format!("\"{}\" ", f.path("中文 & $.md").display())
    );
}

#[test]
fn markdown_modes_search_and_async_navigation_keep_latest_file() {
    let f = Fixture::new();
    let a = f.path("a.rs");
    let b = f.path("b.md");
    fs::write(&a, "fn main() {}\n").unwrap();
    fs::write(&b,"# 中文标题\n\n| 字段 | 含义 |\n| --- | --- |\n| A | 文本 |\n\n![图](image.png)\n\n```rust\nfn main() {}\n```\n").unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    s.open(a, false).unwrap();
    s.open(b.clone(), true).unwrap();
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    rendered(&mut s, &b);
    let d = s.document.as_ref().unwrap();
    assert!(d.markdown);
    assert!(d.rendered().unwrap().plain.join("\n").contains("中文标题"));
    s.key(key(KeyCode::Char('/'))).unwrap();
    s.paste("文本").unwrap();
    s.key(key(KeyCode::Enter)).unwrap();
    assert!(!s.document.as_ref().unwrap().matches.is_empty());
    s.key(key(KeyCode::Char('m'))).unwrap();
    until("source rendered", || {
        pump(&mut s);
        s.document
            .as_ref()
            .unwrap()
            .rendered()
            .is_some_and(|r| !r.markdown)
    });
    assert!(s.document.as_ref().unwrap().rendered().unwrap().plain[0].contains("# 中文标题"));
}

#[test]
fn drag_from_tree_survives_overlay_transition_and_never_targets_debug() {
    let f = Fixture::new();
    let p = f.path("drag.md");
    fs::write(&p, "# Drag\n").unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 120, 30), false);
    until("tree loaded", || {
        pump(&mut s);
        s.files.nodes.contains_key(&p)
    });
    s.open(p.clone(), false).unwrap();
    s.update_layout(Rect::new(0, 0, 120, 30), false);
    rendered(&mut s, &p);
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term.draw(|frame| s.draw(frame)).unwrap();
    let hit = (1..28)
        .find_map(|x| {
            (1..27).find_map(|y| {
                s.files
                    .state
                    .rendered_at((x, y).into())
                    .and_then(|id| (id.last() == Some(&Key::Entry(p.clone()))).then_some((x, y)))
            })
        })
        .unwrap();
    s.mouse(mouse(MouseEventKind::Down(MouseButton::Left), hit.0, hit.1))
        .unwrap();
    s.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 60, 10))
        .unwrap();
    assert!(matches!(s.gesture, Gesture::Dragging { .. }));
    assert!(!s.preview_visible);
    s.update_layout(Rect::new(0, 0, 120, 30), false);
    assert!(matches!(s.gesture, Gesture::Dragging { .. }));
    let (_, action) = s
        .mouse(mouse(MouseEventKind::Up(MouseButton::Left), 60, 10))
        .unwrap();
    assert!(matches!(action,Action::Insert(ref paths) if paths==&vec![p.clone()]));
    assert!(matches!(s.gesture, Gesture::Idle));
    assert_eq!(s.focus, Focus::Terminal);
    s.gesture = Gesture::Dragging { path: p };
    let (_, action) = s
        .mouse(mouse(MouseEventKind::Up(MouseButton::Left), 0, 0))
        .unwrap();
    assert!(matches!(action, Action::None));
}

#[test]
fn watch_refresh_preserves_scroll_and_keeps_stale_content_on_delete() {
    let f = Fixture::new();
    let p = f.path("a.txt");
    fs::write(
        &p,
        (0..50).map(|n| format!("line{n}\n")).collect::<String>(),
    )
    .unwrap();
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    s.open(p.clone(), true).unwrap();
    s.update_layout(Rect::new(0, 0, 160, 30), false);
    rendered(&mut s, &p);
    s.document.as_mut().unwrap().scroll[0] = (20, 0);
    fs::write(&p, "replacement\n").unwrap();
    until("watch replacement", || {
        pump(&mut s);
        s.document
            .as_ref()
            .unwrap()
            .loaded()
            .is_some_and(|l| l.source.as_ref() == "replacement\n")
    });
    until("replacement rendered", || {
        pump(&mut s);
        s.document.as_ref().unwrap().rendered().is_some()
    });
    assert_eq!(s.document.as_ref().unwrap().offset().0, 0);
    fs::remove_file(&p).unwrap();
    until("watch delete", || {
        pump(&mut s);
        !s.document.as_ref().unwrap().stale.is_empty()
    });
    assert_eq!(
        s.document
            .as_ref()
            .unwrap()
            .loaded()
            .unwrap()
            .source
            .as_ref(),
        "replacement\n"
    );
}

#[test]
#[ignore = "release-only large tree and renderer timings"]
fn release_large_directory_and_previews() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let f = Fixture::new();
    for n in 0..30_000 {
        fs::write(f.path(&format!("file{n:05}.rs")), "fn main() {}\n").unwrap();
    }
    let mut s = Sidebar::new(f.0.clone());
    s.update_layout(Rect::new(0, 0, 200, 40), false);
    let mut batch_ms = Vec::new();
    let start = Instant::now();
    while s.files.nodes.len() < files::MAX_NODES {
        let begin = Instant::now();
        load_tree(&mut s.files, &s.io, &f.0);
        batch_ms.push(begin.elapsed().as_secs_f64() * 1000.0);
    }
    assert_eq!(s.files.nodes.len(), files::MAX_NODES);
    assert!(!s.files.dirs[&f.0].complete);
    let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();
    let draw = Instant::now();
    terminal.draw(|frame| s.draw(frame)).unwrap();
    let draw_ms = draw.elapsed().as_secs_f64() * 1000.0;
    let markdown="# 中文说明\n\n| 字段 | 意义 |\n| --- | --- |\n| A | 内容 |\n\n```rust\nfn main() {}\n```\n".repeat(100);
    let cold = Instant::now();
    let code = preview::render(&markdown, &f.path("a.rs"), false, 80, false);
    let code_cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let cold = Instant::now();
    let rendered = preview::render(&markdown, &f.path("a.md"), true, 80, false);
    let markdown_cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let warm = Instant::now();
    let _ = preview::render(&markdown, &f.path("a.md"), true, 80, false);
    let markdown_warm_ms = warm.elapsed().as_secs_f64() * 1000.0;
    let report = serde_json::json!({"directory_entries":30000,"loaded_nodes":s.files.nodes.len(),"load_total_ms":start.elapsed().as_secs_f64()*1000.0,"batch_ms":batch_ms,"tree_draw_ms":draw_ms,"code_cold_ms":code_cold_ms,"markdown_cold_ms":markdown_cold_ms,"markdown_warm_ms":markdown_warm_ms,"code_lines":code.plain.len(),"rendered_lines":rendered.plain.len()});
    fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/sidebar-large-release.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
    assert!(draw_ms <= 50.0, "tree redraw exceeds 50ms");
    assert!(
        batch_ms.iter().all(|ms| *ms <= 200.0),
        "directory batch exceeds 200ms"
    );
}
