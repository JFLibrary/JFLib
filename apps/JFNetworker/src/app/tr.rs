// tr -- JFNetworker's languages: German source texts, English from src/locales/ui.tsv (the
// window) and src/locales/engine.tsv (the catalog, the simulator's events, problems and
// refusals: jfn_model::Msg templates)
//
//   install()              the catalogue for the whole process (ui::i18n::set_catalog), once
//   apply(ui, lang)        the language: Deutsch, English, or the best for the user's
//                          languages (headless runs: --lang, else German); the command table
//                          and the menu bar in it (ui.desc), a new pass
//   t(s) / tf(s, args)     a text in the current language, "{}" filled in order
//   tc("Kabel|Anzahl")     the same with a context after '|' (one German word, two English)
//   menu(m)                a context menu in the current language (built once per language)
//   msg(m) / speed(kbps)   a jfn_model::Msg of the model or the simulator, a link speed
//   en()                   English is active
//   msg_en(m)              a message in English whatever the window's language (the texts an
//                          author's assessment gets in both languages, workspace/author.rs)
//
// The command table and the menus are static German tables; a language gets translated
// copies, made once and kept (labels keep their '&' mnemonics: the catalogue has them too).
// Scripts run German unless their #!args say --lang en.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::Msg;
use ui::i18n::{self, Catalog};
use ui::*;

use crate::app::prefs::Lang;

static UI_TSV: &str = include_str!("../locales/ui.tsv");
static ENGINE_TSV: &str = include_str!("../locales/engine.tsv");

/// the languages of the catalogue's columns (LANG_DE = 0 is the source)
const LANGS: [&str; 2] = ["de", "en"];

// the translated command tables and menus per language (index: the catalogue column)
static CMDS: rt::sync::SpinLock<[Option<&'static [Command]>; 2]> = rt::sync::SpinLock::new([None; 2]);
static MENUS: rt::sync::SpinLock<Vec<(usize, usize, &'static [Mi])>> = rt::sync::SpinLock::new(Vec::new());

/// install the catalogue: the window's texts, then the model's and the simulator's
pub(crate) fn install() {
    let body = ENGINE_TSV.split_once('\n').map_or("", |x| x.1);
    let all: &'static str = Box::leak(alloc::format!("{}\n{}", UI_TSV.trim_end(), body).into_boxed_str());
    i18n::set_catalog(Catalog::from_tsv(all));
}

/// select the language and give the window its command table and menus in it
pub(crate) fn apply(ui: &mut Ui, lang: Lang) {
    let k = match lang {
        Lang::De => 0,
        Lang::En => 1,
        Lang::System => i18n::negotiate(&LANGS, &i18n::user_languages()).unwrap_or(1),
    };
    i18n::set_language(k as u32);
    ui.desc.cmds = commands(k);
    ui.desc.menubar = menu_in(&crate::app::commands::MENUBAR, k);
    ui.repass();
    ui.invalidate();
}

/// a text in the current language (the German source when the catalogue has none)
pub(crate) fn t(s: &'static str) -> &'static str {
    i18n::t(s)
}

/// a text with its "{}" placeholders filled in order
pub(crate) fn tf(s: &'static str, args: &[&str]) -> String {
    i18n::fill(i18n::t(s), args)
}

/// a text with a context after '|' ("Kabel|Anzahl", "Protokoll|PDU") for German words that
/// need two translations: the catalogue keeps the whole key, German shows the part before it
pub(crate) fn tc(s: &'static str) -> &'static str {
    let r = i18n::t(s);
    if core::ptr::eq(r, s) { s.split('|').next().unwrap_or(s) } else { r }
}

/// a message of the model or the simulator (jfn_model::Msg) in the current language
pub(crate) fn msg(m: &Msg) -> String {
    m.text_with(&t)
}

/// a link speed ("1G", "2,5 Mbit/s" / "2.5 Mbit/s") in the current language
pub(crate) fn speed(kbps: u32) -> String {
    msg(&jfn_model::catalog::speed_msg(kbps))
}

/// English is active
pub(crate) fn en() -> bool {
    i18n::language() == i18n::LANG_EN
}

/// a context menu (static, German) in the current language
pub(crate) fn menu(m: &'static [Mi]) -> &'static [Mi] {
    menu_in(m, i18n::language() as usize)
}

// the command table in language k
fn commands(k: usize) -> &'static [Command] {
    let src: &'static [Command] = &crate::app::commands::CMDS;
    if k == 0 {
        return src;
    }
    let mut cache = CMDS.lock();
    if let Some(c) = cache[k.min(1)] {
        return c;
    }
    let v: Vec<Command> = src.iter().map(|c| Command { label: i18n::t(c.label), ..*c }).collect();
    let c: &'static [Command] = Vec::leak(v);
    cache[k.min(1)] = Some(c);
    c
}

// a menu in language k (made once per menu and language)
fn menu_in(m: &'static [Mi], k: usize) -> &'static [Mi] {
    if k == 0 {
        return m;
    }
    let key = m.as_ptr() as usize;
    if let Some(&(_, _, done)) = MENUS.lock().iter().find(|e| e.0 == key && e.1 == k) {
        return done;
    }
    let v: Vec<Mi> = m
        .iter()
        .map(|it| match *it {
            Mi::Menu(label, sub) => Mi::Menu(i18n::t(label), menu_in(sub, k)),
            Mi::Sub(label, sub) => Mi::Sub(i18n::t(label), menu_in(sub, k)),
            Mi::Item(id, label, icon) => Mi::Item(id, i18n::t(label), icon),
            Mi::List(id, n, label) => Mi::List(id, n, i18n::t(label)),
            other => other,
        })
        .collect();
    let done: &'static [Mi] = Vec::leak(v);
    MENUS.lock().push((key, k, done));
    done
}

// German template -> English of both catalogues (made once, for msg_en)
static EN: rt::sync::SpinLock<Option<alloc::collections::BTreeMap<&'static str, &'static str>>> = rt::sync::SpinLock::new(None);

/// a message of the model or the simulator in English, whatever the window's language
pub(crate) fn msg_en(m: &Msg) -> String {
    let mut g = EN.lock();
    let map = g.get_or_insert_with(|| {
        let mut map = alloc::collections::BTreeMap::new();
        for tsv in [UI_TSV, ENGINE_TSV] {
            for line in tsv.lines().skip(1).filter(|l| !l.starts_with('#')) {
                if let Some((de, en)) = line.split_once('\t')
                    && !en.is_empty()
                {
                    map.insert(de, en);
                }
            }
        }
        map
    });
    m.text_with(&|s| map.get(s).copied().unwrap_or(s))
}
