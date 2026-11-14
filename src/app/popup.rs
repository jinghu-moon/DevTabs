//! One terminal panel rendered inside the current TUI. Geometry is in cells.
use super::*;
use crate::{
    presentation::Placement,
    protocol::{Request, Response},
};
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edit {
    Move,
    Resize,
}

#[derive(Default)]
pub(crate) struct Window {
    bounds: Option<Rect>,
    drag: Option<(Edit, u16, u16, Rect)>,
    pub(super) edit: Option<Edit>,
    original: Option<Rect>,
}

impl Window {
    pub(crate) fn editing(&self) -> bool {
        self.edit.is_some()
    }
    pub fn load() -> Self {
        let bounds = crate::settings::data::UiState::read(&crate::config::home())
            .map_err(|e| eprintln!("UI memory: {e:#}"))
            .ok()
            .and_then(|s| s.floating)
            .map(|[x, y, w, h]| Rect::new(x, y, w, h));
        Self {
            bounds,
            ..Self::default()
        }
    }
    pub fn rect(&self, size: Rect) -> Rect {
        let area = Rect {
            height: size.height.saturating_sub(1),
            ..size
        };
        let w = (area.width.saturating_mul(2) / 3)
            .clamp(40, 100)
            .min(area.width);
        let h = (area.height.saturating_mul(2) / 3)
            .clamp(8, 28)
            .min(area.height);
        let r = self.bounds.unwrap_or(Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        ));
        Self::fit(r, area)
    }
    fn fit(r: Rect, area: Rect) -> Rect {
        let w = r.width.max(40).min(area.width);
        let h = r.height.max(8).min(area.height);
        Rect::new(
            r.x.clamp(area.x, area.right().saturating_sub(w)),
            r.y.clamp(area.y, area.bottom().saturating_sub(h)),
            w,
            h,
        )
    }
    fn change(&mut self, edit: Edit, r: Rect, dx: i32, dy: i32, size: Rect) {
        let n = |v: u16, d: i32| (i32::from(v) + d).clamp(0, i32::from(u16::MAX)) as u16;
        let next = match edit {
            Edit::Move => Rect::new(n(r.x, dx), n(r.y, dy), r.width, r.height),
            Edit::Resize => Rect::new(
                r.x,
                r.y,
                n(r.width, dx).min(size.width.saturating_sub(r.x)),
                n(r.height, dy).min(size.height.saturating_sub(1).saturating_sub(r.y)),
            ),
        };
        self.bounds = Some(Self::fit(
            next,
            Rect {
                height: size.height.saturating_sub(1),
                ..size
            },
        ));
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.edit = None;
        self.original = None;
    }
    fn save(&self) -> Result<()> {
        if let Some(r) = self.bounds {
            crate::settings::data::UiState::update(
                &crate::config::home(),
                crate::settings::data::UiChange::Floating([r.x, r.y, r.width, r.height]),
            )?;
        }
        Ok(())
    }
}

impl App {
    pub(super) fn show_popup(&mut self, i: usize) -> Result<()> {
        self.zoom = None;
        if self.popup == Some(i) {
            self.focus = i;
            if let Some(s) = &mut self.sidebar {
                s.focus_agent();
            }
            return Ok(());
        }
        let _perf = crate::perf::span(crate::perf::Metric::FloatingOpen);
        anyhow::ensure!(
            self.size.width >= 60 && self.size.height >= 10,
            "{}",
            crate::i18n::t("terminal.minimum")
        );
        let Response::FloatPlan(plan) = crate::protocol::rpc(Request::PrepareFloat {
            target: self.target(i),
        })?
        else {
            anyhow::bail!("No panel handoff plan");
        };
        let r = ui::content(self.popup_window.rect(self.size));
        // Create a new attachment first: a failure leaves the source usable.
        let mut next = Client::hidden(self.sessions[i].info.clone());
        if let Err(error) = next.attach_view(
            &self.workspace_id,
            r.height,
            r.width,
            false,
            Some(plan.ticket.revision),
        ) {
            let _ = crate::protocol::rpc(Request::CancelFloat {
                revision: plan.ticket.revision,
            });
            crate::perf::add(crate::perf::Count::FloatingFailures, 1);
            return Err(error);
        }
        if let Some(old) = self.popup {
            self.sessions[old].detach();
            self.sessions[old].info.placement = Placement::Collapsed;
        }
        next.info.placement = Placement::Floating;
        self.sessions[i] = next;
        self.popup = Some(i);
        self.focus = i;
        self.selection = None;
        self.popup_window.cancel_drag();
        self.floating_first_frame = Some(plan.ticket.requested_at_ms);
        if let Some(s) = &mut self.sidebar {
            s.focus_agent();
            s.cancel_drag(true);
        }
        self.attach_visible()?;
        Ok(())
    }
    pub(super) fn popup_key(&mut self, key: KeyEvent) {
        let Some(edit) = self.popup_window.edit else {
            return;
        };
        let r = self.popup_window.rect(self.size);
        self.popup_window.original.get_or_insert(r);
        let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
            5
        } else {
            1
        };
        let delta = match key.code {
            KeyCode::Left => Some((-step, 0)),
            KeyCode::Right => Some((step, 0)),
            KeyCode::Up => Some((0, -step)),
            KeyCode::Down => Some((0, step)),
            KeyCode::Esc => {
                self.popup_window.bounds = self.popup_window.original.take();
                self.popup_window.cancel_drag();
                None
            }
            KeyCode::Enter => {
                if let Err(e) = self.popup_window.save() {
                    self.message = e.to_string();
                }
                self.popup_window.cancel_drag();
                None
            }
            _ => None,
        };
        if let Some((dx, dy)) = delta {
            self.popup_window.change(edit, r, dx, dy, self.size);
        }
        if let Err(e) = self.attach_visible() {
            self.message = e.to_string();
        }
    }
    pub(super) fn popup_drag(&mut self, event: MouseEvent) -> bool {
        if self.popup.is_none() || self.zoom.is_some() {
            return false;
        }
        if let Some((edit, x, y, r)) = self.popup_window.drag {
            if matches!(
                event.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            ) {
                self.popup_window.change(
                    edit,
                    r,
                    i32::from(event.column) - i32::from(x),
                    i32::from(event.row) - i32::from(y),
                    self.size,
                );
                if let Err(e) = self.attach_visible() {
                    self.message = e.to_string();
                }
                if matches!(event.kind, MouseEventKind::Up(_)) {
                    self.popup_window.cancel_drag();
                    if let Err(e) = self.popup_window.save() {
                        self.message = e.to_string();
                    }
                }
            }
            return true;
        }
        let r = self.popup_window.rect(self.size);
        if !r.contains((event.column, event.row).into()) {
            return false;
        }
        if self
            .terminal_hits
            .iter()
            .any(|(hit, _)| hit.contains((event.column, event.row).into()))
        {
            return false;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            let edit = if event.row >= r.bottom().saturating_sub(2)
                && event.column >= r.right().saturating_sub(2)
            {
                Some(Edit::Resize)
            } else if event.row == r.y {
                Some(Edit::Move)
            } else {
                None
            };
            if let Some(edit) = edit {
                self.focus = self.popup.unwrap();
                if let Some(s) = &mut self.sidebar {
                    s.focus_agent();
                }
                self.popup_window.drag = Some((edit, event.column, event.row, r));
                self.selection = None;
                return true;
            }
        }
        // The remaining border must never click through to files or a terminal.
        !ui::content(r).contains((event.column, event.row).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cell_geometry_is_bounded_and_moves_without_resizing() {
        let mut w = Window::default();
        for size in [
            Rect::new(0, 0, 140, 36),
            Rect::new(0, 0, 60, 10),
            Rect::new(0, 0, 20, 5),
        ] {
            let r = w.rect(size);
            assert!(r.right() <= size.right() && r.bottom() <= size.height.saturating_sub(1));
            w.change(Edit::Move, r, 9000, -9000, size);
            let moved = w.rect(size);
            assert_eq!((r.width, r.height), (moved.width, moved.height));
            w.change(Edit::Resize, moved, 9000, 9000, size);
            let grown = w.rect(size);
            assert!(
                grown.right() <= size.right() && grown.bottom() <= size.height.saturating_sub(1)
            );
        }
    }
}
