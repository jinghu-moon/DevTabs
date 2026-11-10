//! One lazily loaded syntax theme for Markdown, previews and the editor.
use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;
use syntect::{
    highlighting::{FontStyle, Theme},
    parsing::Scope,
};
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

pub(super) fn syntax(light: bool) -> &'static Theme {
    static THEMES: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();
    THEMES.get_or_init(two_face::theme::extra).get(if light {
        EmbeddedThemeName::CatppuccinLatte
    } else {
        EmbeddedThemeName::CatppuccinMocha
    })
}

pub(super) fn convert(s: syntect::highlighting::Style) -> Style {
    let mut out = Style::new().fg(Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b));
    if s.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    out
}

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub headings: [Style; 6],
    pub strong: Style,
    pub emphasis: Style,
    pub code: Style,
    pub link: Style,
    pub quote: Style,
    pub rule: Style,
    pub block: Style,
}

impl Palette {
    pub fn new(light: bool) -> Self {
        static LIGHT: OnceLock<Palette> = OnceLock::new();
        static DARK: OnceLock<Palette> = OnceLock::new();
        *if light { &LIGHT } else { &DARK }.get_or_init(|| Self::build(light))
    }
    fn build(light: bool) -> Self {
        let theme = syntax(light);
        let accent = if light {
            Color::Rgb(30, 102, 245)
        } else {
            Color::Rgb(137, 180, 250)
        };
        let highlighter = syntect::highlighting::Highlighter::new(theme);
        let scope = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| {
                    let scope = name.parse::<Scope>().ok()?;
                    let c = highlighter.style_mod_for_stack(&[scope]).foreground?;
                    Some(Style::new().fg(Color::Rgb(c.r, c.g, c.b)))
                })
                .unwrap_or_else(|| Style::new().fg(accent))
        };
        let headings = std::array::from_fn(|i| {
            let name = format!("markup.heading.{}.markdown", i + 1);
            let base = scope(&[&name, "markup.heading.markdown"]);
            match i {
                0 => base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                1 => base.add_modifier(Modifier::BOLD),
                2 => base.add_modifier(Modifier::BOLD | Modifier::ITALIC),
                _ => base.add_modifier(Modifier::ITALIC),
            }
        });
        Self {
            headings,
            strong: scope(&["markup.bold.markdown"]).add_modifier(Modifier::BOLD),
            emphasis: scope(&["markup.italic.markdown"]).add_modifier(Modifier::ITALIC),
            code: scope(&[
                "markup.inline.raw.string.markdown",
                "markup.raw.inline.markdown",
            ])
            .bg(if light {
                Color::Rgb(230, 233, 239)
            } else {
                Color::Rgb(35, 39, 48)
            }),
            link: scope(&["markup.underline.link.markdown"]).add_modifier(Modifier::UNDERLINED),
            quote: scope(&["markup.quote.markdown"]),
            rule: Style::new().fg(if light {
                Color::Rgb(124, 127, 147)
            } else {
                Color::Rgb(108, 112, 134)
            }),
            block: Style::new().bg(if light {
                Color::Rgb(230, 233, 239)
            } else {
                Color::Rgb(35, 39, 48)
            }),
        }
    }
}
