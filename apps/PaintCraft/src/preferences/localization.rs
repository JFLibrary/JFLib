//! Explicit UI translation. Document text, file names and plugin names are never
//! passed through the catalog. Unknown labels retain the German source text.
use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use crate::app::Pn;
use ui::{Mi, Ui};
static LANGUAGE: AtomicU32 = AtomicU32::new(0);
const CATALOG: &str = include_str!("../../locales/ui.tsv");
pub(crate) const NAMES: [&str; 3] = ["Deutsch", "English", "Français"];
const CODES: [&str; 3] = ["de", "en", "fr"];
pub(crate) fn code(value: &str) -> u32 { match value { "en" | "English" => 1, "fr" | "Français" => 2, _ => 0 } }
// --lang / jfgui_LANG, as every app reads them ("fr-CA" or a list "fr-CA:en"): the best of the
// catalogue's languages, else English
fn wanted() -> Option<u32> {
    let v = plat::arg_value("--lang").or_else(|| plat::env("jfgui_LANG")).filter(|v| !v.trim().is_empty())?;
    let list: Vec<&str> = v.split([':', ',']).map(str::trim).filter(|t| !t.is_empty()).collect();
    Some(ui::i18n::negotiate(&CODES, &list).map_or(1, |k| k as u32))
}
pub(crate) fn tr(s: &str) -> &str { translate(s, LANGUAGE.load(Ordering::Relaxed)) }
/// the text in the UI language with its "{}" placeholders filled in order
pub(crate) fn tf(s: &str, args: &[&str]) -> alloc::string::String { ui::i18n::fill(tr(s), args) }
/// the decimal separator of the UI language: "." in English, "," in German and French
pub(crate) fn dec() -> &'static str { if LANGUAGE.load(Ordering::Relaxed) == 1 { "." } else { "," } }
fn translate(s: &str, language: u32) -> &str {
    if language == 0 || s.is_empty() { return s; }
    for line in CATALOG.lines() {
        let mut fields = line.split('\t');
        if fields.next() == Some(s) { return fields.nth(language as usize-1).filter(|v| !v.is_empty()).unwrap_or(s); }
    }
    // then the library's own texts in the same language (apply sets it), as ui::i18n::t does;
    // tools/i18n_check.py counts them as translated
    match ui::i18n::library_keys().find(|k| *k == s) { Some(k) => ui::i18n::tr(k), None => s }
}
pub(crate) fn list<'a>(items: &[&'a str]) -> Vec<&'a str> { items.iter().map(|s| tr(s)).collect() }
fn menu(items: &'static [Mi], language: u32) -> &'static [Mi] {
    Box::leak(items.iter().map(|item| match *item {
        Mi::Menu(label, sub) => Mi::Menu(translate(label,language),menu(sub,language)),
        Mi::Sub(label, sub) => Mi::Sub(translate(label,language),menu(sub,language)),
        Mi::Item(id,label,icon) => Mi::Item(id,translate(label,language),icon),
        Mi::List(id,n,label) => Mi::List(id,n,translate(label,language)),
        _ => *item,
    }).collect::<Vec<_>>().into_boxed_slice())
}
static MENUS: rt::sync::SpinLock<[Option<&'static [Mi]>; 3]> = rt::sync::SpinLock::new([None;3]);
pub(crate) fn apply(p: &mut Pn, ui: &mut Ui, language: u32) {
    let language = language.min(2); p.st.st.language = language;
    LANGUAGE.store(language, Ordering::Relaxed);
    ui::i18n::set_language(language); // the library's texts (file dialog, date picker, ...) follow
    for original in &crate::app::commands::PN_CMDS {
        // Runtime-generated recent-file/effect labels stay intact.
        if original.label.is_empty() || original.label.bytes().all(|b| b.is_ascii_digit()) { continue; }
        if let Some(c) = p.cmds.iter_mut().find(|c| c.id == original.id) { c.label = translate(original.label,language); }
    }
    let mut menus=MENUS.lock();
    ui.desc.menubar = *menus[language as usize].get_or_insert_with(||menu(&crate::app::commands::PN_MENUBAR,language));
    p.cmds_dirty = true; ui.repass(); ui.invalidate();
}
pub(crate) fn init(p: &mut Pn, ui: &mut Ui) {
    // a catalogue that only names the columns: this one is read by translate above
    ui::i18n::set_catalog(ui::i18n::Catalog::from_tsv("de\ten\tfr\n"));
    let language = plat::arg_value("--language").map(|s| code(&s)).or_else(wanted).unwrap_or(p.st.st.language);
    apply(p,ui,language);
}
