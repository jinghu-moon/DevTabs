//! Image-only view state and frontend actions. Terminal data never enters a PTY.
use super::*;
use crate::preview::image::{self as pipeline, Display, Presentation, Profile, Request, View};
use ratatui::layout::Size;

pub(super) struct ImageDocument {
    pub view: View,
    pub profile: Profile,
    pub presentation: Option<Presentation>,
    last_geometry: Option<pipeline::Geometry>,
    pub info: Option<pipeline::MetadataInfo>,
    pub actual_display: Option<Display>,
    pub fallback: Option<String>,
    desired: Option<(Size, Display, View, Profile, bool)>,
    pub pending: bool,
    pub revision: u64,
    changed: Instant,
    reload: bool,
    drag: Option<bool>,
}
impl Default for ImageDocument {
    fn default() -> Self {
        Self {
            view: View::default(),
            profile: Profile::default(),
            presentation: None,
            last_geometry: None,
            info: None,
            actual_display: None,
            fallback: None,
            desired: None,
            pending: false,
            revision: 0,
            changed: Instant::now() - Duration::from_secs(1),
            reload: true,
            drag: None,
        }
    }
}
impl ImageDocument {
    fn invalidate(&mut self, reload: bool) {
        self.presentation = None;
        self.desired = None;
        self.pending = false;
        self.reload |= reload;
        self.changed = Instant::now();
    }
    pub fn release(&mut self) {
        self.invalidate(true);
    }
    fn scale(&self) -> f64 {
        self.last_geometry.as_ref().map_or(1.0, |g| g.scale)
    }
    fn pan(&mut self, dx: f64, dy: f64) {
        if self.view.scale.is_none() {
            self.view.scale = Some(self.scale());
        }
        let (w, h) = self
            .info
            .as_ref()
            .map_or((1.0, 1.0), |i| (f64::from(i.width), f64::from(i.height)));
        self.view.pan(
            dx / (w * self.view.scale.unwrap_or(1.0)),
            dy / (h * self.view.scale.unwrap_or(1.0)),
        );
        if let Some(g) = &self.last_geometry {
            let half_x =
                (f64::from(g.pixels.0) / (w * self.view.scale.unwrap_or(1.0)) / 2.0).min(0.5);
            let half_y =
                (f64::from(g.pixels.1) / (h * self.view.scale.unwrap_or(1.0)) / 2.0).min(0.5);
            self.view.center.0 = self.view.center.0.clamp(half_x, 1.0 - half_x);
            self.view.center.1 = self.view.center.1.clamp(half_y, 1.0 - half_y);
        }
        self.invalidate(false);
    }
    pub fn information(&self, path: &std::path::Path, display: Display) -> String {
        let display = self.actual_display.unwrap_or(display);
        let Some(info) = &self.info else {
            return crate::i18n::format(
                "image.not_loaded",
                &[("path", preview::safe_text(&path.to_string_lossy()))],
            );
        };
        crate::i18n::format(
            "image.information",
            &[
                ("path", preview::safe_text(&path.to_string_lossy())),
                ("bytes", info.version.bytes.to_string()),
                ("format", format!("{:?}", info.format)),
                ("width", info.width.to_string()),
                ("height", info.height.to_string()),
                (
                    "alpha",
                    crate::i18n::t(if info.alpha { "ui.yes" } else { "ui.no" }).into_owned(),
                ),
                ("modified", format!("{:?}", info.version.modified)),
                (
                    "zoom",
                    self.view.scale.map_or_else(
                        || crate::i18n::t("image.fit").into_owned(),
                        |scale| format!("{:.1}%", scale * 100.0),
                    ),
                ),
                ("profile", format!("{:?}", self.profile)),
                ("protocol", display.label().into()),
                (
                    "kind",
                    crate::i18n::t(if info.format == pipeline::Format::Svg {
                        "image.svg_notes"
                    } else {
                        "image.raster"
                    })
                    .into_owned(),
                ),
            ],
        )
    }
}
impl Document {
    pub fn loaded(&self) -> Option<&std::sync::Arc<Loaded>> {
        if let Content::Text(t) = &self.content {
            t.loaded.as_ref()
        } else {
            None
        }
    }
    pub fn rendered(&self) -> Option<&Rendered> {
        if let Content::Text(t) = &self.content {
            t.rendered.as_ref()
        } else {
            None
        }
    }
    pub fn rendered_mut(&mut self) -> Option<&mut Rendered> {
        if let Content::Text(t) = &mut self.content {
            t.rendered.as_mut()
        } else {
            None
        }
    }
    pub fn editor(&self) -> Option<&crate::editor::Editor> {
        if let Content::Text(t) = &self.content {
            t.editor.as_ref()
        } else {
            None
        }
    }
    pub fn editor_mut(&mut self) -> Option<&mut crate::editor::Editor> {
        if let Content::Text(t) = &mut self.content {
            t.editor.as_mut()
        } else {
            None
        }
    }
    pub(super) fn set_loaded(&mut self, value: Option<std::sync::Arc<Loaded>>) {
        if let Content::Text(t) = &mut self.content {
            t.loaded = value;
        }
    }
    pub(super) fn set_rendered(&mut self, value: Option<Rendered>) {
        if let Content::Text(t) = &mut self.content {
            t.rendered = value;
        }
    }
    pub(super) fn set_editor(&mut self, value: Option<crate::editor::Editor>) {
        if let Content::Text(t) = &mut self.content {
            t.editor = value;
        }
    }
    pub fn is_image(&self) -> bool {
        matches!(self.content, Content::Image(_))
    }
    pub(super) fn image(&self) -> Option<&ImageDocument> {
        if let Content::Image(i) = &self.content {
            Some(i)
        } else {
            None
        }
    }
    pub(super) fn image_mut(&mut self) -> Option<&mut ImageDocument> {
        if let Content::Image(i) = &mut self.content {
            Some(i)
        } else {
            None
        }
    }
}
impl Sidebar {
    pub fn configure_images(&mut self, display: Display) {
        if self.image_display != display {
            self.image_display = display;
            self.invalidate_image(false);
        }
    }
    pub fn image_output_bytes(&self) -> Option<usize> {
        Some(
            self.document
                .as_ref()?
                .image()?
                .presentation
                .as_ref()?
                .bytes,
        )
    }
    pub fn image_display(&self) -> Display {
        self.image_display
    }
    fn image_viewport(&self) -> Option<Rect> {
        let mut inner = self.document_area()?;
        inner.width = inner.width.saturating_sub(1);
        inner.height = inner.height.saturating_sub(2);
        Some(inner)
    }
    pub(super) fn invalidate_image(&mut self, reload: bool) {
        self.image_worker.cancel();
        self.image_composition.clear();
        if let Some(i) = self.document.as_mut().and_then(Document::image_mut) {
            i.invalidate(reload);
        }
    }
    pub(super) fn image_tick(&mut self, active: bool) -> bool {
        if !active || !self.showing() {
            if self.image_suspended {
                return false;
            }
            self.image_suspended = true;
            self.image_composition.clear();
            self.image_worker.release();
            if let Some(d) = &mut self.document {
                if let Some(i) = d.image_mut() {
                    i.release();
                }
                d.reader.images.release();
            }
            crate::perf::gauge(crate::perf::Gauge::CacheBytes, 0);
            return true;
        }
        self.image_suspended = false;
        if !self.document.as_ref().is_some_and(Document::is_image) {
            return self.inline_tick(active);
        }
        let mut changed = false;
        crate::perf::gauge(
            crate::perf::Gauge::CacheBytes,
            self.image_output_bytes().unwrap_or(0) as u64,
        );
        if let Some(completed) = self.image_worker.take() {
            if let Some(d) = &mut self.document
                && d.path == completed.path
                && let Some(i) = d.image_mut()
                && i.revision == completed.generation
            {
                i.pending = false;
                match completed.result {
                    Ok(pipeline::Prepared::Dimensions(_)) => unreachable!("image requested pixels"),
                    Ok(pipeline::Prepared::Pixels(p)) => {
                        i.view.center = (
                            (p.geometry.origin.0
                                + f64::from(p.geometry.pixels.0) / p.geometry.scale / 2.0)
                                / f64::from(p.info.width),
                            (p.geometry.origin.1
                                + f64::from(p.geometry.pixels.1) / p.geometry.scale / 2.0)
                                / f64::from(p.info.height),
                        );
                        i.last_geometry = Some(p.geometry.clone());
                        i.info = Some(p.info.clone());
                        i.actual_display = Some(p.display);
                        i.fallback = p.fallback.clone();
                        i.presentation = Some(*p);
                        if let Some(desired) = &mut i.desired {
                            desired.2 = i.view;
                        }
                        d.stale.clear();
                        d.loading = false;
                    }
                    Err(e) => {
                        d.stale = format!("Image unavailable: {e:#}\nF5 Retry · O Open externally");
                        d.loading = false;
                    }
                }
                changed = true;
            } else {
                crate::perf::add(crate::perf::Count::ImageStale, 1);
            }
        }
        if !active || !self.showing() {
            return changed;
        }
        let Some(area) = self
            .image_viewport()
            .filter(|r| r.width > 0 && r.height > 0)
        else {
            return changed;
        };
        let Some(d) = &mut self.document else {
            return changed;
        };
        let path = d.path.clone();
        let Some(i) = d.image_mut() else {
            return changed;
        };
        let desired = (
            area.as_size(),
            self.image_display,
            i.view,
            i.profile,
            self.files.light,
        );
        if i.desired != Some(desired) && i.changed.elapsed() >= Duration::from_millis(100) {
            self.serial += 1;
            i.revision = self.serial;
            i.desired = Some(desired);
            i.pending = true;
            i.presentation = None;
            self.image_worker.submit(Request {
                kind: pipeline::RequestKind::Pixels,
                local_target: None,
                generation: self.serial,
                path,
                area: area.as_size(),
                display: self.image_display,
                view: i.view,
                profile: i.profile,
                background: if self.files.light {
                    [250, 250, 250]
                } else {
                    [16, 18, 23]
                },
                reload: i.reload,
            });
            i.reload = false;
            d.loading = true;
            changed = true;
        }
        changed
    }
    /// Used by the actual output owner before drawing. A changed pixel plan clears once.
    pub fn image_plan(&self) -> Option<(Rect, u64)> {
        if self.images_occluded || !self.showing() || self.modal() || self.controls.hover.is_some()
        {
            return None;
        }
        let i = self.document.as_ref()?.image()?;
        let p = i.presentation.as_ref()?;
        let area = self.image_viewport()?;
        if p.protocol.size().width > area.width || p.protocol.size().height > area.height {
            return None;
        }
        Some((
            Rect::new(
                area.x + (area.width - p.protocol.size().width) / 2,
                area.y + (area.height - p.protocol.size().height) / 2,
                p.protocol.size().width,
                p.protocol.size().height,
            ),
            i.revision,
        ))
    }
    pub(super) fn draw_image(&self, frame: &mut Frame, rect: Rect) {
        frame.render_widget(
            Block::default().style(Style::new().bg(if self.files.light {
                Color::Rgb(250, 250, 250)
            } else {
                Color::Rgb(16, 18, 23)
            })),
            crate::ui::preview_content(rect),
        );
        let d = self.document.as_ref().expect("image document");
        let i = d.image().expect("image content");
        frame.render_widget(
            Block::bordered()
                .title(features::shortened(
                    &format!(
                        " {} · {} ",
                        d.path.file_name().unwrap_or_default().to_string_lossy(),
                        crate::i18n::t("image.read_only")
                    ),
                    rect.width.saturating_sub(12) as usize,
                ))
                .border_style(border(self.focus == Focus::Preview)),
            rect,
        );
        frame.render_widget(
            Paragraph::new(crate::i18n::t("more_image")),
            Rect::new(rect.right().saturating_sub(10), rect.y, 8, 1),
        );
        let Some(area) = self.image_viewport() else {
            return;
        };
        if let Some((region, _)) = self.image_plan()
            && let Some(p) = &i.presentation
        {
            if let ratatui_image::protocol::Protocol::Sixel(sixel) = &p.protocol
                && self.image_occlusion.is_some_and(|r| r.intersects(region))
            {
                self.image_composition.draw(
                    frame,
                    sixel,
                    (region, i.revision, 0),
                    self.image_occlusion,
                    Size::new(
                        self.image_display.cell.width,
                        self.image_display.cell.height,
                    ),
                );
            } else {
                frame.render_widget(ratatui_image::Image::new(&p.protocol), region);
            }
        } else if !self.modal() && !self.images_occluded {
            frame.render_widget(
                Paragraph::new(if i.pending || d.loading {
                    crate::i18n::t("preparing_image")
                } else {
                    std::borrow::Cow::Borrowed(d.stale.as_str())
                }),
                area,
            );
        }
        if let Some(inner) = self.document_area() {
            let status = i.info.as_ref().map_or_else(
                || self.image_display.label().to_string(),
                |info| {
                    format!(
                        "{}×{} · {:?} · {} · {:?} · {}",
                        info.width,
                        info.height,
                        info.format,
                        i.view.scale.map_or_else(
                            || format!("{} {:.0}%", crate::i18n::t("image.fit"), i.scale() * 100.0),
                            |scale| format!("{:.0}%", scale * 100.0)
                        ),
                        i.profile,
                        i.actual_display.unwrap_or(self.image_display).label()
                    )
                },
            );
            frame.render_widget(
                Paragraph::new(features::shortened(&status, inner.width as usize))
                    .style(Style::new().fg(Color::DarkGray)),
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            );
            self.draw_image_bars(frame, area, i);
        }
    }
    pub(super) fn image_command(&mut self, command: Command) -> bool {
        let Some(i) = self.document.as_mut().and_then(Document::image_mut) else {
            return false;
        };
        match command {
            Command::ImageFit => i.view = View::default(),
            Command::ImageActual => i.view.scale = Some(1.0),
            Command::ImageZoomIn => i.view.zoom(1.25, i.scale()),
            Command::ImageZoomOut => i.view.zoom(0.8, i.scale()),
            Command::ImageProfile => i.profile = i.profile.next(),
            Command::ImageProtocol => {
                self.image_display.protocol =
                    if self.image_display.protocol == pipeline::ProtocolChoice::Sixel {
                        pipeline::ProtocolChoice::Halfblock
                    } else if !self.image_display.estimated {
                        pipeline::ProtocolChoice::Sixel
                    } else {
                        self.say(crate::i18n::t("image.sixel_cell_size"), false);
                        return true;
                    };
            }
            Command::ImageRefresh => {
                self.invalidate_image(true);
                return true;
            }
            _ => return false,
        }
        self.invalidate_image(false);
        true
    }
    pub(super) fn image_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.focus != Focus::Preview
            || self.modal()
            || !self.document.as_ref().is_some_and(Document::is_image)
        {
            return Ok(false);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            return Ok(!(key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    key.code,
                    KeyCode::PageUp | KeyCode::PageDown | KeyCode::Left | KeyCode::Right
                )));
        }
        let command = match key.code {
            KeyCode::Char('0') => Some(Command::ImageFit),
            KeyCode::Char('1') => Some(Command::ImageActual),
            KeyCode::Char('+' | '=') => Some(Command::ImageZoomIn),
            KeyCode::Char('-') => Some(Command::ImageZoomOut),
            KeyCode::Char('p' | 'P') => Some(Command::ImageProfile),
            _ => None,
        };
        if let Some(c) = command {
            self.image_command(c);
            return Ok(true);
        }
        let cell = self.image_display.cell;
        let page = self.image_viewport().map_or(10, |r| r.height);
        let i = self
            .document
            .as_mut()
            .and_then(Document::image_mut)
            .unwrap();
        match key.code {
            KeyCode::Left => i.pan(-f64::from(cell.width) * 3.0, 0.0),
            KeyCode::Right => i.pan(f64::from(cell.width) * 3.0, 0.0),
            KeyCode::Up => i.pan(0.0, -f64::from(cell.height) * 3.0),
            KeyCode::Down => i.pan(0.0, f64::from(cell.height) * 3.0),
            KeyCode::PageUp => i.pan(0.0, -f64::from(cell.height) * f64::from(page)),
            KeyCode::PageDown => i.pan(0.0, f64::from(cell.height) * f64::from(page)),
            KeyCode::Esc => self.close(),
            KeyCode::Char('o' | 'O') => {
                let path = self.document.as_ref().unwrap().path.clone();
                self.open_external(path)?;
            }
            KeyCode::Char('z' | 'Z') => self.preview_command(Command::Maximize)?,
            KeyCode::F(5) => self.invalidate_image(true),
            KeyCode::F(2) => self.menu(),
            _ => {}
        }
        Ok(true)
    }
    pub(super) fn image_mouse(&mut self, event: MouseEvent) -> Result<bool> {
        if self.modal()
            || !self.showing()
            || !self.document.as_ref().is_some_and(Document::is_image)
        {
            return Ok(false);
        }
        let Some(rect) = self.geometry.preview else {
            return Ok(false);
        };
        let Some(area) = self.image_viewport() else {
            return Ok(false);
        };
        let point = (event.column, event.row).into();
        let dragging = self
            .document
            .as_ref()
            .and_then(Document::image)
            .is_some_and(|i| i.drag.is_some());
        if !rect.contains(point) && !dragging {
            return Ok(false);
        }
        if event.row == rect.y
            && event.column >= rect.right().saturating_sub(10)
            && event.kind == MouseEventKind::Down(MouseButton::Left)
        {
            self.focus = Focus::Preview;
            self.menu();
            return Ok(true);
        }
        if !area.contains(point)
            && !dragging
            && event.column != area.right()
            && event.row != area.bottom()
        {
            return Ok(false);
        }
        let image_anchor = self.image_plan().map_or(area, |p| p.0);
        let i = self
            .document
            .as_mut()
            .and_then(Document::image_mut)
            .unwrap();
        self.focus = Focus::Preview;
        let amount = if event.kind == MouseEventKind::ScrollUp {
            -3.0
        } else {
            3.0
        };
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let old = i.view.scale.unwrap_or(i.scale());
                let new = (old * if amount < 0.0 { 1.25 } else { 0.8 }).clamp(0.125, 8.0);
                if let Some(p) = &i.presentation
                    && let Some(info) = &i.info
                {
                    let dx = f64::from(event.column.saturating_sub(image_anchor.x))
                        * f64::from(self.image_display.cell.width)
                        - f64::from(p.geometry.pixels.0) / 2.0;
                    let dy = f64::from(event.row.saturating_sub(image_anchor.y))
                        * f64::from(self.image_display.cell.height)
                        - f64::from(p.geometry.pixels.1) / 2.0;
                    i.view.pan(
                        dx * (1.0 / old - 1.0 / new) / f64::from(info.width),
                        dy * (1.0 / old - 1.0 / new) / f64::from(info.height),
                    );
                }
                i.view.scale = Some(new);
                i.invalidate(false);
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if event.modifiers.contains(KeyModifiers::SHIFT) {
                    i.pan(amount * f64::from(self.image_display.cell.width), 0.0)
                } else {
                    i.pan(0.0, amount * f64::from(self.image_display.cell.height))
                }
            }
            MouseEventKind::Down(MouseButton::Left)
                if event.column == area.right() || event.row == area.bottom() =>
            {
                i.drag = Some(event.row == area.bottom());
                self.image_drag(event, area);
            }
            MouseEventKind::Drag(MouseButton::Left) if dragging => self.image_drag(event, area),
            MouseEventKind::Up(MouseButton::Left) => i.drag = None,
            MouseEventKind::Down(MouseButton::Right) => self.menu(),
            _ => {}
        }
        Ok(true)
    }
    fn image_drag(&mut self, event: MouseEvent, area: Rect) {
        let Some(i) = self.document.as_mut().and_then(Document::image_mut) else {
            return;
        };
        if i.view.scale.is_none() {
            i.view.scale = Some(i.scale());
        }
        let horizontal = i.drag == Some(true);
        let value = if horizontal {
            f64::from(event.column.saturating_sub(area.x))
                / f64::from(area.width.saturating_sub(1).max(1))
        } else {
            f64::from(event.row.saturating_sub(area.y))
                / f64::from(area.height.saturating_sub(1).max(1))
        };
        if horizontal {
            i.view.center.0 = value.clamp(0.0, 1.0)
        } else {
            i.view.center.1 = value.clamp(0.0, 1.0)
        }
        i.invalidate(false);
    }
    fn draw_image_bars(&self, frame: &mut Frame, area: Rect, i: &ImageDocument) {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
        let Some(p) = &i.presentation else { return };
        let w = (f64::from(p.info.width) * p.geometry.scale).ceil() as usize;
        let h = (f64::from(p.info.height) * p.geometry.scale).ceil() as usize;
        for (total, page, pos, horizontal) in [
            (
                w,
                p.geometry.pixels.0 as usize,
                (p.geometry.origin.0 * p.geometry.scale) as usize,
                true,
            ),
            (
                h,
                p.geometry.pixels.1 as usize,
                (p.geometry.origin.1 * p.geometry.scale) as usize,
                false,
            ),
        ] {
            if total <= page {
                continue;
            }
            let bar = Scrollbar::new(if horizontal {
                ScrollbarOrientation::HorizontalBottom
            } else {
                ScrollbarOrientation::VerticalRight
            })
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_style(Style::new().fg(Color::Cyan));
            frame.render_stateful_widget(
                bar,
                Rect::new(area.x, area.y, area.width + 1, area.height + 1),
                &mut ScrollbarState::new(total.saturating_sub(page) + 1)
                    .position(pos)
                    .viewport_content_length(page),
            );
        }
    }
}
