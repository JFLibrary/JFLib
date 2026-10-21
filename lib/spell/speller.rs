// speller -- finding a language's checker and the Speller the UI uses (see lib.rs)

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::check::{captype, lower, title, Cap};
use crate::token::Token;
use crate::Dictionary;

/// the system: Windows ISpellChecker, the Hunspell folders of Linux, DICPATH
pub const SRC_SYSTEM: u32 = 1;
/// config://hunspell
pub const SRC_USER: u32 = 2;
/// app://hunspell
pub const SRC_APP: u32 = 4;
/// every source
pub const SRC_ALL: u32 = 7;

/// results kept per Speller (then the cache starts again)
const MEMO_MAX: usize = 50_000;

/// where a language's checker lives
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// the system's checker (plat::spell) with this tag
    System(String),
    /// a Hunspell dictionary: the .aff and .dic names, the tag it serves, the SRC_* it came from
    Hunspell { aff: String, dic: String, lang: String, src: u32 },
}

/// the personal word list of a language: "config://spell/<primary language>.dic"
pub fn personal_path(lang: &str) -> String {
    let p = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    format!("config://spell/{}.dic", if p.is_empty() { "default" } else { p.as_str() })
}

// "de-DE" / "de_de" -> ("de", "de")
pub(crate) fn split_tag(t: &str) -> (String, String) {
    let t = t.to_ascii_lowercase().replace('-', "_");
    let mut it = t.split('_');
    let p = String::from(it.next().unwrap_or(""));
    let r = String::from(it.next().unwrap_or(""));
    (p, r)
}

// the main region of a language (as win_spell.rs)
fn main_region(p: &str) -> String {
    String::from(match p {
        "en" => "us",
        "he" => "il",
        "el" => "gr",
        "da" => "dk",
        "sv" => "se",
        "cs" => "cz",
        "uk" => "ua",
        "nb" | "nn" => "no",
        "ar" => "sa",
        "ca" => "es",
        "ja" => "jp",
        "ko" => "kr",
        "zh" => "cn",
        p => p,
    })
}

// how well a dictionary file stem serves the tag (0: not at all)
pub(crate) fn score(stem: &str, p: &str, r: &str) -> u32 {
    let s = stem.to_ascii_lowercase().replace('-', "_");
    let mut parts = s.split('_');
    if parts.next() != Some(p) {
        return 0;
    }
    let sr = parts.next().unwrap_or("");
    let variant = parts.next().is_some();
    let main = main_region(p);
    if !r.is_empty() && sr == r {
        return if variant { 5 } else { 6 };
    }
    if sr.is_empty() {
        return 4;
    }
    if sr == main {
        return if variant { 2 } else { 3 };
    }
    1
}

// the Hunspell folders of a source
fn dirs(src: u32) -> Vec<String> {
    let mut v = Vec::new();
    match src {
        SRC_SYSTEM => v = plat::spell::hunspell_dirs(),
        SRC_USER => v.push(String::from("config://hunspell")),
        SRC_APP => v.push(String::from("app://hunspell")),
        _ => {}
    }
    v
}

// the .aff files of a folder that have a .dic beside them: (stem, aff, dic), sorted
fn pairs(dir: &str) -> Vec<(String, String, String)> {
    let mut names: Vec<String> = Vec::new();
    plat::dir_list(dir, &mut |n, is_dir, _| {
        if !is_dir {
            names.push(String::from(n));
        }
    });
    names.sort();
    let mut out = Vec::new();
    for n in &names {
        let Some(stem) = n.strip_suffix(".aff") else { continue };
        let d = format!("{}.dic", stem);
        if names.iter().any(|x| *x == d) {
            let sep = if dir.ends_with('/') { "" } else { "/" };
            out.push((String::from(stem), format!("{}{}{}", dir, sep, n), format!("{}{}{}", dir, sep, d)));
        }
    }
    out
}

// the best Hunspell dictionary of a source for the tag
fn find_hunspell(lang: &str, src: u32) -> Option<Source> {
    let (p, r) = split_tag(lang);
    if p.is_empty() {
        return None;
    }
    let mut best: Option<(u32, Source)> = None;
    for d in dirs(src) {
        for (stem, aff, dic) in pairs(&d) {
            let s = score(&stem, &p, &r);
            if s > 0 && best.as_ref().is_none_or(|b| s > b.0) {
                best = Some((s, Source::Hunspell { aff, dic, lang: stem.replace('_', "-"), src }));
            }
        }
    }
    best.map(|b| b.1)
}

/// where the checker for a language tag lives (sources tried in the order of lib.rs)
pub fn find(lang: &str, sources: u32) -> Option<Source> {
    for src in [SRC_SYSTEM, SRC_USER, SRC_APP] {
        if sources & src == 0 {
            continue;
        }
        if src == SRC_SYSTEM {
            if let Some(t) = plat::spell::sys_spell_find(lang) {
                return Some(Source::System(t));
            }
        }
        if let Some(s) = find_hunspell(lang, src) {
            return Some(s);
        }
    }
    None
}

/// the language tags there are checkers for (sorted, each once)
pub fn languages(sources: u32) -> Vec<String> {
    let mut v: BTreeSet<String> = BTreeSet::new();
    for src in [SRC_SYSTEM, SRC_USER, SRC_APP] {
        if sources & src == 0 {
            continue;
        }
        if src == SRC_SYSTEM {
            v.extend(plat::spell::sys_spell_languages());
        }
        for d in dirs(src) {
            for (stem, _, _) in pairs(&d) {
                v.insert(stem.replace('_', "-"));
            }
        }
    }
    v.into_iter().collect()
}

enum Backend {
    Hunspell(Arc<Dictionary>),
    System(plat::spell::SysSpeller),
}

/// a checker with its personal word list and a cache of results (see lib.rs)
pub struct Speller {
    backend: Backend,
    lang: String,
    added: BTreeSet<String>,
    ignored: BTreeSet<String>,
    file: Option<String>,
    memo: BTreeMap<String, bool>,
    rev: u32,
}

impl Speller {
    fn new(backend: Backend, lang: &str) -> Speller {
        Speller {
            backend,
            lang: lang.replace('_', "-"),
            added: BTreeSet::new(),
            ignored: BTreeSet::new(),
            file: None,
            memo: BTreeMap::new(),
            rev: 0,
        }
    }

    /// a checker for a Hunspell dictionary
    pub fn with_dictionary(mut dict: Dictionary, lang: &str) -> Speller {
        if !lang.is_empty() {
            dict.set_language(lang);
        }
        let l = String::from(dict.language());
        Speller::new(Backend::Hunspell(Arc::new(dict)), &l)
    }

    /// the checker of a source (Hunspell files are read now)
    pub fn from_source(s: &Source) -> Option<Speller> {
        match s {
            Source::System(t) => plat::spell::SysSpeller::open(t).map(|c| Speller::new(Backend::System(c), t)),
            Source::Hunspell { aff, dic, lang, .. } => Dictionary::load(aff, dic).map(|d| Speller::with_dictionary(d, lang)),
        }
    }

    /// the checker for a language tag (find + from_source)
    pub fn open(lang: &str, sources: u32) -> Option<Speller> {
        Speller::from_source(&find(lang, sources)?)
    }

    /// the language tag ("de-DE")
    pub fn language(&self) -> &str {
        &self.lang
    }

    /// "hunspell" or "system"
    pub fn backend(&self) -> &'static str {
        match self.backend {
            Backend::Hunspell(_) => "hunspell",
            Backend::System(_) => "system",
        }
    }

    /// the Hunspell dictionary (None for the system's checker)
    pub fn dictionary(&self) -> Option<&Dictionary> {
        match &self.backend {
            Backend::Hunspell(d) => Some(&**d),
            Backend::System(_) => None,
        }
    }

    /// WORDCHARS of the dictionary (for tokens())
    pub fn wordchars(&self) -> &[char] {
        match &self.backend {
            Backend::Hunspell(d) => d.wordchars(),
            Backend::System(_) => &[],
        }
    }

    /// changes whenever the personal or the ignored words change (caches of results depend on it)
    pub fn revision(&self) -> u32 {
        self.rev
    }

    // the word is in a list, also as the lower / title case form of a Title / UPPER word
    fn listed(&self, w: &str) -> bool {
        let lists = [&self.added, &self.ignored];
        if lists.iter().any(|l| l.contains(w)) {
            return true;
        }
        let cap = captype(w, false);
        if cap == Cap::Init || cap == Cap::All {
            let l = lower(w);
            if lists.iter().any(|x| x.contains(&l)) {
                return true;
            }
            if cap == Cap::All {
                let t = title(&l);
                return lists.iter().any(|x| x.contains(&t));
            }
        }
        false
    }

    /// the cached result for a word (None: not checked yet)
    pub fn cached(&self, word: &str) -> Option<bool> {
        self.memo.get(word).copied()
    }

    /// the word is correct (personal and ignored words, then the checker; cached)
    pub fn check(&mut self, word: &str) -> bool {
        if let Some(&r) = self.memo.get(word) {
            return r;
        }
        let r = self.listed(word)
            || match &self.backend {
                Backend::Hunspell(d) => d.check(word),
                Backend::System(c) => c.check(word),
            };
        if self.memo.len() >= MEMO_MAX {
            self.memo.clear();
        }
        self.memo.insert(String::from(word), r);
        r
    }

    /// a token of `text` is correct: the word, or the word with the dot that follows it
    /// (an abbreviation)
    pub fn check_token(&mut self, text: &str, t: &Token) -> bool {
        let w = &text[t.start..t.end];
        if self.check(w) {
            return true;
        }
        t.dot && self.check(&text[t.start..t.end + 1])
    }

    /// the misspelled byte ranges of one token: none when it is correct.  A hyphenated word
    /// marks only its wrong parts: from each part on the longest run of parts that is a word
    /// counts as right ("E-Mail" of "E-Mail-Adr"), wrong parts next to each other are one
    /// range ("E-Mial"); "Hasu-Tür" marks "Hasu"; right parts make a right word (a personal
    /// word inside)
    pub fn token_errors(&mut self, text: &str, t: &Token) -> Vec<(usize, usize)> {
        if self.check_token(text, t) {
            return Vec::new();
        }
        let w = &text[t.start..t.end];
        let hyphen = |c: char| matches!(c, '-' | '\u{2010}' | '\u{2011}');
        if !w.contains(hyphen) {
            return alloc::vec![(t.start, t.end)];
        }
        // the parts (byte ranges in text)
        let mut parts: Vec<(usize, usize)> = Vec::new();
        let mut a = t.start;
        for (i, c) in w.char_indices().filter(|c| hyphen(c.1)) {
            parts.push((a, t.start + i));
            a = t.start + i + c.len_utf8();
        }
        parts.push((a, t.end));
        let mut out: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < parts.len() {
            let mut j = parts.len();
            let mut found = false;
            while j > i {
                let (s, e) = (parts[i].0, parts[j - 1].1);
                let last = j == parts.len();
                if s < e && (self.check(&text[s..e]) || (last && t.dot && self.check(&text[s..e + 1]))) {
                    found = true;
                    break;
                }
                j -= 1;
            }
            if found {
                i = j;
                continue;
            }
            let (s, e) = parts[i];
            if s < e {
                match out.last_mut() {
                    Some(l) if i > 0 && l.1 == parts[i - 1].1 => l.1 = e,
                    _ => out.push((s, e)),
                }
            }
            i += 1;
        }
        out
    }

    /// the byte ranges of the misspelled words of a text
    pub fn misspelled(&mut self, text: &str) -> Vec<(usize, usize)> {
        let toks = crate::tokens(text, &self.wordchars().to_vec());
        let mut out = Vec::new();
        for t in &toks {
            out.extend(self.token_errors(text, t));
        }
        out
    }

    /// corrections, best first (at most max)
    pub fn suggest(&mut self, word: &str, max: usize) -> Vec<String> {
        let v = match &self.backend {
            Backend::Hunspell(d) => d.suggest_n(word, max),
            Backend::System(c) => c.suggest(word, max),
        };
        with_personal(v, word, self.added.iter(), max)
    }

    /// suggest() for another thread: the Hunspell dictionary and the personal list as they are
    /// now (None for the system's checker, which stays on its thread)
    pub fn suggester(&self) -> Option<Suggester> {
        match &self.backend {
            Backend::Hunspell(d) => Some(Suggester { dict: d.clone(), added: self.added.iter().cloned().collect() }),
            Backend::System(_) => None,
        }
    }

    /// add a word to the personal list (written to the file of set_personal_file) -> the file
    /// was written (or there is none)
    pub fn add(&mut self, word: &str) -> bool {
        let w = word.trim();
        if w.is_empty() || w.contains('\n') {
            return false;
        }
        self.added.insert(String::from(w));
        self.changed();
        self.save()
    }

    /// take a word off the personal list -> the file was written (or there is none)
    pub fn remove(&mut self, word: &str) -> bool {
        if !self.added.remove(word.trim()) {
            return true;
        }
        self.changed();
        self.save()
    }

    /// accept the word for this session (not written anywhere)
    pub fn ignore(&mut self, word: &str) {
        let w = word.trim();
        if !w.is_empty() {
            self.ignored.insert(String::from(w));
            self.changed();
        }
    }

    /// the personal list
    pub fn personal(&self) -> Vec<String> {
        self.added.iter().cloned().collect()
    }

    /// the words ignored this session
    pub fn ignored(&self) -> Vec<String> {
        self.ignored.iter().cloned().collect()
    }

    /// keep the personal list in a file (read now; None: in memory only)
    pub fn set_personal_file(&mut self, path: Option<&str>) {
        self.file = path.map(String::from);
        if let Some(p) = path {
            if let Some(b) = plat::read_file(p) {
                let text = String::from_utf8_lossy(&b);
                for (i, l) in text.lines().enumerate() {
                    let l = l.trim();
                    if l.is_empty() || l.starts_with('#') || (i == 0 && l.parse::<u64>().is_ok()) {
                        continue;
                    }
                    self.added.insert(String::from(l));
                }
                self.changed();
            }
        }
    }

    fn changed(&mut self) {
        self.memo.clear();
        self.rev = self.rev.wrapping_add(1);
    }

    fn save(&self) -> bool {
        let Some(p) = &self.file else { return true };
        let mut s = String::new();
        for w in &self.added {
            s.push_str(w);
            s.push('\n');
        }
        plat::write_file(p, s.as_bytes())
    }
}

/// a Speller's suggestions for a worker thread (Speller::suggester): the same results as
/// Speller::suggest at the time it was made
pub struct Suggester {
    dict: Arc<Dictionary>,
    added: Vec<String>,
}

impl Suggester {
    /// corrections, best first (at most max)
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        with_personal(self.dict.suggest_n(word, max), word, self.added.iter(), max)
    }
}

// the words of the personal list close to the word first (the list in its order)
fn with_personal<'a>(mut v: Vec<String>, word: &str, added: impl Iterator<Item = &'a String>, max: usize) -> Vec<String> {
    let lw: Vec<char> = lower(word).chars().collect();
    for a in added {
        let la: Vec<char> = lower(a).chars().collect();
        if la != lw && crate::suggest::damerau(&lw, &la) <= 1 + lw.len() / 6 && !v.contains(a) {
            v.insert(0, a.clone());
        }
    }
    v.truncate(max);
    v
}
