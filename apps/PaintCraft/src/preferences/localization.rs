//! Explicit UI translation. Document text, file names and plugin names are never
//! passed through the catalog. Unknown labels retain the German source text.
use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use crate::app::Pn;
use ui::{Mi, Ui};
static LANGUAGE: AtomicU32 = AtomicU32::new(0);
const CATALOG: &str = include_str!("../../locales/ui.tsv");
pub(crate) const NAMES: [&str; 3] = ["Deutsch", "English", "Français"];
pub(crate) fn code(value: &str) -> u32 { match value { "en" | "English" => 1, "fr" | "Français" => 2, _ => 0 } }
pub(crate) fn tr(s: &str) -> &str { translate(s, LANGUAGE.load(Ordering::Relaxed)) }
fn translate(s: &str, language: u32) -> &str {
    if language == 0 || s.is_empty() { return s; }
    for line in CATALOG.lines() {
        let mut fields = line.split('\t');
        if fields.next() == Some(s) { return fields.nth(language as usize-1).filter(|v| !v.is_empty()).unwrap_or(s); }
    }
    s
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
    let language = plat::arg_value("--language").map_or(p.st.st.language, |s| code(&s));
    apply(p,ui,language);
}
