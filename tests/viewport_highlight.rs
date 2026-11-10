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
        let p = std::env::temp_dir().join(format!("DevTabs-viewport-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn code() -> String {
    "let value = 42; // 中文\n".repeat(7000)
}
fn ready(s: &mut Sidebar) {
    let start = Instant::now();
    loop {
        s.tick(true);
        if s.document
            .as_ref()
            .is_some_and(|d| !d.loading && d.rendered.is_some())
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn draw(s: &mut Sidebar) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(100, 40)).unwrap();
    t.draw(|f| s.draw(f)).unwrap();
    t.backend().buffer().clone()
}
#[test]
fn source_preview_does_not_keep_whole_file_colored_spans() {
    let root = Fixture::new();
    let path = root.0.join("large.rs");
    fs::write(&path, code()).unwrap();
    let mut s = Sidebar::new(root.0.clone());
    s.visible = false;
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    s.open(path, true).unwrap();
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    ready(&mut s);
    let r = s.document.as_ref().unwrap().rendered.as_ref().unwrap();
    let colored_rows = r
        .text
        .lines
        .iter()
        .filter(|l| l.spans.iter().any(|s| s.style.fg.is_some()))
        .count()
        + r.code.as_ref().map_or(0, |w| w.text.lines.len());
    assert!(
        colored_rows <= 100,
        "retained {colored_rows} colored rows for a 40-row viewport"
    );
    let b = draw(&mut s);
    let colors = b
        .content
        .iter()
        .filter(|c| !c.symbol().trim().is_empty())
        .map(|c| c.fg)
        .collect::<std::collections::HashSet<_>>();
    assert!(colors.len() >= 4, "actual sidebar lost code colors");
}

#[test]
fn checkpoints_match_full_highlight_in_both_directions_and_themes() {
    use std::sync::{Arc, atomic::AtomicU64};
    let mut source = "fn main() {\n".to_owned();
    source.push_str(&"let x = 7;\n".repeat(200));
    source.push_str("/* outer\n");
    source.push_str(&"inside comment\n".repeat(400));
    source.push_str("/* nested */\nend */\nlet raw = r###\"\n");
    source.push_str(&"raw string 中文\n".repeat(400));
    source.push_str("\"###;\n}\n");
    let source: Arc<str> = Arc::from(source);
    let path = std::path::Path::new("state.rs");
    let mut cache = preview::CodeHighlighter::default();
    let generation = AtomicU64::new(1);
    for light in [false, true] {
        let full = preview::highlight(&source, path, light);
        for top in [0, 450, 850, 1000, 250, 700, 0] {
            let window = cache
                .render(
                    source.clone(),
                    path,
                    preview::CodeViewport { top, rows: 30 },
                    light,
                    &generation,
                    1,
                )
                .unwrap();
            assert!(window.text.lines.len() <= 62);
            for (i, line) in window.text.lines.iter().enumerate() {
                assert_eq!(
                    line,
                    &full.lines[window.first + i],
                    "row {} light={light}",
                    window.first + i
                );
            }
        }
    }
    let before = cache.stats();
    cache
        .render(
            source,
            path,
            preview::CodeViewport { top: 0, rows: 30 },
            true,
            &generation,
            1,
        )
        .unwrap();
    let after = cache.stats();
    assert_eq!(before.parsed_lines, after.parsed_lines);
    assert!(after.cache_hits > before.cache_hits);
    assert!(after.checkpoints <= 9);
}

#[test]
fn changed_source_budget_unknown_syntax_and_cancellation_are_safe() {
    use std::sync::{Arc, atomic::AtomicU64};
    let mut cache = preview::CodeHighlighter::default();
    let generation = AtomicU64::new(1);
    let viewport = preview::CodeViewport { top: 0, rows: 30 };
    let path = std::path::Path::new("state.rs");
    for source in [
        "/* comment\nlet x = 42;",
        "// comment\nlet x = 42;",
        "\tlet x = 42;\r\n",
    ] {
        let window = cache
            .render(Arc::from(source), path, viewport, false, &generation, 1)
            .unwrap();
        assert_eq!(
            window.text,
            preview::highlight(&preview::safe_text(source), path, false)
        );
    }
    for (source, name) in [
        ("x".repeat(5000), "long.rs"),
        ("plain\n".repeat(100), "a.unknown-123"),
    ] {
        let window = cache
            .render(
                Arc::from(source),
                std::path::Path::new(name),
                viewport,
                false,
                &generation,
                1,
            )
            .unwrap();
        assert!(
            window
                .text
                .lines
                .iter()
                .all(|l| l.spans.iter().all(|s| s.style.fg.is_none()))
        );
        assert!(window.covers(viewport, false));
    }
    assert!(
        cache
            .render(Arc::from(code()), path, viewport, false, &generation, 0)
            .is_none()
    );
    assert_eq!(cache.stats().cancelled, 1);
}

#[test]
fn fast_scrolling_draws_current_text_and_only_highlights_after_settling() {
    let root = Fixture::new();
    let path = root.0.join("scroll.rs");
    let mut source = code();
    source.push_str("let final_unique = 123;\n");
    fs::write(&path, source).unwrap();
    let mut s = Sidebar::new(root.0.clone());
    s.visible = false;
    s.open(path, true).unwrap();
    s.update_layout(Rect::new(0, 0, 100, 40), false);
    ready(&mut s);
    let requests = s
        .document
        .as_ref()
        .unwrap()
        .highlight_stats()
        .unwrap()
        .requests;
    let mut frames_ms = Vec::new();
    for top in (100..6100).step_by(100) {
        let started = Instant::now();
        s.document.as_mut().unwrap().scroll[0].0 = top;
        s.tick(true);
        std::hint::black_box(draw(&mut s));
        frames_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(
            s.document
                .as_ref()
                .unwrap()
                .rendered
                .as_ref()
                .unwrap()
                .code
                .as_ref()
                .unwrap()
                .first
                == 0
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    s.document.as_mut().unwrap().scroll[0].0 = 7000;
    let last_move = Instant::now();
    s.tick(true);
    let buffer = draw(&mut s);
    let text = buffer
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    let raw_visible_ms = last_move.elapsed().as_secs_f64() * 1000.0;
    assert!(
        text.contains("final_unique"),
        "scroll must draw the current raw text without waiting"
    );
    assert_eq!(
        s.document
            .as_ref()
            .unwrap()
            .highlight_stats()
            .unwrap()
            .requests,
        requests
    );
    let started = Instant::now();
    loop {
        s.tick(true);
        let d = s.document.as_ref().unwrap();
        if d.rendered
            .as_ref()
            .unwrap()
            .code
            .as_ref()
            .unwrap()
            .line(7000)
            .is_some()
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    let d = s.document.as_ref().unwrap();
    let settled_color_ms = last_move.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(d.highlight_stats().unwrap().requests, requests + 1);
    let window = d.rendered.as_ref().unwrap().code.as_ref().unwrap();
    assert!(
        window
            .line(7000)
            .unwrap()
            .spans
            .iter()
            .any(|s| s.style.fg.is_some())
    );
    let d = s.document.as_mut().unwrap();
    d.selection = Some(((7000, 0), (7000, 23)));
    assert_eq!(d.selected_text().unwrap(), "let final_unique = 123;");
    if let Some(report) = std::env::var_os("DEVTABS_SCROLL_REPORT") {
        frames_ms.sort_by(f64::total_cmp);
        fs::write(report, serde_json::to_vec_pretty(&serde_json::json!({
            "scroll_updates":60,"debounce_ms":80,"raw_visible_ms":raw_visible_ms,
            "last_move_to_color_ms":settled_color_ms,
            "tick_and_draw_p50_ms":frames_ms[30], "tick_and_draw_max_ms":frames_ms[59],
            "stats":d.highlight_stats(),
            "note":"Synthetic real Sidebar/TestBackend workload; final color delay includes debounce and first prefix parse. No terminal flush or human visual assessment."
        })).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "isolated release workload; set DEVTABS_VIEWPORT_REPORT to JSON output"]
fn source_viewport_workload() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let root = Fixture::new();
    let path = root.0.join("large.rs");
    let source = code();
    fs::write(&path, &source).unwrap();
    // Initialize shared grammar/theme equally before measuring document operations.
    std::hint::black_box(preview::highlight(
        "fn warmup() {}",
        std::path::Path::new("warmup.rs"),
        false,
    ));
    let mut cases = Vec::new();
    for iteration in 0..5 {
        let mut s = Sidebar::new(root.0.clone());
        s.visible = false;
        s.update_layout(Rect::new(0, 0, 100, 40), false);
        let start = Instant::now();
        s.open(path.clone(), true).unwrap();
        s.update_layout(Rect::new(0, 0, 100, 40), false);
        ready(&mut s);
        let open_ms = start.elapsed().as_secs_f64() * 1000.0;
        let b = draw(&mut s);
        std::hint::black_box(b);
        let r = s.document.as_ref().unwrap().rendered.as_ref().unwrap();
        let spans = r.text.lines.iter().map(|l| l.spans.len()).sum::<usize>();
        let retained_colored_rows = r
            .text
            .lines
            .iter()
            .filter(|l| l.spans.iter().any(|s| s.style.fg.is_some()))
            .count();
        let window_spans = r.code.as_ref().map_or(0, |w| {
            w.text.lines.iter().map(|l| l.spans.len()).sum::<usize>()
        });
        let window_rows = r.code.as_ref().map_or(0, |w| w.text.lines.len());
        let start = Instant::now();
        for _ in 0..30 {
            std::hint::black_box(draw(&mut s));
        }
        let repaint_mean_ms = start.elapsed().as_secs_f64() * 1000.0 / 30.0;
        cases.push(serde_json::json!({"iteration":iteration,"open_ms":open_ms,"repaint_mean_ms":repaint_mean_ms,"full_text_spans":spans,"full_text_colored_rows":retained_colored_rows,"window_spans":window_spans,"window_rows":window_rows,"memory":process_memory()}));
    }
    fs::write(std::env::var_os("DEVTABS_VIEWPORT_REPORT").expect("report path"),serde_json::to_vec_pretty(&serde_json::json!({"source_bytes":source.len(),"source_lines":7000,"viewport_rows":40,"cases":cases})).unwrap()).unwrap();
}
#[cfg(windows)]
fn process_memory() -> serde_json::Value {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::GetCurrentProcess,
    };
    unsafe {
        let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        m.cb = std::mem::size_of_val(&m) as u32;
        assert_ne!(
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                m.cb
            ),
            0
        );
        serde_json::json!({"working_set_bytes":m.WorkingSetSize,"private_bytes":m.PrivateUsage,"peak_working_set_bytes":m.PeakWorkingSetSize})
    }
}
#[cfg(not(windows))]
fn process_memory() -> serde_json::Value {
    serde_json::Value::Null
}
