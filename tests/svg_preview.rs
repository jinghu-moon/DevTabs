use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use devtabs::{preview::image::*, sidebar::Sidebar};
use ratatui::layout::{Rect, Size};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("devtabs-svg-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, source).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn display() -> Display {
    Display::explicit(
        ProtocolChoice::Sixel,
        Some(CellSize {
            width: 8,
            height: 16,
        }),
    )
}
fn decoded(source: &Source, view: View, background: [u8; 3]) -> icy_sixel::SixelImage {
    let p = prepare(
        source,
        Size::new(8, 8),
        display(),
        view,
        Profile::Screenshot,
        background,
        Budget::default(),
    )
    .unwrap();
    let ratatui_image::protocol::Protocol::Sixel(sixel) = p.protocol else {
        panic!()
    };
    let start = sixel.data.find("\x1bP").unwrap();
    icy_sixel::SixelImage::decode(&sixel.data.as_bytes()[start..]).unwrap()
}
#[test]
fn svg_shapes_viewbox_signature_and_alpha_work_in_both_protocols() {
    let f = Fixture::new();
    let path=f.write("vector.SVG",r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect width="50" height="50" fill="#ff0000" fill-opacity="0.5"/></svg>"##);
    assert!(candidate(&path));
    assert!(supported_signature(&path));
    assert_eq!(inspect(&path, Budget::default()).unwrap(), (100, 50));
    let source = load(&path, Budget::default()).unwrap();
    assert_eq!(source.info.format, Format::Svg);
    assert_eq!(source.bytes(), 0, "vectors do not allocate a source raster");
    let image = decoded(&source, View::default(), [0, 0, 0]);
    let sample = |x: usize, y: usize| &image.pixels[(y * image.width + x) * 4..][..4];
    assert!((120..=135).contains(&sample(8, 8)[0]));
    assert!(sample(image.width - 4, 8)[0] < 5);
    let p = prepare(
        &source,
        Size::new(8, 8),
        Display::default(),
        View::default(),
        Profile::Mixed,
        [255, 255, 255],
        Budget::default(),
    )
    .unwrap();
    assert!(matches!(
        p.protocol,
        ratatui_image::protocol::Protocol::Halfblocks(_)
    ));
    let no_extension = f.0.join("no-extension");
    fs::copy(&path, &no_extension).unwrap();
    assert!(supported_signature(&no_extension));
    assert_eq!(
        load(&no_extension, Budget::default()).unwrap().info.format,
        Format::Svg
    );
}
#[test]
fn svg_zoom_renders_vectors_at_target_scale_and_crops_at_correct_origin() {
    let f = Fixture::new();
    let path=f.write("zoom.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="4" height="8" fill="red"/><rect x="4" width="4" height="8" fill="blue"/></svg>"#);
    let source = load(&path, Budget::default()).unwrap();
    let image = decoded(
        &source,
        View {
            scale: Some(8.0),
            center: (0.5, 0.5),
        },
        [0, 0, 0],
    );
    assert_eq!(image.width, 64);
    assert!((64..70).contains(&image.height)); // Sixel pads its final six-pixel band.
    let at = |x: usize| &image.pixels[(32 * image.width + x) * 4..][..4];
    assert!(at(30)[0] > 245 && at(30)[2] < 10);
    assert!(at(33)[2] > 245 && at(33)[0] < 10);
    let path=f.write("pan.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="100" height="100" fill="red"/><rect x="100" width="100" height="100" fill="blue"/></svg>"#);
    let source = load(&path, Budget::default()).unwrap();
    for (center, channel) in [((0.0, 0.5), 0), ((1.0, 0.5), 2)] {
        let image = decoded(
            &source,
            View {
                scale: Some(1.0),
                center,
            },
            [0, 0, 0],
        );
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .take(image.width * 100)
                .all(|p| p[channel] > 245)
        );
    }
}
#[test]
fn svg_rejects_invalid_xml_dimensions_dtd_and_resource_limits() {
    let f = Fixture::new();
    for (name, xml) in [
        ("broken.svg", "<svg"),
        (
            "wrong.svg",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"/>"#,
        ),
        (
            "zero.svg",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="10"/>"#,
        ),
        (
            "large.svg",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="32769" height="10"/>"#,
        ),
        (
            "pixels.svg",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="10000"/>"#,
        ),
        (
            "dtd.svg",
            r#"<!DOCTYPE svg [<!ENTITY size "100">]><svg xmlns="http://www.w3.org/2000/svg" width="&size;" height="10"/>"#,
        ),
    ] {
        let p = f.write(name, xml);
        assert!(load(&p, Budget::default()).is_err(), "accepted {name}");
        assert!(inspect(&p, Budget::default()).is_err(), "inspected {name}");
    }
    let path = f.write(
        "bytes.svg",
        &format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><!--{}--></svg>"#,
            "x".repeat(4 * 1024 * 1024)
        ),
    );
    assert!(format!("{:#}", load(&path, Budget::default()).err().unwrap()).contains("4 MiB"));
    let path = f.write(
        "deep.svg",
        &format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">{}{}</svg>"#,
            "<g>".repeat(140),
            "</g>".repeat(140)
        ),
    );
    assert!(load(&path, Budget::default()).is_err());
    let path = f.write(
        "nodes.svg",
        &format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">{}</svg>"#,
            "<path/>".repeat(50_001)
        ),
    );
    assert!(load(&path, Budget::default()).is_err());
}
#[test]
fn svg_external_and_embedded_image_references_do_not_load() {
    use base64::Engine;
    let f = Fixture::new();
    let png = f.0.join("existing.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255]))
        .save(&png)
        .unwrap();
    let nested = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#;
    let nested = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(nested)
    );
    let embedded_png = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(fs::read(&png).unwrap())
    );
    for href in [
        png.to_str().unwrap(),
        "https://example.com/image.png",
        &nested,
        &embedded_png,
    ] {
        let path=f.write("external.svg",&format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image href="{href}" width="16" height="16"/></svg>"#));
        let source = load(&path, Budget::default()).unwrap();
        let image = decoded(&source, View::default(), [0, 0, 0]);
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] < 5 && p[1] < 5 && p[2] < 5)
        );
    }
}
fn wait(s: &mut Sidebar, f: impl Fn(&Sidebar) -> bool) {
    let start = Instant::now();
    while !f(s) {
        s.tick(true);
        assert!(start.elapsed() < Duration::from_secs(20), "{}", s.status());
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn svg_markdown_and_standalone_tabs_share_pipeline_and_history() {
    let f = Fixture::new();
    f.write("diagram.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 40"><rect width="80" height="40" fill="lime"/></svg>"#);
    let md = f.write("read.md", "# SVG\n\n![diagram](diagram.svg)\n");
    let mut s = Sidebar::new(f.0.clone());
    s.visible = false;
    s.configure_images(display());
    s.open(md.clone(), true).unwrap();
    s.update_layout(Rect::new(0, 0, 120, 35), false);
    wait(&mut s, |s| !s.image_plans().is_empty());
    s.open(f.0.join("diagram.svg"), true).unwrap();
    wait(&mut s, |s| s.image_plan().is_some());
    assert!(s.document.as_ref().unwrap().is_image());
    s.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
        .unwrap();
    wait(&mut s, |s| !s.image_plans().is_empty());
    assert_eq!(s.document.as_ref().unwrap().path, md);
}
#[test]
fn svg_text_uses_system_font_shapes() {
    let f = Fixture::new();
    let p=f.write("text.svg",r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="5" y="35" font-family="sans-serif" font-size="30" fill="white">DevTabs</text></svg>"#);
    let source = load(&p, Budget::default()).unwrap();
    let image = decoded(&source, View::default(), [0, 0, 0]);
    assert!(
        image.pixels.as_chunks::<4>().0.iter().any(|p| p[0] > 100),
        "system fonts must render text"
    );
}
