use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devtabs::{
    editor::Editor,
    files::{self, Completed},
    perf, preview,
    sidebar::Sidebar,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn measure(name: &str, iterations: usize, mut f: impl FnMut()) -> serde_json::Value {
    let mut values = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        values.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    values.sort_by(f64::total_cmp);
    serde_json::json!({"name":name,"iterations":iterations,"mean_ms":values.iter().sum::<f64>()/iterations as f64,"p50_ms":values[(iterations-1)/2],"p95_ms":values[(iterations*95).div_ceil(100)-1],"max_ms":values[iterations-1]})
}
fn ready(s: &mut Sidebar) {
    let start = Instant::now();
    while s.document.as_ref().is_none_or(|d| d.rendered.is_none()) {
        s.tick(true);
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
#[ignore = "release hotspot workload; set DEVTABS_PERF_DIR to an isolated output directory"]
fn release_hotspots() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    assert!(
        perf::enabled(),
        "Set DEVTABS_PERF_DIR before starting the process"
    );
    let _exit = perf::ExitGuard::new();
    let root = std::env::temp_dir().join(format!("DevTabs-hotspots-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root.clone());
    let mut s = Sidebar::new(root.clone());
    let area = Rect::new(0, 0, 200, 40);
    s.update_layout(area, false);
    // Synthetic metadata isolates repeated UI work from filesystem creation/I/O.
    s.files.request(&root, true, &s.io).unwrap();
    let Completed::Directory {
        path, generation, ..
    } = s.io.rx.recv_timeout(Duration::from_secs(10)).unwrap()
    else {
        panic!()
    };
    let nodes = (0..9999)
        .map(|i| {
            let name = format!("文件-{i:05}.rs");
            files::Node {
                path: root.join(&name),
                name,
                directory: false,
                link: false,
                icon: 'r',
                color: ratatui::style::Color::Blue,
            }
        })
        .collect();
    s.files.apply(path, generation, Ok((nodes, true, 9999)));
    let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();
    let mut cases = vec![measure("tree_10000_repaint", 100, || {
        terminal.draw(|f| s.draw(f)).unwrap();
    })];
    drop(s);
    let path = root.join("search.txt");
    fs::write(&path, format!("{}\n", "x".repeat(80)).repeat(1000)).unwrap();
    let mut s = Sidebar::new(root.clone());
    s.visible = false;
    s.update_layout(area, false);
    s.open(path.clone(), true).unwrap();
    s.update_layout(area, false);
    ready(&mut s);
    cases.push(measure("preview_find_80000_matches", 1, || {
        s.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .unwrap();
        s.paste("x").unwrap();
        s.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
    }));
    assert_eq!(s.document.as_ref().unwrap().matches.len(), 80000);
    cases.push(measure("preview_draw_dense_matches", 10, || {
        terminal.draw(|f| s.draw(f)).unwrap();
    }));
    s.document.as_mut().unwrap().offset_mut().0 = 950;
    cases.push(measure("preview_draw_dense_matches_near_end", 10, || {
        terminal.draw(|f| s.draw(f)).unwrap();
    }));
    s.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap(); // clear search
    cases.push(measure("preview_draw_no_matches", 50, || {
        terminal.draw(|f| s.draw(f)).unwrap();
    }));
    let edit = root.join("edit.txt");
    fs::write(&edit, format!("{}\n", "x".repeat(126)).repeat(4000)).unwrap();
    let mut editor = Editor::new(preview::read(&edit).unwrap()).unwrap();
    editor
        .area
        .move_cursor(ratatui_textarea::CursorMove::Jump(3999, 0));
    editor.paste("changed").unwrap();
    cases.push(measure("editor_navigation_508kb", 200, || {
        editor
            .key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
    }));
    cases.push(measure("editor_dirty_508kb", 200, || {
        std::hint::black_box(editor.dirty());
    }));
    cases.push(measure("editor_type_508kb", 40, || {
        editor
            .key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE))
            .unwrap();
    }));
    let report = serde_json::json!({"cases":cases,"profile":perf::snapshot(),"note":"single process release microbenchmark; wall time; inclusive instrumentation enabled; synthetic 10k node metadata; no timing thresholds in correctness tests"});
    let out = PathBuf::from(std::env::var_os("DEVTABS_PERF_DIR").unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("hotspots.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "{}",
        serde_json::to_string_pretty(&report["cases"]).unwrap()
    );
    drop(fixture);
}
