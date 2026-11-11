//! Static SVG parsing and viewport rasterization; no browser, scripts or file resolver.
use super::*;
use resvg::{tiny_skia, usvg};
use std::sync::OnceLock;

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODES: u32 = 50_000;

pub(super) fn signature(bytes: &[u8]) -> bool {
    static ROOT: OnceLock<regex::Regex> = OnceLock::new();
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(8192)]);
    ROOT.get_or_init(|| {
        regex::Regex::new(
            r"(?s)^\x{FEFF}?\s*(?:<\?.*?\?>\s*|<!--.*?-->\s*)*<(?:[A-Za-z_][\w.-]*:)?svg(?:\s|/?>)",
        )
        .unwrap()
    })
    .is_match(&prefix)
}

pub(super) fn size(tree: &usvg::Tree) -> (u32, u32) {
    (
        tree.size().width().ceil() as u32,
        tree.size().height().ceil() as u32,
    )
}

pub(super) fn parse(bytes: &[u8], budget: Budget, text: bool) -> Result<usvg::Tree> {
    ensure!(bytes.len() <= MAX_BYTES, "SVG exceeds 4 MiB byte budget");
    let xml = std::str::from_utf8(bytes)
        .context("SVG must be UTF-8")?
        .trim_start_matches('\u{feff}');
    let doc = usvg::roxmltree::Document::parse_with_options(
        xml,
        usvg::roxmltree::ParsingOptions {
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .context("Invalid SVG XML (DTD is not supported)")?;
    let root = doc.root_element();
    ensure!(
        root.tag_name().name() == "svg"
            && root.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
        "Expected SVG root in the SVG namespace"
    );
    ensure!(
        doc.descendants()
            .all(|n| n.ancestors().take(130).count() < 130),
        "SVG nesting exceeds 128 levels"
    );
    let has_text = doc
        .descendants()
        .any(|n| n.has_tag_name(("http://www.w3.org/2000/svg", "text")));
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        font_family: "Segoe UI".into(),
        ..Default::default()
    };
    if text && has_text {
        static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut fonts = usvg::fontdb::Database::new();
                fonts.load_system_fonts();
                fonts.set_sans_serif_family("Segoe UI");
                fonts.set_serif_family("Times New Roman");
                fonts.set_monospace_family("Consolas");
                Arc::new(fonts)
            })
            .clone();
    }
    let tree = usvg::Tree::from_xmltree(&doc, &options).context("Cannot parse SVG")?;
    let (w, h) = size(&tree);
    dimensions(w, h, budget)?;
    Ok(tree)
}

pub(super) fn render(tree: &usvg::Tree, geo: &Geometry, background: [u8; 3]) -> Result<RgbaImage> {
    let _time = crate::perf::span(crate::perf::Metric::ImageSvgRender);
    let (w, h) = geo.pixels;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).context("Cannot allocate SVG viewport")?;
    pixmap.fill(tiny_skia::Color::from_rgba8(
        background[0],
        background[1],
        background[2],
        255,
    ));
    // Normalize fractional intrinsic sizes to the pixel grid used by geometry.
    let (sw, sh) = size(tree);
    let scale = geo.scale as f32;
    let transform = tiny_skia::Transform::from_row(
        scale * sw as f32 / tree.size().width(),
        0.0,
        0.0,
        scale * sh as f32 / tree.size().height(),
        -(geo.origin.0 * geo.scale) as f32,
        -(geo.origin.1 * geo.scale) as f32,
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    // Opaque destination => tiny-skia's premultiplied RGB is already straight RGB.
    let bytes = pixmap.take();
    crate::perf::gauge(
        crate::perf::Gauge::TargetPixels,
        u64::from(w) * u64::from(h),
    );
    crate::perf::gauge(crate::perf::Gauge::TargetBytes, bytes.len() as u64);
    RgbaImage::from_raw(w, h, bytes).context("Invalid SVG viewport pixels")
}
