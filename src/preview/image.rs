//! Bounded image preparation. Workers never read terminal input or write output.
pub mod probe;
mod svg;
use anyhow::{Context, Result, bail, ensure};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::protocol::{Protocol, halfblocks::Halfblocks, sixel::Sixel};
use std::{
    fs::{File, Metadata},
    hash::{Hash, Hasher},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime},
};

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ProtocolChoice {
    #[default]
    Auto,
    Sixel,
    Halfblock,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CellSize {
    pub width: u16,
    pub height: u16,
}
impl std::str::FromStr for CellSize {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, String> {
        let (w, h) = value.split_once('x').ok_or("Use WIDTHxHEIGHT")?;
        let size = Self {
            width: w.parse().map_err(|_| "Invalid cell width")?,
            height: h.parse().map_err(|_| "Invalid cell height")?,
        };
        if !(1..=256).contains(&size.width) || !(1..=256).contains(&size.height) {
            return Err("Cell dimensions must be 1..256 pixels".into());
        }
        Ok(size)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub protocol: ProtocolChoice,
    pub cell: CellSize,
    pub estimated: bool,
    pub automatic: bool,
}
impl Default for Display {
    fn default() -> Self {
        Self {
            protocol: ProtocolChoice::Halfblock,
            cell: CellSize {
                width: 8,
                height: 16,
            },
            estimated: true,
            automatic: true,
        }
    }
}
impl Display {
    pub fn explicit(protocol: ProtocolChoice, cell: Option<CellSize>) -> Self {
        // No environment heuristics: Sixel requires measured or explicitly supplied pixels.
        Self {
            protocol: if protocol == ProtocolChoice::Sixel && cell.is_some() {
                ProtocolChoice::Sixel
            } else {
                ProtocolChoice::Halfblock
            },
            cell: cell.unwrap_or(Self::default().cell),
            estimated: cell.is_none(),
            automatic: protocol == ProtocolChoice::Auto,
        }
    }
    pub fn label(self) -> &'static str {
        if self.protocol == ProtocolChoice::Sixel {
            "Sixel"
        } else if self.estimated {
            "Half-block · estimated cell"
        } else {
            "Half-block"
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    Photo,
    Screenshot,
    #[default]
    Mixed,
}
impl Profile {
    pub fn next(self) -> Self {
        match self {
            Self::Mixed => Self::Photo,
            Self::Photo => Self::Screenshot,
            Self::Screenshot => Self::Mixed,
        }
    }
    pub fn options(self) -> icy_sixel::EncodeOptions {
        icy_sixel::EncodeOptions {
            diffusion: match self {
                Self::Photo => 0.875,
                Self::Screenshot => 0.0,
                Self::Mixed => 0.5,
            },
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub scale: Option<f64>,
    pub center: (f64, f64),
}
impl Default for View {
    fn default() -> Self {
        Self {
            scale: None,
            center: (0.5, 0.5),
        }
    }
}
impl View {
    pub fn zoom(&mut self, factor: f64, current: f64) {
        self.scale = Some((self.scale.unwrap_or(current) * factor).clamp(0.125, 8.0));
    }
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.center.0 = (self.center.0 + dx).clamp(0.0, 1.0);
        self.center.1 = (self.center.1 + dy).clamp(0.0, 1.0);
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub file_bytes: u64,
    pub side: u32,
    pub source_pixels: u64,
    pub target_pixels: u64,
    pub encoded_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            file_bytes: 32 * 1024 * 1024,
            side: 32768,
            source_pixels: 32 * 1024 * 1024,
            target_pixels: 2 * 1024 * 1024,
            encoded_bytes: 8 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub identity: Option<(u64, [u8; 16])>,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub digest: u64,
}
fn version(m: &Metadata, digest: u64) -> Version {
    Version {
        identity: None,
        bytes: m.len(),
        modified: m.modified().ok(),
        created: m.created().ok(),
        digest,
    }
}
#[derive(Clone, Debug)]
pub struct MetadataInfo {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub alpha: bool,
    pub version: Version,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    WebP,
    Svg,
}
impl Format {
    fn raster(self) -> Result<ImageFormat> {
        Ok(match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::WebP => ImageFormat::WebP,
            Self::Svg => bail!("SVG is a vector image"),
        })
    }
}
enum SourceData {
    Raster(RgbaImage),
    Svg(Box<resvg::usvg::Tree>),
}
pub struct Source {
    data: SourceData,
    pub info: MetadataInfo,
}
impl Source {
    pub fn bytes(&self) -> usize {
        match &self.data {
            SourceData::Raster(pixels) => pixels.as_raw().len(),
            SourceData::Svg(_) => 0,
        }
    }
}
pub fn candidate(path: &Path) -> bool {
    path.extension().is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().to_str(),
            Some(
                "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "gif"
                    | "apng"
                    | "bmp"
                    | "ico"
                    | "avif"
                    | "tif"
                    | "tiff"
                    | "svg"
            )
        )
    })
}
pub fn supported_signature(path: &Path) -> bool {
    let mut bytes = [0u8; 8192];
    File::open(path)
        .ok()
        .and_then(|mut file| file.read(&mut bytes).ok())
        .is_some_and(|n| {
            svg::signature(&bytes[..n])
                || matches!(
                    image::guess_format(&bytes[..n]),
                    Ok(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
                )
        })
}
fn dimensions(w: u32, h: u32, b: Budget) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= b.side && h <= b.side,
        "Image dimensions exceed budget"
    );
    ensure!(
        u64::from(w) * u64::from(h) <= b.source_pixels,
        "Image pixel budget exceeded"
    );
    Ok(())
}
fn animated(bytes: &[u8], format: ImageFormat) -> Result<bool> {
    // Parse container chunks; byte substrings can also occur in compressed image data.
    let mut at = match format {
        ImageFormat::Png => 8,
        ImageFormat::WebP => 12,
        _ => return Ok(false),
    };
    while at + 8 <= bytes.len() {
        let len = if format == ImageFormat::Png {
            u32::from_be_bytes(bytes[at..at + 4].try_into()?) as usize
        } else {
            u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize
        };
        let name = if format == ImageFormat::Png {
            &bytes[at + 4..at + 8]
        } else {
            &bytes[at..at + 4]
        };
        if name == b"acTL" || name == b"ANIM" || name == b"ANMF" {
            return Ok(true);
        }
        if name == b"VP8X" && bytes.get(at + 8).is_some_and(|v| v & 2 != 0) {
            return Ok(true);
        }
        at = at
            .checked_add(8)
            .and_then(|v| v.checked_add(len))
            .and_then(|v| {
                v.checked_add(if format == ImageFormat::Png {
                    4
                } else {
                    len % 2
                })
            })
            .context("Invalid image chunk length")?;
        ensure!(at <= bytes.len(), "Truncated image container");
    }
    Ok(false)
}
fn read_encoded(path: &Path, budget: Budget) -> Result<(Vec<u8>, Format, Version)> {
    let _time = crate::perf::span(crate::perf::Metric::ImageRead);
    let mut file = File::open(path).context("Cannot open image")?;
    let identity = crate::editor::file_id(&file).ok();
    let before = file.metadata()?;
    ensure!(before.is_file(), "Image must be a regular file");
    ensure!(
        before.len() <= budget.file_bytes,
        "Compressed image exceeds byte budget"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(budget.file_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= budget.file_bytes,
        "Image grew beyond byte budget"
    );
    let after = file.metadata()?;
    ensure!(
        version(&before, 0) == version(&after, 0) && after.len() == bytes.len() as u64,
        "Image changed during read; retry"
    );
    let latest = File::open(path)?;
    ensure!(
        version(&latest.metadata()?, 0) == version(&after, 0)
            && crate::editor::file_id(&latest).ok() == identity,
        "Image was replaced during read; retry"
    );
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut digest);
    let format = match image::guess_format(&bytes) {
        Ok(ImageFormat::Png) => Format::Png,
        Ok(ImageFormat::Jpeg) => Format::Jpeg,
        Ok(ImageFormat::WebP) => Format::WebP,
        _ if svg::signature(&bytes)
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("svg")) =>
        {
            Format::Svg
        }
        Ok(_) => bail!("Unsupported image format (PNG, JPEG, static WebP and SVG only)"),
        Err(e) => return Err(e).context("Unrecognized or damaged image"),
    };
    if format != Format::Svg {
        ensure!(
            !animated(&bytes, format.raster()?)?,
            "Animated images are not supported"
        );
    }
    Ok((
        bytes,
        format,
        Version {
            identity,
            ..version(&after, digest.finish())
        },
    ))
}
fn decoder(bytes: Vec<u8>, format: ImageFormat, budget: Budget) -> Result<impl ImageDecoder> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(budget.side);
    limits.max_image_height = Some(budget.side);
    limits.max_alloc = Some(192 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.into_decoder()?)
}
/// Inspect the bounded container without allocating or decoding its raster.
/// Reading the compressed bytes also validates animation and snapshot identity.
pub fn inspect(path: &Path, budget: Budget) -> Result<(u32, u32)> {
    let _time = crate::perf::span(crate::perf::Metric::ImageInspect);
    let (bytes, format, _) = read_encoded(path, budget)?;
    if format == Format::Svg {
        let tree = svg::parse(&bytes, budget, false)?;
        crate::perf::add(crate::perf::Count::ImageInspections, 1);
        return Ok(svg::size(&tree));
    }
    let mut decoder = decoder(bytes, format.raster()?, budget)?;
    let (w, h) = decoder.dimensions();
    dimensions(w, h, budget)?;
    crate::perf::add(crate::perf::Count::ImageInspections, 1);
    Ok(if decoder.orientation()?.to_exif() >= 5 {
        (h, w)
    } else {
        (w, h)
    })
}
pub fn load(path: &Path, budget: Budget) -> Result<Source> {
    let (bytes, format, version) = read_encoded(path, budget)?;
    let _decode = crate::perf::span(crate::perf::Metric::ImageDecode);
    if format == Format::Svg {
        let tree = svg::parse(&bytes, budget, true)?;
        let (width, height) = svg::size(&tree);
        crate::perf::gauge(crate::perf::Gauge::SourcePixels, 0);
        crate::perf::gauge(crate::perf::Gauge::SourceBytes, 0);
        crate::perf::add(crate::perf::Count::ImageDecodes, 1);
        return Ok(Source {
            data: SourceData::Svg(Box::new(tree)),
            info: MetadataInfo {
                width,
                height,
                format,
                alpha: true,
                version,
            },
        });
    }
    let mut decoder = decoder(bytes, format.raster()?, budget)?;
    let (w, h) = decoder.dimensions();
    dimensions(w, h, budget)?;
    let orientation = decoder.orientation()?;
    let alpha = decoder.color_type().has_alpha();
    let mut decoded = DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let pixels = decoded.into_rgba8();
    crate::perf::gauge(
        crate::perf::Gauge::SourcePixels,
        u64::from(pixels.width()) * u64::from(pixels.height()),
    );
    crate::perf::gauge(
        crate::perf::Gauge::SourceBytes,
        pixels.as_raw().len() as u64,
    );
    crate::perf::add(crate::perf::Count::ImageDecodes, 1);
    Ok(Source {
        info: MetadataInfo {
            width: pixels.width(),
            height: pixels.height(),
            format,
            alpha,
            version,
        },
        data: SourceData::Raster(pixels),
    })
}
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub scale: f64,
    pub origin: (f64, f64),
    pub size: Size,
    pub pixels: (u32, u32),
}
pub fn geometry(
    w: u32,
    h: u32,
    area: Size,
    display: Display,
    view: View,
    budget: Budget,
) -> Result<Geometry> {
    ensure!(area.width > 0 && area.height > 0, "No image viewport space");
    let aw = u32::from(area.width) * u32::from(display.cell.width);
    let ah = u32::from(area.height) * u32::from(display.cell.height);
    ensure!(w > 0 && h > 0 && aw > 0 && ah > 0, "Invalid image geometry");
    ensure!(
        budget.target_pixels > 0,
        "Target image pixel budget is zero"
    );
    let fit = (f64::from(aw) / f64::from(w))
        .min(f64::from(ah) / f64::from(h))
        .min(1.0);
    let mut scale = view.scale.unwrap_or(fit);
    ensure!(
        scale.is_finite() && scale > 0.0 && scale <= 8.0,
        "Invalid image scale"
    );
    let pixels_at = |scale: f64| {
        (
            (f64::from(w) * scale).ceil().min(f64::from(aw)).max(1.0) as u32,
            (f64::from(h) * scale).ceil().min(f64::from(ah)).max(1.0) as u32,
        )
    };
    let within_budget =
        |(tw, th): (u32, u32)| u64::from(tw) * u64::from(th) <= budget.target_pixels;
    if view.scale.is_none() && !within_budget(pixels_at(scale)) {
        // Fit accounts for the viewport AND target allocation. Search on rounded
        // dimensions, including extremely thin images and one-pixel budgets.
        // Manual/100% scales never silently change their pixel ratio.
        let (mut low, mut high) = (0.0, scale);
        for _ in 0..48 {
            let candidate = (low + high) / 2.0;
            if within_budget(pixels_at(candidate)) {
                low = candidate;
            } else {
                high = candidate;
            }
        }
        scale = low;
    }
    let (tw, th) = pixels_at(scale);
    ensure!(
        u64::from(tw) * u64::from(th) <= budget.target_pixels,
        "Target image exceeds pixel budget; use Fit or a smaller viewport"
    );
    let origin = (
        (view.center.0 * f64::from(w) - f64::from(tw) / scale / 2.0)
            .clamp(0.0, (f64::from(w) - f64::from(tw) / scale).max(0.0)),
        (view.center.1 * f64::from(h) - f64::from(th) / scale / 2.0)
            .clamp(0.0, (f64::from(h) - f64::from(th) / scale).max(0.0)),
    );
    Ok(Geometry {
        scale,
        origin,
        size: Size::new(
            tw.div_ceil(u32::from(display.cell.width)) as u16,
            th.div_ceil(u32::from(display.cell.height)) as u16,
        ),
        pixels: (tw, th),
    })
}
pub struct Presentation {
    pub display: Display,
    pub fallback: Option<String>,
    pub protocol: Protocol,
    pub info: MetadataInfo,
    pub geometry: Geometry,
    pub bytes: usize,
}
/// Resample only the visible source crop, never an 800% full image.
pub fn prepare(
    source: &Source,
    area: Size,
    display: Display,
    view: View,
    profile: Profile,
    background: [u8; 3],
    budget: Budget,
) -> Result<Presentation> {
    let geo = geometry(
        source.info.width,
        source.info.height,
        area,
        display,
        view,
        budget,
    )?;
    let output = resample(source, &geo, background)?;
    encode(source, output, geo, display, profile, budget)
}
fn resample(source: &Source, geo: &Geometry, background: [u8; 3]) -> Result<RgbaImage> {
    let _resize = crate::perf::span(crate::perf::Metric::ImageResize);
    let (tw, th) = geo.pixels;
    let input = match &source.data {
        SourceData::Raster(pixels) => pixels,
        SourceData::Svg(tree) => return svg::render(tree, geo, background),
    };
    let x = (geo.origin.0.floor() as u32).saturating_sub(3);
    let y = (geo.origin.1.floor() as u32).saturating_sub(3);
    let visible_width = (f64::from(tw) / geo.scale).min(f64::from(input.width()) - geo.origin.0);
    let visible_height = (f64::from(th) / geo.scale).min(f64::from(input.height()) - geo.origin.1);
    let width = ((geo.origin.0 + visible_width).ceil() as u32)
        .saturating_add(3)
        .min(input.width())
        - x;
    let height = ((geo.origin.1 + visible_height).ceil() as u32)
        .saturating_add(3)
        .min(input.height())
        - y;
    let mut crop = image::imageops::crop_imm(input, x, y, width, height).to_image();
    // Composite before filtering so invisible RGB does not bleed into transparent edges.
    for p in crop.pixels_mut() {
        let alpha = u32::from(p[3]);
        for c in 0..3 {
            p[c] = ((u32::from(p[c]) * alpha + u32::from(background[c]) * (255 - alpha) + 127)
                / 255) as u8;
        }
        p[3] = 255;
    }
    let crop = DynamicImage::ImageRgba8(crop);
    let mut output = DynamicImage::new_rgba8(tw, th);
    let options = fast_image_resize::ResizeOptions::new()
        .resize_alg(fast_image_resize::ResizeAlg::Convolution(
            fast_image_resize::FilterType::CatmullRom,
        ))
        .crop(
            geo.origin.0 - f64::from(x),
            geo.origin.1 - f64::from(y),
            visible_width,
            visible_height,
        )
        .use_alpha(false);
    fast_image_resize::Resizer::new().resize(&crop, &mut output, Some(&options))?;
    let output = output.into_rgba8();
    crate::perf::gauge(
        crate::perf::Gauge::TargetPixels,
        u64::from(tw) * u64::from(th),
    );
    crate::perf::gauge(
        crate::perf::Gauge::TargetBytes,
        (output.as_raw().len() + crop.as_bytes().len()) as u64,
    );
    Ok(output)
}
fn encode(
    source: &Source,
    output: RgbaImage,
    geo: Geometry,
    display: Display,
    profile: Profile,
    budget: Budget,
) -> Result<Presentation> {
    let _encode = crate::perf::span(crate::perf::Metric::ImageEncode);
    let (protocol, bytes) = if display.protocol == ProtocolChoice::Sixel {
        let sixel = Sixel::with_options(
            DynamicImage::ImageRgba8(output),
            geo.size,
            false,
            &profile.options(),
        )?;
        ensure!(
            sixel.data.len() <= budget.encoded_bytes,
            "Sixel output exceeds byte budget; use Half-block or a smaller viewport"
        );
        let bytes = sixel.data.capacity() + std::mem::size_of::<Sixel>();
        (Protocol::Sixel(sixel), bytes)
    } else {
        let halfblocks = Halfblocks::new(DynamicImage::ImageRgba8(output), geo.size)?;
        let bytes = halfblocks.retained_bytes();
        (Protocol::Halfblocks(halfblocks), bytes)
    };
    ensure!(
        bytes <= 32 * 1024 * 1024,
        "Presentation cache budget exceeded"
    );
    crate::perf::add(crate::perf::Count::ImageEncodes, 1);
    Ok(Presentation {
        display,
        fallback: None,
        protocol,
        info: source.info.clone(),
        geometry: geo,
        bytes,
    })
}
#[derive(Clone)]
pub struct Request {
    pub kind: RequestKind,
    pub local_target: Option<(PathBuf, String)>,
    pub generation: u64,
    pub path: PathBuf,
    pub area: Size,
    pub display: Display,
    pub view: View,
    pub profile: Profile,
    pub background: [u8; 3],
    pub reload: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Pixels,
    Dimensions,
}
pub enum Prepared {
    Pixels(Box<Presentation>),
    Dimensions((u32, u32)),
}
pub struct Completed {
    pub generation: u64,
    pub path: PathBuf,
    pub result: Result<Prepared>,
}
#[derive(Default)]
struct Mailbox {
    pending: Option<(Request, Instant)>,
    done: Option<Completed>,
    closed: bool,
    release: bool,
}
pub struct Worker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    generation: Arc<AtomicU64>,
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}
impl Worker {
    pub fn new() -> Self {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let shared = mailbox.clone();
        let current = generation.clone();
        std::thread::Builder::new()
            .name("image-preview".into())
            .spawn(move || {
                let mut cached: Option<(PathBuf, Source)> = None;
                loop {
                    let request = {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if state.closed {
                                return;
                            }
                            if state.release {
                                cached = None;
                                crate::perf::gauge(crate::perf::Gauge::SourceBytes, 0);
                                state.release = false;
                            }
                            if let Some((r, queued)) = state.pending.take() {
                                crate::perf::record(
                                    crate::perf::Metric::ImageQueueWait,
                                    queued.elapsed(),
                                );
                                break r;
                            }
                            state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let result = (|| -> Result<Prepared> {
                        let path = if let Some((base, target)) = &request.local_target {
                            local_path(base, target)?
                        } else {
                            request.path.clone()
                        };
                        if request.kind == RequestKind::Dimensions {
                            return inspect(&path, Budget::default()).map(Prepared::Dimensions);
                        }
                        let unchanged = cached.as_ref().is_some_and(|(p, source)| {
                            *p == path
                                && File::open(&path).ok().is_some_and(|file| {
                                    file.metadata().ok().is_some_and(|meta| {
                                        let v = version(&meta, source.info.version.digest);
                                        v.bytes == source.info.version.bytes
                                            && v.modified == source.info.version.modified
                                            && v.created == source.info.version.created
                                            && crate::editor::file_id(&file).ok()
                                                == source.info.version.identity
                                    })
                                })
                        });
                        if request.reload || !unchanged {
                            crate::perf::add(crate::perf::Count::ImageCacheMisses, 1);
                            cached = None;
                            let source = load(&path, Budget::default())?;
                            cached = Some((path, source));
                        } else {
                            crate::perf::add(crate::perf::Count::ImageCacheHits, 1);
                        }
                        if current.load(Ordering::Relaxed) != request.generation {
                            bail!("Superseded image request");
                        }
                        let source = &cached.as_ref().unwrap().1;
                        let geo = geometry(
                            source.info.width,
                            source.info.height,
                            request.area,
                            request.display,
                            request.view,
                            Budget::default(),
                        )?;
                        let output = resample(source, &geo, request.background)?;
                        if current.load(Ordering::Relaxed) != request.generation {
                            bail!("Superseded image request");
                        }
                        let result = encode(
                            source,
                            output,
                            geo,
                            request.display,
                            request.profile,
                            Budget::default(),
                        );
                        if result.is_err()
                            && request.display.automatic
                            && request.display.protocol == ProtocolChoice::Sixel
                        {
                            let reason = format!("Sixel failed: {:#}", result.err().unwrap());
                            let mut display = request.display;
                            display.protocol = ProtocolChoice::Halfblock;
                            let mut fallback = prepare(
                                source,
                                request.area,
                                display,
                                request.view,
                                request.profile,
                                request.background,
                                Budget::default(),
                            )?;
                            fallback.fallback = Some(reason);
                            crate::perf::add(crate::perf::Count::ImageFallbacks, 1);
                            Ok(Prepared::Pixels(Box::new(fallback)))
                        } else {
                            result.map(|p| Prepared::Pixels(Box::new(p)))
                        }
                    })();
                    if current.load(Ordering::Relaxed) != request.generation {
                        crate::perf::add(crate::perf::Count::ImageStale, 1);
                        continue;
                    }
                    let mut state = shared.0.lock().unwrap_or_else(|e| e.into_inner());
                    if !state.closed && current.load(Ordering::Relaxed) == request.generation {
                        state.done = Some(Completed {
                            generation: request.generation,
                            path: request.path,
                            result,
                        });
                    }
                }
            })
            .expect("image worker");
        Self {
            mailbox,
            generation,
        }
    }
    pub fn submit(&self, request: Request) {
        let mut state = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        self.generation.store(request.generation, Ordering::Relaxed);
        if state.pending.replace((request, Instant::now())).is_some() {
            crate::perf::add(crate::perf::Count::ImageReplaced, 1);
        }
        state.done = None;
        self.mailbox.1.notify_one();
    }
    pub fn take(&self) -> Option<Completed> {
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .done
            .take()
    }
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let mut state = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        state.pending = None;
        state.done = None;
        self.mailbox.1.notify_one();
    }
    pub fn release(&self) {
        self.cancel();
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release = true;
        self.mailbox.1.notify_one();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        self.mailbox.1.notify_one();
    }
}

/// Automatic Markdown loads are restricted to ordinary local paths, including symlink targets.
pub fn local_path(base: &Path, target: &str) -> Result<PathBuf> {
    let crate::preview::reading::Destination::File(mut path, _) =
        crate::preview::reading::destination(base, target)?
    else {
        bail!("Only local image files load automatically; click remote links to open externally");
    };
    fn ordinary(path: &Path) -> Result<()> {
        let value = path.to_string_lossy();
        ensure!(
            !value.starts_with("\\\\")
                && !value.starts_with("//")
                && !value.chars().any(char::is_control),
            "Network and device image paths do not load automatically"
        );
        #[cfg(windows)]
        if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
            ensure!(
                matches!(prefix.kind(), std::path::Prefix::Disk(_)),
                "Network and device image paths do not load automatically"
            );
        }
        Ok(())
    }
    ordinary(&path)?;
    for _ in 0..16 {
        let scan = path.clone();
        let mut resolved = PathBuf::new();
        let components = scan.components().collect::<Vec<_>>();
        let mut changed = false;
        for (index, component) in components.iter().enumerate() {
            resolved.push(component.as_os_str());
            ordinary(&resolved)?;
            if matches!(component, std::path::Component::Normal(_)) {
                let metadata = std::fs::symlink_metadata(&resolved)
                    .context("Local image path is unavailable")?;
                if metadata.file_type().is_symlink() {
                    let link = std::fs::read_link(&resolved)?;
                    ordinary(&link)?;
                    path = resolved.parent().unwrap_or(Path::new(".")).join(link);
                    for tail in &components[index + 1..] {
                        path.push(tail.as_os_str());
                    }
                    ordinary(&path)?;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            let path = dunce::canonicalize(path)?;
            ordinary(&path)?;
            ensure!(path.is_file(), "Image link is not a regular file");
            return Ok(path);
        }
    }
    bail!("Image symlink nesting exceeds limit")
}
