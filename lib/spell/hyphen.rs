// hyphen -- hyphenation: Liang's algorithm on TeX patterns and on the dictionaries of
// libhyphen (LibreOffice / OpenOffice "hyph_xx_YY.dic")
//
//   Hyphenator::from_dic(bytes) -> Option   a libhyphen dictionary: the character set on the
//        first line (UTF-8, ISO8859-1 .. -16, KOI8-R / -U, microsoft-cp1251 ...: aff.rs),
//        LEFTHYPHENMIN / RIGHTHYPHENMIN / COMPOUNDLEFTHYPHENMIN / COMPOUNDRIGHTHYPHENMIN n,
//        NOHYPHEN a,b,... (no break next to these), NEXTLEVEL (the patterns before it find
//        the parts of a compound word, the ones after it hyphenate each part), one pattern per
//        line, '%' / '#' comments
//   Hyphenator::from_tex(text) -> Option    the \patterns{...} and \hyphenation{...} blocks of
//        a TeX pattern file (hyph-utf8's hyph-*.tex; UTF-8, '%' comments)
//   load(path) -> Option                    a .dic file, or .tex (by its \patterns)
//   add_patterns(text)                      more patterns, whitespace-separated ("hy3ph")
//   add_exceptions(text)                    words with every break shown ("ta-ble"): exact
//                                           results for these words
//   set_min(left, right)                    characters a break leaves at least on the left /
//                                           right of the word (default and minimum: the
//                                           dictionary's values, else 2 and 2)
//   hyphenate(word) -> Vec<usize>           where word may break (byte offsets of the first
//                                           character after each break, ascending)
//   hyphenated(word, sep) -> String         word with sep at every break (tests, tools)
//   find_hyphen(lang, SRC_*) -> Option<(path, tag)>, hyphen_languages(SRC_*)
//                                           the dictionaries of a source (see below)
//
// Liang: every pattern is letters with a digit (0..9) between them; for each place between
// two letters of ".word." the largest digit of the matching patterns wins and odd allows a
// break.  The word is lower-cased first (character by character) and digits in it count
// as word boundaries ('.').  TeX patterns: every matching pattern counts.  .dic files are
// made for libhyphen and read as it reads them: its automaton applies at each character only
// the pattern that is its state there (the longest suffix of the text read so far that begins
// some pattern; de_DE's first level is a list of words that relies on it), and it works in two
// levels: the first level's odd places split a word into parts, every part goes through the
// first level again (it may show parts the whole word hid), a part without parts is hyphenated
// by the second level with COMPOUNDLEFT / RIGHTHYPHENMIN at its inner ends; never a break
// after the last character.  A dictionary without NEXTLEVEL gets libhyphen's default first
// level (parts at '-', '\'', U+2013, U+2019, which NOHYPHEN keeps from breaking themselves).
// Then no break leaves fewer than LEFTHYPHENMIN / RIGHTHYPHENMIN characters at the ends, and
// NOHYPHEN removes the breaks next to its strings.  The results equal libhyphen 2.8's on the 21
// dictionaries of the freedesktop SDK (spelltest --hyphenate; 214,000 words, 0 differences).
// Not supported: libhyphen's non-standard hyphenation ("c1k/k=k,1,1": Zucker -> Zuk-ker,
// mainly hu_HU) -- such a pattern keeps the places around its replaced letters from breaking
// (odd digits there count one more), so no break ever needs a changed spelling.
//
// Sources (spell::SRC_*), like the Hunspell dictionaries (speller.rs): SRC_SYSTEM: DICPATH,
// on Linux $XDG_DATA_HOME/hyphen and hyphen, hunspell, myspell, myspell/dicts below every
// XDG_DATA_DIRS folder (/usr/share/hyphen: Debian's hyphen-* packages), on Windows also the
// dictionary extensions of LibreOffice (%ProgramFiles%\LibreOffice\share\extensions\dict-*);
// SRC_USER: config://hyphen; SRC_APP: app://hyphen.  A file hyph_<tag>.dic serves a language
// tag like a Hunspell file NAME.aff: the tag ("de_DE"), the bare language, another region.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::aff::decode;
use crate::speller::{score, split_tag};
use crate::{SRC_APP, SRC_SYSTEM, SRC_USER};

/// one level of patterns: (letters, index of their digits in vals), sorted by letters
#[derive(Default, Clone)]
struct Level {
    pats: Vec<(String, u32)>,
    vals: Vec<u8>,
    /// the longest pattern (characters)
    max_len: usize,
    /// libhyphen's matcher (a .dic file) instead of every matching pattern (TeX)
    longest: bool,
}

impl Level {
    // a pattern "hy3ph" -> (letters, digits: one more than letters); a non-standard pattern
    // ("fes5szi1o/sz=,3,1": the 3rd letter replaced by "sz=", 1 letter) inhibits the breaks
    // around its replaced letters (odd digits + 1; without ",index,count" all its breaks),
    // its other digits count as usual; None without letters
    fn parse(tok: &str) -> Option<(String, Vec<u8>)> {
        let (tok, repl) = match tok.split_once('/') {
            Some((p, r)) => (p, Some(r)),
            None => (tok, None),
        };
        let mut letters = String::new();
        let mut vals = alloc::vec![0u8];
        for c in tok.chars() {
            match c.to_digit(10) {
                Some(d) => *vals.last_mut().unwrap_or(&mut 0) = d as u8,
                None => {
                    letters.push(c);
                    vals.push(0);
                }
            }
        }
        if let Some(r) = repl {
            // the places from before the first replaced letter to after the last one
            let dot = letters.starts_with('.') as usize;
            let n = letters.chars().count();
            let f: Vec<&str> = r.split(',').collect();
            let num = |i: usize| f.get(i).and_then(|v| v.trim().parse::<usize>().ok());
            let (from, to) = match (num(1), num(2)) {
                (Some(i), Some(c)) => (dot + i.saturating_sub(1), dot + i.saturating_sub(1) + c),
                _ => (0, n),
            };
            for (d, v) in vals.iter_mut().enumerate() {
                if *v & 1 == 1 && d >= from && d <= to {
                    *v += 1;
                }
            }
        }
        (!letters.is_empty()).then_some((letters, vals))
    }

    // add patterns (unsorted until finish)
    fn push(&mut self, letters: String, vals: Vec<u8>) {
        self.max_len = self.max_len.max(letters.chars().count());
        let at = self.vals.len() as u32;
        self.vals.extend_from_slice(&vals);
        self.pats.push((letters, at));
    }

    // sort; a pattern given twice: the later one
    fn finish(&mut self) {
        // stable sort keeps the order of equal keys: the last of a run wins
        self.pats.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out: Vec<(String, u32)> = Vec::with_capacity(self.pats.len());
        for p in self.pats.drain(..) {
            match out.last_mut() {
                Some(l) if l.0 == p.0 => *l = p,
                _ => out.push(p),
            }
        }
        self.pats = out;
    }

    fn is_empty(&self) -> bool {
        self.pats.is_empty()
    }

    // where buf stands among the patterns: Ok(index) a pattern, Err(true) the beginning of
    // one (a state of libhyphen's automaton), Err(false) neither
    fn find(&self, buf: &str) -> Result<usize, bool> {
        match self.pats.binary_search_by(|p| p.0.as_str().cmp(buf)) {
            Ok(k) => Ok(k),
            Err(k) => Err(self.pats.get(k).is_some_and(|p| p.0.starts_with(buf))),
        }
    }

    // the digits of pattern k, its first letter at prep[i]
    fn put(&self, k: usize, i: usize, len: usize, g: &mut [u8]) {
        let at = self.pats[k].1 as usize;
        for (d, &v) in self.vals[at..at + len + 1].iter().enumerate() {
            if g[i + d] < v {
                g[i + d] = v;
            }
        }
    }

    // the values between the characters of prep (".word."): g[q] = the place before
    // character q (q in 0..=n)
    fn apply(&self, prep: &[char], buf: &mut String, g: &mut [u8]) {
        let n = prep.len();
        if self.longest {
            // libhyphen: the state is the longest suffix of prep[..=i] that begins a pattern
            let mut s = 0usize;
            for i in 0..n {
                let mut t = s;
                let mut state = None;
                while t <= i {
                    buf.clear();
                    buf.extend(&prep[t..=i]);
                    match self.find(buf) {
                        Ok(k) => state = Some((t, Some(k))),
                        Err(true) => state = Some((t, None)),
                        Err(false) => {
                            t += 1;
                            continue;
                        }
                    }
                    break;
                }
                match state {
                    Some((t, k)) => {
                        s = t;
                        if let Some(k) = k {
                            self.put(k, t, i - t + 1, g);
                        }
                    }
                    None => s = i + 1,
                }
            }
            return;
        }
        for i in 0..n {
            buf.clear();
            for j in i..n.min(i + self.max_len) {
                buf.push(prep[j]);
                match self.find(buf) {
                    Ok(k) => self.put(k, i, j - i + 1, g),
                    Err(true) => {}
                    Err(false) => break, // no pattern begins so: no longer one either
                }
            }
        }
    }
}

/// hyphenation patterns of one language (see the module header)
#[derive(Clone)]
pub struct Hyphenator {
    /// the levels: [parts, hyphenation of each part]
    levels: [Level; 2],
    /// NOHYPHEN of the first level
    nohyphen: Vec<Vec<char>>,
    /// LEFTHYPHENMIN, RIGHTHYPHENMIN, COMPOUNDLEFTHYPHENMIN, COMPOUNDRIGHTHYPHENMIN (0: not set)
    mins: [usize; 4],
    /// set_min
    user: (usize, usize),
    /// lower-case word -> the places after which it breaks (character indices)
    exceptions: BTreeMap<String, Vec<usize>>,
}

impl Default for Hyphenator {
    fn default() -> Hyphenator {
        Hyphenator::new()
    }
}

impl Hyphenator {
    /// no patterns (no breaks) and libhyphen's default first level
    pub fn new() -> Hyphenator {
        let mut h = Hyphenator {
            levels: [Level::default(), Level::default()],
            nohyphen: Vec::new(),
            mins: [0; 4],
            user: (0, 0),
            exceptions: BTreeMap::new(),
        };
        h.default_first_level();
        h
    }

    // libhyphen without NEXTLEVEL: parts at hyphens and apostrophes, never broken themselves
    fn default_first_level(&mut self) {
        let mut l = Level::default();
        for p in ["1-1", "1'1", "1\u{2013}1", "1\u{2019}1"] {
            if let Some((a, v)) = Level::parse(p) {
                l.push(a, v);
            }
        }
        l.finish();
        self.levels[0] = l;
        self.nohyphen = ["'", "\u{2013}", "\u{2019}", "-"].iter().map(|s| s.chars().collect()).collect();
    }

    /// a libhyphen dictionary (hyph_xx_YY.dic); None when it has no pattern
    pub fn from_dic(bytes: &[u8]) -> Option<Hyphenator> {
        let first = bytes.split(|&c| c == b'\n').next().unwrap_or(b"");
        let cset: String = String::from_utf8_lossy(first).trim().trim_start_matches('\u{FEFF}').into();
        let text = decode(bytes, &cset);
        let mut h = Hyphenator::new();
        let mut levels = [Level { longest: true, ..Level::default() }, Level { longest: true, ..Level::default() }];
        let mut nohyphen: Option<Vec<Vec<char>>> = None;
        let mut mins = [0usize; 4];
        let mut lv = 0;
        for line in text.lines().skip(1) {
            let line = line.trim_end_matches('\r');
            // comments; a line starting with a blank has no pattern (libhyphen)
            if line.starts_with(['%', '#', ' ', '\t']) {
                continue;
            }
            let num = |k: &str| line[k.len()..].trim().parse::<usize>().unwrap_or(0);
            // the minimums of the first level count (libhyphen reads the top dictionary's)
            let key = ["LEFTHYPHENMIN", "RIGHTHYPHENMIN", "COMPOUNDLEFTHYPHENMIN", "COMPOUNDRIGHTHYPHENMIN"]
                .iter()
                .position(|k| line.starts_with(k));
            if line.starts_with("NEXTLEVEL") {
                lv = 1;
            } else if let Some(i) = key {
                if lv == 0 {
                    mins[i] = num(["LEFTHYPHENMIN", "RIGHTHYPHENMIN", "COMPOUNDLEFTHYPHENMIN", "COMPOUNDRIGHTHYPHENMIN"][i]);
                }
            } else if let Some(r) = line.strip_prefix("NOHYPHEN") {
                if lv == 0 {
                    nohyphen = Some(r.trim().split(',').filter(|s| !s.is_empty()).map(|s| s.chars().collect()).collect());
                }
            } else if let Some(tok) = line.split_whitespace().next() {
                if let Some((a, v)) = Level::parse(tok) {
                    levels[lv].push(a, v);
                }
            }
        }
        if lv == 1 {
            // two levels of the file
            levels[0].finish();
            levels[1].finish();
            if levels[1].is_empty() {
                return None;
            }
            h.levels = levels;
            h.nohyphen = nohyphen.unwrap_or_default();
            h.mins = mins;
        } else {
            // the default first level, the file's patterns second (libhyphen: the compound
            // minimums are the file's, else its LEFT / RIGHTHYPHENMIN, else 3)
            let [mut l, _] = levels;
            l.finish();
            if l.is_empty() {
                return None;
            }
            h.levels[1] = l;
            let c = |m: usize, plain: usize| if m != 0 { m } else if plain != 0 { plain } else { 3 };
            h.mins = [mins[0], mins[1], c(mins[2], mins[0]), c(mins[3], mins[1])];
        }
        Some(h)
    }

    /// the \patterns{...} and \hyphenation{...} of a TeX file; None when it has no pattern
    pub fn from_tex(text: &str) -> Option<Hyphenator> {
        let mut clean = String::with_capacity(text.len());
        for line in text.lines() {
            // '%' starts a comment (not "\%")
            let mut prev = ' ';
            for c in line.chars() {
                if c == '%' && prev != '\\' {
                    break;
                }
                clean.push(c);
                prev = c;
            }
            clean.push('\n');
        }
        let block = |name: &str| -> Vec<&str> {
            let mut v = Vec::new();
            let mut rest = clean.as_str();
            while let Some(k) = rest.find(name) {
                let after = &rest[k + name.len()..];
                let Some(open) = after.trim_start().strip_prefix('{') else {
                    rest = after;
                    continue;
                };
                let end = open.find('}').unwrap_or(open.len());
                v.push(&open[..end]);
                rest = &open[end..];
            }
            v
        };
        let mut h = Hyphenator::new();
        for b in block("\\patterns") {
            h.add_level1(b);
        }
        if h.levels[1].is_empty() {
            return None;
        }
        for b in block("\\hyphenation") {
            h.add_exceptions(b);
        }
        Some(h)
    }

    /// a .dic file (or a TeX file: .tex)
    pub fn load(path: &str) -> Option<Hyphenator> {
        let b = plat::read_file(path)?;
        if path.ends_with(".tex") {
            Hyphenator::from_tex(&String::from_utf8_lossy(&b))
        } else {
            Hyphenator::from_dic(&b)
        }
    }

    fn add_level1(&mut self, text: &str) {
        for tok in text.split_whitespace() {
            if tok.contains("^^") || tok.starts_with('\\') {
                continue; // TeX's 8-bit notation and commands
            }
            if let Some((a, v)) = Level::parse(tok) {
                self.levels[1].push(a, v);
            }
        }
        self.levels[1].finish();
    }

    /// more patterns (whitespace-separated, "hy3ph"), added to the hyphenating level
    pub fn add_patterns(&mut self, text: &str) {
        self.add_level1(text);
    }

    /// exceptions: words with every break as '-' ("ta-ble", "project"); they replace the
    /// patterns for exactly these words (any case)
    pub fn add_exceptions(&mut self, text: &str) {
        for tok in text.split_whitespace() {
            let mut word = String::new();
            let mut at = Vec::new();
            let mut n = 0usize;
            for c in tok.chars() {
                if c == '-' {
                    if n > 0 {
                        at.push(n - 1);
                    }
                } else {
                    word.extend(c.to_lowercase().next());
                    n += 1;
                }
            }
            if n > 0 {
                at.retain(|&k| k + 1 < n);
                self.exceptions.insert(word, at);
            }
        }
    }

    /// characters a break leaves at least at the start / end of a word (the dictionary's
    /// LEFTHYPHENMIN / RIGHTHYPHENMIN stay the minimum)
    pub fn set_min(&mut self, left: usize, right: usize) {
        self.user = (left, right);
    }

    /// (LEFTHYPHENMIN, RIGHTHYPHENMIN) in effect
    pub fn min(&self) -> (usize, usize) {
        let d = |m: usize| if m > 0 { m } else { 2 };
        (d(self.mins[0]).max(self.user.0), d(self.mins[1]).max(self.user.1))
    }

    /// the patterns of both levels (tests, tools)
    pub fn patterns(&self) -> (usize, usize) {
        (self.levels[0].pats.len(), self.levels[1].pats.len())
    }

    // libhyphen's hnj_hyphen_hyph_: the values after each character of word (lower case) on
    // level lv (never one after the last character); on the first level each part goes
    // through the first level again (it may hold parts that the whole word hid), a part
    // without parts through the second level, with the compound minimums at inner ends
    fn hyph(&self, lv: usize, word: &[char], lend: bool, rend: bool, buf: &mut String) -> Vec<u8> {
        let n = word.len();
        let mut prep: Vec<char> = Vec::with_capacity(n + 2);
        prep.push('.');
        prep.extend(word.iter().map(|&c| if c.is_ascii_digit() { '.' } else { c }));
        prep.push('.');
        let mut g = alloc::vec![0u8; n + 3];
        self.levels[lv].apply(&prep, buf, &mut g);
        // after character k of the word: before character k + 2 of prep
        let mut h: Vec<u8> = (0..n).map(|k| if k + 1 < n { g[k + 2] } else { 0 }).collect();
        if lv == 1 {
            return h;
        }
        // the parts (odd places of the first level)
        let (cl, cr) = (self.mins[2], self.mins[3]);
        let mut begin = 0usize;
        for i in 0..n {
            if h[i] & 1 == 1 || (begin > 0 && i + 1 == n) {
                if i > begin {
                    let part_l = begin == 0 && lend;
                    let part_r = h[i] & 1 == 0 && rend;
                    let p = self.hyph(0, &word[begin..=i], part_l, part_r, buf);
                    h[begin..i].copy_from_slice(&p[..i - begin]);
                }
                begin = i + 1;
            }
        }
        if begin == 0 {
            h = self.hyph(1, word, lend, rend, buf);
            if !lend {
                lhmin(word, &mut h, cl);
            }
            if !rend {
                rhmin(word, &mut h, cr);
            }
        }
        h
    }

    /// where word may break: the byte offset of the first character after each break
    pub fn hyphenate(&self, word: &str) -> Vec<usize> {
        let chars: Vec<char> = word.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
        let n = chars.len();
        let (lmin, rmin) = self.min();
        if n < 2 || n < lmin + rmin || self.levels[1].is_empty() {
            return Vec::new();
        }
        let lower: String = chars.iter().collect();
        let mut h = match self.exceptions.get(&lower) {
            Some(at) => {
                let mut h = alloc::vec![0u8; n];
                for &k in at {
                    h[k] = 1;
                }
                h
            }
            None => {
                let mut buf = String::new();
                self.hyph(0, &chars, true, true, &mut buf)
            }
        };
        lhmin(&chars, &mut h, lmin);
        rhmin(&chars, &mut h, rmin);
        // NOHYPHEN: no break right before or after these
        for nh in &self.nohyphen {
            let m = nh.len();
            if m == 0 || m > n {
                continue;
            }
            for s in 0..=n - m {
                if chars[s..s + m] == nh[..] {
                    h[s + m - 1] = 0;
                    if s > 0 {
                        h[s - 1] = 0;
                    }
                }
            }
        }
        let mut out = Vec::new();
        let mut off = 0usize;
        for (k, c) in word.chars().enumerate() {
            off += c.len_utf8();
            if k + 1 < n && h[k] & 1 == 1 {
                out.push(off);
            }
        }
        out
    }

    /// word with sep at every break ("hy-phen-ation")
    pub fn hyphenated(&self, word: &str, sep: &str) -> String {
        let mut s = String::with_capacity(word.len() + 8);
        let mut last = 0;
        for at in self.hyphenate(word) {
            s.push_str(&word[last..at]);
            s.push_str(sep);
            last = at;
        }
        s.push_str(&word[last..]);
        s
    }
}

// libhyphen's hnj_hyphen_lhmin: no break among the first min characters (leading digits
// count extra)
fn lhmin(word: &[char], h: &mut [u8], min: usize) {
    let digits = word.iter().take_while(|c| c.is_ascii_digit()).count();
    let clear = (min + digits).saturating_sub(1).min(h.len());
    for v in &mut h[..clear] {
        *v = 0;
    }
}

// libhyphen's hnj_hyphen_rhmin: no break among the last min characters (trailing digits count
// extra; the first place stays to lhmin)
fn rhmin(word: &[char], h: &mut [u8], min: usize) {
    let n = h.len();
    let digits = word.iter().rev().take_while(|c| c.is_ascii_digit()).count();
    let clear = (min + digits).min(n.saturating_sub(1));
    for v in &mut h[n - clear..] {
        *v = 0;
    }
}

// the folders of a source
fn dirs(src: u32) -> Vec<String> {
    let mut v = Vec::new();
    match src {
        SRC_SYSTEM => {
            v = plat::spell::hunspell_dirs();
            v.extend(plat::spell::hyphen_dirs());
            if cfg!(windows) {
                for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
                    let Some(p) = plat::env(pf) else { continue };
                    let ext = format!("{}\\LibreOffice\\share\\extensions", p.trim_end_matches('\\'));
                    let mut subs = Vec::new();
                    plat::dir_list(&ext, &mut |n, is_dir, _| {
                        if is_dir && n.starts_with("dict-") {
                            subs.push(format!("{}\\{}", ext, n));
                        }
                    });
                    subs.sort();
                    v.extend(subs);
                }
            }
        }
        SRC_USER => v.push(String::from("config://hyphen")),
        SRC_APP => v.push(String::from("app://hyphen")),
        _ => {}
    }
    v
}

// the hyph_*.dic files of a folder: (stem after "hyph_", path), sorted
fn files(dir: &str) -> Vec<(String, String)> {
    let mut names: Vec<String> = Vec::new();
    plat::dir_list(dir, &mut |n, is_dir, _| {
        if !is_dir && n.starts_with("hyph_") && n.ends_with(".dic") {
            names.push(String::from(n));
        }
    });
    names.sort();
    let sep = if dir.ends_with('/') || dir.ends_with('\\') { "" } else { "/" };
    names.iter().map(|n| (String::from(&n[5..n.len() - 4]), format!("{}{}{}", dir, sep, n))).collect()
}

/// the hyphenation dictionary for a language tag: (path, the tag it serves); sources in the
/// order SRC_SYSTEM, SRC_USER, SRC_APP (as spell::find)
pub fn find_hyphen(lang: &str, sources: u32) -> Option<(String, String)> {
    let (p, r) = split_tag(lang);
    if p.is_empty() {
        return None;
    }
    for src in [SRC_SYSTEM, SRC_USER, SRC_APP] {
        if sources & src == 0 {
            continue;
        }
        let mut best: Option<(u32, String, String)> = None;
        for d in dirs(src) {
            for (stem, path) in files(&d) {
                let s = score(&stem, &p, &r);
                if s > 0 && best.as_ref().is_none_or(|b| s > b.0) {
                    best = Some((s, path, stem.replace('_', "-")));
                }
            }
        }
        if let Some((_, path, tag)) = best {
            return Some((path, tag));
        }
    }
    None
}

/// the language tags there are hyphenation dictionaries for (sorted, each once)
pub fn hyphen_languages(sources: u32) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for src in [SRC_SYSTEM, SRC_USER, SRC_APP] {
        if sources & src != 0 {
            for d in dirs(src) {
                v.extend(files(&d).into_iter().map(|(s, _)| s.replace('_', "-")));
            }
        }
    }
    v.sort();
    v.dedup();
    v
}
