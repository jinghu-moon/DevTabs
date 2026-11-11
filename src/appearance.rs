//! Shared semantic application chrome; PTY cells retain their original colors.
use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    Terminal,
    Tokyo,
    Catppuccin,
    Light,
}
static ACTIVE: AtomicU8 = AtomicU8::new(0);
impl Theme {
    pub fn next(self) -> Self {
        match self {
            Self::Terminal => Self::Tokyo,
            Self::Tokyo => Self::Catppuccin,
            Self::Catppuccin => Self::Light,
            Self::Light => Self::Terminal,
        }
    }
    pub fn label(self) -> std::borrow::Cow<'static, str> {
        crate::i18n::t(match self {
            Self::Terminal => "terminal",
            Self::Tokyo => "tokyo",
            Self::Catppuccin => "catppuccin",
            Self::Light => "light",
        })
    }
}
pub fn set(theme: Theme) {
    ACTIVE.store(theme as u8, Ordering::Relaxed)
}
pub fn theme() -> Theme {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Theme::Tokyo,
        2 => Theme::Catppuccin,
        3 => Theme::Light,
        _ => Theme::Terminal,
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub fg: Color,
    pub bg: Color,
    pub muted: Color,
    pub accent: Color,
    pub selected: Color,
    pub error: Color,
}
pub fn palette() -> Palette {
    match theme() {
        Theme::Terminal => Palette {
            fg: Color::Reset,
            bg: Color::Reset,
            muted: Color::Rgb(145, 155, 174),
            accent: Color::Cyan,
            selected: Color::DarkGray,
            error: Color::Red,
        },
        Theme::Tokyo => Palette {
            fg: Color::Rgb(192, 202, 245),
            bg: Color::Rgb(26, 27, 38),
            muted: Color::Rgb(145, 156, 185),
            accent: Color::Rgb(125, 207, 255),
            selected: Color::Rgb(41, 46, 66),
            error: Color::Rgb(247, 118, 142),
        },
        Theme::Catppuccin => Palette {
            fg: Color::Rgb(205, 214, 244),
            bg: Color::Rgb(30, 30, 46),
            muted: Color::Rgb(147, 153, 178),
            accent: Color::Rgb(137, 180, 250),
            selected: Color::Rgb(69, 71, 90),
            error: Color::Rgb(243, 139, 168),
        },
        Theme::Light => Palette {
            fg: Color::Rgb(38, 44, 55),
            bg: Color::Rgb(245, 246, 248),
            muted: Color::Rgb(100, 110, 125),
            accent: Color::Rgb(0, 82, 160),
            selected: Color::Rgb(211, 225, 242),
            error: Color::Rgb(180, 20, 30),
        },
    }
}
pub fn style() -> Style {
    let p = palette();
    Style::new().fg(p.fg).bg(p.bg)
}
