// i18n_collate -- sorting by language (collation) and Unicode case mapping
//
//   collate(a, b) -> Ordering           two texts in the order of the user's language
//                                       (locale_tag(): the format locale, as Windows'
//                                       CompareStringEx): base letters first, then accents,
//                                       then case: "a" < "A" < "á" < "ä" < "b"
//   collator() / Collator::new(tag)     a collator with options: .strength(COLL_PRIMARY ..
//                                       COLL_IDENTICAL), .numeric(true) ("Datei 2" before
//                                       "Datei 10"), .phonebook(true) (German phone book:
//                                       ä = ae); .compare(a, b), .key(s) (a sort key: keys
//                                       compare like their texts), .sort(items, text).  The
//                                       tag may carry "-u-co-phonebk", "-u-kn", "-u-ks-level1"
//   Locale::collate(a, b) / collator()  the same for a fixed region
//   to_upper(s) / to_lower(s) / fold(s) case mapping of all of Unicode in the user's language:
//                                       ß -> SS, "ΟΔΟΣ" -> "οδος" (final sigma), Turkish and
//                                       Azeri i / İ and ı / I; fold() for comparisons (ß and
//                                       ẞ -> ss, ς -> σ)
//   to_upper_in(lang, s) / to_lower_in / fold_in   the same for a language
//   eq_ignore_case(a, b)                equal except for case (also composed against
//                                       decomposed accents: "É" = "e\u{301}")
//
// The collation is a compact form of the Unicode Collation Algorithm, not the CLDR data:
// every character becomes collation elements (primary: the letter, secondary: its accents,
// tertiary: case and variants such as ß or ſ); the canonical decompositions come from the
// generated table i18n_unicode.rs (tools/gen_unicode.py).  Order of the primaries: spaces,
// punctuation and symbols (by code point), digits (by value), Latin letters a .. z, then the
// letters of the other scripts by code point (Greek, Cyrillic, Hebrew, Arabic, ..., kana as
// hiragana, CJK).  Language rules where they are usual: German (ä like a; phone book ä = ae),
// Swedish / Finnish (å ä ö after z), Danish / Norwegian (æ ø å after z, aa = å), Spanish
// (ñ after n), Czech / Slovak (č ř š ž, ch after h), Polish (ą ć ę ł ń ó ś ź ż), Turkish
// (ç ğ ı ö ş ü, I is the capital of ı).  Everywhere: ß = ss, æ = ae, œ = oe, ø ł đ like o l d
// with a secondary difference, ð þ ŋ ı letters of their own, Cyrillic і ї й ґ є ў ђ ј љ њ ћ
// џ in their alphabetical places, Hebrew final forms as variants.
//
// core and alloc bring the default case mappings of all of Unicode (char::to_uppercase,
// str::to_lowercase with the final sigma); this module adds the language rules (Turkish,
// Azeri) and case folding.  Not done: Lithuanian dot rules, Greek upper case without accents.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

#[path = "i18n_unicode.rs"]
mod tables;
use tables::{COMPAT, DECOMP, MARKS, ZEROS};

use super::Locale;

/// collation strength: base letters only ("a" = "A" = "ä")
pub const COLL_PRIMARY: u32 = 1;
/// collation strength: base letters and accents ("a" = "A" < "ä"): sorting without case
pub const COLL_SECONDARY: u32 = 2;
/// collation strength: base letters, accents and case ("a" < "A" < "ä"; the default)
pub const COLL_TERTIARY: u32 = 3;
/// collation strength: as COLL_TERTIARY, then the code points (only equal texts compare equal)
pub const COLL_IDENTICAL: u32 = 4;

// primary weights (0: ignorable); Latin letters 32 apart, the others 4 apart, so that a
// language can put letters of its own after (or before) them
const P_SPACE: u32 = 0x0100_0000;
const P_PUNCT: u32 = 0x0200_0000;
const P_DIGIT: u32 = 0x0400_0000;
const P_NUMBER: u32 = P_DIGIT + 0x100; // numeric: the count of digits comes first
const P_LATIN: u32 = 0x0800_0000;
const P_OTHER: u32 = 0x1000_0000;

// secondary weights: the letter itself, the accents (MARK_ORDER, then by code point), strokes
const S_COMMON: u32 = 1;
const S_STROKE: u32 = 0x40;
const S_LIG: u32 = 0x41;
// the accents in the order of the UCA's secondary weights
const MARK_ORDER: [char; 16] = [
    '\u{301}', '\u{300}', '\u{306}', '\u{302}', '\u{30C}', '\u{30A}', '\u{308}', '\u{30B}', '\u{303}', '\u{307}', '\u{327}', '\u{328}',
    '\u{304}', '\u{323}', '\u{331}', '\u{313}',
];
const DIAER: u32 = mark_weight('\u{308}');

const fn mark_weight(m: char) -> u32 {
    let mut k = 0;
    while k < MARK_ORDER.len() {
        if MARK_ORDER[k] == m {
            return 2 + k as u32;
        }
        k += 1;
    }
    0x100 + m as u32
}

// a letter of a language rule: the primary of `base` moved by `step` (0: the letter itself),
// an optional secondary weight after it, a tertiary variant (Danish "aa", ß), an optional
// second letter (expansions: ß = ss, phone book ä = ae)
#[derive(Clone, Copy)]
struct Tail {
    base: char,
    step: i8,
    sec: u32,
    var: bool,
    then: char,
}

// a letter of its own `step` places after (before) base
const fn at(base: char, step: i8) -> Tail {
    Tail { base, step, sec: 0, var: false, then: '\0' }
}
// base with a secondary difference
const fn like(base: char, sec: u32) -> Tail {
    Tail { base, step: 0, sec, var: false, then: '\0' }
}
// a tertiary variant of base (final forms)
const fn var(base: char) -> Tail {
    Tail { base, step: 0, sec: 0, var: true, then: '\0' }
}
// two letters: a, the secondary weight, b
const fn exp(a: char, sec: u32, b: char, var: bool) -> Tail {
    Tail { base: a, step: 0, sec, var, then: b }
}

// the patterns are lower case and decomposed (NFD)
type Rules = &'static [(&'static str, Tail)];

// every language: letters without a decomposition
static ROOT: Rules = &[
    ("ß", exp('s', 0, 's', true)),
    ("æ", exp('a', 0, 'e', true)),
    ("œ", exp('o', 0, 'e', true)),
    ("ð", at('d', 1)),
    ("ə", at('e', 1)),
    ("ı", at('i', 1)),
    ("ŋ", at('n', 1)),
    ("þ", at('z', 1)),
    ("ø", like('o', S_STROKE)),
    ("ł", like('l', S_STROKE)),
    ("đ", like('d', S_STROKE)),
    ("ħ", like('h', S_STROKE)),
    ("ŧ", like('t', S_STROKE)),
    ("ƀ", like('b', S_STROKE)),
    ("ɨ", like('i', S_STROKE)),
    ("ƶ", like('z', S_STROKE)),
    ("ǥ", like('g', S_STROKE)),
    // Cyrillic letters of Ukrainian, Belarusian, Serbian, Macedonian in their places
    ("ґ", at('г', 1)),
    ("є", at('е', 1)),
    ("і", at('и', 1)),
    ("і\u{308}", at('и', 2)),
    ("и\u{306}", at('й', 0)),
    ("ђ", at('д', 1)),
    ("ѕ", at('з', 1)),
    ("ј", at('й', 1)),
    ("љ", at('л', 1)),
    ("њ", at('н', 1)),
    ("ћ", at('т', 1)),
    ("у\u{306}", at('у', 1)),
    ("џ", at('ч', 1)),
    // Hebrew final forms
    ("ך", var('כ')),
    ("ם", var('מ')),
    ("ן", var('נ')),
    ("ף", var('פ')),
    ("ץ", var('צ')),
];

static DE_PHONEBOOK: Rules = &[("a\u{308}", exp('a', DIAER, 'e', false)), ("o\u{308}", exp('o', DIAER, 'e', false)), ("u\u{308}", exp('u', DIAER, 'e', false))];

static SV: Rules = &[
    ("a\u{30A}", at('z', 1)),
    ("a\u{308}", at('z', 2)),
    ("æ", Tail { sec: S_LIG, ..at('z', 2) }),
    ("o\u{308}", at('z', 3)),
    ("ø", Tail { sec: S_STROKE, ..at('z', 3) }),
    ("u\u{308}", like('y', DIAER)),
];

static FI: Rules = &[
    ("a\u{30A}", at('z', 1)),
    ("a\u{308}", at('z', 2)),
    ("æ", Tail { sec: S_LIG, ..at('z', 2) }),
    ("o\u{308}", at('z', 3)),
    ("ø", Tail { sec: S_STROKE, ..at('z', 3) }),
    ("o\u{303}", Tail { sec: mark_weight('\u{303}'), ..at('z', 3) }),
    ("u\u{308}", like('y', DIAER)),
];

static DA: Rules = &[
    ("æ", at('z', 1)),
    ("a\u{308}", Tail { sec: DIAER, ..at('z', 1) }),
    ("ø", at('z', 2)),
    ("o\u{308}", Tail { sec: DIAER, ..at('z', 2) }),
    ("a\u{30A}", at('z', 3)),
    ("aa", Tail { var: true, ..at('z', 3) }),
    ("u\u{308}", like('y', DIAER)),
];

static ES: Rules = &[("n\u{303}", at('n', 1))];

static CS: Rules = &[("c\u{30C}", at('c', 1)), ("ch", at('h', 1)), ("r\u{30C}", at('r', 1)), ("s\u{30C}", at('s', 1)), ("z\u{30C}", at('z', 1))];

static SK: Rules = &[
    ("a\u{308}", at('a', 1)),
    ("c\u{30C}", at('c', 1)),
    ("ch", at('h', 1)),
    ("o\u{302}", at('o', 1)),
    ("r\u{30C}", at('r', 1)),
    ("s\u{30C}", at('s', 1)),
    ("z\u{30C}", at('z', 1)),
];

static PL: Rules = &[
    ("a\u{328}", at('a', 1)),
    ("c\u{301}", at('c', 1)),
    ("e\u{328}", at('e', 1)),
    ("ł", at('l', 1)),
    ("n\u{301}", at('n', 1)),
    ("o\u{301}", at('o', 1)),
    ("s\u{301}", at('s', 1)),
    ("z\u{301}", at('z', 1)),
    ("z\u{307}", at('z', 2)),
];

static TR: Rules = &[
    ("c\u{327}", at('c', 1)),
    ("g\u{306}", at('g', 1)),
    ("ı", at('i', -1)),
    ("o\u{308}", at('o', 1)),
    ("s\u{327}", at('s', 1)),
    ("u\u{308}", at('u', 1)),
];

// the language part of a tag ("de-CH" -> "de"; old codes as today's)
fn language(tag: &str) -> &str {
    let l = tag.split(['-', '_', '.', '@']).next().unwrap_or("");
    if l.eq_ignore_ascii_case("no") || l.eq_ignore_ascii_case("nn") { "nb" } else { l }
}

fn turkic(tag: &str) -> bool {
    let l = language(tag);
    l.eq_ignore_ascii_case("tr") || l.eq_ignore_ascii_case("az")
}

/// compares texts in the order of a language (see the module comment); copy it freely
#[derive(Clone, Copy)]
pub struct Collator {
    rules: Rules,
    german: bool,
    turkish: bool,
    phonebook: bool,
    numeric: bool,
    strength: u32,
}

impl Collator {
    /// the collator of a language (a BCP 47 tag or a POSIX name; unknown languages sort by the
    /// common rules); "-u-co-phonebk", "-u-kn" and "-u-ks-level1 / 2 / 3 / identic" set the
    /// options as in ICU, also "de-DE_phoneb" (Windows) and "@collation=phonebook" (POSIX)
    pub fn new(tag: &str) -> Collator {
        let l = language(tag).to_ascii_lowercase();
        let rules: Rules = match l.as_str() {
            "sv" => SV,
            "fi" => FI,
            "da" | "nb" => DA,
            "es" => ES,
            "cs" => CS,
            "sk" => SK,
            "pl" => PL,
            "tr" => TR,
            _ => &[],
        };
        let mut c = Collator { rules, german: l == "de", turkish: l == "tr" || l == "az", phonebook: false, numeric: false, strength: COLL_TERTIARY };
        let low = tag.to_ascii_lowercase();
        let parts: Vec<&str> = low.split(['-', '_']).collect();
        c.phonebook = low.contains("collation=phonebook") || parts.contains(&"phoneb");
        if let Some(u) = parts.iter().position(|p| *p == "u") {
            let ext = &parts[u + 1..];
            for (k, key) in ext.iter().enumerate() {
                let val = ext.get(k + 1).copied().filter(|v| v.len() > 2).unwrap_or("");
                match *key {
                    "co" => c.phonebook = val == "phonebk",
                    "kn" => c.numeric = val != "false",
                    "ks" => {
                        c.strength = match val {
                            "level1" => COLL_PRIMARY,
                            "level2" => COLL_SECONDARY,
                            "identic" => COLL_IDENTICAL,
                            _ => COLL_TERTIARY,
                        }
                    }
                    _ => {}
                }
            }
        }
        c
    }

    /// the same collator with another strength (COLL_PRIMARY .. COLL_IDENTICAL)
    pub fn strength(mut self, s: u32) -> Collator {
        self.strength = s.clamp(COLL_PRIMARY, COLL_IDENTICAL);
        self
    }

    /// numbers by their value: "Datei 2" before "Datei 10", "007" = "7" up to the tertiary level
    pub fn numeric(mut self, on: bool) -> Collator {
        self.numeric = on;
        self
    }

    /// German phone book order (ä = ae, ö = oe, ü = ue); no effect in other languages
    pub fn phonebook(mut self, on: bool) -> Collator {
        self.phonebook = on;
        self
    }

    /// compare two texts
    pub fn compare(&self, a: &str, b: &str) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        self.key(a).cmp(&self.key(b))
    }

    /// the sort key of a text: keys compare as their texts do (sort_by_cached_key, indexes)
    pub fn key(&self, s: &str) -> Vec<u32> {
        let ces = self.elements(s);
        let mut k: Vec<u32> = ces.iter().filter(|e| e.p != 0).map(|e| e.p).collect();
        if self.strength >= COLL_SECONDARY {
            k.push(0);
            k.extend(ces.iter().filter(|e| e.s != 0).map(|e| e.s));
        }
        if self.strength >= COLL_TERTIARY {
            k.push(0);
            k.extend(ces.iter().filter(|e| e.p != 0).map(|e| e.t));
        }
        if self.strength >= COLL_IDENTICAL {
            k.push(0);
            k.extend(s.chars().map(|c| c as u32 + 1));
        }
        k
    }

    /// sort items by a text of each (stable; each key is computed once)
    pub fn sort<T>(&self, items: &mut [T], text: impl Fn(&T) -> &str) {
        items.sort_by_cached_key(|x| self.key(text(x)));
    }

    fn rules(&self) -> Rules {
        if self.german && self.phonebook { DE_PHONEBOOK } else { self.rules }
    }

    // the text lower case and decomposed, each letter with its case and variant flag
    fn chars(&self, s: &str) -> Vec<Ch> {
        let mut out: Vec<Ch> = Vec::with_capacity(s.len());
        for c in s.chars() {
            let (c, var) = match c as u32 {
                0xFF01..=0xFF5E => (char::from_u32(c as u32 - 0xFEE0).unwrap_or(c), true), // fullwidth ASCII
                0x30A1..=0x30F6 => (char::from_u32(c as u32 - 0x60).unwrap_or(c), true),   // katakana as hiragana
                _ => (c, false),
            };
            if let Some(letters) = compat(c) {
                for x in letters.chars() {
                    self.push_char(&mut out, x, true);
                }
            } else {
                self.push_char(&mut out, c, var);
            }
        }
        out
    }

    fn push_char(&self, out: &mut Vec<Ch>, c: char, var: bool) {
        if self.turkish && (c == 'I' || c == 'İ') {
            out.push(Ch { c: if c == 'I' { 'ı' } else { 'i' }, upper: true, var });
            return;
        }
        let upper = c.to_lowercase().next() != Some(c);
        for l in c.to_lowercase() {
            // letters that fold to another one (ς -> σ, ſ -> s, µ -> μ): variants of it
            let (l, v) = match single(l.to_uppercase()).and_then(|u| single(u.to_lowercase())) {
                Some(f) if f != l && l != 'ı' => (f, true),
                _ => (l, false),
            };
            let mut first = true;
            decompose(l, &mut |d| {
                out.push(if first { Ch { c: d, upper, var: var || v } } else { Ch { c: d, upper: false, var: false } });
                first = false;
            });
        }
    }

    // the collation elements of a text
    fn elements(&self, s: &str) -> Vec<Ce> {
        let chs = self.chars(s);
        let mut ces: Vec<Ce> = Vec::with_capacity(chs.len() + 4);
        let mut i = 0;
        while i < chs.len() {
            if let Some((n, t)) = rule_at(self.rules(), &chs[i..]).or_else(|| rule_at(ROOT, &chs[i..])) {
                let tert = tertiary(chs[i].upper, chs[i].var || t.var);
                ces.push(Ce { p: primary_of(t.base).wrapping_add_signed(t.step as i32), s: S_COMMON, t: tert });
                if t.sec != 0 {
                    ces.push(Ce { p: 0, s: t.sec, t: 0 });
                }
                if t.then != '\0' {
                    ces.push(Ce { p: primary_of(t.then), s: S_COMMON, t: tert });
                }
                i += n;
                continue;
            }
            let ch = chs[i];
            i += 1;
            if is_mark(ch.c) {
                ces.push(Ce { p: 0, s: mark_weight(ch.c), t: 0 });
            } else if ch.c.is_whitespace() {
                ces.push(Ce { p: P_SPACE + ch.c as u32, s: S_COMMON, t: tertiary(false, ch.var) });
            } else if ignorable(ch.c) {
                // soft hyphen, joiners, direction marks, controls
            } else if let Some(d) = digit(ch.c) {
                if !self.numeric {
                    ces.push(Ce { p: P_DIGIT + d, s: S_COMMON, t: tertiary(false, ch.var || !ch.c.is_ascii_digit()) });
                    continue;
                }
                // a whole number: the count of its digits, then the digits (leading zeros count
                // only at the tertiary level)
                let mut digits = alloc::vec![(d, ch)];
                while let Some(d) = chs.get(i).and_then(|c| digit(c.c).map(|d| (d, *c))) {
                    digits.push(d);
                    i += 1;
                }
                let zeros = digits.iter().take(digits.len() - 1).take_while(|d| d.0 == 0).count();
                let sig = &digits[zeros..];
                ces.push(Ce { p: P_NUMBER + sig.len() as u32, s: S_COMMON, t: 1 + zeros as u32 });
                for (d, c) in sig {
                    ces.push(Ce { p: P_DIGIT + d, s: S_COMMON, t: tertiary(false, c.var || !c.c.is_ascii_digit()) });
                }
            } else {
                let p = if ch.c.is_alphabetic() { primary_of(ch.c) } else { P_PUNCT + ch.c as u32 };
                ces.push(Ce { p, s: S_COMMON, t: tertiary(ch.upper, ch.var) });
            }
        }
        ces
    }
}

// one character of a text after case mapping and decomposition
#[derive(Clone, Copy)]
struct Ch {
    c: char,
    upper: bool,
    var: bool,
}

// a collation element: primary (0: ignorable), secondary, tertiary weight
struct Ce {
    p: u32,
    s: u32,
    t: u32,
}

// lower case 1, its variants 2, upper case 3, upper case variants 4
fn tertiary(upper: bool, var: bool) -> u32 {
    1 + var as u32 + 2 * upper as u32
}

// the primary weight of a letter
fn primary_of(c: char) -> u32 {
    match c {
        'a'..='z' => P_LATIN + (c as u32 - 'a' as u32 + 1) * 32,
        _ => P_OTHER + c as u32 * 4,
    }
}

// the longest pattern of the rules at the start of chs -> (its length, the letter)
fn rule_at(rules: Rules, chs: &[Ch]) -> Option<(usize, Tail)> {
    let mut best: Option<(usize, Tail)> = None;
    for (pat, t) in rules {
        let mut n = 0;
        let ok = pat.chars().all(|p| {
            let hit = chs.get(n).is_some_and(|c| c.c == p);
            n += 1;
            hit
        });
        if ok && best.is_none_or(|b| n > b.0) {
            best = Some((n, *t));
        }
    }
    best
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let c = it.next()?;
    if it.next().is_none() { Some(c) } else { None }
}

// the canonical decomposition of c, recursively (DECOMP: one step each)
fn decompose(c: char, out: &mut impl FnMut(char)) {
    let cp = c as u32;
    if cp >= 0xC0 && cp <= 0xFFFF {
        if let Ok(k) = DECOMP.binary_search_by_key(&(cp as u16), |e| e.0) {
            let (_, a, b) = DECOMP[k];
            decompose(char::from_u32(a as u32).unwrap_or(c), out);
            if b != 0 {
                decompose(char::from_u32(b as u32).unwrap_or(c), out);
            }
            return;
        }
    }
    out(c);
}

// the letters of a compatibility character (ĳ, ﬁ, ª, ²)
fn compat(c: char) -> Option<&'static str> {
    let cp = c as u32;
    if !(0xAA..=0xFFFF).contains(&cp) {
        return None;
    }
    COMPAT.binary_search_by_key(&(cp as u16), |e| e.0).ok().map(|k| COMPAT[k].1)
}

fn is_mark(c: char) -> bool {
    let cp = c as u32;
    if !(0x300..=0xFFFF).contains(&cp) {
        return false;
    }
    let k = MARKS.partition_point(|r| (r.1 as u32) < cp);
    k < MARKS.len() && MARKS[k].0 as u32 <= cp
}

fn ignorable(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

// the value of a decimal digit of any script
fn digit(c: char) -> Option<u32> {
    let cp = c as u32;
    if c.is_ascii_digit() {
        return Some(cp - '0' as u32);
    }
    if cp < 0x660 {
        return None;
    }
    let k = ZEROS.partition_point(|&z| z <= cp);
    (k > 0 && cp - ZEROS[k - 1] <= 9).then(|| cp - ZEROS[k - 1])
}

/// the user's collator (the language of locale_tag(), tertiary strength)
pub fn collator() -> Collator {
    Collator::new(&super::locale_tag())
}

/// compare two texts in the order of the user's language (see the module comment)
pub fn collate(a: &str, b: &str) -> Ordering {
    collator().compare(a, b)
}

impl Locale {
    /// compare two texts in the order of the region's language
    pub fn collate(&self, a: &str, b: &str) -> Ordering {
        self.collator().compare(a, b)
    }
    /// the collator of the region's language
    pub fn collator(&self) -> Collator {
        Collator::new(self.tag())
    }
}

// ---------------------------------------------------------------- case

/// upper case in the user's language: "straße" -> "STRASSE", Turkish "i" -> "İ"
pub fn to_upper(s: &str) -> String {
    to_upper_in(&super::locale_tag(), s)
}

/// lower case in the user's language: "ΟΔΟΣ" -> "οδος", Turkish "I" -> "ı"
pub fn to_lower(s: &str) -> String {
    to_lower_in(&super::locale_tag(), s)
}

/// case folding in the user's language (for comparisons, not for display): "Straße" ->
/// "strasse", "ΣΟΦΟΣ" and "σοφος" -> "σοφοσ"
pub fn fold(s: &str) -> String {
    fold_in(&super::locale_tag(), s)
}

/// equal except for case in the user's language ("STRASSE" = "Straße"); composed and
/// decomposed accents are equal too
pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    a == b || nfd(&fold(a)) == nfd(&fold(b))
}

/// upper case in a language (BCP 47 tag)
pub fn to_upper_in(lang: &str, s: &str) -> String {
    if turkic(lang) && s.contains('i') { s.replace('i', "İ").to_uppercase() } else { s.to_uppercase() }
}

/// lower case in a language (BCP 47 tag)
pub fn to_lower_in(lang: &str, s: &str) -> String {
    if turkic(lang) && s.contains(['I', 'İ']) {
        s.replace("I\u{307}", "i").replace('I', "ı").replace('İ', "i").to_lowercase()
    } else {
        s.to_lowercase()
    }
}

/// case folding in a language (BCP 47 tag): the full folding of Unicode's CaseFolding.txt,
/// Turkish and Azeri with their I / ı and İ / i
pub fn fold_in(lang: &str, s: &str) -> String {
    let tr = turkic(lang);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'I' if tr => out.push('ı'),
            'İ' if tr => out.push('i'),
            'ı' => out.push(c),
            // Cherokee folds to its capitals
            '\u{13A0}'..='\u{13F5}' | '\u{13F8}'..='\u{13FD}' | '\u{AB70}'..='\u{ABBF}' => out.extend(c.to_uppercase()),
            _ => {
                for u in c.to_uppercase() {
                    for l in u.to_lowercase() {
                        if l == 'ß' { out.push_str("ss") } else { out.push(l) }
                    }
                }
            }
        }
    }
    out
}

// the canonical decomposition of a text (without reordering the marks)
fn nfd(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        decompose(c, &mut |d| out.push(d));
    }
    out
}
