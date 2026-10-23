// check -- is a word correct: case variants, affixes, compounds, break points
//
//   look(word, opt) -> Miss / Hit / Forbidden    one word as written (no case variants): a
//                      stem, an affixed form (one prefix and one suffix, or two suffixes) or a
//                      compound
//   check_cased(word)  the case rules: a Title word may be a lower-case one ("Haus" at a
//                      sentence start of "haus"), an UPPER word any of the three (Title first;
//                      the hidden Title entries of mixed case words count: "KDF" for "KdF");
//                      KEEPCASE words only as written; CHECKSHARPS: "SS" of an UPPER word may
//                      be "ß"; a forbidden form ends the search (no other case, no break)
//
// Affixes: a prefix and a suffix (both cross product; the prefix's continuation class may
// enable the suffix), or two suffixes (the inner one's continuation class names the outer
// one).  NEEDAFFIX roots need an affix, an affix with NEEDAFFIX another one; CIRCUMFIX: a
// prefix and the (inner) suffix both have it or neither (a prefix alone may, as in Hunspell);
// ONLYINCOMPOUND roots or affixes only inside compounds; a FORBIDDENWORD root rejects the word
// and its affixed forms (also as a compound).  KEEPCASE and FORCEUCASE come from the root.
//
// Compounds: parts of at least COMPOUNDMIN characters, each a stem or affixed form with
// COMPOUNDFLAG or the flag of its place (COMPOUNDBEGIN / MIDDLE / END) -- from the root, or
// the continuation class of a prefix standing alone, else of the (inner) suffix (Hunspell's
// rules).  Prefixes are allowed on the first and middle parts, suffixes on the last one;
// elsewhere the affix needs COMPOUNDPERMITFLAG; never affixes or roots with
// COMPOUNDFORBIDFLAG.  CHECKCOMPOUNDCASE (no upper case letter at a boundary),
// CHECKCOMPOUNDDUP (no part twice in a row), CHECKCOMPOUNDTRIPLE (no three equal letters at a
// boundary; SIMPLIFIEDTRIPLE: "Schiffahrt" for "Schiff" + "fahrt"), CHECKCOMPOUNDREP (no
// compound that a REP rule without ^ / $ turns into an entry), CHECKCOMPOUNDPATTERN (also with
// the flags of the parts' roots), COMPOUNDWORDMAX; no compound the list has as two words ("a
// cappella"); a forbidden entry ends a part only with a compound flag of its place; FORCEUCASE
// on the last part: the compound must not be all lower case.  COMPOUNDRULE: parts that are
// dictionary words (the last one may have affixes) whose flags match a rule ("n*1t": number
// parts, then one with flag 1, then one with t).  Two parts are tried first; a compound of
// three or more counts only when no simple correction of it is a word (suggest.rs: one edit,
// or two words; Hunspell 1.7 -- "Fahrradweg" is "Fahrrad weg"; not for words starting with a
// digit).  In suggestion mode compounds have two parts only.  The search is bounded
// (Ctx::steps) and remembers its parts.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::aff::{bit, has, Affix, Flag, Group, RuleEl, ONLYUPCASE};
use crate::dic::Homonyms;
use crate::Dictionary;

/// how a word is capitalised
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cap {
    /// no upper-case letter
    No,
    /// the first letter upper case, the others lower case
    Init,
    /// every letter upper case
    All,
    /// mixed, starting upper case ("McDonald")
    Huh,
    /// mixed, starting lower case ("iPhone")
    HuhInit,
}

/// the capitalisation of w (CHECKSHARPS: 'ß' counts as neither case)
pub(crate) fn captype(w: &str, sharps: bool) -> Cap {
    let (mut up, mut low) = (0usize, 0usize);
    let mut first_up = false;
    let mut first = true;
    for c in w.chars() {
        let neutral = sharps && c == 'ß';
        if c.is_uppercase() && !neutral {
            up += 1;
            if first {
                first_up = true;
            }
        } else if c.is_lowercase() && !neutral {
            low += 1;
        }
        if c.is_alphabetic() {
            first = false;
        }
    }
    if up == 0 {
        Cap::No
    } else if up == 1 && first_up {
        Cap::Init
    } else if low == 0 {
        Cap::All
    } else if first_up {
        Cap::Huh
    } else {
        Cap::HuhInit
    }
}

pub(crate) fn lower(w: &str) -> String {
    w.chars().flat_map(|c| c.to_lowercase()).collect()
}

pub(crate) fn upper(w: &str) -> String {
    w.chars().flat_map(|c| c.to_uppercase()).collect()
}

/// the first letter upper case, the rest as it is
pub(crate) fn title(w: &str) -> String {
    let mut it = w.chars();
    match it.next() {
        Some(c) => c.to_uppercase().chain(it).collect(),
        None => String::new(),
    }
}

/// what a found word carries
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Hit {
    pub(crate) keepcase: bool,
    pub(crate) forceucase: bool,
    /// a compound part: its root's flag set (Words::set; only when a CHECKCOMPOUNDPATTERN looks
    /// at the roots) and whether it has affixes
    pub(crate) set: u32,
    pub(crate) affixed: bool,
}

/// the result of looking one word up
#[derive(Clone, Copy, Debug)]
pub(crate) enum Look {
    Miss,
    Hit(Hit),
    Forbidden,
}

/// options of a lookup
#[derive(Clone, Copy)]
pub(crate) struct Opt {
    /// compounds are tried
    pub(crate) compounds: bool,
    /// suggestion mode: NOSUGGEST words do not count, compounds of two parts only
    pub(crate) sug: bool,
    /// ONLYUPCASE entries count (every lookup but the one of a Title word as typed)
    pub(crate) upcase: bool,
}

pub(crate) const CHECK: Opt = Opt { compounds: true, sug: false, upcase: true };

/// the work a check may do (compound search steps)
pub(crate) struct Ctx {
    pub(crate) steps: u32,
    /// BREAK pieces still allowed to be looked up (the splits of a long hyphenated word grow
    /// exponentially)
    pub(crate) breaks: u32,
    /// the compound parts seen by earlier lookups (suggestions: the candidates share most
    /// of their parts)
    pub(crate) parts: Option<Parts>,
}

pub(crate) const BREAK_WORK: u32 = 8192;

impl Ctx {
    pub(crate) fn new(steps: u32) -> Ctx {
        Ctx { steps, breaks: BREAK_WORK, parts: None }
    }
}

/// compound parts and their results: an open-addressing table of 64-bit hashes of (part,
/// place) -- no texts (a collision is as likely as a cosmic ray); full: no more entries
pub(crate) struct Parts {
    keys: Vec<u64>,
    vals: Vec<Option<Hit>>,
    n: usize,
}

const PARTS_SLOTS: usize = 1 << 15;

impl Parts {
    pub(crate) fn new() -> Parts {
        Parts { keys: alloc::vec![0; PARTS_SLOTS], vals: alloc::vec![None; PARTS_SLOTS], n: 0 }
    }

    fn key(p: &str, pos: Pos) -> u64 {
        let h = p.bytes().fold(0xCBF2_9CE4_8422_2325u64 ^ pos as u64, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3));
        (h ^ (h >> 31)) | 1
    }

    fn get(&self, p: &str, pos: Pos) -> Option<Option<Hit>> {
        let k = Parts::key(p, pos);
        let mut i = k as usize & (PARTS_SLOTS - 1);
        loop {
            match self.keys[i] {
                0 => return None,
                x if x == k => return Some(self.vals[i]),
                _ => i = (i + 1) & (PARTS_SLOTS - 1),
            }
        }
    }

    fn insert(&mut self, p: &str, pos: Pos, h: Option<Hit>) {
        if self.n * 4 >= PARTS_SLOTS * 3 {
            return;
        }
        let k = Parts::key(p, pos);
        let mut i = k as usize & (PARTS_SLOTS - 1);
        while self.keys[i] != 0 && self.keys[i] != k {
            i = (i + 1) & (PARTS_SLOTS - 1);
        }
        if self.keys[i] == 0 {
            self.n += 1;
        }
        self.keys[i] = k;
        self.vals[i] = h;
    }
}

/// a derivation of a word from a root: the root's flags and the affixes
pub(crate) struct Deriv<'a> {
    pub(crate) flags: &'a [Flag],
    pub(crate) pfx: Option<&'a Affix>,
    pub(crate) sfx: Option<&'a Affix>,  // the (inner) suffix
    pub(crate) sfx2: Option<&'a Affix>, // the outer suffix of two
}

impl Deriv<'_> {
    /// an affix carries f in its continuation class
    fn cont(&self, f: Flag) -> bool {
        self.pfx.is_some_and(|a| a.has(f)) || self.sfx.is_some_and(|a| a.has(f)) || self.sfx2.is_some_and(|a| a.has(f))
    }

    /// the root or an affix carries f
    fn any(&self, f: Flag) -> bool {
        has(self.flags, f) || self.cont(f)
    }

    /// f for the compound rules: the root, or the continuation class of a prefix standing
    /// alone, else of the (inner) suffix
    fn cflag(&self, f: Flag) -> bool {
        has(self.flags, f)
            || match self.sfx {
                Some(s) => s.has(f),
                None => self.pfx.is_some_and(|p| p.has(f)),
            }
    }

    fn affixes(&self) -> usize {
        self.pfx.is_some() as usize + self.sfx.is_some() as usize + self.sfx2.is_some() as usize
    }
}

/// a part's place in a compound
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pos {
    Begin,
    Middle,
    End,
}

impl Dictionary {
    // ------------------------------------------------------------------ affixes

    /// f(group, root, its entries when known) for every group of suffix rules whose text ends
    /// w (the rules' conditions are not tested yet); hw: the entries of w; true when f
    /// accepted one
    fn each_sfx<'a>(&'a self, w: &str, hw: Option<Homonyms<'a>>, f: &mut dyn FnMut(&'a Group, &str, Option<Homonyms<'a>>) -> bool) -> bool {
        let a = &self.aff;
        let mut cut = w.len();
        let mut chars = w.char_indices().rev();
        let mut root = String::new();
        // (texts of one character or more only when w ends with a last byte of one)
        let kmax = if w.as_bytes().last().is_some_and(|&b| bit(&a.sfx_last, b)) { a.sfx_max } else { 0 };
        for k in 0..=kmax {
            if k > 0 {
                match chars.next() {
                    Some((i, _)) => cut = i,
                    None => break,
                }
            }
            let Some(groups) = a.sfx_by_add.get(&w[cut..]) else { continue };
            let rest = &w[..cut];
            if rest.is_empty() && !a.fullstrip {
                continue;
            }
            for g in groups {
                root.clear();
                root.push_str(rest);
                root.push_str(&g.strip);
                let known = if k == 0 && g.strip.is_empty() { hw } else { None };
                if !root.is_empty() && f(g, &root, known) {
                    return true;
                }
            }
        }
        false
    }

    /// f(rule, root, its entries) for every prefix rule whose text starts w and whose
    /// condition holds for the root it leaves; hw: the entries of w; true when f accepted one
    fn each_pfx<'a>(&'a self, w: &str, hw: Option<Homonyms<'a>>, f: &mut dyn FnMut(&'a Affix, &str, Homonyms<'a>) -> bool) -> bool {
        let a = &self.aff;
        let mut cut = 0;
        let mut ends = w.char_indices().map(|(i, c)| i + c.len_utf8()); // after the k-th character
        let mut root = String::new();
        let kmax = if w.as_bytes().first().is_some_and(|&b| bit(&a.pfx_first, b)) { a.pfx_max } else { 0 };
        for k in 0..=kmax {
            if k > 0 {
                match ends.next() {
                    Some(e) => cut = e,
                    None => break,
                }
            }
            let Some(groups) = a.pfx_by_add.get(&w[..cut]) else { continue };
            let rest = &w[cut..];
            if rest.is_empty() && !a.fullstrip {
                continue;
            }
            for g in groups {
                root.clear();
                root.push_str(&g.strip);
                root.push_str(rest);
                if root.is_empty() {
                    continue;
                }
                let mut homs = if k == 0 && g.strip.is_empty() { hw } else { None };
                for &i in &g.rules {
                    let r = &a.pfx[i as usize];
                    if !r.cond_ok(&root, false) {
                        continue;
                    }
                    let h = *homs.get_or_insert_with(|| self.words.get(&root));
                    if f(r, &root, h) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// f(derivation) for every way to derive w from a root with affixes (hw: the entries of w,
    /// the caller looked them up); true when f accepted one.  One pass over the suffixes: a
    /// suffix alone, as the outer one of two, or with a prefix (cross products, cross(): once
    /// per root); then the prefixes alone.
    pub(crate) fn analyze<'a>(&'a self, w: &str, hw: Homonyms<'a>, f: &mut dyn FnMut(&Deriv) -> bool) -> bool {
        let a = &self.aff;
        let found = self.each_sfx(w, Some(hw), &mut |g, root, known| {
            let (mut homs, mut cross) = (known, false);
            for &i in &g.rules {
                let s = &a.sfx[i as usize];
                if !s.cond_ok(root, true) {
                    continue;
                }
                for flags in *homs.get_or_insert_with(|| self.words.get(root)) {
                    if has(flags, s.flag) && f(&Deriv { flags, pfx: None, sfx: Some(s), sfx2: None }) {
                        return true;
                    }
                }
                // s as the outer of two suffixes: an inner one whose continuation names s
                if has(&a.sfx_outer, s.flag) && self.inner_sfx(root, s, &mut *f) {
                    return true;
                }
                cross |= s.cross;
            }
            cross && !a.pfx.is_empty() && self.cross(w, root, g, homs, &mut *f)
        });
        found
            || self.each_pfx(w, Some(hw), &mut |p, _, homs| {
                for flags in homs {
                    if has(flags, p.flag) && f(&Deriv { flags, pfx: Some(p), sfx: None, sfx2: None }) {
                        return true;
                    }
                }
                false
            })
    }

    // the inner suffix of two: `root` is w without the outer suffix s
    fn inner_sfx(&self, root: &str, s: &Affix, f: &mut dyn FnMut(&Deriv) -> bool) -> bool {
        let a = &self.aff;
        self.each_sfx(root, None, &mut |g1, root1, _| {
            g1.rules.iter().map(|&i| &a.sfx[i as usize]).any(|s1| {
                s1.has(s.flag) && s1.cond_ok(root1, true) && self.words.get(root1).any(|flags| has(flags, s1.flag) && f(&Deriv { flags, pfx: None, sfx: Some(s1), sfx2: Some(s) }))
            })
        })
    }

    // the cross products of the suffix rules of group g with a prefix: `root` is w without
    // their text (the strip added; hr: its entries when known); the prefix's text starts w
    // before the suffix's part, its condition holds for w without it, the suffix's for the
    // root without both
    fn cross<'a>(&'a self, w: &str, root: &str, g: &Group, hr: Option<Homonyms<'a>>, f: &mut dyn FnMut(&Deriv) -> bool) -> bool {
        let a = &self.aff;
        let keep = root.len() - g.strip.len(); // w[..keep] == root[..keep]
        let mut cut = 0;
        let mut ends = root.char_indices().map(|(i, c)| i + c.len_utf8());
        let mut base = String::new();
        let kmax = if root.as_bytes().first().is_some_and(|&b| bit(&a.pfx_first, b)) { a.pfx_max } else { 0 };
        for k in 0..=kmax {
            if k > 0 {
                match ends.next() {
                    Some(e) => cut = e,
                    None => break,
                }
            }
            if cut > keep {
                break;
            }
            let Some(pgroups) = a.pfx_by_add.get(&root[..cut]) else { continue };
            for pg in pgroups {
                if pg.strip.is_empty() && cut == keep && !a.fullstrip {
                    continue;
                }
                base.clear();
                base.push_str(&pg.strip);
                base.push_str(&root[cut..]);
                if base.is_empty() {
                    continue;
                }
                let mut homs = if k == 0 && pg.strip.is_empty() { hr } else { None };
                for &j in &pg.rules {
                    let p = &a.pfx[j as usize];
                    if !p.cross || !p.cond_ok2(&pg.strip, &w[cut..]) {
                        continue;
                    }
                    let h = *homs.get_or_insert_with(|| self.words.get(&base));
                    for &i in &g.rules {
                        let s = &a.sfx[i as usize];
                        if !s.cross || !s.cond_ok(&base, true) {
                            continue;
                        }
                        for flags in h {
                            if (has(flags, s.flag) || p.has(s.flag)) && (has(flags, p.flag) || s.has(p.flag)) && f(&Deriv { flags, pfx: Some(p), sfx: Some(s), sfx2: None }) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    // NEEDAFFIX on affixes and CIRCUMFIX: the affixes of d fit together
    fn affixes_ok(&self, d: &Deriv) -> bool {
        let a = &self.aff;
        if a.needaffix != 0 {
            let needy = d.pfx.is_some_and(|x| x.has(a.needaffix)) as usize
                + d.sfx.is_some_and(|x| x.has(a.needaffix)) as usize
                + d.sfx2.is_some_and(|x| x.has(a.needaffix)) as usize;
            if needy > 0 && d.affixes() < 2 {
                return false;
            }
        }
        if a.circumfix != 0 {
            if let Some(s) = d.sfx {
                if d.pfx.is_some_and(|x| x.has(a.circumfix)) != s.has(a.circumfix) {
                    return false;
                }
            }
        }
        true
    }

    // an affixed form standing alone: Some(true) a word, Some(false) a forbidden one, None no
    // form that stands alone
    fn alone(&self, d: &Deriv, opt: Opt) -> Option<bool> {
        let a = &self.aff;
        if d.any(a.onlyincompound) || !self.affixes_ok(d) {
            return None;
        }
        if has(d.flags, a.forbidden) {
            return Some(false);
        }
        if d.cont(a.forbidden) || (!opt.upcase && has(d.flags, ONLYUPCASE)) || (opt.sug && d.any(a.nosuggest)) {
            return None;
        }
        Some(true)
    }

    // what a part's root carries (FORCEUCASE matters for the last part of a compound only)
    fn hit_of(&self, flags: &[Flag], affixed: bool) -> Hit {
        let a = &self.aff;
        let set = if a.cpd_pat_roots { self.words.set_index(flags) } else { 0 };
        Hit { keepcase: has(flags, a.keepcase), forceucase: has(flags, a.forceucase), set, affixed }
    }

    // ------------------------------------------------------------------ one word

    /// w as written: a stem, an affixed form or a compound
    pub(crate) fn look(&self, w: &str, opt: Opt, cx: &mut Ctx) -> Look {
        let a = &self.aff;
        let mut found: Option<Hit> = None;
        let hw = self.words.get(w);
        for flags in hw {
            if has(flags, a.forbidden) {
                return Look::Forbidden;
            }
            if has(flags, a.needaffix) || has(flags, a.onlyincompound) || (opt.sug && has(flags, a.nosuggest)) || (!opt.upcase && has(flags, ONLYUPCASE)) {
                continue;
            }
            let h = Hit { keepcase: has(flags, a.keepcase), ..Hit::default() };
            if found.is_none_or(|f| f.keepcase && !h.keepcase) {
                found = Some(h);
            }
        }
        if let Some(h) = found {
            return Look::Hit(h);
        }
        let (mut hit, mut forbidden) = (None, false);
        self.analyze(w, hw, &mut |d| match self.alone(d, opt) {
            Some(true) => {
                hit = Some(Hit { keepcase: has(d.flags, a.keepcase), ..Hit::default() });
                true
            }
            Some(false) => {
                forbidden = true;
                false
            }
            None => false,
        });
        if let Some(h) = hit {
            return Look::Hit(h);
        }
        if forbidden {
            return Look::Forbidden;
        }
        if opt.compounds && a.compounds() {
            if let Some(h) = self.compound(w, opt, cx) {
                return Look::Hit(h);
            }
        }
        Look::Miss
    }

    // a case variant of the word typed: KEEPCASE words do not count (CHECKSHARPS: they may
    // in the lower case variant of a Title word with 'ß', as in Hunspell)
    fn variant(&self, w: &str, opt: Opt, sharp_ok: bool, cx: &mut Ctx) -> Look {
        match self.look(w, opt, cx) {
            Look::Hit(h) if h.keepcase && !(sharp_ok && self.aff.sharps && w.contains('ß')) => Look::Miss,
            r => r,
        }
    }

    /// w with the case rules (see the module header): Hit, Miss or Forbidden (a forbidden
    /// form ends the search)
    pub(crate) fn check_cased(&self, w: &str, opt: Opt, cx: &mut Ctx) -> Look {
        let cap = captype(w, self.aff.sharps);
        let typed = Opt { upcase: opt.upcase && cap != Cap::Init, ..opt };
        match self.look(w, typed, cx) {
            Look::Hit(h) if !(h.forceucase && cap == Cap::No) => return Look::Hit(h),
            Look::Forbidden => return Look::Forbidden,
            _ => {}
        }
        match cap {
            Cap::Init => self.variant(&lower(w), opt, true, cx),
            Cap::All => {
                let l = lower(w);
                if self.aff.sharps && l.contains("ss") && (self.sharps(&l, 0, 0, false, opt, cx) || self.sharps(&title(&l), 0, 0, false, opt, cx)) {
                    return Look::Hit(Hit::default());
                }
                match self.variant(&title(&l), opt, false, cx) {
                    Look::Miss => self.variant(&l, opt, false, cx),
                    r => r,
                }
            }
            _ => Look::Miss,
        }
    }

    // CHECKSHARPS: every "ss" from byte `from` on may be "ß" (at least one; five at most);
    // KEEPCASE words count here, as in Hunspell
    fn sharps(&self, s: &str, from: usize, n: u32, rep: bool, opt: Opt, cx: &mut Ctx) -> bool {
        match s[from..].find("ss").map(|i| i + from) {
            Some(i) if n < 5 => {
                let mut v = String::with_capacity(s.len());
                v.push_str(&s[..i]);
                v.push('ß');
                v.push_str(&s[i + 2..]);
                self.sharps(&v, i + 'ß'.len_utf8(), n + 1, true, opt, cx) || self.sharps(s, i + 2, n + 1, rep, opt, cx)
            }
            _ => rep && matches!(self.look(s, opt, cx), Look::Hit(_)),
        }
    }

    /// BREAK: the word split at a break pattern, every piece correct
    pub(crate) fn check_break(&self, w: &str, depth: u32, opt: Opt, cx: &mut Ctx) -> bool {
        if depth > 8 || w.is_empty() {
            return false;
        }
        let piece = |s: &str, cx: &mut Ctx| {
            if s.is_empty() || crate::is_number(s) {
                return true;
            }
            if cx.breaks == 0 {
                return false;
            }
            cx.breaks -= 1;
            matches!(self.check_cased(s, opt, cx), Look::Hit(_)) || self.check_break(s, depth + 1, opt, cx)
        };
        for pat in &self.aff.breaks {
            if let Some(p) = pat.strip_prefix('^') {
                if !p.is_empty() && w.len() > p.len() && w.starts_with(p) && piece(&w[p.len()..], cx) {
                    return true;
                }
            } else if let Some(p) = pat.strip_suffix('$') {
                if !p.is_empty() && w.len() > p.len() && w.ends_with(p) && piece(&w[..w.len() - p.len()], cx) {
                    return true;
                }
            } else if !pat.is_empty() {
                let mut from = 0;
                while let Some(i) = w[from..].find(pat.as_str()).map(|i| i + from) {
                    let (l, r) = (&w[..i], &w[i + pat.len()..]);
                    if !l.is_empty() && !r.is_empty() && piece(l, cx) && piece(r, cx) {
                        return true;
                    }
                    from = i + pat.len();
                }
            }
        }
        false
    }

    // ------------------------------------------------------------------ compounds

    /// w as a compound (see the module header: two parts first, three or more only when no
    /// simple correction is a word, two parts only in suggestion mode)
    fn compound(&self, w: &str, opt: Opt, cx: &mut Ctx) -> Option<Hit> {
        let a = &self.aff;
        let mut b: Vec<usize> = w.char_indices().map(|(i, _)| i).collect();
        b.push(w.len());
        if b.len() - 1 < a.cpd_min * 2 {
            return None;
        }
        let rules = !a.cpd_rules.is_empty();
        let mut memo = BTreeMap::new();
        let mut seq = Vec::new();
        if a.cpd_flags() {
            if let Some(h) = self.cpd_rec(w, &b, 0, 0, "", None, None, 2, opt, cx, &mut memo) {
                return (!self.word_pair(w)).then_some(h);
            }
        }
        if rules && self.rule_rec(w, &b, 0, "", 2, &mut seq, cx) {
            return (!self.word_pair(w)).then(Hit::default);
        }
        if opt.sug {
            return None;
        }
        let h = if a.cpd_flags() { self.cpd_rec(w, &b, 0, 0, "", None, None, usize::MAX, opt, cx, &mut memo) } else { None };
        let h = h.or_else(|| (rules && self.rule_rec(w, &b, 0, "", usize::MAX, &mut seq, cx)).then(Hit::default))?;
        if self.word_pair(w) || (!w.starts_with(|c: char| c.is_ascii_digit()) && self.simple_sug(w)) {
            return None;
        }
        Some(h)
    }

    // the parts of w from character s on (parts before: `parts`, the last of them `prev` with
    // the result `left`, the first one's KEEPCASE `keep`), lim parts at most in all
    #[allow(clippy::too_many_arguments)]
    fn cpd_rec(&self, w: &str, b: &[usize], s: usize, parts: usize, prev: &str, left: Option<Hit>, keep: Option<bool>, lim: usize, opt: Opt, cx: &mut Ctx, memo: &mut BTreeMap<(usize, usize, Pos), Option<Hit>>) -> Option<Hit> {
        let a = &self.aff;
        let m = a.cpd_min;
        let n = b.len() - 1;
        let lim = if a.cpd_max == 0 { lim } else { lim.min(a.cpd_max) };
        if n < s + 2 * m {
            return None;
        }
        let pos = if parts == 0 { Pos::Begin } else { Pos::Middle };
        for e in (s + m)..=(n - m) {
            if cx.steps == 0 {
                return None;
            }
            cx.steps -= 1;
            let part = &w[b[s]..b[e]];
            if a.cpd_dup && part == prev {
                continue;
            }
            // two parts: a last part known to be none first
            if lim == 2 && !a.simple_triple && matches!(self.part_known(w, b, e, n, Pos::End, cx, memo), Some(None)) {
                continue;
            }
            let Some(ph) = self.part_memo(w, b, s, e, pos, opt, cx, memo) else { continue };
            if !self.boundary_ok(w, b, e) || left.is_some_and(|l| !self.patterns_ok(w, b[s], &l, &ph)) {
                continue;
            }
            let keep = keep.unwrap_or(ph.keepcase);
            let tail = &w[b[e]..];
            if parts + 2 <= lim {
                if let Some(h) = self.part_memo(w, b, e, n, Pos::End, opt, cx, memo) {
                    if !(a.cpd_dup && tail == part) && self.patterns_ok(w, b[e], &ph, &h) && !(a.cpd_rep && self.rep_word(w, cx)) {
                        return Some(Hit { keepcase: keep, forceucase: h.forceucase, ..Hit::default() });
                    }
                }
                // SIMPLIFIEDTRIPLE: "Schiff" + "ahrt" read as "Schiff" + "fahrt"
                if a.simple_triple {
                    let mut it = part.chars().rev();
                    if let (Some(c1), Some(c2)) = (it.next(), it.next()) {
                        if c1 == c2 && !tail.starts_with(c1) {
                            let mut t = String::with_capacity(tail.len() + 4);
                            t.push(c1);
                            t.push_str(tail);
                            if let Some(h) = self.part(&t, Pos::End, opt, cx) {
                                return Some(Hit { keepcase: keep, forceucase: h.forceucase, ..Hit::default() });
                            }
                        }
                    }
                }
            }
            if parts + 3 <= lim {
                if let Some(h) = self.cpd_rec(w, b, e, parts + 1, part, Some(ph), Some(keep), lim, opt, cx, memo) {
                    return Some(h);
                }
            }
        }
        None
    }

    // COMPOUNDRULE: the parts of w from character s on (the flag sets of the parts before in
    // seq, the last of them `prev`), lim parts at most in all; every part is a dictionary word
    // (the last one may have affixes: its root's flags count), the flag sets so far must begin
    // a match of a rule, all of them match one
    #[allow(clippy::too_many_arguments)]
    fn rule_rec<'a>(&'a self, w: &str, b: &[usize], s: usize, prev: &str, lim: usize, seq: &mut Vec<&'a [Flag]>, cx: &mut Ctx) -> bool {
        let a = &self.aff;
        let m = a.cpd_min;
        let n = b.len() - 1;
        let lim = if a.cpd_max == 0 { lim } else { lim.min(a.cpd_max) };
        if seq.len() >= lim || n < s + m {
            return false;
        }
        for e in (s + m)..=n {
            if (e < n && (n - e < m || seq.len() + 2 > lim)) || (e == n && seq.is_empty()) {
                continue;
            }
            if cx.steps == 0 {
                return false;
            }
            cx.steps -= 1;
            let part = &w[b[s]..b[e]];
            if a.cpd_dup && part == prev {
                continue;
            }
            let hp = self.words.get(part);
            for flags in hp {
                if has(flags, a.forbidden) || has(flags, a.needaffix) || has(flags, ONLYUPCASE) {
                    continue;
                }
                seq.push(flags);
                let ok = if e == n { self.rules_match(seq, true) } else { self.rules_match(seq, false) && self.rule_rec(w, b, e, part, lim, seq, cx) };
                seq.pop();
                if ok {
                    return true;
                }
            }
            // the last part with affixes (a prefix needs COMPOUNDPERMITFLAG there)
            if e == n {
                let found = self.analyze(part, hp, &mut |d| {
                    if d.any(a.forbidden) || has(d.flags, a.needaffix) || has(d.flags, ONLYUPCASE) || d.pfx.is_some_and(|x| !x.has(a.cpd_permit)) || !self.affixes_ok(d) {
                        return false;
                    }
                    let mut all: Vec<&[Flag]> = seq.clone();
                    all.push(d.flags);
                    self.rules_match(&all, true)
                });
                if found {
                    return true;
                }
            }
        }
        false
    }

    // the flag sets match a COMPOUNDRULE (full: completely; else they may begin a match)
    fn rules_match(&self, seq: &[&[Flag]], full: bool) -> bool {
        self.aff.cpd_rules.iter().any(|r| rule_at(r, seq, full))
    }

    #[allow(clippy::too_many_arguments)]
    fn part_memo(&self, w: &str, b: &[usize], s: usize, e: usize, pos: Pos, opt: Opt, cx: &mut Ctx, memo: &mut BTreeMap<(usize, usize, Pos), Option<Hit>>) -> Option<Hit> {
        if let Some(h) = self.part_known(w, b, s, e, pos, cx, memo) {
            return h;
        }
        let p = &w[b[s]..b[e]];
        let h = self.part(p, pos, opt, cx);
        match cx.parts.as_mut() {
            Some(c) => c.insert(p, pos, h),
            None => {
                memo.insert((s, e, pos), h);
            }
        }
        h
    }

    // the result for a part when it is known (this word's memo, the parts of Ctx)
    #[allow(clippy::too_many_arguments)]
    fn part_known(&self, w: &str, b: &[usize], s: usize, e: usize, pos: Pos, cx: &Ctx, memo: &BTreeMap<(usize, usize, Pos), Option<Hit>>) -> Option<Option<Hit>> {
        match cx.parts.as_ref() {
            Some(c) => c.get(&w[b[s]..b[e]], pos),
            None => memo.get(&(s, e, pos)).copied(),
        }
    }

    // one part of a compound at its place
    fn part(&self, p: &str, pos: Pos, opt: Opt, cx: &mut Ctx) -> Option<Hit> {
        let a = &self.aff;
        let posflag = match pos {
            Pos::Begin => a.cpd_begin,
            Pos::Middle => a.cpd_middle,
            Pos::End => a.cpd_end,
        };
        if cx.steps == 0 {
            return None;
        }
        cx.steps -= 1;
        let hp = self.words.get(p);
        for flags in hp {
            if has(flags, a.needaffix) || has(flags, a.cpd_forbid) || has(flags, ONLYUPCASE) || (opt.sug && has(flags, a.nosuggest)) {
                continue;
            }
            if has(flags, a.compound) || has(flags, posflag) {
                // (a forbidden entry ends the part only with a compound flag of its place; else
                // a form from another root may still be one: nl "pagina-" + "instellingen")
                return if has(flags, a.forbidden) { None } else { Some(self.hit_of(flags, false)) };
            }
        }
        let mut hit = None;
        self.analyze(p, hp, &mut |d| {
            if d.any(a.forbidden) || has(d.flags, a.cpd_forbid) || d.cont(a.cpd_forbid) || has(d.flags, ONLYUPCASE) || (opt.sug && d.any(a.nosuggest)) {
                return false;
            }
            // prefixes on the first and middle parts, suffixes on the last (else they need
            // COMPOUNDPERMITFLAG)
            if d.pfx.is_some_and(|x| pos == Pos::End && !x.has(a.cpd_permit)) {
                return false;
            }
            let sfx_ok = |x: Option<&Affix>| x.is_none_or(|x| pos == Pos::End || x.has(a.cpd_permit));
            if !sfx_ok(d.sfx) || !sfx_ok(d.sfx2) || !self.affixes_ok(d) {
                return false;
            }
            if !(d.cflag(a.compound) || d.cflag(posflag)) {
                return false;
            }
            hit = Some(self.hit_of(d.flags, true));
            true
        });
        hit
    }

    // the boundary before character e: CHECKCOMPOUNDCASE, -TRIPLE, -PATTERN
    fn boundary_ok(&self, w: &str, b: &[usize], e: usize) -> bool {
        let a = &self.aff;
        let ch = |i: usize| -> Option<char> { if i + 1 < b.len() { w[b[i]..].chars().next() } else { None } };
        let (l, r) = (ch(e - 1), ch(e));
        if a.cpd_case {
            if let (Some(l), Some(r)) = (l, r) {
                if (l.is_uppercase() || r.is_uppercase()) && l != '-' && r != '-' {
                    return false;
                }
            }
        }
        if a.cpd_triple {
            let eq3 = |i: usize| -> bool {
                match (ch(i), ch(i + 1), ch(i + 2)) {
                    (Some(x), Some(y), Some(z)) => x == y && y == z,
                    _ => false,
                }
            };
            if (e >= 2 && eq3(e - 2)) || eq3(e - 1) {
                return false;
            }
        }
        for p in a.cpd_patterns.iter().filter(|p| !p.roots()) {
            if (p.end.is_empty() && p.begin.is_empty()) || !w[..b[e]].ends_with(p.end.as_str()) || !w[b[e]..].starts_with(p.begin.as_str()) {
                continue;
            }
            return false;
        }
        true
    }

    // the CHECKCOMPOUNDPATTERNs that look at the roots: the boundary at byte `at` between the
    // parts l and r
    fn patterns_ok(&self, w: &str, at: usize, l: &Hit, r: &Hit) -> bool {
        let a = &self.aff;
        if !a.cpd_pat_roots {
            return true;
        }
        for p in a.cpd_patterns.iter().filter(|p| p.roots()) {
            let end_ok = if p.end == "0" { !l.affixed } else { w[..at].ends_with(p.end.as_str()) };
            if end_ok
                && w[at..].starts_with(p.begin.as_str())
                && (p.cond == 0 || has(self.words.set(l.set), p.cond))
                && (p.cond2 == 0 || has(self.words.set(r.set), p.cond2))
            {
                return false;
            }
        }
        true
    }

    // CHECKCOMPOUNDREP: a REP rule turns the compound into a (non-compound) word (as Hunspell:
    // the rules without ^ / $ only; any entry of the list counts, also one only for compounds,
    // and any affixed form that may stand alone)
    fn rep_word(&self, w: &str, _cx: &mut Ctx) -> bool {
        let a = &self.aff;
        let entry = |v: &str| {
            let mut hv = self.words.get(v);
            hv.next().is_some() || self.analyze(v, self.words.get(v), &mut |d| !d.cont(a.onlyincompound) && self.affixes_ok(d))
        };
        for r in &self.aff.rep {
            if r.from.is_empty() || r.start || r.end {
                continue;
            }
            let mut from = 0;
            while let Some(i) = w[from..].find(r.from.as_str()).map(|i| i + from) {
                let mut v = String::with_capacity(w.len() + r.to.len());
                v.push_str(&w[..i]);
                v.push_str(&r.to);
                v.push_str(&w[i + r.from.len()..]);
                if entry(&v) {
                    return true;
                }
                from = i + w[i..].chars().next().map_or(1, |c| c.len_utf8());
            }
        }
        false
    }

    // the compound is a pair of words the dictionary has with a space ("a cappella"); Hunspell
    // rejects it then
    fn word_pair(&self, w: &str) -> bool {
        if !self.words.spaced {
            return false;
        }
        let mut v = String::with_capacity(w.len() + 1);
        w.char_indices().skip(1).any(|(i, _)| {
            v.clear();
            v.push_str(&w[..i]);
            v.push(' ');
            v.push_str(&w[i..]);
            self.words.get(&v).next().is_some()
        })
    }
}

// the flag sets match the rule r (full: completely; else they may be the beginning of a match)
fn rule_at(r: &[RuleEl], seq: &[&[Flag]], full: bool) -> bool {
    let Some(&(f, q)) = r.first() else { return seq.is_empty() };
    let Some(&first) = seq.first() else { return !full || r.iter().all(|e| e.1 != b'1') };
    let here = has(first, f);
    match q {
        b'*' => rule_at(&r[1..], seq, full) || (here && rule_at(r, &seq[1..], full)),
        b'?' => rule_at(&r[1..], seq, full) || (here && rule_at(&r[1..], &seq[1..], full)),
        _ => here && rule_at(&r[1..], &seq[1..], full),
    }
}
