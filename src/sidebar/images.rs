//! Markdown image scheduling and the bounded, shared presentation cache.
use super::*;
use crate::preview::image::{
    self as pipeline, Display, Prepared, Profile, ProtocolChoice, Request, RequestKind, View,
};
use ratatui_image::{
    protocol::Protocol,
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
#[derive(Default)]
pub(super) struct InlineImages {
    source: Option<Arc<str>>,
    geometry: Option<(u16, Display, bool)>,
    heights: BTreeMap<usize, u16>,
    entries: BTreeMap<usize, Entry>,
    cache: VecDeque<Cached>,
    metadata: BTreeMap<PathBuf, (u32, u32)>,
    pending: Option<(u64, usize, RequestKind)>,
    last_top: Option<usize>,
    last_change: Option<Instant>,
    suppressed: bool,
}
struct Entry {
    path: Option<PathBuf>,
    target: String,
    error: Option<String>,
}
struct Cached {
    path: PathBuf,
    protocol: SlicedProtocol,
    bytes: usize,
    generation: u64,
    height: u16,
}
impl InlineImages {
    pub fn heights(&self) -> BTreeMap<usize, u16> {
        self.heights.clone()
    }
    pub fn release(&mut self) {
        self.cache.clear();
        self.pending = None;
        self.source = None;
        self.entries.clear();
        self.metadata.clear();
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.cache
            .iter()
            .map(|c| c.path.clone())
            .chain(self.metadata.keys().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn changed(&self, paths: &std::collections::HashSet<PathBuf>) -> bool {
        self.cache.iter().any(|c| paths.contains(&c.path))
            || self.metadata.keys().any(|p| paths.contains(p))
    }
    pub fn invalidate_pixels(&mut self) {
        self.cache.clear();
        self.pending = None;
        self.entries.clear();
        self.metadata.clear();
    }
    pub fn bytes(&self) -> usize {
        self.cache.iter().map(|c| c.bytes).sum()
    }
}
impl Sidebar {
    pub(super) fn inline_tick(&mut self, active: bool) -> bool {
        let Some(area) = self.document_area().filter(|a| a.width > 0 && a.height > 0) else {
            return false;
        };
        let Some(d) = &mut self.document else {
            return false;
        };
        let Some(r) = d.rendered().filter(|r| r.markdown && !d.editing) else {
            return false;
        };
        let completed = self.image_worker.take();
        let source = r.source.clone();
        let geometry = (area.width, self.image_display, self.files.light);
        let state = &mut d.reader.images;
        crate::perf::gauge(crate::perf::Gauge::CacheBytes, state.bytes() as u64);
        if state
            .source
            .as_ref()
            .is_none_or(|s| !Arc::ptr_eq(s, &source))
        {
            *state = InlineImages::default();
            state.source = Some(source);
            self.image_worker.cancel();
        }
        if state.geometry != Some(geometry) {
            state.cache.clear();
            state.pending = None;
            state.entries.clear();
            state.geometry = Some(geometry);
            self.image_worker.cancel();
        }
        let mut changed = false;
        let mut relayout = false;
        if let Some(done) = completed
            && let Some((generation, key, _)) = state.pending
            && generation == done.generation
        {
            state.pending = None;
            if let Some(entry) = state.entries.get_mut(&key) {
                match done.result {
                    Ok(Prepared::Dimensions(dimensions)) => {
                        state
                            .metadata
                            .insert(entry.path.as_ref().unwrap().clone(), dimensions);
                    }
                    Ok(Prepared::Pixels(p)) => {
                        let height = p.protocol.size().height.clamp(1, 48);
                        let path = entry.path.as_ref().unwrap().clone();
                        state
                            .metadata
                            .insert(path.clone(), (p.info.width, p.info.height));
                        let _slice = crate::perf::span(crate::perf::Metric::ImageSlice);
                        let protocol = match p.protocol {
                            Protocol::Sixel(s) => {
                                SlicedProtocol::from_sixel(s, self.image_display.cell.height)
                            }
                            Protocol::Halfblocks(h) => SlicedProtocol::Halfblocks(h),
                            _ => unreachable!("supported protocols"),
                        };
                        let bytes = protocol.retained_bytes();
                        while state.bytes().saturating_add(bytes) > 32 * 1024 * 1024
                            && !state.cache.is_empty()
                        {
                            state.cache.pop_front();
                        }
                        state.cache.push_back(Cached {
                            path,
                            protocol,
                            bytes,
                            generation,
                            height,
                        });
                        if state.heights.insert(key, height) != Some(height) {
                            relayout = true;
                        }
                    }
                    Err(e) => entry.error = Some(format!("{e:#}")),
                }
                changed = true;
            }
        }
        let top = d.offset().0;
        let state = &mut d.reader.images;
        if state.last_top != Some(top) {
            let now = Instant::now();
            let scrolled = state.last_top.is_some();
            state.last_top = Some(top);
            state.last_change = Some(now);
            // Even slower wheel gestures can retransmit a large image at every
            // step. Keep the layout/cache and resume only after scrolling stops.
            if scrolled {
                state.suppressed = true;
                self.image_worker.cancel();
                state.pending = None;
                changed = true;
            }
        }
        if state.suppressed
            && state
                .last_change
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(150))
        {
            state.suppressed = false;
            changed = true;
        }
        if relayout {
            d.invalidate_view();
            return true;
        }
        if !active || !self.showing() {
            return changed;
        }
        let d = self.document.as_ref().unwrap();
        let Some(r) = d.rendered() else {
            return changed;
        };
        let screen = d.screen_rows(area.height as usize);
        // Source-backed rows are already lazily laid out. Never scan all Markdown targets.
        let near =
            top.saturating_sub(area.height as usize)..top.saturating_add(area.height as usize * 2);
        let mut visible = Vec::new();
        let mut nearby = std::collections::BTreeSet::new();
        let mut neighbors = Vec::new();
        for row in near.start..near.end.min(r.rows.len()) {
            if let Some(image) = &r.rows[row].image
                && nearby.insert(image.source.start)
            {
                neighbors.push((
                    image.source.start,
                    image.destination.to_string(),
                    row.abs_diff(top),
                    false,
                ));
            }
        }
        for (y, row) in screen.iter().enumerate() {
            if let Some(image) = r.rows.get(*row).and_then(|r| r.image.as_ref())
                && !visible
                    .iter()
                    .any(|(key, _, _, _)| *key == image.source.start)
            {
                visible.push((image.source.start, image.destination.to_string(), y, true));
            }
        }
        visible.sort_by_key(|(_, _, y, _)| y.abs_diff(area.height as usize / 2));
        neighbors.sort_by_key(|(_, _, distance, _)| *distance);
        neighbors.retain(|(key, _, _, _)| !visible.iter().any(|(v, _, _, _)| key == v));
        visible.extend(neighbors);
        let base = d.path.clone();
        let state = &mut self.document.as_mut().unwrap().reader.images;
        if state
            .pending
            .is_some_and(|(_, key, _)| !nearby.contains(&key))
        {
            self.image_worker.cancel();
            state.pending = None;
        }
        // Do not submit while scrolling rapidly. Preserve heights and cached presentations.
        if state.suppressed || self.images_occluded || self.modal() {
            return changed;
        }
        let state = &mut self.document.as_mut().unwrap().reader.images;
        for (key, target, _, is_visible) in visible {
            if let std::collections::btree_map::Entry::Vacant(slot) = state.entries.entry(key) {
                let path = match preview::reading::destination(&base, &target) {
                    Ok(preview::reading::Destination::File(p, _)) => Some(p),
                    _ => None,
                };
                let error = path
                    .is_none()
                    .then(|| "Remote/unsupported image · click to open externally".to_string());
                slot.insert(Entry {
                    path,
                    target,
                    error,
                });
            }
            let entry = &state.entries[&key];
            if entry.error.is_some() {
                continue;
            }
            let path = entry.path.as_ref().unwrap();
            if let Some(&(w, h)) = state.metadata.get(path)
                && let Ok(g) = pipeline::geometry(
                    w,
                    h,
                    ratatui::layout::Size::new(area.width, 48),
                    self.image_display,
                    View::default(),
                    pipeline::Budget::default(),
                )
                && state.heights.insert(key, g.size.height.clamp(1, 48))
                    != Some(g.size.height.clamp(1, 48))
            {
                self.document.as_mut().unwrap().invalidate_view();
                return true;
            }
            if let Some(cached) = state.cache.iter().find(|c| &c.path == path) {
                if state.heights.insert(key, cached.height) != Some(cached.height) {
                    self.document.as_mut().unwrap().invalidate_view();
                    return true;
                }
                continue;
            }
            if !is_visible && state.metadata.contains_key(path) {
                continue;
            }
            if state.pending.is_some() {
                continue;
            }
            self.serial += 1;
            let kind = if is_visible {
                RequestKind::Pixels
            } else {
                RequestKind::Dimensions
            };
            state.pending = Some((self.serial, key, kind));
            self.image_worker.submit(Request {
                kind,
                generation: self.serial,
                path: path.clone(),
                local_target: Some((base.clone(), entry.target.clone())),
                area: ratatui::layout::Size::new(area.width, 48),
                display: self.image_display,
                view: View::default(),
                profile: Profile::Mixed,
                background: if self.files.light {
                    [250, 250, 250]
                } else {
                    [16, 18, 23]
                },
                reload: false,
            });
        }
        changed
    }
    pub fn image_plans(&self) -> Vec<(Rect, u64, u16)> {
        if self.image_display.protocol != ProtocolChoice::Sixel {
            return Vec::new();
        }
        if let Some((area, revision)) = self.image_plan() {
            return crate::image_output::composed_plans((area, revision, 0), self.image_occlusion);
        }
        self.inline_regions()
            .into_iter()
            .flat_map(|(area, cached, offset, key)| {
                crate::image_output::composed_plans(
                    (area, cached.generation.wrapping_add(key as u64), offset),
                    self.image_occlusion,
                )
            })
            .collect()
    }
    fn inline_regions(&self) -> Vec<(Rect, &Cached, u16, usize)> {
        if self.images_occluded || !self.showing() || self.modal() || self.controls.hover.is_some()
        {
            return Vec::new();
        }
        let Some(d) = self.document.as_ref().filter(|d| d.markdown && !d.editing) else {
            return Vec::new();
        };
        let Some(r) = d.rendered() else {
            return Vec::new();
        };
        let Some(area) = self.document_area() else {
            return Vec::new();
        };
        let state = &d.reader.images;
        if state.suppressed {
            return Vec::new();
        }
        let screen = d.screen_rows(area.height as usize);
        let mut out = Vec::new();
        let mut y = 0;
        while y < screen.len() {
            let Some(image) = r.rows.get(screen[y]).and_then(|row| row.image.as_ref()) else {
                y += 1;
                continue;
            };
            let start = y;
            let key = image.source.start;
            let offset = image.offset;
            y += 1;
            while y < screen.len()
                && r.rows
                    .get(screen[y])
                    .and_then(|row| row.image.as_ref())
                    .is_some_and(|i| {
                        i.source.start == key && i.offset == offset + (y - start) as u16
                    })
            {
                y += 1;
            }
            let Some(entry) = state.entries.get(&key) else {
                continue;
            };
            let Some(path) = &entry.path else { continue };
            if let Some(cached) = state.cache.iter().find(|c| &c.path == path)
                && cached.height == image.height
            {
                out.push((
                    Rect::new(
                        area.x,
                        area.y + start as u16,
                        area.width.min(cached.protocol.size().width),
                        (y - start) as u16,
                    ),
                    cached,
                    offset,
                    key,
                ));
            }
        }
        out
    }
    pub(super) fn draw_inline_images(&self, frame: &mut Frame) {
        for (area, cached, offset, key) in self.inline_regions() {
            if let Some(sixel) = cached.protocol.sixel()
                && self.image_occlusion.is_some_and(|r| r.intersects(area))
            {
                self.image_composition.draw(
                    frame,
                    sixel,
                    (area, cached.generation.wrapping_add(key as u64), offset),
                    self.image_occlusion,
                    ratatui::layout::Size::new(
                        self.image_display.cell.width,
                        self.image_display.cell.height,
                    ),
                );
                continue;
            }
            frame.render_widget(
                SlicedImage::new(
                    &cached.protocol,
                    SignedPosition::from((0, -(offset as i16))),
                ),
                area,
            );
        }
        let Some(d) = self.document.as_ref() else {
            return;
        };
        let Some(r) = d.rendered() else { return };
        let Some(area) = self.document_area() else {
            return;
        };
        let state = &d.reader.images;
        for (y, row) in d.screen_rows(area.height as usize).into_iter().enumerate() {
            let Some(image) = r.rows.get(row).and_then(|r| r.image.as_ref()) else {
                continue;
            };
            let entry = state.entries.get(&image.source.start);
            let loaded = entry
                .and_then(|e| e.path.as_ref())
                .is_some_and(|p| state.cache.iter().any(|c| &c.path == p));
            if (!loaded || state.suppressed) && (image.offset == 0 || y == 0) {
                let message =
                    entry
                        .and_then(|e| e.error.as_deref())
                        .unwrap_or(if state.suppressed {
                            "Image · scrolling…"
                        } else {
                            "Loading local image…"
                        });
                frame.render_widget(
                    Paragraph::new(features::shortened(message, area.width as usize))
                        .style(Style::new().fg(Color::DarkGray)),
                    Rect::new(area.x, area.y + y as u16, area.width, 1),
                );
            }
        }
    }
}
