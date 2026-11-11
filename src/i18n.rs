//! Application UI text only. Never translate paths, source, commands or terminal output.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    #[default]
    Auto,
    EnUs,
    ZhCn,
}
impl Language {
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::ZhCn,
            Self::ZhCn => Self::EnUs,
            Self::EnUs => Self::Auto,
        }
    }
    pub fn resolved(self) -> Self {
        if self != Self::Auto {
            return self;
        }
        for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(key)
                && !value.is_empty()
            {
                return if value.to_ascii_lowercase().starts_with("zh") {
                    Self::ZhCn
                } else {
                    Self::EnUs
                };
            }
        }
        #[cfg(windows)]
        {
            if unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff == 4
            {
                return Self::ZhCn;
            }
        }
        Self::EnUs
    }
}
#[cfg(test)]
thread_local! {static OVERRIDE:std::cell::Cell<Option<Language>>=const {std::cell::Cell::new(None)};}
#[cfg(test)]
pub(crate) fn with_language<R>(lang: Language, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<Language>);
    impl Drop for Reset {
        fn drop(&mut self) {
            OVERRIDE.set(self.0)
        }
    }
    let old = OVERRIDE.replace(Some(lang));
    let _reset = Reset(old);
    f()
}

static PREFERENCE: AtomicU8 = AtomicU8::new(0);
pub fn preference() -> Language {
    match PREFERENCE.load(Ordering::Relaxed) {
        1 => Language::EnUs,
        2 => Language::ZhCn,
        _ => Language::Auto,
    }
}
pub fn set(language: Language) {
    PREFERENCE.store(language as u8, Ordering::Relaxed);
    rust_i18n::set_locale(language.resolved().locale());
}
pub fn language() -> Language {
    #[cfg(test)]
    if let Some(lang) = OVERRIDE.get() {
        return lang.resolved();
    }
    if &*rust_i18n::locale() == "zh-CN" {
        Language::ZhCn
    } else {
        Language::EnUs
    }
}
impl Language {
    pub fn locale(self) -> &'static str {
        match self.resolved() {
            Self::ZhCn => "zh-CN",
            _ => "en-US",
        }
    }
}
/// Thin UI adapter: all translation behavior belongs to rust-i18n.
pub fn in_language(lang: Language, key: &str) -> std::borrow::Cow<'_, str> {
    rust_i18n::t!(key, locale = lang.locale())
}
pub fn t(key: &str) -> std::borrow::Cow<'_, str> {
    in_language(language(), key)
}
pub fn format(key: &str, args: &[(&str, String)]) -> String {
    format_in(language(), key, args)
}
pub fn format_in(lang: Language, key: &str, args: &[(&str, String)]) -> String {
    let template = in_language(lang, key);
    let (keys, values): (Vec<_>, Vec<_>) = args
        .iter()
        .map(|(key, value)| (*key, value.clone()))
        .unzip();
    rust_i18n::replace_patterns(&template, &keys, &values)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locales_switch_without_changing_user_values() {
        for (lang, expected) in [(Language::EnUs, "Projects"), (Language::ZhCn, "项目")] {
            with_language(lang, || {
                assert_eq!(t("projects"), expected);
                assert_eq!(
                    in_language(lang, r"D:\项目\README.md"),
                    r"D:\项目\README.md"
                );
            });
        }
    }
}
