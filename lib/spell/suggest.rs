// suggest -- corrections for a misspelled word
//
// Candidates in this order (each one must pass the check in suggestion mode: NOSUGGEST and
// forbidden words never come back):
//   capitalised / upper case form, REP replacements ("_" = a space: two words), MAP
//   (related characters, e.g. a / á / à), adjacent swaps, swaps over a distance of 2..4,
//   KEY (a neighbour key on the keyboard rows, an upper-case letter), a character too many,
//   a character missing (TRY characters), a character moved by 2..4, a wrong character (TRY),
//   a doubled pair ("vacacation"), two words ("allright" -> "all right"; not with
//   NOSPLITSUGS)
// first without compounds, then (fewer than half found) again with compounds of two parts (as
// Hunspell; the candidates share most of their parts, so Ctx::parts remembers the parts'
// results for the whole search).  Too few: the edit distance to the dictionary -- the roots
// most similar by shared 1..3 letter groups (picked first by their length and bit sets of
// their letters and letter pairs), their affixed forms within Damerau-Levenshtein distance
// 1..3 (by length), nearest first.  An UPPER word gets upper-case suggestions (searched in
// lower case), a Title word title-case ones.  The work is bounded (a number of checks, not a
// clock), so the result is the same on every machine.
//
// simple_sug(word): one edit of pass 1 or two words is a word, checked as written without
// compounds -- the test check.rs makes for compounds of three or more parts.

use alloc::string::String;
use alloc::vec::Vec;

use crate::aff::{has, Flag, ONLYUPCASE};
use crate::check::{captype, lower, title, upper, Cap, Ctx, Look, Opt, Parts, BREAK_WORK};
use crate::Dictionary;

/// suggestions at most
pub(crate) const SUG_MAX: usize = 8;
const WORK_PASS1: u32 = 12_000; // candidate checks without compounds
const WORK_PASS2: u32 = 6_000; // ... with compounds
const WORK_SIMPLE: u32 = 6_000; // ... of simple_sug
const CPD_STEPS: u32 = 300; // compound search steps per candidate
const NGRAM_QUICK: usize = 1000; // roots kept by the quick score
const NGRAM_ROOTS: usize = 40; // ... of them kept by the 1..3-gram score
const WORD_MAX: usize = 64; // longer words get no suggestions

struct Sugs {
    out: Vec<String>,
    max: usize,
    work: u32,
    cpd: bool,
    /// candidates checked as written, without case variants (simple_sug)
    exact: bool,
    /// the lookups' context (the compound parts the candidates share)
    cx: Ctx,
}

impl Dictionary {
    /// suggestions for `word` (at most `max`)
    pub(crate) fn suggest_n(&self, word: &str, max: usize) -> Vec<String> {
        let w0 = self.clean(word);
        let w = w0.trim_end_matches('.');
        if w.is_empty() || w.chars().count() > WORD_MAX || max == 0 {
            return Vec::new();
        }
        let cap = captype(w, self.aff.sharps);
        let mut s = Sugs { out: Vec::new(), max, work: WORK_PASS1, cpd: false, exact: false, cx: Ctx { steps: 0, breaks: BREAK_WORK, parts: Some(Parts::new()) } };
        // pass 1: without compounds
        let base: String = if cap == Cap::All { lower(w) } else { String::from(w) };
        self.generate(&base, cap, &mut s, true);
        if cap == Cap::Init {
            self.generate(&lower(w), cap, &mut s, true);
        }
        // pass 2: compounds
        if s.out.len() < max.div_ceil(2) && self.aff.compounds() {
            s.cpd = true;
            s.work = WORK_PASS2;
            self.generate(&base, cap, &mut s, false);
        }
        // the edit distance to the dictionary
        if s.out.len() < 3 {
            self.ngram(&base, cap, &mut s);
        }
        // in the case of the word typed
        let mut out: Vec<String> = Vec::new();
        for x in s.out {
            let y = match cap {
                Cap::All => upper(&x),
                Cap::Init => title(&x),
                _ => x,
            };
            if y != w && !out.contains(&y) {
                out.push(y);
            }
        }
        out.truncate(max);
        out
    }

    /// a simple correction of w is a word: one edit of pass 1 or two words, checked as written
    /// and without compounds (Hunspell's test for compounds of three or more parts, check.rs)
    pub(crate) fn simple_sug(&self, w: &str) -> bool {
        if w.chars().count() > WORD_MAX {
            return false;
        }
        let mut s = Sugs { out: Vec::new(), max: 1, work: WORK_SIMPLE, cpd: false, exact: true, cx: Ctx::new(0) };
        self.generate(w, captype(w, self.aff.sharps), &mut s, true);
        !s.out.is_empty()
    }

    // a candidate: added when it is a correct word (two words: both)
    fn cand(&self, c: &str, cap: Cap, s: &mut Sugs) {
        if s.out.len() >= s.max || s.work == 0 || c.is_empty() {
            return;
        }
        if s.out.iter().any(|x| x == c) {
            return;
        }
        s.work -= 1;
        if s.exact {
            let ok = match c.split_once(' ') {
                Some((a, b)) => !a.is_empty() && !b.is_empty() && self.exact_ok(a) && self.exact_ok(b),
                None => self.exact_ok(c),
            };
            if ok {
                s.out.push(String::from(c));
            }
            return;
        }
        // checked in the case of the word typed (an UPPER word's candidates are lower case)
        let cc = match cap {
            Cap::All => upper(c),
            Cap::Init => title(c),
            _ => String::from(c),
        };
        let ok = match cc.split_once(' ') {
            Some((a, b)) => !a.is_empty() && !b.is_empty() && self.sug_ok(a, false, &mut s.cx) && self.sug_ok(b, false, &mut s.cx),
            None => self.sug_ok(&cc, s.cpd, &mut s.cx),
        };
        if ok {
            s.out.push(String::from(c));
        }
    }

    // c is correct in suggestion mode
    fn sug_ok(&self, c: &str, cpd: bool, cx: &mut Ctx) -> bool {
        let opt = Opt { compounds: cpd, sug: true, upcase: true };
        cx.steps = CPD_STEPS;
        matches!(self.check_cased(c, opt, cx), Look::Hit(_))
    }

    // c as written is a word (no case variants, no compounds; suggestion mode)
    fn exact_ok(&self, c: &str) -> bool {
        let opt = Opt { compounds: false, sug: true, upcase: false };
        matches!(self.look(c, opt, &mut Ctx::new(0)), Look::Hit(_))
    }

    // the generators (full: pass 1; the compound pass leaves out the capitalised forms and
    // the two-word split)
    fn generate(&self, w: &str, cap: Cap, s: &mut Sugs, full: bool) {
        let cs: Vec<char> = w.chars().collect();
        let n = cs.len();
        let mk = |v: &[char]| -> String { v.iter().collect() };
        // capitalised / upper case (of a word typed in lower case)
        if full && cap == Cap::No {
            self.cand(&title(w), cap, s);
            self.cand(&upper(w), cap, s);
        }
        // REP
        for r in &self.aff.rep {
            if r.from.is_empty() {
                continue;
            }
            let mut from = 0;
            while let Some(i) = w[from..].find(r.from.as_str()).map(|i| i + from) {
                if (!r.start || i == 0) && (!r.end || i + r.from.len() == w.len()) {
                    let mut v = String::with_capacity(w.len() + r.to.len());
                    v.push_str(&w[..i]);
                    v.push_str(&r.to);
                    v.push_str(&w[i + r.from.len()..]);
                    self.cand(&v, cap, s);
                }
                from = i + r.from.len();
            }
        }
        // MAP
        if !self.aff.map.is_empty() {
            let mut buf = String::new();
            self.map_rec(w, 0, &mut buf, false, cap, s, 0);
        }
        // adjacent swaps; for 4 and 5 letters also both ends at once
        let mut v = cs.clone();
        for i in 0..n.saturating_sub(1) {
            if v[i] != v[i + 1] {
                v.swap(i, i + 1);
                self.cand(&mk(&v), cap, s);
                v.swap(i, i + 1);
            }
        }
        if n == 4 || n == 5 {
            v.swap(0, 1);
            v.swap(n - 2, n - 1);
            self.cand(&mk(&v), cap, s);
            v.swap(0, 1);
            v.swap(n - 2, n - 1);
        }
        // swaps over a distance of 2..4
        for i in 0..n {
            for d in 2..=4 {
                if i + d < n && v[i] != v[i + d] {
                    v.swap(i, i + d);
                    self.cand(&mk(&v), cap, s);
                    v.swap(i, i + d);
                }
            }
        }
        // KEY: upper-case letter, neighbour keys
        for i in 0..n {
            let c = cs[i];
            if full {
                let mut u = c.to_uppercase();
                if let (Some(uc), None) = (u.next(), u.next()) {
                    if uc != c {
                        v[i] = uc;
                        self.cand(&mk(&v), cap, s);
                    }
                }
            }
            for row in &self.aff.key {
                for (j, &k) in row.iter().enumerate() {
                    if k != c {
                        continue;
                    }
                    for nb in [j.checked_sub(1), Some(j + 1)].into_iter().flatten() {
                        if let Some(&kc) = row.get(nb) {
                            if kc != c {
                                v[i] = kc;
                                self.cand(&mk(&v), cap, s);
                            }
                        }
                    }
                }
            }
            v[i] = c;
        }
        // a character too many
        if n > 1 {
            for i in 0..n {
                if i > 0 && cs[i] == cs[i - 1] {
                    continue; // the same word as removing the one before
                }
                let mut x = cs.clone();
                x.remove(i);
                self.cand(&mk(&x), cap, s);
            }
        }
        // a character missing
        let try_chars = self.try_chars();
        for i in 0..=n {
            for &t in &try_chars {
                let mut x = cs.clone();
                x.insert(i, t);
                self.cand(&mk(&x), cap, s);
            }
        }
        // a character moved by 2..4
        for i in 0..n {
            for d in 2..=4usize {
                if i + d < n {
                    let mut x = cs.clone();
                    let c = x.remove(i);
                    x.insert(i + d, c);
                    self.cand(&mk(&x), cap, s);
                }
                if i >= d {
                    let mut x = cs.clone();
                    let c = x.remove(i);
                    x.insert(i - d, c);
                    self.cand(&mk(&x), cap, s);
                }
            }
        }
        // a wrong character
        for i in 0..n {
            for &t in &try_chars {
                if t != cs[i] {
                    v[i] = t;
                    self.cand(&mk(&v), cap, s);
                }
            }
            v[i] = cs[i];
        }
        // a doubled pair: "vacacation"
        for i in 0..n.saturating_sub(3) {
            if cs[i] == cs[i + 2] && cs[i + 1] == cs[i + 3] {
                let mut x = cs.clone();
                x.drain(i + 2..i + 4);
                self.cand(&mk(&x), cap, s);
            }
        }
        // two words (pass 1)
        if full && !self.aff.nosplitsugs {
            for i in 1..n {
                let mut x = String::with_capacity(w.len() + 1);
                x.extend(&cs[..i]);
                x.push(' ');
                x.extend(&cs[i..]);
                self.cand(&x, cap, s);
            }
        }
    }

    // TRY, or the letters of the word list's start when the file has none
    fn try_chars(&self) -> Vec<char> {
        if !self.aff.try_chars.is_empty() {
            return self.aff.try_chars.clone();
        }
        let mut v: Vec<char> = Vec::new();
        for (w, _) in self.words.iter().take(2000) {
            for c in w.chars().flat_map(|c| c.to_lowercase()) {
                if c.is_alphabetic() && !v.contains(&c) {
                    v.push(c);
                }
            }
        }
        v
    }

    // MAP: every way to replace characters of a related group (one replacement at least)
    #[allow(clippy::too_many_arguments)]
    fn map_rec(&self, w: &str, i: usize, buf: &mut String, changed: bool, cap: Cap, s: &mut Sugs, depth: u32) {
        if s.work == 0 || s.out.len() >= s.max || depth > 6 {
            return;
        }
        if i >= w.len() {
            if changed {
                let c = buf.clone();
                self.cand(&c, cap, s);
            }
            return;
        }
        let len = buf.len();
        // keep the character
        let ch = w[i..].chars().next().map_or(1, |c| c.len_utf8());
        buf.push_str(&w[i..i + ch]);
        self.map_rec(w, i + ch, buf, changed, cap, s, depth);
        buf.truncate(len);
        // replace a group member at i by the others
        for g in &self.aff.map {
            for m in g {
                if m.is_empty() || !w[i..].starts_with(m.as_str()) {
                    continue;
                }
                for o in g {
                    if o == m {
                        continue;
                    }
                    buf.push_str(o);
                    self.map_rec(w, i + m.len(), buf, true, cap, s, depth + 1);
                    buf.truncate(len);
                }
            }
        }
    }

    // the edit distance to the dictionary (see the module header)
    fn ngram(&self, w: &str, cap: Cap, s: &mut Sugs) {
        let a = &self.aff;
        let lw: Vec<char> = lower(w).chars().collect();
        let wl = lw.len();
        if wl < 2 {
            return;
        }
        // every root: a quick score from bit sets of its letters and letter pairs; the best
        // NGRAM_QUICK of them get the 1..3-gram score, the best NGRAM_ROOTS of those are kept
        let sw = Sig::of(lw.iter().copied());
        let mut quick: Vec<(i32, u32)> = Vec::new(); // score, entry
        for (i, (root, flags)) in self.words.iter().enumerate() {
            // the length first (characters: the bytes that start one)
            let n = root.bytes().filter(|&b| b & 0xC0 != 0x80).count();
            if n + 5 < wl || n > wl + 3 || has(flags, a.forbidden) || has(flags, a.nosuggest) || has(flags, ONLYUPCASE) {
                continue;
            }
            let sr = Sig::of(root.chars().map(lower1));
            quick.push((sw.score(&sr) - (sr.n as i32 - wl as i32).abs(), i as u32));
        }
        best_first(&mut quick, NGRAM_QUICK);
        let mut lr: Vec<char> = Vec::new();
        let mut best: Vec<(i32, u32)> = quick
            .iter()
            .map(|&(_, i)| {
                lr.clear();
                lr.extend(self.words.entry(i as usize).0.chars().flat_map(|c| c.to_lowercase()));
                (ngram_score(&lw, &lr) - (lr.len() as i32 - wl as i32).abs(), i)
            })
            .collect();
        best_first(&mut best, NGRAM_ROOTS);
        // their forms near the word
        let limit = match wl {
            0..=4 => 1,
            5..=8 => 2,
            _ => 3,
        };
        let mut forms: Vec<(usize, usize, String)> = Vec::new(); // distance, -prefix, form
        for &(_, i) in &best {
            let (root, flags) = self.words.entry(i as usize);
            self.forms(root, flags, &mut |f: String| {
                let lf: Vec<char> = f.chars().flat_map(|c| c.to_lowercase()).collect();
                if lf.len() + limit < wl || lf.len() > wl + limit {
                    return;
                }
                let d = damerau(&lw, &lf);
                if d == 0 || d > limit {
                    return;
                }
                let pre = lw.iter().zip(lf.iter()).take_while(|(x, y)| x == y).count();
                if !forms.iter().any(|x| x.2 == f) {
                    forms.push((d, usize::MAX - pre, f));
                }
            });
        }
        forms.sort();
        s.cpd = false;
        s.work = s.work.max(64);
        for (_, _, f) in forms {
            let f = if cap == Cap::All { lower(&f) } else { f };
            self.cand(&f, cap, s);
        }
    }

    // f(form) for a root and its affixed forms (one suffix, one prefix, both)
    fn forms(&self, root: &str, flags: &[Flag], f: &mut dyn FnMut(String)) {
        let a = &self.aff;
        if !has(flags, a.needaffix) && !has(flags, a.onlyincompound) {
            f(String::from(root));
        }
        let mut sfx_forms: Vec<(String, usize)> = Vec::new();
        for (i, r) in a.sfx.iter().enumerate() {
            if has(flags, r.flag) && !r.has(a.onlyincompound) && !r.has(a.needaffix) && root.ends_with(r.strip.as_str()) && r.cond_ok(root, true) {
                let mut x = String::from(&root[..root.len() - r.strip.len()]);
                x.push_str(&r.add);
                if !x.is_empty() {
                    sfx_forms.push((x.clone(), i));
                    f(x);
                }
            }
        }
        for p in &a.pfx {
            if !has(flags, p.flag) || p.has(a.onlyincompound) || p.has(a.needaffix) || !root.starts_with(p.strip.as_str()) || !p.cond_ok(root, false) {
                continue;
            }
            let mut x = String::from(&p.add);
            x.push_str(&root[p.strip.len()..]);
            f(x);
            if p.cross {
                for (sf, i) in &sfx_forms {
                    if a.sfx[*i].cross && sf.starts_with(p.strip.as_str()) {
                        let mut y = String::from(&p.add);
                        y.push_str(&sf[p.strip.len()..]);
                        f(y);
                    }
                }
            }
        }
    }
}

// the n best (score, entry) pairs, best first (equal scores: the earlier entry first)
fn best_first(v: &mut Vec<(i32, u32)>, n: usize) {
    let order = |a: &(i32, u32), b: &(i32, u32)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
    if v.len() > n && n > 0 {
        v.select_nth_unstable_by(n - 1, order);
        v.truncate(n);
    }
    v.sort_unstable_by(order);
}

/// bit sets of the letters and letter pairs of a word (hashed), its length
struct Sig {
    uni: u64,
    bi: [u64; 2],
    n: usize,
}

impl Sig {
    fn of(it: impl Iterator<Item = char>) -> Sig {
        let mut s = Sig { uni: 0, bi: [0; 2], n: 0 };
        let mut prev: Option<char> = None;
        for c in it {
            s.uni |= 1 << ((c as u32).wrapping_mul(0x9E37_79B1) >> 26);
            if let Some(p) = prev {
                let h = (p as u32).wrapping_mul(31).wrapping_add(c as u32).wrapping_mul(0x9E37_79B1) >> 25;
                s.bi[(h >> 6) as usize] |= 1 << (h & 63);
            }
            prev = Some(c);
            s.n += 1;
        }
        s
    }

    fn score(&self, o: &Sig) -> i32 {
        ((self.bi[0] & o.bi[0]).count_ones() * 2 + (self.bi[1] & o.bi[1]).count_ones() * 2 + (self.uni & o.uni).count_ones()) as i32
    }
}

// a character in lower case (one character: the quick score needs no more)
fn lower1(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

// shared letter groups of length 1..3 (each group of a counted once)
fn ngram_score(a: &[char], b: &[char]) -> i32 {
    let mut sc = 0;
    for n in 1..=3usize {
        if a.len() < n {
            break;
        }
        for i in 0..=a.len() - n {
            let g = &a[i..i + n];
            if b.windows(n).any(|x| x == g) {
                sc += n as i32;
            }
        }
    }
    sc
}

/// the optimal string alignment distance (Damerau-Levenshtein with adjacent swaps)
pub(crate) fn damerau(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = alloc::vec![0usize; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in 0..=n {
        d[at(i, 0)] = i;
    }
    for j in 0..=m {
        d[at(0, j)] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = (a[i - 1] != b[j - 1]) as usize;
            let mut v = (d[at(i - 1, j)] + 1).min(d[at(i, j - 1)] + 1).min(d[at(i - 1, j - 1)] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[at(i - 2, j - 2)] + 1);
            }
            d[at(i, j)] = v;
        }
    }
    d[at(n, m)]
}
