//! Mouse-accessible command list for exclusive scopes, including unbound actions.
use super::*;
use crossterm::event::{KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct Actions {
    owner: Option<&'static str>,
    selected: usize,
}
pub enum ActionResult {
    Unhandled,
    Handled,
    Execute(&'static str),
}
impl Actions {
    pub fn open(&self) -> bool {
        self.owner.is_some()
    }
    pub fn close(&mut self) {
        self.owner = None;
    }
    fn commands<'a>(index: &'a Index, context: Option<&str>) -> Vec<&'a Command> {
        index
            .commands
            .iter()
            .filter(|c| Some(c.context) == context)
            .collect()
    }
    fn button(area: Rect) -> Rect {
        let width = (Self::title().width() as u16).min(area.width);
        Rect::new(
            area.right().saturating_sub(width),
            area.y,
            width,
            u16::from(area.height > 0),
        )
    }
    fn title() -> &'static str {
        if crate::i18n::language() == crate::i18n::Language::ZhCn {
            "[操作]"
        } else {
            "[Actions]"
        }
    }
    fn popup(area: Rect, count: usize) -> Rect {
        let width = area.width.min(88);
        let height = area
            .height
            .min(count.saturating_add(3).min(u16::MAX as usize) as u16);
        Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    }
    pub fn key(
        &mut self,
        index: &Index,
        context: Option<&'static str>,
        key: KeyEvent,
    ) -> ActionResult {
        let Some(owner) = self.owner else {
            return ActionResult::Unhandled;
        };
        if Some(owner) != context {
            self.close();
            return ActionResult::Handled;
        }
        if key.kind != KeyEventKind::Press {
            return ActionResult::Handled;
        }
        let commands = Self::commands(index, context);
        match key.code {
            KeyCode::Esc => self.close(),
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(commands.len().saturating_sub(1))
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = commands.len().saturating_sub(1),
            KeyCode::Enter => {
                let id = commands.get(self.selected).map(|c| c.id);
                self.close();
                if let Some(id) = id {
                    return ActionResult::Execute(id);
                }
            }
            _ => {}
        }
        ActionResult::Handled
    }
    pub fn mouse(
        &mut self,
        index: &Index,
        context: Option<&'static str>,
        area: Rect,
        event: MouseEvent,
    ) -> ActionResult {
        let commands = Self::commands(index, context);
        if self.open() {
            if self.owner != context {
                self.close();
                return ActionResult::Handled;
            }
            let popup = Self::popup(area, commands.len());
            let body = popup.inner(Margin::new(1, 1));
            let first = self
                .selected
                .saturating_sub(body.height.saturating_sub(1) as usize);
            match event.kind {
                MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
                MouseEventKind::ScrollDown => {
                    self.selected = (self.selected + 1).min(commands.len().saturating_sub(1))
                }
                MouseEventKind::Down(MouseButton::Left)
                    if body.contains((event.column, event.row).into()) =>
                {
                    let id = commands
                        .get(first + (event.row - body.y) as usize)
                        .map(|c| c.id);
                    self.close();
                    if let Some(id) = id {
                        return ActionResult::Execute(id);
                    }
                }
                MouseEventKind::Down(_) => self.close(),
                _ => {}
            }
            return ActionResult::Handled;
        }
        if !commands.is_empty()
            && event.kind == MouseEventKind::Down(MouseButton::Left)
            && Self::button(area).contains((event.column, event.row).into())
        {
            self.owner = context;
            self.selected = 0;
            return ActionResult::Handled;
        }
        ActionResult::Unhandled
    }
    pub fn draw(
        &self,
        frame: &mut Frame,
        index: &Index,
        context: Option<&'static str>,
        area: Rect,
    ) {
        let commands = Self::commands(index, context);
        if commands.is_empty() || area.width < 4 || area.height < 3 {
            return;
        }
        let palette = crate::appearance::palette();
        frame.render_widget(
            Paragraph::new(Self::title()).style(Style::new().fg(palette.accent)),
            Self::button(area),
        );
        if self.owner != context || !self.open() {
            return;
        }
        let popup = Self::popup(area, commands.len());
        crate::ui::clear_popup(frame, popup);
        frame.render_widget(
            Block::bordered()
                .title(Self::title())
                .style(crate::appearance::style()),
            popup,
        );
        let body = popup.inner(Margin::new(1, 1));
        let first = self
            .selected
            .saturating_sub(body.height.saturating_sub(1) as usize);
        let lines = commands
            .iter()
            .enumerate()
            .skip(first)
            .take(body.height as usize)
            .map(|(n, c)| {
                let key = index.preferred(c.id, c.context);
                let key = if key.is_empty() {
                    if crate::i18n::language() == crate::i18n::Language::ZhCn {
                        "未绑定".into()
                    } else {
                        "Unbound".into()
                    }
                } else {
                    key
                };
                Line::styled(
                    format!(
                        "{} {} · {}",
                        if n == self.selected { "▸" } else { " " },
                        key,
                        command_label(c.id)
                    ),
                    if n == self.selected {
                        Style::new().fg(palette.accent)
                    } else {
                        crate::appearance::style()
                    },
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unbound_actions_remain_mouse_accessible_and_scope_changes_close_the_menu() {
        let index = Index::build(
            &toml::from_str(
                r#"
            [[bindings]]
            command = "presets.apply"
            context = "dialog.presets"
            disabled = true
        "#,
            )
            .unwrap(),
        )
        .unwrap();
        let area = Rect::new(3, 2, 60, 14);
        let click = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: Mods::NONE,
        };
        let mut actions = Actions::default();
        assert!(matches!(
            actions.mouse(
                &index,
                Some("dialog.presets"),
                area,
                click(area.right() - 1, area.y)
            ),
            ActionResult::Handled
        ));
        assert!(actions.open());
        let popup = Actions::popup(area, 3);
        assert!(matches!(
            actions.mouse(
                &index,
                Some("dialog.presets"),
                area,
                click(popup.x + 1, popup.y + 1)
            ),
            ActionResult::Execute("presets.apply")
        ));
        assert!(!actions.open());
        actions.mouse(
            &index,
            Some("dialog.presets"),
            area,
            click(area.right() - 1, area.y),
        );
        assert!(matches!(
            actions.key(
                &index,
                Some("dialog.options"),
                KeyEvent::new(KeyCode::Enter, Mods::NONE)
            ),
            ActionResult::Handled
        ));
        assert!(!actions.open());
    }
    #[test]
    fn hints_and_actions_use_effective_keys_in_both_languages_themes_and_small_frames() {
        crate::i18n::with_language(crate::i18n::Language::EnUs, || {
            let index = Index::build(
                &toml::from_str(
                    r#"
                [[bindings]]
                command = "presets.apply"
                context = "dialog.presets"
                add = ["F12"]
                remove = ["Enter"]
                preferred = "F12"
                [[bindings]]
                command = "presets.delete"
                context = "dialog.presets"
                disabled = true
            "#,
                )
                .unwrap(),
            )
            .unwrap();
            for language in [crate::i18n::Language::ZhCn, crate::i18n::Language::EnUs] {
                crate::i18n::with_language(language, || {
                    for theme in [
                        crate::appearance::Theme::Tokyo,
                        crate::appearance::Theme::Light,
                    ] {
                        crate::appearance::set(theme);
                        for (width, height) in [(24, 8), (60, 18), (120, 35)] {
                            let mut terminal = ratatui::Terminal::new(
                                ratatui::backend::TestBackend::new(width, height),
                            )
                            .unwrap();
                            let actions = Actions {
                                owner: Some("dialog.presets"),
                                selected: 0,
                            };
                            terminal
                                .draw(|f| actions.draw(f, &index, Some("dialog.presets"), f.area()))
                                .unwrap();
                            let text = terminal
                                .backend()
                                .buffer()
                                .content
                                .iter()
                                .map(|c| c.symbol())
                                .collect::<String>();
                            assert!(text.contains("F12"));
                            assert!(!text.contains("Enter"));
                            assert!(!text.contains("Delete"));
                            assert!(text.replace(' ', "").contains(
                                if language == crate::i18n::Language::ZhCn {
                                    "未绑定"
                                } else {
                                    "Unbound"
                                }
                            ));
                        }
                    }
                });
            }
            assert!(index.hint("dialog.presets").contains("F12"));
            assert!(!index.hint("dialog.presets").contains("Enter"));
            crate::appearance::set(crate::appearance::Theme::Tokyo);
        });
    }
}
