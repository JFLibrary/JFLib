// aff -- the affix file (.aff) of a Hunspell dictionary: options, flags, affix rules, the
// suggestion tables (TRY, KEY, REP, MAP)
//
// The file is decoded first (SET: UTF-8, ISO8859-1 .. -16, KOI8-R / -U, microsoft-cp1251 /
// -cp1252, TIS620-2533; charsets.rs) and parsed line by line; the .dic file is decoded the same
// way.  Flags are numbers (u32): FLAG (default) one character each, FLAG long two characters,
// FLAG num comma separated decimals, FLAG UTF-8 one Unicode character each; with AF aliases the
// flag fields of the .dic and of the affixes' continuation classes are alias numbers.
//
// Understood: SET, FLAG, AF, TRY, KEY, REP, MAP, PFX / SFX (strip, add, continuation classes,
// conditions with [..], [^..] and '.', cross products), NEEDAFFIX / PSEUDOROOT, FORBIDDENWORD,
// ONLYINCOMPOUND, KEEPCASE, NOSUGGEST, FORCEUCASE, CIRCUMFIX, COMPOUNDFLAG, COMPOUNDBEGIN /
// MIDDLE / END, COMPOUNDRULE (flags in parentheses for FLAG long / num; '*' and '?'),
// COMPOUNDPERMITFLAG, COMPOUNDFORBIDFLAG, COMPOUNDMIN, COMPOUNDWORDMAX, CHECKCOMPOUNDDUP / CASE /
// REP / TRIPLE, SIMPLIFIEDTRIPLE, CHECKCOMPOUNDPATTERN (with flags; a replacement is ignored),
// CHECKSHARPS, WORDCHARS, IGNORE, ICONV, BREAK, NOSPLITSUGS, FULLSTRIP, LANG.  Ignored
// (documented in lib.rs): COMPOUNDSYLLABLE, COMPOUNDROOT, COMPLEXPREFIXES, OCONV, PHONE, AM and
// the morphological fields, SUBSTANDARD, WARN, the suggestion limits (MAXNGRAMSUGS, MAXDIFF,
// ONLYMAXDIFF, MAXCPDSUGS) and every unknown keyword.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::charsets;

/// a flag (0: an option that is not set)
pub(crate) type Flag = u32;

/// the flag of the hidden Title case entries of mixed case words (dic.rs; Hunspell's
/// ONLYUPCASEFLAG): they count only for a word typed in UPPER case
pub(crate) const ONLYUPCASE: Flag = 0xFFFF_FFF0;

/// a COMPOUNDRULE element: a flag, once (b'1'), any number of times (b'*') or at most once (b'?')
pub(crate) type RuleEl = (Flag, u8);

/// how flags are written (FLAG)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FlagMode {
    Char,
    Long,
    Num,
    Utf8,
}

/// one element of an affix condition
#[derive(Clone, Debug)]
pub(crate) enum CondEl {
    Any,
    Ch(char),
    Set(bool, Vec<char>), // negated, the characters
}

impl CondEl {
    fn hit(&self, c: char) -> bool {
        match self {
            CondEl::Any => true,
            CondEl::Ch(x) => *x == c,
            CondEl::Set(neg, cs) => cs.contains(&c) != *neg,
        }
    }
}

/// a prefix or suffix rule
#[derive(Clone, Debug)]
pub(crate) struct Affix {
    pub(crate) flag: Flag,
    pub(crate) cross: bool,
    /// characters the rule removes from the root (prefix: at its start, suffix: at its end)
    pub(crate) strip: String,
    /// what it adds
    pub(crate) add: String,
    /// the condition on the root (prefix: its start, suffix: its end)
    pub(crate) cond: Vec<CondEl>,
    /// continuation flags (sorted)
    pub(crate) cont: Vec<Flag>,
}

impl Affix {
    /// a prefix's condition holds for the start of head + tail
    pub(crate) fn cond_ok2(&self, head: &str, tail: &str) -> bool {
        let mut it = head.chars().chain(tail.chars());
        self.cond.iter().all(|e| it.next().is_some_and(|c| e.hit(c)))
    }

    /// the condition holds for the start (prefix) / end (suffix) of `root`
    pub(crate) fn cond_ok(&self, root: &str, suffix: bool) -> bool {
        if self.cond.is_empty() {
            return true;
        }
        if suffix {
            let mut it = root.chars().rev();
            self.cond.iter().rev().all(|e| it.next().is_some_and(|c| e.hit(c)))
        } else {
            let mut it = root.chars();
            self.cond.iter().all(|e| it.next().is_some_and(|c| e.hit(c)))
        }
    }

    pub(crate) fn has(&self, f: Flag) -> bool {
        has(&self.cont, f)
    }
}

/// f is in the sorted flag set (0 never is)
pub(crate) fn has(set: &[Flag], f: Flag) -> bool {
    f != 0 && set.binary_search(&f).is_ok()
}

/// the rules with the same `add` and `strip` (they leave the same root: one lookup for all)
#[derive(Clone, Debug, Default)]
pub(crate) struct Group {
    pub(crate) strip: String,
    pub(crate) rules: Vec<u32>,
}

/// the affix rules by their `add` text: sorted keys (texts of up to 7 bytes packed into the
/// number, longer ones hashed), the groups of each text
#[derive(Default)]
pub(crate) struct AddIndex {
    keys: Vec<u64>,
    texts: Vec<String>,
    groups: Vec<Vec<Group>>,
}

fn add_key(s: &str) -> u64 {
    let b = s.as_bytes();
    if b.len() <= 7 {
        b.iter().fold(b.len() as u64, |k, &c| (k << 8) | c as u64)
    } else {
        b.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x0000_0100_0000_01B3)) | (1 << 63)
    }
}

impl AddIndex {
    /// the groups of the rules adding s
    pub(crate) fn get(&self, s: &str) -> Option<&[Group]> {
        let k = add_key(s);
        let mut i = self.keys.partition_point(|&x| x < k);
        while i < self.keys.len() && self.keys[i] == k {
            if self.texts[i] == s {
                return Some(&self.groups[i]);
            }
            i += 1;
        }
        None
    }

    // the index of the rules (i, rule)
    fn build<'r>(rules: impl Iterator<Item = (usize, &'r Affix)>) -> AddIndex {
        let mut map: BTreeMap<(u64, String), Vec<Group>> = BTreeMap::new();
        for (i, r) in rules {
            let gs = map.entry((add_key(&r.add), r.add.clone())).or_default();
            match gs.iter_mut().find(|g| g.strip == r.strip) {
                Some(g) => g.rules.push(i as u32),
                None => gs.push(Group { strip: r.strip.clone(), rules: alloc::vec![i as u32] }),
            }
        }
        let mut x = AddIndex::default();
        for ((k, t), g) in map {
            x.keys.push(k);
            x.texts.push(t);
            x.groups.push(g);
        }
        x
    }
}

/// a REP rule: from -> to, anchored at the word's start / end
#[derive(Clone, Debug)]
pub(crate) struct Rep {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) start: bool,
    pub(crate) end: bool,
}

/// a CHECKCOMPOUNDPATTERN: no compound boundary between a part ending with `end` (whose root
/// has the flag `cond`; end "0": a part without affixes) and one starting with `begin` (root
/// flag `cond2`)
#[derive(Clone, Debug)]
pub(crate) struct CpdPattern {
    pub(crate) end: String,
    pub(crate) begin: String,
    pub(crate) cond: Flag,
    pub(crate) cond2: Flag,
}

impl CpdPattern {
    /// the pattern looks at the parts' roots (flags, "0")
    pub(crate) fn roots(&self) -> bool {
        self.cond != 0 || self.cond2 != 0 || self.end == "0"
    }
}

/// the parsed affix file
pub(crate) struct Aff {
    pub(crate) encoding: String,
    pub(crate) mode: FlagMode,
    pub(crate) aliases: Vec<Vec<Flag>>,
    pub(crate) lang: String,
    pub(crate) try_chars: Vec<char>,
    pub(crate) key: Vec<Vec<char>>,
    pub(crate) rep: Vec<Rep>,
    pub(crate) map: Vec<Vec<String>>,
    pub(crate) pfx: Vec<Affix>,
    pub(crate) sfx: Vec<Affix>,
    /// the rules by their `add` text
    pub(crate) pfx_by_add: AddIndex,
    pub(crate) sfx_by_add: AddIndex,
    /// longest `add` in characters
    pub(crate) pfx_max: usize,
    pub(crate) sfx_max: usize,
    /// suffix flags some suffix's continuation class names (the outer one of two suffixes)
    pub(crate) sfx_outer: Vec<Flag>,
    /// the first bytes of the prefix texts, the last bytes of the suffix texts (not empty)
    pub(crate) pfx_first: [u64; 4],
    pub(crate) sfx_last: [u64; 4],
    pub(crate) forbidden: Flag,
    pub(crate) needaffix: Flag,
    pub(crate) onlyincompound: Flag,
    pub(crate) keepcase: Flag,
    pub(crate) nosuggest: Flag,
    pub(crate) forceucase: Flag,
    pub(crate) circumfix: Flag,
    pub(crate) compound: Flag,
    pub(crate) cpd_begin: Flag,
    pub(crate) cpd_middle: Flag,
    pub(crate) cpd_end: Flag,
    pub(crate) cpd_permit: Flag,
    pub(crate) cpd_forbid: Flag,
    pub(crate) cpd_min: usize,
    pub(crate) cpd_max: usize, // COMPOUNDWORDMAX (0: no limit)
    pub(crate) cpd_dup: bool,
    pub(crate) cpd_case: bool,
    pub(crate) cpd_rep: bool,
    pub(crate) cpd_triple: bool,
    pub(crate) simple_triple: bool,
    pub(crate) cpd_patterns: Vec<CpdPattern>,
    /// some CHECKCOMPOUNDPATTERN looks at the roots of the parts
    pub(crate) cpd_pat_roots: bool,
    pub(crate) cpd_rules: Vec<Vec<RuleEl>>,
    pub(crate) sharps: bool,
    pub(crate) wordchars: Vec<char>,
    pub(crate) ignore: Vec<char>,
    pub(crate) iconv: Vec<(String, String)>,
    pub(crate) breaks: Vec<String>,
    pub(crate) nosplitsugs: bool,
    pub(crate) fullstrip: bool,
    /// keywords seen but not supported (for Dictionary::ignored)
    pub(crate) unsupported: Vec<String>,
}

/// the default FORBIDDENWORD flag (Hunspell's 65510)
const FORBIDDEN_DEFAULT: Flag = 65510;

impl Aff {
    fn new() -> Aff {
        Aff {
            encoding: String::from("ISO8859-1"),
            mode: FlagMode::Char,
            aliases: Vec::new(),
            lang: String::new(),
            try_chars: Vec::new(),
            key: Vec::new(),
            rep: Vec::new(),
            map: Vec::new(),
            pfx: Vec::new(),
            sfx: Vec::new(),
            pfx_by_add: AddIndex::default(),
            sfx_by_add: AddIndex::default(),
            pfx_max: 0,
            sfx_max: 0,
            sfx_outer: Vec::new(),
            pfx_first: [0; 4],
            sfx_last: [0; 4],
            forbidden: FORBIDDEN_DEFAULT,
            needaffix: 0,
            onlyincompound: 0,
            keepcase: 0,
            nosuggest: 0,
            forceucase: 0,
            circumfix: 0,
            compound: 0,
            cpd_begin: 0,
            cpd_middle: 0,
            cpd_end: 0,
            cpd_permit: 0,
            cpd_forbid: 0,
            cpd_min: 3,
            cpd_max: 0,
            cpd_dup: false,
            cpd_case: false,
            cpd_rep: false,
            cpd_triple: false,
            simple_triple: false,
            cpd_patterns: Vec::new(),
            cpd_pat_roots: false,
            cpd_rules: Vec::new(),
            sharps: false,
            wordchars: Vec::new(),
            ignore: Vec::new(),
            iconv: Vec::new(),
            breaks: Vec::new(),
            nosplitsugs: false,
            fullstrip: false,
            unsupported: Vec::new(),
        }
    }

    /// compounds are possible (compound flags or COMPOUNDRULE)
    pub(crate) fn compounds(&self) -> bool {
        self.cpd_flags() || !self.cpd_rules.is_empty()
    }

    /// any compound flag is set
    pub(crate) fn cpd_flags(&self) -> bool {
        self.compound != 0 || self.cpd_begin != 0 || self.cpd_middle != 0 || self.cpd_end != 0
    }

    // a COMPOUNDRULE: flags ("(..)" each for FLAG long / num) and the quantifiers '*' and '?'
    fn rule(&self, s: &str) -> Vec<RuleEl> {
        let mut out: Vec<RuleEl> = Vec::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '*' | '?' => {
                    if let Some(last) = out.last_mut() {
                        last.1 = c as u8;
                    }
                }
                '(' => {
                    let mut g = String::new();
                    for d in it.by_ref() {
                        if d == ')' {
                            break;
                        }
                        g.push(d);
                    }
                    out.push((self.flag1(&g), b'1'));
                }
                c => out.push((self.flag1(c.encode_utf8(&mut [0; 4])), b'1')),
            }
        }
        out.retain(|e| e.0 != 0);
        out
    }

    /// parse the flags of a .dic word or a continuation class (aliases resolved), sorted
    pub(crate) fn flags(&self, s: &str) -> Vec<Flag> {
        if !self.aliases.is_empty() {
            if let Ok(n) = s.trim().parse::<usize>() {
                if n >= 1 && n <= self.aliases.len() {
                    return self.aliases[n - 1].clone();
                }
            }
        }
        let mut v = self.flags_raw(s);
        v.sort_unstable();
        v.dedup();
        v
    }

    // the flags of a field without aliases (unsorted)
    fn flags_raw(&self, s: &str) -> Vec<Flag> {
        match self.mode {
            FlagMode::Char | FlagMode::Utf8 => s.chars().map(|c| c as Flag).collect(),
            FlagMode::Long => {
                let cs: Vec<char> = s.chars().collect();
                cs.chunks(2).map(|p| ((p[0] as Flag & 0xFFFF) << 16) | p.get(1).map_or(0, |&c| c as Flag & 0xFFFF)).collect()
            }
            FlagMode::Num => s.split(',').filter_map(|n| n.trim().parse::<Flag>().ok()).filter(|&f| f != 0).collect(),
        }
    }

    // one flag (an option's value)
    fn flag1(&self, s: &str) -> Flag {
        self.flags_raw(s).first().copied().unwrap_or(0)
    }
}

/// decode a dictionary file in the character set `enc` (SET) to UTF-8; a UTF-8 byte order
/// mark is dropped, invalid UTF-8 is replaced
pub(crate) fn decode(b: &[u8], enc: &str) -> String {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    let e = enc.to_ascii_lowercase().replace(['-', '_'], "");
    if e == "utf8" {
        return String::from_utf8_lossy(b).into_owned();
    }
    let table = charsets::table(enc);
    let mut s = String::with_capacity(b.len() + b.len() / 8);
    for &c in b {
        if c < 0x80 {
            s.push(c as char);
        } else if let Some(t) = table {
            s.push(char::from_u32(t[c as usize - 0x80] as u32).unwrap_or('\u{FFFD}'));
        } else {
            s.push(c as char); // ISO8859-1 (and unknown sets)
        }
    }
    s
}

/// the SET value of an affix file (read from its raw bytes), default ISO8859-1
pub(crate) fn encoding_of(aff: &[u8]) -> String {
    for line in aff.split(|&c| c == b'\n') {
        let l = line.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(line);
        let t: Vec<&[u8]> = l.split(|c| c.is_ascii_whitespace()).filter(|f| !f.is_empty()).collect();
        if t.len() >= 2 && t[0] == b"SET" {
            return String::from_utf8_lossy(t[1]).into_owned();
        }
    }
    if aff.starts_with(&[0xEF, 0xBB, 0xBF]) { String::from("UTF-8") } else { String::from("ISO8859-1") }
}

// a condition text: "." = none, else characters, [..] and [^..] sets, '.' any
fn parse_cond(s: &str) -> Vec<CondEl> {
    if s == "." || s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '[' => {
                let mut j = i + 1;
                let neg = cs.get(j) == Some(&'^');
                if neg {
                    j += 1;
                }
                let mut set = Vec::new();
                while j < cs.len() && cs[j] != ']' {
                    set.push(cs[j]);
                    j += 1;
                }
                out.push(CondEl::Set(neg, set));
                i = j + 1;
            }
            '.' => {
                out.push(CondEl::Any);
                i += 1;
            }
            c => {
                out.push(CondEl::Ch(c));
                i += 1;
            }
        }
    }
    out
}

// "0" = empty; '_' in REP means a space
fn zero(s: &str) -> String {
    if s == "0" { String::new() } else { String::from(s) }
}

// MAP group: characters and (multi character) groups in parentheses
fn parse_map(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '(' => {
                inside = true;
                cur.clear();
            }
            ')' => {
                inside = false;
                if !cur.is_empty() {
                    out.push(core::mem::take(&mut cur));
                }
            }
            c if inside => cur.push(c),
            c => out.push(String::from(c)),
        }
    }
    out
}

/// parse a decoded affix file
pub(crate) fn parse(text: &str, encoding: &str) -> Aff {
    let mut a = Aff::new();
    a.encoding = String::from(encoding);
    let mut breaks: Option<Vec<String>> = None;
    let mut rules_open = false;
    // affix headers: flag -> (entries still expected, cross product)
    let mut pending: BTreeMap<(bool, Flag), (usize, bool)> = BTreeMap::new();
    // (pass 1: FLAG and AF must be known before any flag is read)
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 2 && f[0] == "FLAG" {
            a.mode = match f[1] {
                "long" => FlagMode::Long,
                "num" => FlagMode::Num,
                "UTF-8" | "utf-8" | "UTF8" => FlagMode::Utf8,
                _ => FlagMode::Char,
            };
        }
    }
    let mut af_open = false;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() || f[0].starts_with('#') {
            continue;
        }
        let v1 = f.get(1).copied().unwrap_or("");
        match f[0] {
            "SET" | "FLAG" => {}
            "AF" => {
                // "AF n" opens the table, then "AF flags" rows
                if !af_open && f.len() == 2 && v1.parse::<usize>().is_ok() && a.aliases.is_empty() {
                    af_open = true;
                } else if f.len() >= 2 {
                    let mut v = a.flags_raw(v1);
                    v.sort_unstable();
                    v.dedup();
                    a.aliases.push(v);
                }
            }
            "LANG" => a.lang = String::from(v1),
            "TRY" => a.try_chars = v1.chars().collect(),
            "KEY" => a.key = v1.split('|').map(|r| r.chars().collect()).collect(),
            "WORDCHARS" => a.wordchars = v1.chars().collect(),
            "IGNORE" => a.ignore = v1.chars().collect(),
            "REP" => {
                if f.len() >= 3 {
                    let mut from = String::from(f[1]);
                    let start = from.starts_with('^');
                    if start {
                        from.remove(0);
                    }
                    let end = from.ends_with('$') && from.len() > 1;
                    if end {
                        from.pop();
                    }
                    a.rep.push(Rep { from: from.replace('_', " "), to: f[2].replace('_', " "), start, end });
                }
            }
            "MAP" => {
                if f.len() >= 2 && v1.parse::<usize>().is_err() {
                    a.map.push(parse_map(v1));
                }
            }
            "ICONV" => {
                // (as in Hunspell "0" is the digit here: uk_UA turns Latin letters into "0", so
                // a Latin word becomes a number and counts as correct)
                if f.len() >= 3 {
                    a.iconv.push((String::from(f[1]), f[2].replace('_', " ")));
                }
            }
            "BREAK" => {
                let b = breaks.get_or_insert_with(Vec::new);
                if f.len() >= 2 && v1.parse::<usize>().is_err() {
                    b.push(String::from(v1));
                }
            }
            "CHECKCOMPOUNDPATTERN" => {
                // "end[/flag] begin[/flag] [replacement]" (the count line has one field)
                if f.len() >= 3 {
                    let split = |s: &str| match s.split_once('/') {
                        Some((t, fl)) => (String::from(t), a.flag1(fl)),
                        None => (String::from(s), 0),
                    };
                    let ((end, cond), (begin, cond2)) = (split(f[1]), split(f[2]));
                    let p = CpdPattern { end, begin: zero(&begin), cond, cond2 };
                    a.cpd_pat_roots |= p.roots();
                    a.cpd_patterns.push(p);
                    if f.len() >= 4 && !f[3].starts_with('#') {
                        note(&mut a, "CHECKCOMPOUNDPATTERN (replacement)");
                    }
                }
            }
            "COMPOUNDRULE" => {
                // "COMPOUNDRULE n" opens the table, then one rule per line
                if !rules_open && v1.parse::<usize>().is_ok() {
                    rules_open = true;
                } else if !v1.is_empty() {
                    let r = a.rule(v1);
                    if !r.is_empty() {
                        a.cpd_rules.push(r);
                    }
                }
            }
            "FORBIDDENWORD" => a.forbidden = a.flag1(v1),
            "NEEDAFFIX" | "PSEUDOROOT" => a.needaffix = a.flag1(v1),
            "ONLYINCOMPOUND" => a.onlyincompound = a.flag1(v1),
            "KEEPCASE" => a.keepcase = a.flag1(v1),
            "NOSUGGEST" => a.nosuggest = a.flag1(v1),
            "FORCEUCASE" => a.forceucase = a.flag1(v1),
            "CIRCUMFIX" => a.circumfix = a.flag1(v1),
            "COMPOUNDFLAG" => a.compound = a.flag1(v1),
            "COMPOUNDBEGIN" => a.cpd_begin = a.flag1(v1),
            "COMPOUNDMIDDLE" => a.cpd_middle = a.flag1(v1),
            "COMPOUNDEND" | "COMPOUNDLAST" => a.cpd_end = a.flag1(v1),
            "COMPOUNDPERMITFLAG" => a.cpd_permit = a.flag1(v1),
            "COMPOUNDFORBIDFLAG" => a.cpd_forbid = a.flag1(v1),
            "COMPOUNDMIN" => a.cpd_min = v1.parse::<usize>().unwrap_or(3).max(1),
            "COMPOUNDWORDMAX" => a.cpd_max = v1.parse::<usize>().unwrap_or(0),
            "CHECKCOMPOUNDDUP" => a.cpd_dup = true,
            "CHECKCOMPOUNDCASE" => a.cpd_case = true,
            "CHECKCOMPOUNDREP" => a.cpd_rep = true,
            "CHECKCOMPOUNDTRIPLE" => a.cpd_triple = true,
            "SIMPLIFIEDTRIPLE" => a.simple_triple = true,
            "CHECKSHARPS" => a.sharps = true,
            "NOSPLITSUGS" => a.nosplitsugs = true,
            "FULLSTRIP" => a.fullstrip = true,
            "PFX" | "SFX" => {
                let suffix = f[0] == "SFX";
                if f.len() < 4 {
                    continue;
                }
                let flag = a.flag1(f[1]);
                let key = (suffix, flag);
                let left = pending.get(&key).map_or(0, |p| p.0);
                if left == 0 {
                    // header: PFX flag Y|N count
                    let n = f[3].parse::<usize>().unwrap_or(0);
                    pending.insert(key, (n, f[2] == "Y"));
                    continue;
                }
                let cross = pending.get(&key).is_some_and(|p| p.1);
                if let Some(p) = pending.get_mut(&key) {
                    p.0 -= 1;
                }
                let strip = zero(f[2]);
                let (add, cont) = match f[3].find('/') {
                    Some(i) => (zero(&f[3][..i]), a.flags(&f[3][i + 1..])),
                    None => (zero(f[3]), Vec::new()),
                };
                let add: String = add.chars().filter(|c| !a.ignore.contains(c)).collect();
                let cond = parse_cond(f.get(4).copied().unwrap_or("."));
                let r = Affix { flag, cross, strip, add, cond, cont };
                if suffix { a.sfx.push(r) } else { a.pfx.push(r) }
            }
            "COMPOUNDSYLLABLE" | "COMPOUNDROOT" | "COMPLEXPREFIXES" | "OCONV" | "PHONE" | "AM" | "SUBSTANDARD"
            | "COMPOUNDMORESUFFIXES" | "SYLLABLENUM" => {
                note(&mut a, f[0]);
            }
            _ => {}
        }
    }
    a.breaks = breaks.unwrap_or_else(|| ["-", "^-", "-$"].iter().map(|s| String::from(*s)).collect());
    a.pfx_by_add = AddIndex::build(a.pfx.iter().enumerate());
    a.sfx_by_add = AddIndex::build(a.sfx.iter().enumerate());
    a.pfx_max = a.pfx.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
    a.sfx_max = a.sfx.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
    let mut outer: Vec<Flag> = a.sfx.iter().map(|r| r.flag).filter(|&f| a.sfx.iter().any(|r| r.has(f))).collect();
    outer.sort_unstable();
    outer.dedup();
    a.sfx_outer = outer;
    for r in &a.pfx {
        if let Some(&b) = r.add.as_bytes().first() {
            a.pfx_first[b as usize >> 6] |= 1 << (b & 63);
        }
    }
    for r in &a.sfx {
        if let Some(&b) = r.add.as_bytes().last() {
            a.sfx_last[b as usize >> 6] |= 1 << (b & 63);
        }
    }
    a
}

/// byte b is in the set
pub(crate) fn bit(set: &[u64; 4], b: u8) -> bool {
    set[b as usize >> 6] & (1 << (b & 63)) != 0
}

fn note(a: &mut Aff, k: &str) {
    if !a.unsupported.iter().any(|u| u == k) {
        a.unsupported.push(String::from(k));
    }
}
