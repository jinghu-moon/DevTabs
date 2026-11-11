//! Reproducible generated images; no private files or terminal UI required.
use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use serde_json::json;
use std::{fs, path::PathBuf, time::Instant};
fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)].min(f64::MAX)
}
fn error(a: &[u8], b: &[u8]) -> f64 {
    a.iter()
        .zip(b)
        .enumerate()
        .filter(|(i, _)| i % 4 != 3)
        .map(|(_, (&x, &y))| (f64::from(x) - f64::from(y)).abs())
        .sum::<f64>()
        / (a.len() / 4 * 3) as f64
}
#[test]
#[ignore = "release image pipeline benchmark; writes generated fixtures and JSON under artifacts"]
fn image_pipeline_comparison() {
    let out = std::env::var_os("DEVTABS_IMAGE_BENCH_DIR")
        .map_or_else(|| PathBuf::from("artifacts/image-benchmark"), PathBuf::from);
    fs::create_dir_all(&out).unwrap();
    let mut results = Vec::new();
    for name in [
        "gradient",
        "texture",
        "code-dark",
        "code-light",
        "mixed",
        "alpha",
    ] {
        let source = RgbaImage::from_fn(3840, 2160, |x, y| {
            let code = ((y % 32 > 10 && y % 32 < 19) && (x % 140 < 100)) || x % 401 < 2;
            let noise = ((x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) >> 16) as u8;
            match name {
                "gradient" => Rgba([
                    (x * 255 / 3840) as u8,
                    (y * 255 / 2160) as u8,
                    ((x + y) * 255 / 6000) as u8,
                    255,
                ]),
                "texture" => Rgba([noise, (x % 255) as u8, noise / 2, 255]),
                "code-dark" => {
                    if code {
                        Rgba([170, 215, 255, 255])
                    } else {
                        Rgba([16, 18, 23, 255])
                    }
                }
                "code-light" => {
                    if code {
                        Rgba([30, 60, 110, 255])
                    } else {
                        Rgba([250, 250, 250, 255])
                    }
                }
                "mixed" => {
                    if x < 1920 {
                        Rgba([noise, (y % 255) as u8, 128, 255])
                    } else if code {
                        Rgba([200, 220, 255, 255])
                    } else {
                        Rgba([16, 18, 23, 255])
                    }
                }
                _ => Rgba([200, 128, 80, ((x + y) % 256) as u8]),
            }
        });
        source.save(out.join(format!("{name}.png"))).unwrap();
        let mut source = source;
        for p in source.pixels_mut() {
            let alpha = u32::from(p[3]);
            for c in 0..3 {
                p[c] = ((u32::from(p[c]) * alpha + 16 * (255 - alpha) + 127) / 255) as u8;
            }
            p[3] = 255;
        }
        let source = DynamicImage::ImageRgba8(source);
        for (filter, label) in [
            (FilterType::CatmullRom, "bicubic"),
            (FilterType::Lanczos3, "lanczos3"),
        ] {
            let mut old = Vec::new();
            let mut new = Vec::new();
            let mut difference = 0.0;
            for _ in 0..16 {
                let start = Instant::now();
                let expected = source.resize_exact(640, 360, filter).to_rgba8();
                old.push(start.elapsed().as_secs_f64() * 1000.0);
                let mut actual = DynamicImage::new_rgba8(640, 360);
                let mut resizer = fast_image_resize::Resizer::new();
                let options = fast_image_resize::ResizeOptions::new().resize_alg(
                    fast_image_resize::ResizeAlg::Convolution(if label == "bicubic" {
                        fast_image_resize::FilterType::CatmullRom
                    } else {
                        fast_image_resize::FilterType::Lanczos3
                    }),
                );
                let start = Instant::now();
                resizer
                    .resize(&source, &mut actual, Some(&options))
                    .unwrap();
                new.push(start.elapsed().as_secs_f64() * 1000.0);
                difference = error(expected.as_raw(), actual.as_bytes());
            }
            results.push(json!({"fixture":name,"comparison":"resize","filter":label,"samples":16,"image_p50_ms":percentile(&mut old,0.5),"image_p95_ms":percentile(&mut old,0.95),"fast_p50_ms":percentile(&mut new,0.5),"fast_p95_ms":percentile(&mut new,0.95),"mae_between_resizers":difference}));
        }
        let mut target = source
            .resize_exact(640, 360, FilterType::CatmullRom)
            .to_rgba8();
        for p in target.pixels_mut() {
            let a = u32::from(p[3]);
            for c in 0..3 {
                p[c] = ((u32::from(p[c]) * a + 16 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
        for (profile, diffusion) in [("photo", 0.875), ("screenshot", 0.0), ("mixed", 0.5)] {
            for version in ["0.5.1", "0.7.0"] {
                let mut times = Vec::new();
                let mut bytes = 0;
                let mut mae = 0.0;
                for _ in 0..16 {
                    let at = Instant::now();
                    let encoded = if version == "0.5.1" {
                        icy_sixel::sixel_encode(
                            target.as_raw(),
                            640,
                            360,
                            &icy_sixel::EncodeOptions {
                                diffusion,
                                ..Default::default()
                            },
                        )
                        .unwrap()
                    } else {
                        icy_sixel_candidate::sixel_encode(
                            target.as_raw(),
                            640,
                            360,
                            &icy_sixel_candidate::EncodeOptions {
                                diffusion,
                                ..Default::default()
                            },
                        )
                        .unwrap()
                    };
                    times.push(at.elapsed().as_secs_f64() * 1000.0);
                    bytes = encoded.len();
                    let decoded = icy_sixel::SixelImage::decode(encoded.as_bytes()).unwrap();
                    assert_eq!((decoded.width, decoded.height), (640, 360));
                    mae = error(target.as_raw(), &decoded.pixels);
                }
                results.push(json!({"fixture":name,"comparison":"sixel","version":version,"profile":profile,"samples":16,"p50_ms":percentile(&mut times,0.5),"p95_ms":percentile(&mut times,0.95),"output_bytes":bytes,"mae_rgb_0_255":mae}));
            }
        }
    }
    fs::write(out.join("comparison.json"),serde_json::to_vec_pretty(&json!({"source":"DevTabs programmatically generated; MIT","original":[3840,2160],"target":[640,360],"environment":"Windows release build; terminal output excluded","process_memory":devtabs::perf::memory(),"results":results})).unwrap()).unwrap();
    println!("Image benchmark written to {}", out.display());
}

#[test]
#[ignore = "release cold/load, metadata, continuous crop and real-backend hot redraw workload"]
fn image_worker_workload() {
    use devtabs::{
        image_output::Writer,
        preview::image::{
            self as pipeline, Budget, CellSize, Display, Profile, ProtocolChoice, View,
        },
    };
    use ratatui::{
        Terminal,
        backend::CrosstermBackend,
        layout::{Rect, Size},
        widgets::Paragraph,
    };
    let out = PathBuf::from("artifacts/image-benchmark");
    fs::create_dir_all(&out).unwrap();
    let path = out.join("workload.png");
    RgbaImage::from_fn(3840, 2160, |x, y| {
        let color = ((x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) >> 16) as u8;
        Rgba([color, (x % 256) as u8, (y % 256) as u8, 255])
    })
    .save(&path)
    .unwrap();
    let display = Display::explicit(
        ProtocolChoice::Sixel,
        Some(CellSize {
            width: 8,
            height: 18,
        }),
    );
    let area = Size::new(80, 20);
    let mut cold = Vec::new();
    let mut header = Vec::new();
    for _ in 0..16 {
        let at = Instant::now();
        assert_eq!(
            pipeline::inspect(&path, Budget::default()).unwrap(),
            (3840, 2160)
        );
        header.push(at.elapsed().as_secs_f64() * 1000.0);
        let at = Instant::now();
        let source = pipeline::load(&path, Budget::default()).unwrap();
        let _ = pipeline::prepare(
            &source,
            area,
            display,
            View::default(),
            Profile::Mixed,
            [16, 18, 23],
            Budget::default(),
        )
        .unwrap();
        cold.push(at.elapsed().as_secs_f64() * 1000.0);
    }
    let source = pipeline::load(&path, Budget::default()).unwrap();
    let mut interaction = Vec::new();
    for n in 0..16 {
        let at = Instant::now();
        let _ = pipeline::prepare(
            &source,
            area,
            display,
            View {
                scale: Some(1.0),
                center: (f64::from(n) / 15.0, 0.5),
            },
            Profile::Mixed,
            [16, 18, 23],
            Budget::default(),
        )
        .unwrap();
        interaction.push(at.elapsed().as_secs_f64() * 1000.0);
    }
    let p = pipeline::prepare(
        &source,
        area,
        display,
        View::default(),
        Profile::Mixed,
        [16, 18, 23],
        Budget::default(),
    )
    .unwrap();
    let mut writer = Writer::new(Vec::new());
    let mut terminal = Terminal::new(CrosstermBackend::new(&mut writer)).unwrap();
    terminal.resize(Rect::new(0, 0, 82, 24)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                ratatui_image::Image::new(&p.protocol),
                Rect::new(1, 1, 80, 20),
            )
        })
        .unwrap();
    let mut hot = Vec::new();
    for n in 0..1000 {
        let at = Instant::now();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    ratatui_image::Image::new(&p.protocol),
                    Rect::new(1, 1, 80, 20),
                );
                frame.render_widget(
                    Paragraph::new(format!("Agent revision {n}")),
                    Rect::new(0, 23, 30, 1),
                );
            })
            .unwrap();
        hot.push(at.elapsed().as_secs_f64() * 1000.0);
    }
    drop(terminal);
    assert_eq!(writer.counter().transfers, 1);
    let stats = |values: &mut [f64]| json!({"samples":values.len(), "p50_ms":percentile(values,0.5), "p95_ms":percentile(values,0.95)});
    fs::write(out.join("workload.json"), serde_json::to_vec_pretty(&json!({
        "source":"DevTabs generated 4K texture; MIT; workload.png", "original":[3840,2160], "target":[640,360],
        "protocol":"Sixel; 8x18 diagnostic cell; byte sink, not a real terminal", "cache":"OS file cache uncontrolled; cold means no DevTabs decoded/presentation cache",
        "cold_read_decode_prepare":stats(&mut cold), "metadata_no_raster_decode":stats(&mut header),
        "continuous_100_percent_crops":stats(&mut interaction), "hot_agent_redraw":stats(&mut hot),
        "initial_image_transfers":1,"extra_image_transfers":writer.counter().transfers-1,"sixel_bytes":writer.counter().image_bytes,
        "process_memory":devtabs::perf::memory()
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "release before/after Sixel damage output comparison; byte sink only"]
fn image_scroll_output_comparison() {
    use devtabs::{
        image_output::{PixelDamage, Writer},
        preview::image::{
            self as pipeline, Budget, CellSize, Display, Profile, ProtocolChoice, View,
        },
    };
    use ratatui::{
        Terminal,
        backend::CrosstermBackend,
        layout::{Rect, Size},
        widgets::Paragraph,
    };
    let path = PathBuf::from("artifacts/image-benchmark/scroll.png");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    RgbaImage::from_fn(160, 400, |x, y| Rgba([x as u8, y as u8, 128, 255]))
        .save(&path)
        .unwrap();
    let source = pipeline::load(&path, Budget::default()).unwrap();
    let display = Display::explicit(
        ProtocolChoice::Sixel,
        Some(CellSize {
            width: 8,
            height: 18,
        }),
    );
    let p = pipeline::prepare(
        &source,
        Size::new(20, 10),
        display,
        View::default(),
        Profile::Mixed,
        [16, 18, 23],
        Budget::default(),
    )
    .unwrap();
    let size = p.protocol.size();
    let mut results = Vec::new();
    for localized in [false, true] {
        let mut writer = Writer::new(Vec::<u8>::new());
        // Match the app's Fullscreen clear semantics. Fixed uses another clear.
        let mut t = Terminal::new(CrosstermBackend::new(&mut writer)).unwrap();
        let screen = t.size().unwrap();
        assert!(screen.width >= 80 && screen.height >= 28);
        let mut damage = PixelDamage::default();
        let mut previous = Vec::new();
        let mut elapsed = Vec::new();
        for n in 0..61 {
            let area = Rect::new(screen.width / 2, 2 + n % 15, size.width, size.height);
            let plan = vec![(area, 1, 0)];
            let render = |frame: &mut ratatui::Frame| {
                frame.render_widget(Paragraph::new("UNCHANGED-AGENT"), Rect::new(0, 0, 30, 1));
                frame.render_widget(
                    Paragraph::new("UNCHANGED-DEBUG"),
                    Rect::new(0, screen.height - 1, 30, 1),
                );
                frame.render_widget(ratatui_image::Image::new(&p.protocol), area);
            };
            let at = Instant::now();
            if localized {
                damage.draw(&mut t, &plan, render).unwrap();
            } else {
                if !previous.is_empty() && previous != plan {
                    t.clear().unwrap();
                }
                t.draw(render).unwrap();
                previous = plan;
            }
            elapsed.push(at.elapsed().as_secs_f64() * 1000.0);
        }
        drop(t);
        let text = String::from_utf8_lossy(writer.inner());
        let clears = text.matches("\x1b[2J").count();
        let agent = text.matches("UNCHANGED-AGENT").count();
        if localized {
            assert_eq!(clears, 0);
            assert_eq!(agent, 1);
        } else {
            assert_eq!(clears, 60);
            assert_eq!(agent, 61);
        }
        results.push(json!({"path":if localized {"localized"} else {"old_full_clear"},"viewport":[screen.width,screen.height],"frames":61,"full_screen_clears":clears,"unchanged_agent_writes":agent,"bytes":writer.inner().len(),"sixel_transfers":writer.counter().transfers,"p50_ms":percentile(&mut elapsed,0.5),"p95_ms":percentile(&mut elapsed,0.95)}));
    }
    fs::write("artifacts/image-benchmark/scroll-output.json",serde_json::to_vec_pretty(&json!({"environment":"Windows release; fullscreen backend; byte sink, no terminal display timing","results":results})).unwrap()).unwrap();
}
