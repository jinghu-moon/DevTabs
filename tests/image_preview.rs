use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use devtabs::{
    image_output::{DcsCounter, Writer},
    preview::image::*,
    sidebar::Sidebar,
};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Rect, Size},
};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("devtabs-images-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn png(&self, name: &str, w: u32, h: u32) -> PathBuf {
        let p = self.0.join(name);
        RgbaImage::from_fn(w, h, |x, y| Rgba([x as u8, y as u8, 128, 255]))
            .save(&p)
            .unwrap();
        p
    }
    fn sidebar(&self, path: PathBuf, display: Display) -> Sidebar {
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
            "timeout: {}",
            s.document.as_ref().map_or("", |d| &d.stale)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(s: &mut Sidebar, code: KeyCode) {
    s.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
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
fn content_identification_and_budgets() {
    let f = Fixture::new();
    let p = f.png("有效 图.png", 16, 12);
    let source = load(&p, Budget::default()).unwrap();
    assert_eq!((source.info.width, source.info.height), (16, 12));
    assert_eq!(source.bytes(), 16 * 12 * 4);
    let b = Budget {
        file_bytes: 1,
        ..Default::default()
    };
    assert!(load(&p, b).is_err());
    let b = Budget {
        source_pixels: 191,
        ..Default::default()
    };
    assert!(load(&p, b).is_err());
    let b = Budget {
        side: 15,
        ..Default::default()
    };
    assert!(load(&p, b).is_err());
    let renamed = f.0.join("renamed.jpg");
    fs::copy(&p, &renamed).unwrap();
    assert_eq!(
        load(&renamed, Budget::default()).unwrap().info.format,
        Format::Png
    );
    fs::write(&renamed, b"damaged").unwrap();
    assert!(load(&renamed, Budget::default()).is_err());
    let gif = f.0.join("test.gif");
    fs::write(&gif, b"GIF89a\x01\0\x01\0").unwrap();
    assert!(format!("{:#}", load(&gif, Budget::default()).err().unwrap()).contains("Unsupported"));
}
#[test]
fn static_webp_jpeg_animation_and_exif_orientation() {
    let f = Fixture::new();
    for (name, format) in [
        ("test.webp", image::ImageFormat::WebP),
        ("test.jpg", image::ImageFormat::Jpeg),
    ] {
        let p = f.0.join(name);
        DynamicImage::new_rgb8(20, 12)
            .save_with_format(&p, format)
            .unwrap();
        assert_eq!(load(&p, Budget::default()).unwrap().info.width, 20);
    }
    let p = f.0.join("test.jpg");
    let original = fs::read(&p).unwrap();
    // EXIF IFD0 orientation=6 (90° clockwise), little-endian TIFF.
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut bytes = original[..2].to_vec();
    bytes.extend_from_slice(b"\xff\xe1");
    bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    bytes.extend_from_slice(exif);
    bytes.extend_from_slice(&original[2..]);
    fs::write(&p, bytes).unwrap();
    let source = load(&p, Budget::default()).unwrap();
    assert_eq!((source.info.width, source.info.height), (12, 20));
    let p = f.png("animated.png", 8, 8);
    let mut bytes = fs::read(&p).unwrap();
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&8u32.to_be_bytes());
    chunk.extend_from_slice(b"acTL");
    chunk.extend_from_slice(&[0; 12]);
    bytes.splice(33..33, chunk);
    fs::write(&p, bytes).unwrap();
    assert!(format!("{:#}", load(&p, Budget::default()).err().unwrap()).contains("Animated"));
    let p = f.0.join("test.webp");
    let mut bytes = fs::read(&p).unwrap();
    bytes.splice(12..12, b"ANIM\x06\0\0\0\0\0\0\0\0\0".iter().copied());
    fs::write(&p, bytes).unwrap();
    assert!(format!("{:#}", load(&p, Budget::default()).err().unwrap()).contains("Animated"));
}
#[test]
fn zoom_geometry_is_bounded_and_keeps_original_pixels() {
    let b = Budget::default();
    let d = sixel();
    let area = Size::new(80, 20);
    let fit = geometry(4000, 2000, area, d, View::default(), b).unwrap();
    assert!(fit.scale < 0.125 || fit.scale < 1.0);
    assert!(fit.size.width <= 80 && fit.size.height <= 20);
    let actual = geometry(
        4000,
        2000,
        area,
        d,
        View {
            scale: Some(1.0),
            center: (1.0, 1.0),
        },
        b,
    )
    .unwrap();
    assert_eq!(actual.scale, 1.0);
    assert_eq!(actual.pixels, (640, 340));
    assert_eq!(actual.origin, (3360.0, 1660.0));
    let zoom = geometry(
        4000,
        2000,
        area,
        d,
        View {
            scale: Some(8.0),
            center: (0.5, 0.5),
        },
        b,
    )
    .unwrap();
    assert_eq!(zoom.pixels, actual.pixels);
    assert!(geometry(10, 10, Size::new(0, 10), d, View::default(), b).is_err());
    let mut b = b;
    b.target_pixels = 1;
    assert_eq!(
        geometry(4000, 2000, area, d, View::default(), b)
            .unwrap()
            .pixels,
        (1, 1)
    );
    let tiny = geometry(2, 2, area, d, View::default(), Budget::default()).unwrap();
    assert_eq!(tiny.scale, 1.0);
}
#[test]
fn fit_includes_pixel_budget_while_actual_size_never_silently_downsamples() {
    // User regression: 2560x1440 JPEG at a large/high-DPI viewport.
    let budget = Budget::default();
    let display = Display::explicit(
        ProtocolChoice::Sixel,
        Some(CellSize {
            width: 16,
            height: 32,
        }),
    );
    let area = Size::new(200, 70);
    let fit = geometry(2560, 1440, area, display, View::default(), budget).unwrap();
    assert!(u64::from(fit.pixels.0) * u64::from(fit.pixels.1) <= budget.target_pixels);
    assert!((fit.scale - 0.754).abs() < 0.002);
    assert!((f64::from(fit.pixels.0) / f64::from(fit.pixels.1) - 2560.0 / 1440.0).abs() < 0.002);
    assert!(fit.size.width <= area.width && fit.size.height <= area.height);
    assert!(
        geometry(
            2560,
            1440,
            area,
            display,
            View {
                scale: Some(1.0),
                ..Default::default()
            },
            budget
        )
        .is_err()
    );
    for (w, h) in [(1, 32768), (32768, 1), (2560, 1440), (2, 2)] {
        for pixels in [1, 2, 17, 101, 2 * 1024 * 1024] {
            let fit = geometry(
                w,
                h,
                area,
                display,
                View::default(),
                Budget {
                    target_pixels: pixels,
                    ..budget
                },
            )
            .unwrap();
            assert!(u64::from(fit.pixels.0) * u64::from(fit.pixels.1) <= pixels);
            assert!(fit.pixels.0 > 0 && fit.pixels.1 > 0);
        }
    }
    assert!(
        geometry(
            2560,
            1440,
            area,
            display,
            View::default(),
            Budget {
                target_pixels: 0,
                ..budget
            }
        )
        .is_err()
    );
}

#[test]
fn removed_image_repaints_even_equal_blank_cells_without_invalidating_other_panes() {
    use devtabs::image_output::PixelDamage;
    use ratatui::{backend::TestBackend, buffer::CellDiffOption, widgets::Paragraph};
    use std::num::NonZeroU16;
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut damage = PixelDamage::default();
    let image = Rect::new(10, 4, 8, 5);
    let mut f = t.get_frame();
    f.render_widget(Paragraph::new("UNCHANGED"), Rect::new(0, 0, 10, 1));
    for y in image.y..image.bottom() {
        for x in image.x..image.right() {
            f.buffer_mut()[(x, y)].set_diff_option(CellDiffOption::Skip);
        }
    }
    f.buffer_mut()[(image.x, image.y)]
        .set_symbol("pixels")
        .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()));
    damage.apply(&mut f, &[(image, 1, 0)]);
    let previous = f.buffer_mut().clone();
    f.buffer_mut().reset();
    f.render_widget(Paragraph::new("UNCHANGED"), Rect::new(0, 0, 10, 1));
    damage.apply(&mut f, &[]);
    let updates = previous.diff(f.buffer_mut());
    assert_eq!(
        updates.len(),
        usize::from(image.width) * usize::from(image.height)
    );
    assert!(
        updates
            .iter()
            .all(|(x, y, _)| image.contains((*x, *y).into()))
    );
    assert!(updates.iter().all(|(_, _, cell)| cell.symbol() == " "));
}

#[test]
#[ignore = "requires DEVTABS_IMAGE_TEST_FILE; checks a user's real image without opening UI"]
fn real_image_fit_at_large_viewport() {
    let path = PathBuf::from(
        std::env::var_os("DEVTABS_IMAGE_TEST_FILE").expect("set DEVTABS_IMAGE_TEST_FILE"),
    );
    let source = load(&path, Budget::default()).unwrap();
    for protocol in [ProtocolChoice::Sixel, ProtocolChoice::Halfblock] {
        let display = Display::explicit(
            protocol,
            Some(CellSize {
                width: 16,
                height: 32,
            }),
        );
        let p = prepare(
            &source,
            Size::new(200, 70),
            display,
            View::default(),
            Profile::Mixed,
            [16, 18, 23],
            Budget::default(),
        )
        .unwrap();
        assert!(
            u64::from(p.geometry.pixels.0) * u64::from(p.geometry.pixels.1)
                <= Budget::default().target_pixels
        );
        println!(
            "{:?}: original {}x{}, target {}x{}, scale {:.3}%, retained {} bytes",
            protocol,
            source.info.width,
            source.info.height,
            p.geometry.pixels.0,
            p.geometry.pixels.1,
            p.geometry.scale * 100.0,
            p.bytes
        );
    }
}
#[test]
fn alpha_background_profiles_and_encoded_limits_are_real() {
    let f = Fixture::new();
    let p = f.0.join("alpha.png");
    RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 0]))
        .save(&p)
        .unwrap();
    let src = load(&p, Budget::default()).unwrap();
    let prepared = prepare(
        &src,
        Size::new(4, 4),
        sixel(),
        View::default(),
        Profile::Screenshot,
        [16, 18, 23],
        Budget::default(),
    )
    .unwrap();
    let ratatui_image::protocol::Protocol::Sixel(s) = prepared.protocol else {
        panic!()
    };
    let start = s.data.find("\x1bP").unwrap();
    let decoded = icy_sixel::SixelImage::decode(&s.data.as_bytes()[start..]).unwrap();
    assert!(
        decoded
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] < 30 && p[1] < 30 && p[2] < 30)
    );
    let budget = Budget {
        encoded_bytes: 1,
        ..Default::default()
    };
    assert!(
        prepare(
            &src,
            Size::new(4, 4),
            sixel(),
            View::default(),
            Profile::Mixed,
            [0, 0, 0],
            budget
        )
        .is_err()
    );
    assert_eq!(Profile::Photo.options().diffusion, 0.875);
    assert_eq!(Profile::Screenshot.options().diffusion, 0.0);
    assert_eq!(Profile::Mixed.options().diffusion, 0.5);
}
#[test]
fn single_writer_no_static_retransmit_overlay_restore_and_resize() {
    let f = Fixture::new();
    let p = f.png("screen.png", 80, 60);
    let mut s = f.sidebar(p, sixel());
    wait(&mut s, |s| s.image_plan().is_some());
    let mut output = Writer::new(Vec::<u8>::new());
    let mut terminal = Terminal::new(CrosstermBackend::new(&mut output)).unwrap();
    terminal.resize(Rect::new(0, 0, 120, 35)).unwrap();
    terminal.draw(|frame| s.draw(frame)).unwrap();
    let before = 1;
    for _ in 0..30 {
        s.tick(true);
        terminal.draw(|frame| s.draw(frame)).unwrap();
    }
    key(&mut s, KeyCode::F(2));
    assert!(s.image_plan().is_none());
    terminal.clear().unwrap();
    terminal.draw(|f| s.draw(f)).unwrap();
    key(&mut s, KeyCode::Esc);
    assert!(s.image_plan().is_some());
    terminal.draw(|f| s.draw(f)).unwrap();
    drop(terminal);
    assert_eq!(output.counter().transfers, before + 1);
    s.update_layout(Rect::new(0, 0, 100, 30), false);
    s.tick(true);
    assert!(s.image_plan().is_none());
    wait(&mut s, |s| s.image_plan().is_some());
    let area = s.image_plan().unwrap().0;
    assert!(area.right() < 100 && area.bottom() < 28);
}
#[test]
fn image_text_history_key_isolation_and_inactive_release() {
    let f = Fixture::new();
    let p = f.png("screen.png", 800, 600);
    let text = f.0.join("read.rs");
    fs::write(&text, "fn main() {}\n").unwrap();
    let mut s = f.sidebar(p.clone(), Display::default());
    wait(&mut s, |s| s.image_plan().is_some());
    assert!(s.document.as_ref().unwrap().loaded().is_none());
    assert!(s.document.as_ref().unwrap().rendered().is_none());
    key(&mut s, KeyCode::Char('1'));
    key(&mut s, KeyCode::Right);
    wait(&mut s, |s| s.image_plan().is_some());
    for c in ['m', 'e', '/', 'w', 'v', 'd', 'b', 't', 'g'] {
        key(&mut s, KeyCode::Char(c));
        assert!(!s.modal(), "image consumed text command {c}");
    }
    assert!(s.document.as_ref().unwrap().editor().is_none());
    s.open(text.clone(), true).unwrap();
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().rendered().is_some()
    });
    assert!(!s.document.as_ref().unwrap().is_image());
    assert!(s.image_plan().is_none());
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    wait(&mut s, |s| s.image_plan().is_some());
    assert_eq!(s.document.as_ref().unwrap().path, p);
    key(&mut s, KeyCode::Char('i'));
    let mut t = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
    t.draw(|f| s.draw(f)).unwrap();
    let text = t
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("100.0%"));
}
#[test]
fn corrupted_image_reports_error_and_retry_works() {
    let f = Fixture::new();
    let p = f.0.join("broken.png");
    fs::write(&p, b"broken").unwrap();
    let mut s = f.sidebar(p.clone(), Display::default());
    wait(&mut s, |s| !s.document.as_ref().unwrap().loading);
    assert!(
        s.document
            .as_ref()
            .unwrap()
            .stale
            .contains("Image unavailable")
    );
    assert!(s.document.as_ref().unwrap().loaded().is_none());
    f.png("broken.png", 16, 16);
    key(&mut s, KeyCode::F(5));
    wait(&mut s, |s| s.image_plan().is_some());
    assert!(s.document.as_ref().unwrap().stale.is_empty());
}
#[test]
fn cli_cell_validation_and_safe_fallback() {
    use clap::Parser;
    use devtabs::launch::Options;
    assert!("0x16".parse::<CellSize>().is_err());
    assert!("9x257".parse::<CellSize>().is_err());
    assert!("9x18".parse::<CellSize>().is_ok());
    assert_eq!(
        Display::explicit(ProtocolChoice::Sixel, None).protocol,
        ProtocolChoice::Halfblock
    );
    let args = Options::try_parse_from([
        "devtabs",
        "--image-protocol",
        "sixel",
        "--image-cell-size",
        "9x18",
    ])
    .unwrap();
    assert_eq!(args.image_protocol, ProtocolChoice::Sixel);
}
#[test]
fn dcs_counter_handles_partial_writes_and_non_image_dcs() {
    let data = b"hello\x1bP0;1qabc\x1b\\world\x1bP1;2pabc\x1b\\";
    let mut counter = DcsCounter::default();
    for byte in data {
        counter.feed(&[*byte]);
    }
    assert_eq!(counter.transfers, 1);
    assert_eq!(counter.image_bytes, 11);
    let mut writer = Writer::new(Vec::new());
    writer.write_all(data).unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.counter().transfers, 1);
}
fn input(c: char) -> devtabs::events::Input {
    devtabs::events::Input {
        event: Event::Key(KeyEvent::new(
            if c == '\x1b' {
                KeyCode::Esc
            } else {
                KeyCode::Char(c)
            },
            KeyModifiers::NONE,
        )),
        win32: None,
        repeat: 1,
    }
}
#[test]
fn fragmented_probe_keeps_keys_and_only_consumes_matching_replies() {
    let mut probe = probe::Probe::default();
    probe.begin();
    let mut keys = Vec::new();
    for c in "x\x1b[?65;1;4c\x1b[6;17;8ty".chars() {
        keys.extend(probe.feed(input(c)));
    }
    keys.extend(probe.flush());
    assert_eq!(probe.sixel, Some(true));
    assert_eq!(
        probe.cell,
        Some(CellSize {
            width: 8,
            height: 17
        })
    );
    assert_eq!(keys.len(), 2);
    let mut probe = probe::Probe::default();
    probe.begin();
    let mut keys = Vec::new();
    for c in "\x1b[6;0;8t".chars() {
        keys.extend(probe.feed(input(c)));
    }
    keys.extend(probe.flush());
    assert_eq!(keys.len(), 8);
    assert!(probe.cell.is_none());
    let mut probe = probe::Probe::default();
    probe.begin();
    let mut keys = Vec::new();
    for c in format!("\x1b[?{}c", "1".repeat(180)).chars() {
        keys.extend(probe.feed(input(c)));
    }
    keys.extend(probe.flush());
    assert_eq!(keys.len(), 184);
}
#[test]
fn latest_request_wins_and_cancel_does_not_restore_old_result() {
    let f = Fixture::new();
    let p = f.png("screen.png", 1000, 700);
    let worker = Worker::new();
    for generation in 1..=50 {
        worker.submit(Request {
            kind: devtabs::preview::image::RequestKind::Pixels,
            local_target: None,
            generation,
            path: p.clone(),
            area: Size::new(20, 10),
            display: Display::default(),
            view: View::default(),
            profile: Profile::Mixed,
            background: [0, 0, 0],
            reload: false,
        });
    }
    let at = Instant::now();
    loop {
        if let Some(done) = worker.take() {
            assert_eq!(done.generation, 50);
            assert!(done.result.is_ok());
            break;
        }
        assert!(at.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.cancel();
    assert!(worker.take().is_none());
}

#[test]
fn probe_preserves_native_keys_interleaved_with_fragmented_replies() {
    let mut probe = probe::Probe::default();
    probe.begin();
    for c in "\x1b[6;".chars() {
        assert!(probe.feed(input(c)).is_empty());
    }
    let mut key = input('x');
    key.win32 = Some(b"\x1b[88;0;120;1;0;1_".to_vec());
    let out = probe.feed(key);
    assert_eq!(out.len(), 1);
    for c in "17;8t".chars() {
        assert!(probe.feed(input(c)).is_empty());
    }
    assert_eq!(
        probe.cell,
        Some(CellSize {
            width: 8,
            height: 17
        })
    );
}
#[test]
fn native_probe_timeout_overlong_and_late_replies_never_enter_agent_input() {
    fn response(c: char) -> devtabs::events::Input {
        let mut record = input(c);
        record.win32 = Some(format!("\x1b[0;0;{};1;0;1_", c as u32).into_bytes());
        record
    }
    let mut probe = probe::Probe::default();
    probe.begin();
    for c in "\x1b[6;".chars() {
        assert!(probe.feed(response(c)).is_empty());
    }
    assert!(
        probe
            .expire(Instant::now() + Duration::from_millis(200))
            .is_empty()
    );
    let mut key = input('x');
    key.win32 = Some(b"\x1b[88;0;120;1;0;1_".to_vec());
    assert_eq!(probe.feed(key).len(), 1);
    for c in "17;8t".chars() {
        assert!(probe.feed(response(c)).is_empty());
    }
    assert!(probe.cell.is_none()); // Partial response is discarded, never accepted as a measurement.
    for c in format!("\x1b[?{}c", "1".repeat(180)).chars() {
        assert!(probe.feed(response(c)).is_empty());
    }
    for c in "\x1b[?65;1;4c\x1b[6;19;9t".chars() {
        assert!(probe.feed(response(c)).is_empty());
    }
    assert_eq!(probe.sixel, Some(true));
    assert_eq!(
        probe.cell,
        Some(CellSize {
            width: 9,
            height: 19
        })
    );
    let mut probe = probe::Probe::default();
    probe.begin();
    assert!(probe.feed(input('\x1b')).is_empty());
    let released = probe.expire(Instant::now() + Duration::from_millis(200));
    assert_eq!(released.len(), 1); // Unknown provenance still preserves ordinary Escape.
}
#[test]
fn cancelling_unsaved_text_to_image_keeps_original_document_and_history() {
    let f = Fixture::new();
    let image = f.png("pic.png", 16, 16);
    let path = f.0.join("source.rs");
    fs::write(&path, "fn main() {}\n").unwrap();
    let mut s = f.sidebar(path.clone(), Display::default());
    wait(&mut s, |s| {
        s.document.as_ref().unwrap().rendered().is_some()
    });
    key(&mut s, KeyCode::Char('e'));
    wait(&mut s, |s| s.document.as_ref().unwrap().editing);
    key(&mut s, KeyCode::Char('x'));
    assert!(s.dirty());
    s.open(image, true).unwrap();
    assert!(s.modal());
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.document.as_ref().unwrap().path, path);
    assert!(s.dirty());
    assert!(!s.document.as_ref().unwrap().is_image());
}
#[test]
fn profiled_static_image_has_zero_extra_preparation_or_protocol_output() {
    let f = Fixture::new();
    let reports = f.0.join("reports");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "profiled_static_child",
            "--ignored",
            "--nocapture",
        ])
        .env("DEVTABS_PERF_DIR", &reports)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "isolated environment fixture for profiling"]
fn profiled_static_child() {
    let _exit = devtabs::perf::ExitGuard::new();
    let f = Fixture::new();
    let path = f.png("screen.png", 800, 600);
    let mut s = f.sidebar(path, sixel());
    wait(&mut s, |s| s.image_plan().is_some());
    let mut output = Writer::new(Vec::new());
    let mut terminal = Terminal::new(CrosstermBackend::new(&mut output)).unwrap();
    terminal.resize(Rect::new(0, 0, 120, 35)).unwrap();
    let mut pixels = devtabs::image_output::PixelDamage::default();
    pixels
        .draw(&mut terminal, &s.image_plans(), |f| s.draw(f))
        .unwrap();
    let before = devtabs::perf::snapshot();
    let steady_since = Instant::now();
    for n in 0..1000 {
        s.tick(true);
        pixels
            .draw(&mut terminal, &s.image_plans(), |frame| {
                s.draw(frame);
                frame.render_widget(
                    ratatui::widgets::Paragraph::new(format!("Agent revision {n}")),
                    Rect::new(0, 34, 20, 1),
                );
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(steady_since.elapsed() >= Duration::from_secs(10));
    let after = devtabs::perf::snapshot();
    for name in [
        "image.decodes",
        "image.encodes",
        "image.transfers",
        "image.output_bytes",
    ] {
        let old = before.counts.iter().find(|c| c.name == name).unwrap().value;
        let new = after.counts.iter().find(|c| c.name == name).unwrap().value;
        assert_eq!(old, new, "static image changed {name}");
    }
    assert!(after.process_memory.is_some());
}

#[test]
fn image_signature_works_without_extension_and_hidden_preview_releases_presentation() {
    let f = Fixture::new();
    let original = f.png("source.png", 80, 60);
    let path = f.0.join("without-extension");
    fs::rename(original, &path).unwrap();
    let mut s = f.sidebar(path, Display::default());
    wait(&mut s, |s| s.image_plan().is_some());
    assert!(s.document.as_ref().unwrap().is_image());
    s.preview_visible = false;
    s.tick(true);
    assert!(s.image_output_bytes().is_none());
    s.preview_visible = true;
    s.update_layout(Rect::new(0, 0, 120, 35), false);
    wait(&mut s, |s| s.image_plan().is_some());
}

#[test]
fn image_more_button_does_not_trigger_hidden_text_toolbar_close() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let f = Fixture::new();
    let path = f.png("pic.png", 40, 40);
    let mut s = f.sidebar(path.clone(), Display::default());
    wait(&mut s, |s| s.image_plan().is_some());
    let rect = s.geometry.preview.unwrap();
    s.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.right() - 6,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    })
    .unwrap();
    assert!(s.menu.is_some());
    assert_eq!(s.document.as_ref().unwrap().path, path);
}
