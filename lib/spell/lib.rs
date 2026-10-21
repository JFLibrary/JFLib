// spell -- spell checking: Hunspell dictionaries read by the library itself, the system's
// checker (Windows ISpellChecker), a personal word list
//
//   Dictionary::from_bytes(aff, dic) / load(aff_path, dic_path)   a Hunspell dictionary
//        (.aff + .dic, any SET encoding of aff.rs); add_dic(bytes) a further word list;
//        check(word), suggest(word), add_word(word); wordchars(), language(), words(),
//        unsupported() (the .aff keywords it ignored)
//   find(lang, SRC_*) -> Source        where the checker for a language tag lives
//   languages(SRC_*) -> tags           the languages there are checkers for
//   Speller::open(lang, SRC_*) / from_source(&source) / with_dictionary(dict, lang)
//        the checker the UI uses: check(word), check_token(text, &token), cached(word),
//        suggest(word), add(word) / remove(word) (the personal word list, kept in a file:
//        set_personal_file), ignore(word) (this session), misspelled(text) -> byte ranges,
//        language(), backend(), revision() (changes when the word lists change);
//        suggester() -> Suggester: suggest(word, max) on another thread (Hunspell only)
//   tokens(text, wordchars) -> Token   the words of a text worth checking (token.rs)
//   personal_path(lang)                "config://spell/<language>.dic"
//   Hyphenator::from_dic(bytes) / from_tex(text) / load(path), hyphenate(word)   hyphenation
//        (Liang's patterns: libhyphen's hyph_xx_YY.dic, TeX's \patterns; hyphen.rs);
//        find_hyphen(lang, SRC_*) / hyphen_languages(SRC_*): the dictionaries of the sources
//
// Sources (SRC_*), tried in this order for every language:
//   SRC_SYSTEM  Windows: ISpellChecker (plat::spell) for the language, then the Hunspell
//               folders of DICPATH; Linux: DICPATH, $XDG_DATA_HOME/hunspell
//               (~/.local/share/hunspell), and hunspell, myspell, myspell/dicts below every
//               XDG_DATA_DIRS folder (/usr/local/share, /usr/share); Web / Android: none
//   SRC_USER    config://hunspell -- dictionaries the user put there
//   SRC_APP     app://hunspell -- dictionaries the app ships (apps/NAME/assets/hunspell)
// A dictionary file NAME.aff (with NAME.dic beside it) serves a tag when NAME, read without
// case and with '-' as '_', is the tag ("de_DE"), a variant of it ("de_DE_frami"), the bare
// language ("de"), or the language in another region ("de_AT"; the main region first:
// "de_DE", "en_US").  Headless runs and tests use SRC_APP only, so their results never
// depend on the machine.
//
// What the Hunspell reader supports and ignores: aff.rs (format), check.rs (affix and compound
// rules), suggest.rs (suggestions).  The checks follow Hunspell 1.7 (compared on de_DE_frami,
// en_US, en_GB: docs/ROADMAP.md 4), with one difference on purpose: an UPPER word with a break
// point ("CD-ROM", "E-MAIL-ADRESSE") is correct when its pieces are (Hunspell splits the Title
// case form it made and rejects "Cd-rom").  Not supported: COMPOUNDSYLLABLE, COMPOUNDROOT,
// COMPOUNDMORESUFFIXES, COMPLEXPREFIXES (two prefixes), a prefix together with two suffixes,
// morphology (AM, the fields of the .dic), PHONE suggestions, OCONV, the replacements of
// CHECKCOMPOUNDPATTERN (simplified compounds), the Turkish / Azeri casing of LANG; no clock
// limits (Hunspell's results depend on the machine's speed, these never do).
//
// Personal word list: one word per line (a first line with a number is skipped), UTF-8.  A
// word added in lower case is also accepted Capitalised and in UPPER case; "Paris" only so.
// Words ignored for the session work the same way and are never written.  The system's
// checker never learns them (Windows' own word list stays untouched).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod aff;
mod charsets;
mod check;
mod dic;
mod hyphen;
mod speller;
mod suggest;
mod token;

use alloc::string::String;
use alloc::vec::Vec;

pub use hyphen::{find_hyphen, hyphen_languages, Hyphenator};
pub use speller::{find, languages, personal_path, Source, Speller, Suggester, SRC_ALL, SRC_APP, SRC_SYSTEM, SRC_USER};
pub use token::{tokens, Token};

use check::{Ctx, Look, CHECK};

/// compound search steps for one check
const CHECK_STEPS: u32 = 4000;

/// a Hunspell dictionary: the affix rules and the word list
pub struct Dictionary {
    pub(crate) aff: aff::Aff,
    pub(crate) words: dic::Words,
    lang: String,
}

impl Dictionary {
    /// a dictionary from the bytes of its .aff and .dic files
    pub fn from_bytes(aff: &[u8], dic: &[u8]) -> Dictionary {
        let enc = aff::encoding_of(aff);
        let text = aff::decode(aff, &enc);
        let a = aff::parse(&text, &enc);
        let mut words = dic::Words::new();
        dic::load(&mut words, &a, &aff::decode(dic, &enc));
        let lang = String::from(a.lang.as_str()).replace('_', "-");
        Dictionary { aff: a, words, lang }
    }

    /// a dictionary from two files (any vfs name: app://, config://, a path)
    pub fn load(aff_path: &str, dic_path: &str) -> Option<Dictionary> {
        let aff = plat::read_file(aff_path)?;
        let dic = plat::read_file(dic_path)?;
        Some(Dictionary::from_bytes(&aff, &dic))
    }

    /// a further word list in the dictionary's encoding and flags
    pub fn add_dic(&mut self, dic: &[u8]) {
        let text = aff::decode(dic, &self.aff.encoding);
        dic::load(&mut self.words, &self.aff, &text);
    }

    /// a word as if the .dic had it ("word" or "word/flags")
    pub fn add_word(&mut self, line: &str) {
        if let Some((w, flags)) = dic::split_line(line) {
            let f = if flags.is_empty() { Vec::new() } else { self.aff.flags(flags) };
            dic::add(&mut self.words, &self.aff, &w, f);
        }
    }

    /// the language: the LANG of the .aff, or what the loader set ("de-DE")
    pub fn language(&self) -> &str {
        &self.lang
    }

    /// set the language tag (find() names it after the file)
    pub fn set_language(&mut self, tag: &str) {
        self.lang = tag.replace('_', "-");
    }

    /// WORDCHARS: characters that belong to words besides the letters
    pub fn wordchars(&self) -> &[char] {
        &self.aff.wordchars
    }

    /// the number of entries in the word list
    pub fn words(&self) -> usize {
        self.words.len()
    }

    /// .aff keywords this reader ignored
    pub fn unsupported(&self) -> &[String] {
        &self.aff.unsupported
    }

    /// the word is correct (case variants, a trailing abbreviation dot, numbers, break points)
    pub fn check(&self, word: &str) -> bool {
        let w = self.clean(word);
        if w.is_empty() || is_number(&w) {
            return true;
        }
        let t = w.trim_end_matches('.');
        if t.is_empty() {
            return true;
        }
        let mut cx = Ctx::new(CHECK_STEPS);
        match self.check_cased(t, CHECK, &mut cx) {
            Look::Hit(_) => return true,
            Look::Forbidden => return false,
            Look::Miss => {}
        }
        if t.len() < w.len() {
            let mut d = String::from(t);
            d.push('.');
            match self.check_cased(&d, CHECK, &mut cx) {
                Look::Hit(_) => return true,
                Look::Forbidden => return false,
                Look::Miss => {}
            }
        }
        // (a forbidden word is never split at its break points)
        self.check_break(t, 0, CHECK, &mut cx)
    }

    /// corrections for a misspelled word, best first (at most 8)
    pub fn suggest(&self, word: &str) -> Vec<String> {
        self.suggest_n(word, suggest::SUG_MAX)
    }

    // IGNORE characters removed, ICONV applied
    pub(crate) fn clean(&self, w: &str) -> String {
        let a = &self.aff;
        let s: String = if a.ignore.is_empty() { String::from(w) } else { w.chars().filter(|c| !a.ignore.contains(c)).collect() };
        if a.iconv.is_empty() {
            return s;
        }
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < s.len() {
            let best = a.iconv.iter().filter(|(f, _)| !f.is_empty() && s[i..].starts_with(f.as_str())).max_by_key(|(f, _)| f.len());
            match best {
                Some((f, t)) => {
                    out.push_str(t);
                    i += f.len();
                }
                None => {
                    let c = s[i..].chars().next().unwrap_or(' ');
                    out.push(c);
                    i += c.len_utf8();
                }
            }
        }
        out
    }
}

/// digits with separators ("1.000,50", "12-15", "3/4")
pub(crate) fn is_number(w: &str) -> bool {
    w.chars().any(|c| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '/' | ':' | '%'))
}
