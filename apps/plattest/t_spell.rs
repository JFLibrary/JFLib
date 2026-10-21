// t_spell -- the system's spell checker (lib/plat/spell.rs)
//
//   plattest --test spell     Windows: the real ISpellCheckerFactory / ISpellChecker: the
//                             supported languages, sys_spell_find (every listed tag, the bare
//                             language, an unknown tag), then for the first language with sample
//                             words here: a correct and a misspelled word, the suggestions;
//                             Windows without spell checking languages: only the empty answers.
//                             Linux / Web / Android: no system checker (None, empty).  Then the
//                             Hunspell folders (DICPATH first) and that a manifest leaves them
//                             readable.

use alloc::format;
use alloc::string::String;

use plat::spell::{hunspell_dirs, sys_spell_find, sys_spell_languages, SysSpeller};

use crate::T;

// (language, a correct word, a misspelling whose suggestions contain it)
const SAMPLES: &[(&str, &str, &str)] = &[
    ("de", "Haus", "Hasu"),
    ("en", "house", "hosue"),
    ("fr", "maison", "maisno"),
    ("es", "casa", "csaa"),
    ("it", "casa", "csaa"),
    ("nl", "huis", "hius"),
];

fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or("")
}

pub fn t_spell(t: &mut T) {
    checker(t);
    dict_dirs(t);
}

// the Hunspell folders: DICPATH first; readable under any manifest (the policy is installed
// last: it stays for the rest of the process)
fn dict_dirs(t: &mut T) {
    let mine = if cfg!(windows) { "C:/jfgui-dicts" } else { "/opt/jfgui-dicts" };
    plat::setenv("DICPATH", Some(mine));
    let dirs = hunspell_dirs();
    t.check(dirs.first().map(String::as_str) == Some(mine), &format!("spell: DICPATH comes first ({:?})", dirs));
    t.check(cfg!(windows) || dirs.iter().any(|d| d == "/usr/share/hunspell"), "spell: Linux searches /usr/share/hunspell");
    if plat::policy::install("# plattest --test spell: only what is always allowed\n").is_ok() {
        let read = |p: &str| plat::policy::fs_allowed(p, plat::policy::FS_READ);
        t.check(read(&format!("{}/de_DE.aff", mine)), "spell: a dictionary of DICPATH is readable under a manifest");
        t.check(cfg!(windows) || read("/usr/share/hunspell/de_DE.dic"), "spell: /usr/share/hunspell is readable under a manifest");
        t.info("  (one PERMDENY line follows: a folder outside the rules)\n");
        t.check(!read(&format!("{}-other/x.aff", mine)), "spell: other folders stay closed");
    } else {
        t.check(false, "spell: a manifest could be installed");
    }
    plat::setenv("DICPATH", None);
}

// the system's checker
fn checker(t: &mut T) {
    let langs = sys_spell_languages();
    t.info(&format!("  system spell checker: {:?}\n", langs));
    t.check(sys_spell_find("xx-QQ").is_none(), "spell: an unknown language is not found");
    if !cfg!(windows) {
        t.check(langs.is_empty() && sys_spell_find("de-DE").is_none() && SysSpeller::open("de-DE").is_none(), "spell: no system checker here");
        return;
    }
    if langs.is_empty() {
        t.info("  no spell checking languages installed: nothing more to check\n");
        t.check(sys_spell_find("en-US").is_none(), "spell: no language, nothing found");
        return;
    }
    for l in &langs {
        t.check(sys_spell_find(l).as_deref() == Some(l.as_str()), &format!("spell: sys_spell_find({}) finds itself", l));
    }
    let p = primary(&langs[0]);
    t.check(sys_spell_find(p).is_some_and(|x| primary(&x).eq_ignore_ascii_case(p)), &format!("spell: the bare language {} finds a region of it", p));
    let found = SAMPLES.iter().find_map(|&(p, good, bad)| langs.iter().find(|l| primary(l).eq_ignore_ascii_case(p)).map(|l| (String::from(l.as_str()), good, bad)));
    let Some((tag, good, bad)) = found else {
        t.info("  no language with sample words: check / suggest not tested\n");
        return;
    };
    let Some(sp) = SysSpeller::open(&tag) else {
        t.check(false, &format!("spell: SysSpeller::open({})", tag));
        return;
    };
    t.check(sp.language() == tag, &format!("spell: the checker of {}", tag));
    t.check(sp.check(good), &format!("spell: {:?} is correct ({})", good, tag));
    t.check(!sp.check(bad), &format!("spell: {:?} is misspelled ({})", bad, tag));
    let s = sp.suggest(bad, 8);
    t.check(s.iter().any(|x| x == good), &format!("spell: suggestions for {:?} contain {:?} ({:?})", bad, good, s));
    // (Windows offers similar words for a correct one, too; never the word itself)
    let s = sp.suggest(good, 8);
    t.check(s.len() <= 8 && !s.iter().any(|x| x == good), &format!("spell: suggestions for the correct {:?} without itself ({:?})", good, s));
    t.check(sp.check("") && sp.suggest("", 8).is_empty(), "spell: the empty word");
}
