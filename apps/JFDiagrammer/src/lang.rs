// lang -- the five UI languages (FEATURES.md 3.17): the catalogue's columns, switching at run
// time (LG-02), the command table of a language with its Office shortcuts (LG-08), the letters
// of the bold / italic / underline icons, the region for units and formats (LG-07)
//
//   LANGS                     the catalogue's columns de en es it fr (locales/ui.tsv)
//   column(choice)            the column for the option "Sprache" (0 = the system's)
//   apply(ui, col, own)       switch: the catalogue column, the command table with the
//                             language's shortcuts (and own ones), a new pass; nothing the
//                             app shows is kept translated, so the next pass is all in the
//                             new language
//   letter_icon(id, col)      BOLD / ITALIC / UNDERLINE: the icon with the letter of the
//                             language (F/B/N/G, K/I/C, U/S); other commands their own
//   region(choice)            the locale for numbers, dates and units (0 = the language's)
//
// The glossary (lang/glossary.tsv, LG-06) and check_lang.py (LG-11) keep the five columns
// consistent; the shortcuts of each language are lang/shortcuts.tsv.

use base::Ini;
use ui::i18n;
use ui::Ui;

use crate::commands::{self, BOLD, ITALIC, UNDERLINE};
use crate::icons::*;

/// the catalogue's columns (the first is the source)
pub(crate) const LANGS: [&str; 5] = ["de", "en", "es", "it", "fr"];

/// the column of the option value: 0 = the system's language (the user's list, --lang in
/// scripts; English when none of the five fits), 1..=5 = LANGS[choice - 1]
pub(crate) fn column(choice: usize) -> usize {
    if (1..=LANGS.len()).contains(&choice) {
        choice - 1
    } else {
        i18n::negotiate(&LANGS, &i18n::user_languages()).unwrap_or(1)
    }
}

/// switch the UI to column `col`: texts, the command table with the language's shortcuts and
/// the own ones of `own` ([keys]), a new pass
pub(crate) fn apply(ui: &mut Ui, col: usize, own: &Ini) {
    i18n::set_language(col as u32);
    ui.desc.cmds = commands::table(col, own);
    ui.repass();
    ui.invalidate();
}

/// the icon of command `id` in language column `col`: the text formats show the letter of the
/// language's shortcut (icons/icons.txt: bold de F, en B, es N, it/fr G; italic de/es K, en/fr
/// I, it C; underline de/en U, es/it/fr S)
pub(crate) fn letter_icon(id: u32, col: usize) -> u32 {
    let code = LANGS.get(col).copied().unwrap_or("de");
    match id {
        BOLD => match code {
            "en" => JD_BOLD_B,
            "es" => JD_BOLD_N,
            "it" | "fr" => JD_BOLD_G,
            _ => JD_BOLD_F,
        },
        ITALIC => match code {
            "en" | "fr" => JD_ITALIC_I,
            "it" => JD_ITALIC_C,
            _ => JD_ITALIC_K,
        },
        UNDERLINE => match code {
            "es" | "it" | "fr" => JD_UNDERLINE_S,
            _ => JD_UNDERLINE_U,
        },
        _ => commands::def(id).map_or(0, |d| d.icon),
    }
}

/// the regions of the option "Region" (LG-07): the stored word is the locale tag
pub(crate) const REGIONS: [&str; 7] = ["", "de-DE", "en-US", "en-GB", "es-ES", "it-IT", "fr-FR"];

/// apply the region of the option (0 = the one of the UI language: de-DE, en-US, ...)
pub(crate) fn region(choice: usize, col: usize) {
    let tag = match REGIONS.get(choice).copied().unwrap_or("") {
        "" => ["de-DE", "en-US", "es-ES", "it-IT", "fr-FR"][col.min(4)],
        t => t,
    };
    i18n::set_locale(tag);
}
