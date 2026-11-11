use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use devtabs::{
    preview::{self, image::*, reading::ReadingOptions},
    sidebar::Sidebar,
};
use image::{Rgba, RgbaImage};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
    layout::Rect,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("devtabs-md-images-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn png(&self, name: &str, w: u32, h: u32) {
        RgbaImage::from_fn(w, h, |x, y| Rgba([x as u8, y as u8, 64, 255]))
            .save(self.0.join(name))
            .unwrap();
    }
    fn sidebar(&self, source: &str, display: Display) -> Sidebar {
        let path = self.0.join("read.md");
        fs::write(&path, source).unwrap();
        let mut s = Sidebar::new(self.0.clone());
        s.visible = false;
        s.configure_images(display);
        s.open(path, true).unwrap();
        s.update_layout(Rect::new(0, 0, 120, 35), false);
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
    while !f(s) {
        s.tick(true);
        assert!(
            at.elapsed() < Duration::from_secs(15),
            "timeout {}",
            s.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn sixel() -> Display {
    Display::explicit(
        ProtocolChoice::Sixel,
        Some(CellSize {
            width: 8,
            height: 17,
        }),
    )
}
#[test]
fn markdown_image_and_panel_share_plan_geometry_without_repreparing_source() {
    let f = Fixture::new();
    f.png("picture.png", 400, 600);
    let mut s = f.sidebar("# Read\n\n![picture](picture.png)\n\nend\n", sixel());
    wait(&mut s, |s| !s.image_plans().is_empty());
    let original = s.image_plans()[0];
    let cover = Rect::new(original.0.x + 2, original.0.y + 3, original.0.width / 2, 4);
    s.image_occlusion = Some(cover);
    let parts = s.image_plans();
    assert!(!parts.is_empty());
    assert!(parts.iter().all(|(r, _, _)| !r.intersects(cover)));
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
    terminal.draw(|frame| s.draw(frame)).unwrap();
    for (area, _, _) in parts {
        let payload = terminal.backend().buffer()[area.as_position()].symbol();
        assert!(payload.contains("\x1bP"));
    }
    s.image_occlusion = None;
    s.tick(true);
    assert_eq!(
        s.image_plans()[0],
        original,
        "panel movement/collapse preserves encoded generation"
    );
}
#[test]
fn placeholders_are_source_backed_bounded_blocks_and_caption_is_searchable() {
    let source: Arc<str> = "before ![my alt](picture.png) after\n\nend\n".into();
    let mut cache = preview::MarkdownCache::default();
    let r = cache.render_reading(
        source.clone(),
        70,
        false,
        preview::CodeViewport { top: 0, rows: 30 },
        true,
        &ReadingOptions::default(),
    );
    let rows = r
        .rows
        .iter()
        .filter(|r| r.image.is_some())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    let start = source.find("![").unwrap();
    assert!(
        rows.iter()
            .all(|r| r.image.as_ref().unwrap().source.start == start)
    );
    assert!(r.plain.iter().any(|s| s.contains("my alt")));
    let first = r.rows.iter().position(|r| r.image.is_some()).unwrap();
    assert_eq!(
        r.action_at(first, 0),
        Some(preview::ReadingAction::Image("picture.png".into()))
    );
    let options = ReadingOptions {
        image_heights: BTreeMap::from([(start, 12)]),
        ..Default::default()
    };
    let r = cache.render_reading(
        source,
        70,
        false,
        preview::CodeViewport { top: 0, rows: 30 },
        true,
        &options,
    );
    assert_eq!(r.rows.iter().filter(|r| r.image.is_some()).count(), 12);
}
#[test]
fn local_path_accepts_encoded_unicode_parent_and_rejects_network_device_remote() {
    let f = Fixture::new();
    f.png("中文 图.png", 8, 8);
    let base = f.0.join("read.md");
    assert!(
        local_path(&base, "%E4%B8%AD%E6%96%87%20%E5%9B%BE.png")
            .unwrap()
            .ends_with("中文 图.png")
    );
    for path in [
        "https://example.com/image.png",
        "//server/share/pic.png",
        "\\\\server\\share\\pic.png",
        "\\\\.\\pipe\\thing",
        "\\\\?\\C:\\image.png",
        "file:///C:/pic.png",
    ] {
        assert!(local_path(&base, path).is_err(), "accepted {path}");
    }
    fs::create_dir(f.0.join("child")).unwrap();
    assert!(local_path(&f.0.join("child/read.md"), "../中文%20图.png").is_ok());
}
#[test]
fn inline_load_relayout_duplicate_reuse_and_image_click_returns_to_markdown() {
    let f = Fixture::new();
    f.png("picture.png", 160, 100);
    let source = "# Reading\n\n![one](picture.png)\n\n![two](picture.png)\n";
    let mut s = f.sidebar(source, sixel());
    wait(&mut s, |s| s.image_plans().len() == 2);
    let r = s.document.as_ref().unwrap().rendered().unwrap();
    assert_eq!(r.rows.iter().filter(|row| row.image.is_some()).count(), 12);
    let plan = s.image_plans()[0];
    s.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: plan.0.x + 1,
        row: plan.0.y,
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
    s.mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: plan.0.x + 1,
        row: plan.0.y,
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
    wait(&mut s, |s| s.document.as_ref().unwrap().is_image());
    assert!(s.document.as_ref().unwrap().path.ends_with("picture.png"));
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    wait(&mut s, |s| s.image_plans().len() == 2);
    assert!(s.document.as_ref().unwrap().path.ends_with("read.md"));
}
#[test]
fn hidden_details_remote_and_offscreen_images_do_not_load() {
    let f = Fixture::new();
    f.png("picture.png", 120, 80);
    let source = format!(
        "# Reading\n\n<details><summary>Hidden</summary>\n\n![hidden](picture.png)\n\n</details>\n\n![remote](https://example.com/a.png)\n\n{}\n\n![late](picture.png)\n",
        (0..200)
            .map(|i| format!("paragraph {i}\n\n"))
            .collect::<String>()
    );
    let mut s = f.sidebar(&source, sixel());
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().rendered().is_some()
    });
    for _ in 0..30 {
        s.tick(true);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(s.image_plans().is_empty());
    let r = s.document.as_ref().unwrap().rendered().unwrap();
    assert!(
        r.rows
            .iter()
            .filter_map(|r| r.image.as_ref())
            .all(|i| i.destination.starts_with("https:"))
    );
    let mut t = Terminal::new(TestBackend::new(120, 35)).unwrap();
    t.draw(|f| s.draw(f)).unwrap();
    let text = t
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Remote/unsupported image"));
}
#[test]
fn nearby_dimensions_are_prefetched_without_offscreen_pixel_decoding() {
    let f = Fixture::new();
    if std::env::var_os("DEVTABS_IMAGE_METADATA_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "nearby_dimensions_are_prefetched_without_offscreen_pixel_decoding",
                "--nocapture",
            ])
            .env("DEVTABS_IMAGE_METADATA_CHILD", "1")
            .env("DEVTABS_PERF_DIR", f.0.join("reports"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _exit = devtabs::perf::ExitGuard::new();
    f.png("near.png", 160, 400);
    f.png("far.png", 160, 400);
    let source = format!(
        "{}![near](near.png)\n\n{}![far](far.png)\n",
        (0..18).map(|i| format!("row {i}\n\n")).collect::<String>(),
        (0..200)
            .map(|i| format!("distant {i}\n\n"))
            .collect::<String>()
    );
    let mut s = f.sidebar(&source, sixel());
    let count = |name: &str| {
        devtabs::perf::snapshot()
            .counts
            .into_iter()
            .find(|c| c.name == name)
            .unwrap()
            .value
    };
    wait(&mut s, |_| count("image.inspections") == 1);
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .and_then(|d| d.rendered())
            .is_some_and(|r| {
                r.rows
                    .iter()
                    .filter(|row| {
                        row.image
                            .as_ref()
                            .is_some_and(|i| &*i.destination == "near.png")
                    })
                    .count()
                    == 24
            })
    });
    for _ in 0..30 {
        s.tick(true);
    }
    assert_eq!(count("image.inspections"), 1);
    assert_eq!(count("image.decodes"), 0);
    assert_eq!(count("image.encodes"), 0);
    assert!(s.image_plans().is_empty());
}
#[test]
fn rapid_scroll_keeps_heights_and_restores_only_final_viewport() {
    let f = Fixture::new();
    f.png("picture.png", 160, 400);
    let mut s = f.sidebar(
        "# Reading\n\n![one](picture.png)\n\n![two](picture.png)\n\nend\n",
        sixel(),
    );
    wait(&mut s, |s| {
        s.document
            .as_ref()
            .and_then(|d| d.rendered())
            .is_some_and(|r| r.rows.iter().filter(|r| r.image.is_some()).count() == 48)
    });
    let height = s.document.as_ref().unwrap().rendered().unwrap().rows.len();
    for n in 1..=4 {
        s.document.as_mut().unwrap().scroll[1].0 = n;
        s.tick(true);
    }
    assert!(s.image_plans().is_empty());
    wait(&mut s, |s| !s.image_plans().is_empty());
    assert_eq!(
        s.document.as_ref().unwrap().rendered().unwrap().rows.len(),
        height
    );
}
#[test]
fn sliced_non_six_cell_height_never_draws_outside_visible_rows_and_does_not_reencode() {
    use devtabs::image_output::{DcsCounter, Writer};
    let f = Fixture::new();
    f.png("picture.png", 80, 400);
    let mut s = f.sidebar("![one](picture.png)\n\nend\n", sixel());
    wait(&mut s, |s| !s.image_plans().is_empty());
    s.document.as_mut().unwrap().scroll[1].0 = 3;
    let mut bytes = Vec::new();
    let mut terminal = Terminal::new(CrosstermBackend::new(Writer::new(&mut bytes))).unwrap();
    terminal.resize(Rect::new(0, 0, 120, 35)).unwrap();
    terminal.draw(|f| s.draw(f)).unwrap();
    drop(terminal);
    let mut counter = DcsCounter::default();
    counter.feed(&bytes);
    assert_eq!(counter.transfers, 1);
    // Every emitted image can be decoded independently; its pixels fit its clipped viewport.
    let text = String::from_utf8_lossy(&bytes);
    let start = text.find("\x1bP").unwrap();
    let end = start + text[start..].find("\x1b\\").unwrap() + 2;
    let decoded = icy_sixel::SixelImage::decode(text[start..end].as_bytes()).unwrap();
    let region = s.image_plans()[0].0;
    assert!(
        decoded.height <= usize::from(region.height) * 17,
        "{} pixels > {} rows",
        decoded.height,
        region.height
    );
}

#[test]
fn markdown_scroll_cleans_locally_and_reuses_cached_image_after_settling() {
    use devtabs::image_output::{DcsCounter, PixelDamage, Writer};
    use ratatui::widgets::Paragraph;
    use std::{
        cell::RefCell,
        io::{self, Write},
        rc::Rc,
    };
    #[derive(Clone, Default)]
    struct Recording(Rc<RefCell<Vec<u8>>>);
    impl Write for Recording {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let recording = Recording::default();
    let transfers = || {
        let mut count = DcsCounter::default();
        count.feed(&recording.0.borrow());
        count.transfers
    };
    let f = Fixture::new();
    f.png("picture.png", 160, 600);
    let mut s = f.sidebar("# Reading\n\n![one](picture.png)\n\nend\n", sixel());
    wait(&mut s, |s| !s.image_plans().is_empty());
    // Warm up after initial dimension-driven relayout has settled.
    let original = s.image_plans()[0].1;
    let mut pixels = PixelDamage::default();
    let mut t = Terminal::with_options(
        CrosstermBackend::new(Writer::new(recording.clone())),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 120, 35)),
        },
    )
    .unwrap();
    t.resize(Rect::new(0, 0, 120, 35)).unwrap();
    let draw = |t: &mut Terminal<CrosstermBackend<Writer<Recording>>>,
                pixels: &mut PixelDamage,
                s: &mut Sidebar| {
        pixels
            .draw(t, &s.image_plans(), |frame| {
                s.draw(frame);
                frame.render_widget(Paragraph::new("UNCHANGED-AGENT"), Rect::new(0, 34, 30, 1));
            })
            .unwrap();
    };
    draw(&mut t, &mut pixels, &mut s);
    let start = recording.0.borrow().len();
    let initial_transfers = transfers();
    for n in 1..=6 {
        s.document.as_mut().unwrap().scroll[1].0 = n;
        s.tick(true);
        assert!(
            s.image_plans().is_empty(),
            "every scroll, including the first, defers image output"
        );
        draw(&mut t, &mut pixels, &mut s);
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(transfers(), initial_transfers);
    wait(&mut s, |s| !s.image_plans().is_empty());
    assert_eq!(
        s.image_plans()[0].1,
        original,
        "scrolling reuses the encoded generation"
    );
    draw(&mut t, &mut pixels, &mut s);
    assert_eq!(transfers(), initial_transfers + 1);
    let bytes = recording.0.borrow()[start..].to_vec();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains("\x1b[2J"),
        "scroll must not erase the whole terminal"
    );
    assert!(
        !text.contains("UNCHANGED-AGENT"),
        "unchanged Agent pane must not be repainted"
    );
    assert_eq!(
        text.matches("\x1b[?2026h").count(),
        text.matches("\x1b[?2026l").count()
    );
    assert!(text.contains("\x1b[?2026h"));
    // More covers the image, then restores the SAME clipped sequence.
    s.key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    draw(&mut t, &mut pixels, &mut s);
    s.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    draw(&mut t, &mut pixels, &mut s);
    assert_eq!(transfers(), initial_transfers + 2);
}
