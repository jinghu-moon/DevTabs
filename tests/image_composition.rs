use devtabs::image_output::{Composition, PixelDamage, Writer, composed_plans, visible_parts};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Rect, Size},
    widgets::Paragraph,
};
use ratatui_image::protocol::sixel::Sixel;
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Recording(Rc<RefCell<Vec<u8>>>);
impl Write for Recording {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode(data: &str) -> icy_sixel::SixelImage {
    icy_sixel::SixelImage::decode(&data.as_bytes()[data.find("\x1bP").unwrap()..]).unwrap()
}
fn sample() -> Sixel {
    let image = RgbaImage::from_fn(160, 204, |x, y| {
        // Large uniform runs plus color changes on both sides of band boundaries.
        Rgba([
            ((x / 20) * 30) as u8,
            ((y / 3) * 3) as u8,
            ((x + y) % 7 * 35) as u8,
            255,
        ])
    });
    Sixel::new(DynamicImage::ImageRgba8(image), Size::new(20, 12), false).unwrap()
}

#[test]
fn encoded_rectangle_crop_preserves_palette_and_exact_pixels_at_every_band_phase() {
    let source = sample();
    let original = decode(&source.data);
    for y in 0..6 {
        for x in [0, 1, 11] {
            let crop = Rect::new(x, y, 9, 5);
            let result = source.crop_cells(crop, Size::new(8, 17)).unwrap();
            let pixels = decode(&result.data);
            assert_eq!(pixels.width, 72);
            assert!(pixels.height >= 85 && pixels.height <= 90);
            assert!(
                pixels.pixels[85 * 72 * 4..]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[3] == 0)
            );
            for row in 0..85 {
                let start = ((usize::from(y) * 17 + row) * original.width + usize::from(x) * 8) * 4;
                assert_eq!(
                    &pixels.pixels[row * 72 * 4..(row + 1) * 72 * 4],
                    &original.pixels[start..start + 72 * 4],
                    "cell ({x}, {y}), pixel row {row}"
                );
            }
        }
    }
}

#[test]
fn visible_rectangles_partition_image_without_overlapping_panel() {
    let image = Rect::new(5, 4, 20, 12);
    for x in 0..30 {
        for y in 0..20 {
            let cover = Rect::new(x, y, 10, 7);
            let parts = visible_parts(image, Some(cover));
            assert!(parts.len() <= 4);
            for yy in image.y..image.bottom() {
                for xx in image.x..image.right() {
                    let p = (xx, yy).into();
                    assert_eq!(
                        parts.iter().filter(|r| r.contains(p)).count(),
                        usize::from(!cover.contains(p))
                    );
                }
            }
        }
    }
}

#[test]
fn moving_resizing_collapsing_panel_reuses_static_payloads_and_cleans_exposed_cells() {
    let source = sample();
    let original = decode(&source.data);
    let composition = Composition::default();
    let plan = (Rect::new(5, 4, 20, 12), 123, 0);
    let recording = Recording::default();
    let transfers = || {
        let mut counter = devtabs::image_output::DcsCounter::default();
        counter.feed(&recording.0.borrow());
        counter.transfers
    };
    let mut t = Terminal::with_options(
        CrosstermBackend::new(Writer::new(recording.clone())),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 40, 25)),
        },
    )
    .unwrap();
    let mut damage = PixelDamage::default();
    for cover in [
        None,
        Some(Rect::new(10, 8, 8, 3)),
        Some(Rect::new(15, 7, 18, 10)),
        Some(Rect::new(0, 0, 40, 25)),
        Some(Rect::new(30, 1, 8, 10)),
        None,
    ] {
        let parts = composed_plans(plan, cover);
        damage
            .draw(&mut t, &parts, |frame| {
                composition.draw(frame, &source, plan, cover, Size::new(8, 17));
                if let Some(cover) = cover {
                    frame.render_widget(ratatui::widgets::Clear, cover);
                    frame.render_widget(Paragraph::new("DEBUG"), cover);
                }
            })
            .unwrap();
        let initial = transfers();
        for _ in 0..4 {
            let frame = t
                .draw(|frame| {
                    composition.draw(frame, &source, plan, cover, Size::new(8, 17));
                    if let Some(cover) = cover {
                        frame.render_widget(ratatui::widgets::Clear, cover);
                        frame.render_widget(Paragraph::new("DEBUG"), cover);
                    }
                })
                .unwrap();
            for (area, _, offset) in &parts {
                let payload = frame.buffer[area.as_position()].symbol();
                let image = decode(payload);
                assert_eq!(image.width, area.width as usize * 8);
                let height = area.height as usize * 17;
                assert!(image.height >= height && image.height < height + 6);
                assert!(
                    image.pixels[height * image.width * 4..]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[3] == 0)
                );
                assert!(cover.is_none_or(|r| !r.intersects(*area)));
                for row in 0..height {
                    let start = ((usize::from(*offset) * 17 + row) * original.width
                        + usize::from(area.x - plan.0.x) * 8)
                        * 4;
                    assert_eq!(
                        &image.pixels[row * image.width * 4..(row + 1) * image.width * 4],
                        &original.pixels[start..start + image.width * 4]
                    );
                }
            }
        }
        assert_eq!(
            transfers(),
            initial,
            "unchanged panel must not retransmit images"
        );
    }
}

#[test]
fn composition_statistics_prove_static_debug_frames_reuse_clips() {
    if std::env::var_os("DEVTABS_COMPOSITION_STATS_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "devtabs-composition-stats-{}",
            uuid::Uuid::new_v4()
        ));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "composition_statistics_prove_static_debug_frames_reuse_clips",
                "--nocapture",
            ])
            .env("DEVTABS_COMPOSITION_STATS_CHILD", "1")
            .env("DEVTABS_PERF_DIR", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        let _ = std::fs::remove_dir_all(root);
        return;
    }
    let source = sample();
    let composition = Composition::default();
    let plan = (Rect::new(5, 4, 20, 12), 1, 0);
    let panel = Rect::new(10, 8, 8, 3);
    let cover = Some(panel);
    let parts = composed_plans(plan, cover);
    let recording = Recording::default();
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(Writer::new(recording.clone())),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 40, 25)),
        },
    )
    .unwrap();
    let mut damage = PixelDamage::default();
    let mut draw = |n| {
        damage
            .draw(&mut terminal, &parts, |frame| {
                composition.draw(frame, &source, plan, cover, Size::new(8, 17));
                let cover = panel;
                frame.render_widget(ratatui::widgets::Clear, cover);
                frame.render_widget(Paragraph::new(format!("DEBUG {n}")), cover);
            })
            .unwrap()
    };
    draw(0);
    let before = devtabs::perf::snapshot();
    for n in 1..=100 {
        draw(n);
    }
    let after = devtabs::perf::snapshot();
    let count = |report: &devtabs::perf::Report, name| {
        report.counts.iter().find(|c| c.name == name).unwrap().value
    };
    for name in [
        "image.decodes",
        "image.encodes",
        "image.clips",
        "image.transfers",
        "image.output_bytes",
    ] {
        assert_eq!(
            count(&before, name),
            count(&after, name),
            "static Debug output changes {name}"
        );
    }
    assert_eq!(
        count(&after, "image.composition_cache_hits")
            - count(&before, "image.composition_cache_hits"),
        parts.len() as u64 * 100
    );
    println!(
        "100 Debug frames: clips {} -> {}, transfers {} -> {}, image bytes {} -> {}, cache hits +{}",
        count(&before, "image.clips"),
        count(&after, "image.clips"),
        count(&before, "image.transfers"),
        count(&after, "image.transfers"),
        count(&before, "image.output_bytes"),
        count(&after, "image.output_bytes"),
        parts.len() * 100
    );
}
