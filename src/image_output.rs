//! Counts real protocol bytes, even when write() splits a DCS across calls.
use ratatui::{Frame, backend::Backend, buffer::CellDiffOption, layout::Rect};
use std::io::{self, Write};

pub type ImagePlan = (Rect, u64, u16);

/// Subtract one opaque panel without changing image size or source coordinates.
pub fn visible_parts(area: Rect, cover: Option<Rect>) -> Vec<Rect> {
    let Some(hit) = cover
        .map(|r| r.intersection(area))
        .filter(|r| !r.is_empty())
    else {
        return vec![area];
    };
    [
        Rect::new(area.x, area.y, area.width, hit.y - area.y),
        Rect::new(
            area.x,
            hit.bottom(),
            area.width,
            area.bottom() - hit.bottom(),
        ),
        Rect::new(area.x, hit.y, hit.x - area.x, hit.height),
        Rect::new(hit.right(), hit.y, area.right() - hit.right(), hit.height),
    ]
    .into_iter()
    .filter(|r| !r.is_empty())
    .collect()
}

pub fn composed_plans(plan: ImagePlan, cover: Option<Rect>) -> Vec<ImagePlan> {
    use std::hash::{Hash, Hasher};
    visible_parts(plan.0, cover)
        .into_iter()
        .map(|area| {
            if area == plan.0 {
                return plan;
            }
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (plan, area).hash(&mut hash);
            (area, hash.finish(), plan.2 + area.y - plan.0.y)
        })
        .collect()
}

#[derive(Default)]
pub struct Composition {
    // Geometry changes replace only the affected payloads. A static Debug frame
    // does no decoding, quantization or clipping work, and transmits no image.
    cache: std::cell::RefCell<std::collections::VecDeque<CachedPiece>>,
}
type CachedPiece = (
    ImagePlan,
    ratatui::layout::Size,
    ratatui_image::protocol::Protocol,
);
impl Composition {
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }
    pub fn draw(
        &self,
        frame: &mut Frame,
        source: &ratatui_image::protocol::sixel::Sixel,
        plan: ImagePlan,
        cover: Option<Rect>,
        cell: ratatui::layout::Size,
    ) {
        use ratatui_image::{Image, protocol::Protocol};
        let mut cache = self.cache.borrow_mut();
        for piece in composed_plans(plan, cover) {
            if let Some((_, _, protocol)) = cache.iter().find(|(p, c, _)| *p == piece && *c == cell)
            {
                crate::perf::add(crate::perf::Count::ImageCompositionHits, 1);
                frame.render_widget(Image::new(protocol), piece.0);
                continue;
            }
            let crop = Rect::new(piece.0.x - plan.0.x, piece.2, piece.0.width, piece.0.height);
            let _clip = crate::perf::span(crate::perf::Metric::ImageSlice);
            let Some(sixel) = source.crop_cells(crop, cell) else {
                continue;
            };
            crate::perf::add(crate::perf::Count::ImageClips, 1);
            let protocol = Protocol::Sixel(sixel);
            // Retain at most 32 MiB of derived encodings and 128 visible pieces.
            let bytes = |p: &Protocol| match p {
                Protocol::Sixel(s) => s.data.capacity(),
                _ => 0,
            };
            while !cache.is_empty()
                && (cache.len() >= 128
                    || cache.iter().map(|(_, _, p)| bytes(p)).sum::<usize>() + bytes(&protocol)
                        > 32 * 1024 * 1024)
            {
                cache.pop_front();
            }
            frame.render_widget(Image::new(&protocol), piece.0);
            cache.push_back((piece, cell, protocol));
        }
    }
}

/// Repaint only cells formerly occupied by changed/removed Sixel images.
/// Ordinary buffer equality cannot detect pixels left behind in blank cells.
#[derive(Default)]
pub struct PixelDamage {
    previous: Vec<ImagePlan>,
}
impl PixelDamage {
    pub fn apply(&mut self, frame: &mut Frame, next: &[ImagePlan]) {
        let mut damaged = false;
        for (area, _, _) in self.previous.iter().filter(|plan| !next.contains(plan)) {
            let area = area.intersection(frame.area());
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let cell = &mut frame.buffer_mut()[(x, y)];
                    // New images own their pixels and clearing sequences. Repaint
                    // exposed text/blank cells, preserving image anchors and Skip.
                    if matches!(cell.diff_option, CellDiffOption::None) {
                        cell.set_diff_option(CellDiffOption::AlwaysUpdate);
                    }
                }
            }
            damaged |= !area.is_empty();
        }
        if damaged {
            crate::perf::add(crate::perf::Count::ImageClears, 1);
        }
        self.previous.clear();
        self.previous.extend_from_slice(next);
    }
    /// Keep image erasing and replacement in one synchronized output transaction.
    /// Unsupported terminals ignore DEC 2026; no capability assumption is needed.
    pub fn draw<B, F>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
        next: &[ImagePlan],
        render: F,
    ) -> io::Result<()>
    where
        B: Backend<Error = io::Error> + Write,
        F: FnOnce(&mut Frame),
    {
        let images = !self.previous.is_empty() || !next.is_empty();
        if images {
            crossterm::queue!(
                terminal.backend_mut(),
                crossterm::terminal::BeginSynchronizedUpdate
            )?;
        }
        let result = terminal
            .draw(|frame| {
                render(frame);
                self.apply(frame, next);
            })
            .map(|_| ());
        let end = if images {
            crossterm::execute!(
                terminal.backend_mut(),
                crossterm::terminal::EndSynchronizedUpdate
            )
        } else {
            Ok(())
        };
        result.and(end)
    }
}
#[derive(Default)]
pub struct DcsCounter {
    escape: bool,
    dcs: bool,
    dcs_escape: bool,
    bytes: u64,
    pub transfers: u64,
    pub image_bytes: u64,
    header: Vec<u8>,
    sixel: bool,
}
impl DcsCounter {
    pub fn feed(&mut self, data: &[u8]) {
        for &byte in data {
            if self.dcs {
                self.bytes += 1;
                if self.dcs_escape && byte == b'\\' {
                    if self.sixel {
                        self.transfers += 1;
                        self.image_bytes += self.bytes;
                        crate::perf::add(crate::perf::Count::ImageTransfers, 1);
                        crate::perf::add(crate::perf::Count::ImageBytes, self.bytes);
                    }
                    self.dcs = false;
                    self.dcs_escape = false;
                    self.escape = false;
                    self.header.clear();
                    continue;
                }
                if !self.sixel && self.header.len() < 64 {
                    if byte == b'q' {
                        self.sixel = true;
                    } else if byte.is_ascii_digit() || matches!(byte, b';' | b':') {
                        self.header.push(byte);
                    } else {
                        self.header.resize(64, 0);
                    }
                }
                self.dcs_escape = byte == 0x1b;
            } else if self.escape && byte == b'P' {
                self.dcs = true;
                self.bytes = 2;
                self.sixel = false;
                self.escape = false;
                self.header.clear();
            } else {
                self.escape = byte == 0x1b;
            }
        }
    }
}
pub struct Writer<W> {
    inner: W,
    counter: DcsCounter,
}
impl<W> Writer<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            counter: DcsCounter::default(),
        }
    }
    pub fn counter(&self) -> &DcsCounter {
        &self.counter
    }
    pub fn inner(&self) -> &W {
        &self.inner
    }
}
impl<W: Write> Write for Writer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _time = crate::perf::span(crate::perf::Metric::ImageOutput);
        let n = self.inner.write(buf)?;
        self.counter.feed(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        let _time = crate::perf::span(crate::perf::Metric::ImageFlush);
        self.inner.flush()
    }
}
pub type Terminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<Writer<io::Stdout>>>;
