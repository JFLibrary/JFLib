// tr -- the console's languages: German source texts, English from locales/ui.tsv
//
//   init(lang)        install the catalogue and the language (also for the library's texts)
//   t(s)              a text in the current language
//   tf(s, args)       with "{}" placeholders filled
//   en()              English is active (number and date formats)

use alloc::string::String;

use ui::i18n::{self, Catalog};

pub use ui::i18n::{LANG_DE, LANG_EN};

/// install the catalogue and select a language (LANG_DE / LANG_EN)
pub fn init(lang: u32) {
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")));
    i18n::set_language(lang);
}

/// the text in the current language
pub fn t(s: &'static str) -> &'static str {
    i18n::t(s)
}

/// the text with its "{}" placeholders filled in order
pub fn tf(s: &'static str, args: &[&str]) -> String {
    i18n::fill(i18n::t(s), args)
}

/// English is active
pub fn en() -> bool {
    i18n::language() == LANG_EN
}
