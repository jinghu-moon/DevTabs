//! Backend and TestBackend event tests. No interactive terminal is opened.
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers as Mods, MouseButton, MouseEvent, MouseEventKind as Mouse,
};
use devtabs::{
    explorer::{self, Completed, Operation, Scope, Status, Worker},
    files::{Files, Key},
    sidebar::{Action, Command, Focus, Sidebar},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("DevTabs-explorer-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
    fn write(&self, p: &str, text: &str) -> PathBuf {
        let path = self.path(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn key(s: &mut Sidebar, code: KeyCode, modifiers: Mods) {
    s.key(KeyEvent::new(code, modifiers)).unwrap();
}
fn mouse(s: &mut Sidebar, kind: Mouse, x: u16, y: u16, modifiers: Mods) -> Action {
    s.mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers,
    })
    .unwrap()
    .1
}
#[track_caller]
fn wait(s: &mut Sidebar, mut ready: impl FnMut(&Sidebar) -> bool) {
    let start = Instant::now();
    loop {
        s.tick(true);
        if ready(s) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Timed out: {} · focus {:?} busy {} modal {} document {:?}",
            s.notice,
            s.focus,
            s.project_busy(),
            s.modal(),
            s.document.as_ref().map(|d| (&d.path, d.loading, d.editing))
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn draw(s: &mut Sidebar) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(180, 32)).unwrap();
    s.update_layout(Rect::new(0, 0, 180, 32), false);
    t.draw(|f| s.draw(f)).unwrap();
    t.backend().buffer().clone()
}
fn hit(s: &Sidebar, p: &Path) -> (u16, u16) {
    let r = s.geometry.tree.unwrap();
    for y in r.y + 1..r.bottom() - 1 {
        if s.files
            .state
            .rendered_at((r.x + 8, y).into())
            .and_then(|id| id.last())
            == Some(&Key::Entry(p.to_owned()))
        {
            return (r.x + 8, y);
        }
    }
    panic!("No row for {}", p.display())
}
fn start(f: &Fixture) -> Sidebar {
    let mut s = Sidebar::new(f.0.clone());
    s.focus = Focus::Tree;
    s.update_layout(Rect::new(0, 0, 180, 32), false);
    wait(&mut s, |s| {
        s.files.dirs.get(&s.files.root).is_some_and(|d| d.complete)
    });
    draw(&mut s);
    s
}
fn menu(s: &mut Sidebar, c: Command) {
    s.focus = Focus::Tree;
    s.menu();
    let m = s.menu.as_mut().unwrap();
    m.selected = m
        .items
        .iter()
        .position(|(_, v)| std::mem::discriminant(v) == std::mem::discriminant(&c))
        .unwrap();
    key(s, KeyCode::Enter, Mods::NONE);
}
fn isolated(name: &str, run: impl FnOnce()) {
    if std::env::var_os("DEVTABS_EXPLORER_TEST").is_some() {
        run();
        return;
    }
    let home = Fixture::new();
    let stdout = home.path("stdout.log");
    let stderr = home.path("stderr.log");
    let mut child = Process::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("DEVTABS_EXPLORER_TEST", "1")
        .env("DEVTABS_HOME", &home.0)
        .env("DEVTABS_PERF_DIR", home.path("perf"))
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "Isolated explorer fixture timed out: {name}\n{}",
                fs::read_to_string(&stderr).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "{}\n{}",
        fs::read_to_string(stdout).unwrap_or_default(),
        fs::read_to_string(stderr).unwrap_or_default()
    );
}
fn search(worker: &Worker, id: u64, query: &str, scope: Scope) -> explorer::Search {
    worker.search(id, query.into(), scope).unwrap();
    loop {
        match worker.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Completed::Search { id: got, result } if got == id => return result,
            _ => {}
        }
    }
}
fn scope(revision: u64) -> Scope {
    Scope {
        hidden: false,
        ignored: false,
        revision,
    }
}

#[test]
fn project_search_covers_unexpanded_paths_respects_ignore_and_latest_query() {
    let f = Fixture::new();
    let found = f.write("src/深层/module/terminal_image_preview.rs", "code");
    f.write(".gitignore", "ignored/\n");
    f.write("ignored/secret.rs", "secret");
    f.write(".git/hidden.rs", "metadata");
    let files = Files::new(f.0.clone());
    assert!(!files.nodes.contains_key(&found));
    let worker = Worker::new(f.0.clone());
    let result = search(&worker, 1, "tiprs", scope(0));
    assert_eq!(result.paths, vec![found]);
    assert!(search(&worker, 2, "secret", scope(0)).paths.is_empty());
    let mut all = scope(1);
    all.hidden = true;
    all.ignored = true;
    assert_eq!(search(&worker, 3, "secret", all.clone()).total, 1);
    assert!(search(&worker, 4, "hidden", all).paths.is_empty());
    worker.search(5, "wrong".into(), scope(0)).unwrap();
    worker.search(6, "terminal".into(), scope(0)).unwrap();
    loop {
        if let Completed::Search { id: 6, result } =
            worker.rx.recv_timeout(Duration::from_secs(10)).unwrap()
        {
            assert_eq!(result.total, 1);
            break;
        }
    }
}
#[test]
fn git_porcelain_rename_conflict_unicode_and_directory_aggregation() {
    let f = Fixture::new();
    let git=explorer::parse_git(&f.0,"",b"?? fresh/\0?? loose.txt\0!! build/\0R  new name.rs\0old name.rs\0UU src/conflict.rs\0 M src/file.rs\0");
    assert_eq!(
        explorer::status_for(&git, &f.path("src/conflict.rs")),
        Some(Status::Conflict)
    );
    assert_eq!(
        explorer::status_for(&git, &f.path("src")),
        Some(Status::Conflict)
    );
    assert_eq!(
        explorer::status_for(&git, &f.path("new name.rs")),
        Some(Status::Renamed)
    );
    assert_eq!(
        explorer::status_for(&git, &f.path("build/deep/a")),
        Some(Status::Ignored)
    );
    assert_eq!(
        explorer::status_for(&git, &f.path("fresh/deep/a")),
        Some(Status::Untracked)
    );
    assert_eq!(explorer::status_for(&git, &f.path("clean.rs")), None);
    let git = explorer::parse_git(&f.0, "sub/", " M sub/中文.rs\0 M outside.rs\0".as_bytes());
    assert_eq!(
        explorer::status_for(&git, &f.path("中文.rs")),
        Some(Status::Modified)
    );
    assert_eq!(git.entries.len(), 2);
}
fn git(root: &Path, args: &[&str]) {
    let out = Process::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn real_git_status_handles_project_subdirectory_and_ignored_untracked_directories() {
    let f = Fixture::new();
    git(&f.0, &["init", "--quiet"]);
    f.write("sub/tracked.rs", "old");
    f.write("sub/clean.rs", "clean");
    f.write(".gitignore", "build/\n");
    git(&f.0, &["add", "."]);
    git(
        &f.0,
        &[
            "-c",
            "user.name=DevTabs Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    f.write("sub/tracked.rs", "new");
    f.write("sub/new/file.rs", "new");
    f.write("sub/build/a.rs", "ignored");
    let status = explorer::git_status(&f.path("sub")).unwrap();
    assert_eq!(
        explorer::status_for(&status, &f.path("sub/tracked.rs")),
        Some(Status::Modified)
    );
    assert_eq!(explorer::status_for(&status, &f.path("sub/clean.rs")), None);
    assert_eq!(
        explorer::status_for(&status, &f.path("sub/new/file.rs")),
        Some(Status::Untracked)
    );
    assert_eq!(
        explorer::status_for(&status, &f.path("sub/build/a.rs")),
        Some(Status::Ignored)
    );
}
#[test]
fn file_operations_validate_paths_create_types_and_never_replace_existing_files() {
    let f = Fixture::new();
    let file = explorer::apply_change(
        &f.0,
        Operation::File {
            parent: f.0.clone(),
            name: "file.rs".into(),
        },
    );
    assert!(file.error.is_none());
    assert!(f.path("file.rs").is_file());
    let dir = explorer::apply_change(
        &f.0,
        Operation::Directory {
            parent: f.0.clone(),
            name: "folder".into(),
        },
    );
    assert!(dir.error.is_none());
    assert!(f.path("folder").is_dir());
    fs::write(f.path("existing.rs"), "retained").unwrap();
    assert!(
        explorer::apply_change(
            &f.0,
            Operation::Rename {
                path: f.path("file.rs"),
                name: "existing.rs".into()
            }
        )
        .error
        .is_some()
    );
    assert_eq!(
        fs::read_to_string(f.path("existing.rs")).unwrap(),
        "retained"
    );
    assert!(f.path("file.rs").exists());
    let rename = explorer::apply_change(
        &f.0,
        Operation::Rename {
            path: f.path("file.rs"),
            name: "中文.rs".into(),
        },
    );
    assert!(rename.error.is_none());
    assert!(f.path("中文.rs").is_file());
    for invalid in [
        "",
        "..",
        "../escape",
        "a/b",
        "NUL.txt",
        "COM1",
        "a.",
        "a ",
        ".git",
        "bad\nname",
    ] {
        assert!(explorer::child_name(&f.0, invalid).is_err(), "{invalid:?}");
    }
    assert!(
        explorer::apply_change(&f.0, Operation::Trash(vec![f.0.clone()]))
            .error
            .is_some()
    );
    assert!(f.0.exists());
    let outside = Fixture::new();
    assert!(
        explorer::apply_change(
            &f.0,
            Operation::File {
                parent: outside.0.clone(),
                name: "escape".into()
            }
        )
        .error
        .is_some()
    );
    assert!(!outside.path("escape").exists());
}
#[test]
fn batch_agent_validation_is_atomic_and_paste_never_contains_enter() {
    let f = Fixture::new();
    let a = f.write("a space.rs", "");
    let b = f.write("中文.md", "");
    assert!(explorer::validate_paths(vec![a.clone(), f.path("missing")]).is_err());
    let paths = explorer::validate_paths(vec![a.clone(), b.clone()]).unwrap();
    let text = explorer::paths_input(&paths).unwrap();
    assert!(
        text.contains(&a.to_string_lossy().to_string())
            && text.contains(&b.to_string_lossy().to_string())
    );
    assert!(!text.contains(['\r', '\n']));
    assert!(explorer::validate_paths(vec![f.0.clone()]).is_err());
    assert!(explorer::paths_input(&[]).is_err());
    assert!(explorer::paths_input(&vec![a; 129]).is_err());
}
#[test]
fn quick_open_preview_follow_and_toggle() {
    isolated("quick_open_preview_follow_and_toggle", || {
        let f = Fixture::new();
        let path = f.write("src/reading/deep/target.md", "# Target\n");
        let mut s = start(&f);
        assert!(!s.files.nodes.contains_key(&path));
        key(&mut s, KeyCode::Char('p'), Mods::CONTROL);
        s.paste("target").unwrap();
        wait(&mut s, |s| {
            s.search_results().is_some_and(|p| p.contains(&path))
        });
        key(&mut s, KeyCode::Enter, Mods::NONE);
        wait(&mut s, |s| {
            s.files.selected_path() == Some(path.clone())
                && s.document.as_ref().is_some_and(|d| !d.loading)
        });
        assert_eq!(s.focus, Focus::Preview);
        assert!(s.files.state.opened().len() >= 2);
        s.focus = Focus::Tree;
        menu(&mut s, Command::Follow);
        assert!(!s.sidebar_preferences().follow);
        s.files.state.select_first();
        let other = f.write("other.md", "# Other");
        s.open(other.clone(), true).unwrap();
        wait(&mut s, |s| {
            s.document
                .as_ref()
                .is_some_and(|d| d.path == other && !d.loading)
        });
        assert_eq!(s.files.selected_path(), Some(f.0.clone()));
    });
}
#[test]
fn ctrl_shift_selection_drag_batch_right_click_and_escape() {
    isolated(
        "ctrl_shift_selection_drag_batch_right_click_and_escape",
        || {
            let f = Fixture::new();
            let a = f.write("a.rs", "a");
            let b = f.write("b.rs", "b");
            let c = f.write("c.rs", "c");
            let mut s = start(&f);
            let (x, y) = hit(&s, &a);
            mouse(&mut s, Mouse::Down(MouseButton::Left), x, y, Mods::CONTROL);
            mouse(&mut s, Mouse::Up(MouseButton::Left), x, y, Mods::CONTROL);
            draw(&mut s);
            let (x, y) = hit(&s, &c);
            mouse(&mut s, Mouse::Down(MouseButton::Left), x, y, Mods::SHIFT);
            mouse(&mut s, Mouse::Up(MouseButton::Left), x, y, Mods::SHIFT);
            assert_eq!(s.files.selected_paths.len(), 3);
            assert!(s.files.selected_paths.contains(&b));
            assert!(s.document.is_none());
            draw(&mut s);
            let (x, y) = hit(&s, &b);
            mouse(&mut s, Mouse::Down(MouseButton::Left), x, y, Mods::NONE);
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 65, 10, Mods::NONE);
            let action = mouse(&mut s, Mouse::Up(MouseButton::Left), 65, 10, Mods::NONE);
            assert!(matches!(action,Action::Insert(p) if p==vec![a,b,c]));
            s.focus = Focus::Tree;
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(s.files.selected_paths.is_empty());
            assert_eq!(s.focus, Focus::Tree);
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert_eq!(s.focus, Focus::Terminal);
        },
    );
}
#[test]
fn file_name_click_selects_directory_arrow_expands_and_right_click_opens_menu() {
    isolated(
        "file_name_click_selects_directory_arrow_expands_and_right_click_opens_menu",
        || {
            let f = Fixture::new();
            f.write("dir/child.txt", "child");
            let mut s = start(&f);
            let dir = f.path("dir");
            let (x, y) = hit(&s, &dir);
            mouse(&mut s, Mouse::Down(MouseButton::Left), x + 5, y, Mods::NONE);
            assert!(!s.files.state.opened().contains(&s.files.identifier(&dir)));
            mouse(&mut s, Mouse::Down(MouseButton::Left), 5, y, Mods::NONE);
            wait(&mut s, |s| {
                s.files.nodes.contains_key(&f.path("dir/child.txt"))
            });
            draw(&mut s);
            let (x, y) = hit(&s, &f.path("dir/child.txt"));
            mouse(&mut s, Mouse::Down(MouseButton::Right), x, y, Mods::NONE);
            assert!(s.menu.is_some());
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(s.menu.is_none());
        },
    );
}
#[test]
fn new_file_directory_rename_preserve_tabs_and_guard_dirty_buffers() {
    isolated(
        "new_file_directory_rename_preserve_tabs_and_guard_dirty_buffers",
        || {
            let f = Fixture::new();
            let old = f.write("old.rs", "fn old() {}\n");
            let mut s = start(&f);
            s.open(old.clone(), true).unwrap();
            wait(&mut s, |s| {
                s.document.as_ref().is_some_and(|d| d.loaded().is_some())
            });
            s.files.state.select(s.files.identifier(&old));
            menu(&mut s, Command::Rename);
            for _ in 0.."old.rs".len() {
                key(&mut s, KeyCode::Backspace, Mods::NONE);
            }
            s.paste("renamed.rs").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            let renamed = f.path("renamed.rs");
            wait(&mut s, |s| {
                !s.project_busy()
                    && s.document
                        .as_ref()
                        .is_some_and(|d| d.path == renamed && !d.loading)
            });
            assert!(!old.exists());
            assert!(renamed.exists());
            s.focus = Focus::Preview;
            key(&mut s, KeyCode::Char('e'), Mods::NONE);
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| d.editing));
            s.paste("// unsaved\n").unwrap();
            assert!(s.dirty());
            s.files.state.select(s.files.identifier(&renamed));
            menu(&mut s, Command::Rename);
            for _ in 0.."renamed.rs".len() {
                key(&mut s, KeyCode::Backspace, Mods::NONE);
            }
            s.paste("changed.rs").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            assert!(s.modal() && !s.project_busy());
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(renamed.exists() && !f.path("changed.rs").exists());
            assert!(s.dirty());
            // Discard through the existing dirty prompt, then perform the rename.
            menu(&mut s, Command::Rename);
            for _ in 0.."renamed.rs".len() {
                key(&mut s, KeyCode::Backspace, Mods::NONE);
            }
            s.paste("changed.rs").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            key(&mut s, KeyCode::Char('d'), Mods::NONE);
            wait(&mut s, |s| {
                !s.project_busy() && f.path("changed.rs").exists()
            });
            assert_eq!(
                fs::read_to_string(f.path("changed.rs")).unwrap(),
                "fn old() {}\n"
            );
            s.focus = Focus::Tree;
            s.files.state.select_first();
            key(&mut s, KeyCode::Char('n'), Mods::CONTROL);
            s.paste("fresh.txt").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            wait(&mut s, |s| {
                !s.project_busy() && f.path("fresh.txt").is_file()
            });
            s.files.state.select_first();
            key(&mut s, KeyCode::Char('N'), Mods::CONTROL | Mods::SHIFT);
            s.paste("folder").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            wait(&mut s, |s| !s.project_busy() && f.path("folder").is_dir());
        },
    );
}
#[test]
fn truncated_tree_names_keep_a_blank_before_each_trailing_badge() {
    isolated(
        "truncated_tree_names_keep_a_blank_before_each_trailing_badge",
        || {
            let f = Fixture::new();
            let path = f.write("very-long-file-name-that-will-be-truncated.txt", "test");
            let mut s = start(&f);
            s.files.git.entries.insert(path.clone(), Status::Modified);
            s.open(path.clone(), true).unwrap();
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| !d.loading));
            s.files.reproject();
            let buffer = draw(&mut s);
            let (_, y) = hit(&s, &path);
            let tree = s.geometry.tree.unwrap();
            let line: String = (tree.x + 1..tree.right() - 1)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert!(
                line.contains(" [P] M"),
                "badges must be separated: {line:?}"
            );
            s.preview_visible = false;
            s.files.reproject();
            let buffer = draw(&mut s);
            let (_, y) = hit(&s, &path);
            let line: String = (tree.x + 1..tree.right() - 1)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let m = line.rfind('M').unwrap();
            assert!(
                line[..m].ends_with(' '),
                "truncated name needs a gap before Git badge: {line:?}"
            );
        },
    );
}
#[test]
fn both_dividers_drag_plain_borders_preserve_minima_cancel_and_persist_ratio() {
    isolated(
        "both_dividers_drag_plain_borders_preserve_minima_cancel_and_persist_ratio",
        || {
            let f = Fixture::new();
            let file = f.write("read.txt", "hello\n");
            let mut s = start(&f);
            s.open(file, true).unwrap();
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| !d.loading));
            draw(&mut s);
            let tree = s.geometry.tree.unwrap();
            mouse(
                &mut s,
                Mouse::Down(MouseButton::Left),
                tree.right() - 1,
                10,
                Mods::NONE,
            );
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 35, 10, Mods::NONE);
            mouse(&mut s, Mouse::Up(MouseButton::Left), 35, 10, Mods::NONE);
            assert_eq!(s.geometry.tree.unwrap().width, 36);
            let preview = s.geometry.preview.unwrap();
            assert!(!s.geometry.overlay);
            mouse(
                &mut s,
                Mouse::Down(MouseButton::Left),
                preview.x,
                10,
                Mods::NONE,
            );
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 126, 10, Mods::NONE);
            assert_eq!(s.geometry.preview.unwrap().x, 126);
            assert_eq!(s.geometry.agent.unwrap().width, 90);
            mouse(&mut s, Mouse::Up(MouseButton::Left), 126, 10, Mods::NONE);
            let ratio = s.sidebar_preferences().agent_ratio;
            wait(&mut s, |_| {
                devtabs::config::Config::load()
                    .ok()
                    .and_then(|c| c.projects.get(&devtabs::config::key(&f.0)).cloned())
                    .is_some_and(|p| p.sidebar.agent_ratio == ratio)
            });
            let mut next = Sidebar::new(f.0.clone());
            next.open(s.document.as_ref().unwrap().path.clone(), true)
                .unwrap();
            next.update_layout(Rect::new(0, 0, 180, 32), false);
            wait(&mut next, |s| s.sidebar_preferences().agent_ratio == ratio);
            draw(&mut next);
            assert_eq!(next.geometry.preview.unwrap().x, 126);
            mouse(&mut s, Mouse::Down(MouseButton::Left), 125, 10, Mods::NONE);
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 0, 10, Mods::NONE);
            assert_eq!(s.geometry.agent.unwrap().width, 62);
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert_eq!(s.geometry.preview.unwrap().x, 126);
            mouse(&mut s, Mouse::Down(MouseButton::Left), 126, 10, Mods::NONE);
            mouse(&mut s, Mouse::Up(MouseButton::Left), 179, 10, Mods::NONE);
            assert_eq!(s.geometry.preview.unwrap().width, 42);
            assert!(matches!(s.gesture, devtabs::sidebar::Gesture::Idle));
            for width in [60, 84, 120, 180, 300] {
                s.update_layout(Rect::new(0, 0, width, 32), false);
                if let Some(a) = s.geometry.agent {
                    assert!(a.width >= 62);
                }
                if !s.geometry.overlay
                    && let Some(p) = s.geometry.preview
                {
                    assert!(p.width >= 42);
                }
            }
        },
    );
}
#[test]
fn resize_is_persistent_and_preserves_agent_minimum_width() {
    isolated(
        "resize_is_persistent_and_preserves_agent_minimum_width",
        || {
            let f = Fixture::new();
            let mut s = start(&f);
            let tree = s.geometry.tree.unwrap();
            mouse(
                &mut s,
                Mouse::Down(MouseButton::Left),
                tree.right(),
                10,
                Mods::NONE,
            );
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 45, 10, Mods::NONE);
            mouse(&mut s, Mouse::Up(MouseButton::Left), 45, 10, Mods::NONE);
            assert_eq!(s.geometry.tree.unwrap().width, 46);
            assert!(s.geometry.agent.unwrap().width >= 62);
            wait(&mut s, |_| {
                devtabs::config::Config::load()
                    .ok()
                    .and_then(|c| c.projects.get(&devtabs::config::key(&f.0)).cloned())
                    .is_some_and(|p| p.sidebar.width == 46)
            });
            let mut next = Sidebar::new(f.0.clone());
            next.update_layout(Rect::new(0, 0, 180, 32), false);
            wait(&mut next, |s| s.sidebar_preferences().width == 46);
            draw(&mut next);
            assert_eq!(next.geometry.tree.unwrap().width, 46);
            for width in [0, 60, 80, 100, 120, 180, 400] {
                for desired in [0, 20, 40, 80, 200] {
                    let p = devtabs::sidebar::panels_with_width(
                        Rect::new(0, 0, width, 32),
                        true,
                        true,
                        true,
                        desired,
                    );
                    if let Some(a) = p.agent {
                        assert!(a.width >= 62);
                        assert!(a.right() <= width);
                    }
                }
            }
        },
    );
}
#[test]
fn compact_projection_and_refresh_preserve_selection_expansion_and_scroll() {
    isolated(
        "compact_projection_and_refresh_preserve_selection_expansion_and_scroll",
        || {
            let f = Fixture::new();
            let p = f.write("src/preview/image/one.rs", "one");
            for i in 0..80 {
                f.write(&format!("file{i:03}.txt"), "");
            }
            let mut s = start(&f);
            s.open(p.clone(), false).unwrap();
            wait(&mut s, |s| s.files.selected_path() == Some(p.clone()));
            let b = draw(&mut s);
            let text = b.content.iter().map(|c| c.symbol()).collect::<String>();
            assert!(text.contains("src / preview /"));
            assert!(s.files.identifier(&p).len() < 5);
            s.files.state.select_first();
            draw(&mut s);
            s.files.state.scroll_down(35);
            draw(&mut s);
            let offset = s.files.state.get_offset();
            let opened = s.files.state.opened().clone();
            s.files.refresh(&s.io).unwrap();
            wait(&mut s, |s| s.files.dirs.values().all(|d| !d.loading));
            draw(&mut s);
            assert_eq!(s.files.state.get_offset(), offset);
            assert_eq!(s.files.state.opened(), &opened);
            menu(&mut s, Command::Compact);
            assert!(!s.files.compact);
            assert_eq!(s.files.identifier(&p).len(), 5);
            draw(&mut s);
            assert!(
                s.files
                    .state
                    .flatten(&s.files.items(None, 60))
                    .iter()
                    .any(|row| row.identifier.last() == Some(&Key::Entry(p.clone())))
            );
        },
    );
}
#[test]
fn modified_filter_and_preview_marker_survive_agent_focus() {
    isolated(
        "modified_filter_and_preview_marker_survive_agent_focus",
        || {
            let f = Fixture::new();
            let clean = f.write("clean.rs", "clean");
            let modified = f.write("modified.rs", "new");
            let mut s = start(&f);
            s.files.git = explorer::parse_git(&f.0, "", b" M modified.rs\0");
            menu(&mut s, Command::Modified);
            let b = draw(&mut s);
            let text = b.content.iter().map(|c| c.symbol()).collect::<String>();
            assert!(text.contains("modified.rs"));
            assert!(!text.contains("clean.rs"));
            s.open(modified, true).unwrap();
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| !d.loading));
            s.focus_agent();
            let b = draw(&mut s);
            assert!(
                b.content
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>()
                    .contains("[P]")
            );
            assert!(clean.exists());
        },
    );
}
#[cfg(windows)]
#[test]
fn recycle_bin_round_trip_only_touches_generated_fixture() {
    let f = Fixture::new();
    let p = f.write("devtabs-recycle-round-trip.txt", "recoverable");
    let result = explorer::apply_change(&f.0, Operation::Trash(vec![p.clone()]));
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(!p.exists());
    let entries = trash::os_limited::list()
        .unwrap()
        .into_iter()
        .filter(|e| {
            dunce::simplified(&e.original_parent) == f.0 && e.name == p.file_name().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    trash::os_limited::restore_all(entries).unwrap();
    assert_eq!(fs::read_to_string(&p).unwrap(), "recoverable");
}

#[test]
fn rename_directory_saves_dirty_file_and_remaps_tabs_history_and_bookmarks() {
    isolated(
        "rename_directory_saves_dirty_file_and_remaps_tabs_history_and_bookmarks",
        || {
            let f = Fixture::new();
            let a = f.write("old/a.rs", "fn a() {}\n");
            let b = f.write("old/b.rs", "fn b() {}\n");
            let mut s = start(&f);
            s.open(b.clone(), true).unwrap();
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| !d.loading));
            s.open(a.clone(), true).unwrap();
            wait(&mut s, |s| {
                s.document.as_ref().is_some_and(|d| d.loaded().is_some())
            });
            let store = devtabs::config::home().join("bookmarks.json");
            let entry = devtabs::bookmarks::Entry {
                id: uuid::Uuid::new_v4(),
                project: f.0.clone(),
                path: a.clone(),
                byte: 0,
                line: 1,
                anchor: "fn a()".into(),
                markdown: false,
                note: "fixture".into(),
            };
            devtabs::bookmarks::update(&store, &f.0, devtabs::bookmarks::Operation::Put(entry))
                .unwrap();
            key(&mut s, KeyCode::Char('e'), Mods::NONE);
            wait(&mut s, |s| s.document.as_ref().is_some_and(|d| d.editing));
            s.paste("// saved ").unwrap();
            assert!(s.dirty());
            s.files.state.select(s.files.identifier(&f.path("old")));
            menu(&mut s, Command::Rename);
            key(&mut s, KeyCode::Char('u'), Mods::CONTROL);
            s.paste("new").unwrap();
            key(&mut s, KeyCode::Enter, Mods::NONE);
            assert!(s.modal() && !s.project_busy());
            key(&mut s, KeyCode::Char('s'), Mods::NONE);
            let new = f.path("new/a.rs");
            wait(&mut s, |s| {
                !s.project_busy()
                    && s.document
                        .as_ref()
                        .is_some_and(|d| d.path == new && !d.loading)
            });
            assert!(fs::read_to_string(&new).unwrap().contains("// saved "));
            assert!(!f.path("old").exists());
            let entries =
                devtabs::bookmarks::update(&store, &f.0, devtabs::bookmarks::Operation::List)
                    .unwrap();
            assert_eq!(entries[0].path, new);
            key(&mut s, KeyCode::Left, Mods::ALT);
            wait(&mut s, |s| {
                s.document
                    .as_ref()
                    .is_some_and(|d| d.path == f.path("new/b.rs") && !d.loading)
            });
            // Delete remains reversible and requires the explicit popup confirmation.
            s.files.state.select(s.files.identifier(&f.path("new")));
            menu(&mut s, Command::Trash);
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(f.path("new/a.rs").exists());
        },
    );
}

#[test]
fn reopening_project_search_refreshes_unwatched_directories() {
    isolated(
        "reopening_project_search_refreshes_unwatched_directories",
        || {
            let f = Fixture::new();
            f.write("unopened/existing.txt", "");
            let mut s = start(&f);
            s.quick_open();
            s.paste("future").unwrap();
            wait(&mut s, |s| {
                s.search_results().is_some() && !s.explorer_metrics().is_empty()
            });
            key(&mut s, KeyCode::Esc, Mods::NONE);
            let new = f.write("unopened/future.txt", "");
            s.quick_open();
            s.paste("future").unwrap();
            wait(&mut s, |s| {
                s.search_results().is_some_and(|r| r.contains(&new))
            });
            assert!(!s.files.nodes.contains_key(&new));
        },
    );
}

#[test]
#[ignore = "release-only project index/search measurements; no interactive UI"]
fn project_search_benchmark() {
    let f = Fixture::new();
    for i in 0..4000 {
        f.write(
            &format!("src/module{:02}/terminal_image_{i:04}.rs", i % 40),
            "",
        );
    }
    let worker = Worker::new(f.0.clone());
    let start = Instant::now();
    let cold = search(&worker, 1, "ti", scope(0));
    let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cold.scanned, 4000);
    assert_eq!(cold.paths.len(), explorer::RESULT_LIMIT);
    let mut warm = Vec::new();
    let mut cold_repeat = Vec::new();
    for i in 2..12 {
        let start = Instant::now();
        let hit = search(&worker, i, "ti", scope(0));
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(hit.total, cold.total);
    }
    for i in 12..17 {
        let start = Instant::now();
        let hit = search(&worker, i, "ti", scope(i));
        cold_repeat.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(hit.total, cold.total);
    }
    warm.sort_by(f64::total_cmp);
    cold_repeat.sort_by(f64::total_cmp);
    let median = warm[warm.len() / 2];
    let rescan = cold_repeat[cold_repeat.len() / 2];
    let data = serde_json::json!({"files":cold.scanned,"result_limit":explorer::RESULT_LIMIT,"first_index_and_search_ms":cold_ms,"rescan_every_query_median_ms":rescan,"cached_query_median_ms":median,"speedup":rescan/median,"all_matches":cold.total});
    fs::create_dir_all("artifacts").unwrap();
    fs::write(
        "artifacts/explorer-search-benchmark.json",
        serde_json::to_vec_pretty(&data).unwrap(),
    )
    .unwrap();
    println!("{data}");
    assert!(
        median < rescan,
        "Index reuse should reduce measured repeated-search time"
    );
}

#[test]
fn repeated_shift_navigation_and_drag_escape_preserve_selection() {
    isolated(
        "repeated_shift_navigation_and_drag_escape_preserve_selection",
        || {
            let f = Fixture::new();
            let a = f.write("a.txt", "");
            let b = f.write("b.txt", "");
            let c = f.write("c.txt", "");
            let mut s = start(&f);
            s.files.state.select(s.files.identifier(&a));
            key(&mut s, KeyCode::Down, Mods::SHIFT);
            key(&mut s, KeyCode::Down, Mods::SHIFT);
            assert_eq!(s.files.selected_paths.len(), 3);
            assert!(
                s.files.selected_paths.contains(&a)
                    && s.files.selected_paths.contains(&b)
                    && s.files.selected_paths.contains(&c)
            );
            s.gesture = devtabs::sidebar::Gesture::Dragging { path: a };
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(matches!(s.gesture, devtabs::sidebar::Gesture::Idle));
            assert_eq!(s.files.selected_paths.len(), 3);
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert!(s.files.selected_paths.is_empty());
        },
    );
}

#[test]
fn wide_tree_gives_duplicate_names_full_width_and_resize_escape_restores_width() {
    isolated(
        "wide_tree_gives_duplicate_names_full_width_and_resize_escape_restores_width",
        || {
            let f = Fixture::new();
            let a = f.write("left/long-module-name-that-needs-truncation.rs", "");
            let b = f.write("right/long-module-name-that-needs-truncation.rs", "");
            let mut s = start(&f);
            s.open(a.clone(), false).unwrap();
            wait(&mut s, |s| s.files.selected_path() == Some(a.clone()));
            s.open(b.clone(), false).unwrap();
            wait(&mut s, |s| s.files.selected_path() == Some(b.clone()));
            let r = s.geometry.tree.unwrap();
            mouse(
                &mut s,
                Mouse::Down(MouseButton::Left),
                r.right(),
                10,
                Mods::NONE,
            );
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 59, 10, Mods::NONE);
            mouse(&mut s, Mouse::Up(MouseButton::Left), 59, 10, Mods::NONE);
            let buf = draw(&mut s);
            let left = hit(&s, &a).1;
            let right = hit(&s, &b).1;
            let line = |y| (0..60).map(|x| buf[(x, y)].symbol()).collect::<String>();
            for y in [left, right] {
                let row = line(y);
                assert!(
                    !row.contains(" · "),
                    "tree hierarchy already identifies the directory: {row}"
                );
                assert!(
                    row.contains("long-module-name-that-needs-truncation.rs"),
                    "filename should use recovered width: {row}"
                );
            }
            mouse(&mut s, Mouse::Down(MouseButton::Left), 60, 10, Mods::NONE);
            mouse(&mut s, Mouse::Drag(MouseButton::Left), 72, 10, Mods::NONE);
            key(&mut s, KeyCode::Esc, Mods::NONE);
            assert_eq!(s.sidebar_preferences().width, 60);
            assert_eq!(s.geometry.tree.unwrap().width, 60);
        },
    );
}

#[test]
fn selection_markers_are_visible_without_rebuilding_tree_labels() {
    isolated(
        "selection_markers_are_visible_without_rebuilding_tree_labels",
        || {
            let f = Fixture::new();
            let a = f.write("a.txt", "");
            f.write("b.txt", "");
            let mut s = start(&f);
            let calls = || {
                devtabs::perf::snapshot()
                    .timings
                    .iter()
                    .find(|t| t.name == "tree.build_items")
                    .map_or(0, |t| t.calls)
            };
            let before = calls();
            assert!(before > 0);
            for _ in 0..20 {
                key(&mut s, KeyCode::Down, Mods::NONE);
                draw(&mut s);
                key(&mut s, KeyCode::Up, Mods::NONE);
                draw(&mut s);
            }
            assert_eq!(calls(), before);
            s.files.state.select(s.files.identifier(&a));
            draw(&mut s);
            let (x, y) = hit(&s, &a);
            s.files.selected_paths.clear();
            mouse(&mut s, Mouse::Down(MouseButton::Left), x, y, Mods::CONTROL);
            mouse(&mut s, Mouse::Up(MouseButton::Left), x, y, Mods::CONTROL);
            let buf = draw(&mut s);
            assert_eq!(buf[(2, y)].symbol(), "*");
            mouse(&mut s, Mouse::Down(MouseButton::Left), x, y, Mods::CONTROL);
            mouse(&mut s, Mouse::Up(MouseButton::Left), x, y, Mods::CONTROL);
            let buf = draw(&mut s);
            assert_ne!(buf[(2, y)].symbol(), "*");
            assert_eq!(calls(), before);
        },
    );
}

#[test]
fn git_badges_use_semantic_colors_and_retain_character_labels() {
    isolated("git_badges_use_semantic_colors", || {
        use ratatui::style::Color;
        let f = Fixture::new();
        let cases = [
            ("changed.rs", Status::Modified, "M", Color::Yellow),
            ("new.rs", Status::Untracked, "?", Color::Green),
            ("conflict.rs", Status::Conflict, "U", Color::Red),
        ];
        for (name, _, _, _) in cases {
            f.write(name, "test");
        }
        let mut s = start(&f);
        for (name, status, _, _) in cases {
            s.files.git.entries.insert(f.path(name), status);
        }
        s.files.reproject();
        let tree = s.geometry.tree.unwrap();
        for (name, _, badge, color) in cases {
            s.focus = devtabs::sidebar::Focus::Tree;
            s.files.state.select(s.files.identifier(&f.path(name)));
            let b = draw(&mut s);
            let (_, y) = hit(&s, &f.path(name));
            let cell = (tree.x + 1..tree.right() - 1)
                .rev()
                .map(|x| &b[(x, y)])
                .find(|c| c.symbol() == badge)
                .unwrap();
            assert_eq!(cell.fg, color, "{name}");
        }
    });
}
