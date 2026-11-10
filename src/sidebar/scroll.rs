//! Ratatui draws scrollbars; this adapter supplies document-aware mouse dragging.
use super::*;
use ratatui::{
    buffer::Buffer,
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Tree,
    Preview,
    Horizontal,
    Editor,
    EditorHorizontal,
}
#[derive(Clone, Copy)]
pub(super) struct Drag {
    target: Target,
    grab: usize,
    source_units: Option<(usize, usize)>,
}
struct Track {
    target: Target,
    rect: Rect,
    total: usize,
    page: usize,
    position: usize,
    bytes: bool,
}
impl Track {
    fn horizontal(&self) -> bool {
        matches!(self.target, Target::Horizontal | Target::EditorHorizontal)
    }
    fn widget(&self) -> Scrollbar<'static> {
        Scrollbar::new(if self.horizontal() {
            ScrollbarOrientation::HorizontalBottom
        } else {
            ScrollbarOrientation::VerticalRight
        })
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_symbol("█")
        .track_symbol(Some(if self.horizontal() { "─" } else { "│" }))
        .thumb_style(Style::new().fg(Color::Cyan))
        .track_style(Style::new().fg(Color::DarkGray))
    }
    fn max(&self) -> usize {
        self.total.saturating_sub(1)
    }
    fn state(&self) -> ScrollbarState {
        ScrollbarState::new(self.total)
            .position(self.position.min(self.max()))
            .viewport_content_length(self.page)
    }
    fn length(&self) -> usize {
        (if self.horizontal() {
            self.rect.width
        } else {
            self.rect.height
        }) as usize
    }
    fn coordinate(&self, event: MouseEvent) -> usize {
        if self.horizontal() {
            event.column.saturating_sub(self.rect.x) as usize
        } else {
            event.row.saturating_sub(self.rect.y) as usize
        }
    }
    fn thumb(&self) -> (usize, usize) {
        let mut buf = Buffer::empty(self.rect);
        self.widget().render(self.rect, &mut buf, &mut self.state());
        let positions: Vec<_> = buf
            .content
            .iter()
            .enumerate()
            .filter(|(_, c)| c.symbol() == "█")
            .map(|(i, _)| i)
            .collect();
        (*positions.first().unwrap_or(&0), positions.len().max(1))
    }
}
impl Sidebar {
    fn tracks(&self) -> Vec<Track> {
        let mut tracks = Vec::new();
        if let Some(rect) = self.geometry.tree {
            let inner = crate::ui::content(rect);
            let count = self
                .tree_view
                .as_ref()
                .and_then(|v| v.viewport.as_ref())
                .map_or(0, |v| v.row_count);
            if count > inner.height as usize && inner.height > 0 {
                tracks.push(Track {
                    target: Target::Tree,
                    rect: Rect::new(rect.right() - 1, inner.y, 1, inner.height),
                    total: count,
                    page: inner.height as usize,
                    position: self.files.state.get_offset(),
                    bytes: false,
                });
            }
        }
        if let Some(rect) = self.geometry.preview
            && let Some(d) = &self.document
        {
            let inner = crate::ui::content(rect);
            let (total, page, position, bytes, target) = if d.editing {
                let Some(e) = &d.editor else {
                    return tracks;
                };
                (
                    e.area.lines().len(),
                    inner.height as usize,
                    e.area.scroll_offset().0 as usize,
                    false,
                    Target::Editor,
                )
            } else if let Some(r) = &d.rendered {
                if d.markdown && !r.complete {
                    let page = d
                        .screen_rows(inner.height as usize)
                        .iter()
                        .filter_map(|&row| r.rows.get(row))
                        .flat_map(|row| &row.segments)
                        .map(|s| s.source.end)
                        .max()
                        .unwrap_or(1)
                        .saturating_sub(d.reading_byte())
                        .max(1);
                    (
                        r.source.len(),
                        page,
                        d.reader.seek.unwrap_or_else(|| d.reading_byte()),
                        true,
                        Target::Preview,
                    )
                } else {
                    (
                        r.plain.len(),
                        inner.height as usize,
                        d.offset().0,
                        false,
                        Target::Preview,
                    )
                }
            } else {
                return tracks;
            };
            if total > page && inner.height > 0 {
                tracks.push(Track {
                    target,
                    rect: Rect::new(rect.right() - 1, inner.y, 1, inner.height),
                    total,
                    page,
                    position,
                    bytes,
                });
            }
            if !d.editing
                && let Some(r) = &d.rendered
            {
                let total = d
                    .screen_rows(inner.height as usize)
                    .into_iter()
                    .filter_map(|row| r.plain.get(row))
                    .map(|s| s.width())
                    .max()
                    .unwrap_or(0);
                let page = inner.width.saturating_sub(if d.markdown { 0 } else { 6 }) as usize;
                if total > page && inner.width > 0 {
                    tracks.push(Track {
                        target: Target::Horizontal,
                        rect: Rect::new(inner.x, rect.bottom() - 1, inner.width, 1),
                        total: total.saturating_sub(page) + 1,
                        page,
                        position: d.offset().1,
                        bytes: false,
                    });
                }
            }
            if d.editing
                && let Some(e) = &d.editor
            {
                let page = inner.width.saturating_sub(e.area.line_number_width()) as usize;
                let total = e
                    .area
                    .lines()
                    .iter()
                    .skip(e.area.scroll_offset().0 as usize)
                    .take(inner.height as usize)
                    .map(|s| preview::safe_text(s).width())
                    .max()
                    .unwrap_or(0);
                if total > page && inner.width > 0 {
                    tracks.push(Track {
                        target: Target::EditorHorizontal,
                        rect: Rect::new(inner.x, rect.bottom() - 1, inner.width, 1),
                        total: total.saturating_sub(page) + 1,
                        page,
                        position: e.area.scroll_offset().1 as usize,
                        bytes: false,
                    });
                }
            }
        }
        if let Some(drag) = self.reader.drag
            && let Some((total, page)) = drag.source_units
        {
            for track in &mut tracks {
                if track.target == drag.target {
                    track.total = total;
                    track.page = page;
                    track.bytes = true;
                    track.position = self
                        .document
                        .as_ref()
                        .map_or(0, |d| d.reader.seek.unwrap_or_else(|| d.reading_byte()));
                }
            }
        }
        tracks
    }
    pub(super) fn draw_scrollbars(&self, frame: &mut Frame) {
        for track in self.tracks() {
            frame.render_stateful_widget(track.widget(), track.rect, &mut track.state());
        }
    }
    pub(super) fn scrollbar_mouse(&mut self, event: MouseEvent) -> bool {
        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left))
            && self.reader.drag.take().is_some()
        {
            return true;
        }
        let down = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
        let dragging = matches!(event.kind, MouseEventKind::Drag(MouseButton::Left));
        if !(down || dragging) {
            return false;
        }
        let tracks = self.tracks();
        let target = self.reader.drag.map(|d| d.target);
        let Some(track) = tracks.into_iter().find(|t| {
            if dragging {
                Some(t.target) == target
            } else {
                t.rect.contains((event.column, event.row).into())
            }
        }) else {
            return false;
        };
        let at = track
            .coordinate(event)
            .min(track.length().saturating_sub(1));
        let (start, len) = track.thumb();
        let on_thumb = (start..start + len).contains(&at);
        if down {
            self.reader.drag = Some(Drag {
                target: track.target,
                grab: if on_thumb { at - start } else { len / 2 },
                source_units: track.bytes.then_some((track.total, track.page)),
            });
            if let Some(d) = &mut self.document {
                d.reader.pressed = None;
            }
        }
        if dragging || !on_thumb {
            let grab = self.reader.drag.map_or(0, |d| d.grab);
            let span = track.length().saturating_sub(len);
            let start = at.saturating_sub(grab).min(span);
            let value = if start == span {
                track.max()
            } else {
                start.saturating_mul(track.max().saturating_add(track.page)) / track.length().max(1)
            }
            .min(track.max());
            match track.target {
                Target::Tree => {
                    self.files.state.scroll_up(usize::MAX);
                    self.files.state.scroll_down(value);
                    if let Some(v) = &mut self.tree_view {
                        v.viewport = None;
                    }
                    self.focus = Focus::Tree;
                }
                Target::Editor => {
                    if let Some(e) = self.document.as_mut().and_then(|d| d.editor.as_mut()) {
                        let old = e.area.scroll_offset().0 as usize;
                        let delta = (value as i64 - old as i64)
                            .clamp(i16::MIN as i64, i16::MAX as i64)
                            as i16;
                        e.area.scroll((delta, 0));
                        self.focus = Focus::Editor;
                    }
                }
                Target::EditorHorizontal => {
                    if let Some(e) = self.document.as_mut().and_then(|d| d.editor.as_mut()) {
                        let old = e.area.scroll_offset().1 as usize;
                        let delta = (value as i64 - old as i64)
                            .clamp(i16::MIN as i64, i16::MAX as i64)
                            as i16;
                        e.area.scroll((0, delta));
                        self.focus = Focus::Editor;
                    }
                }
                Target::Horizontal => {
                    if let Some(d) = &mut self.document {
                        d.offset_mut().1 = value;
                        self.focus = Focus::Preview;
                    }
                }
                Target::Preview => {
                    if track.bytes {
                        self.seek_reading(value);
                    } else if let Some(d) = &mut self.document {
                        d.offset_mut().0 = value;
                    }
                    self.focus = Focus::Preview;
                }
            }
        }
        true
    }
}
pub(super) fn tree_rows(
    items: &[TreeItem<'_, Key>],
    opened: &std::collections::HashSet<Vec<Key>>,
    path: &mut Vec<Key>,
) -> usize {
    let mut count = 0;
    for item in items {
        count += item.height();
        path.push(item.identifier().clone());
        if opened.contains(path) {
            count += tree_rows(item.children(), opened, path);
        }
        path.pop();
    }
    count
}
