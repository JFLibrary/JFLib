// segment -- Unicode text segmentation: grapheme clusters and words (UAX #29), line break
// opportunities (UAX #14), Unicode 15.0 (segment_data.rs, tools/gen_segment.py)
//
//   grapheme_next(s, i) -> index     the end of the grapheme cluster that starts at or holds i
//   grapheme_prev(s, i) -> index     the start of the cluster before i (0 at the start)
//   cluster_end(s, i) -> index       the end of the cluster that starts at boundary i (walks
//                                    over a text cluster by cluster)
//   is_grapheme_boundary(s, i)
//   graphemes(s) -> Vec<usize>       every cluster boundary, 0 and s.len() included
//   pictographic_cluster(s, a, b)    the cluster a..b is an emoji sequence (Backspace deletes it
//                                    whole; other clusters lose their last code point)
//   word_bounds(s) -> Vec<usize>     every word boundary, 0 and s.len() included
//   word_at(s, i) -> (start, end)    the word segment that holds i (at s.len(): the last one)
//   word_kind(s, a, b) -> WK_*       WK_WORD (a letter or number in it), WK_SPACE (only white
//                                    space), WK_PUNCT
//   word_next(s, i) / word_prev(s, i)   Ctrl+Right / Ctrl+Left: the start of the next word
//                                    after i (spaces skipped) / of the word before i
//   line_breaks(s) -> Vec<(usize, bool)>   break opportunities (index where a line may start,
//                                    mandatory), s.len() last (mandatory)
//   wrap_end(s, pos, pe, fits) -> end   where a line from pos ends inside pos..pe: the last
//                                    opportunity whose text (without trailing spaces) fits,
//                                    else graphemes while they fit (at least one); a mandatory
//                                    break inside ends it
//   wrap_end_hyph(s, pos, pe, fits, hyph) -> (end, hyphen)   the same with hyphenation: a line
//                                    may end after a soft hyphen (U+00AD) or, with a hook
//                                    (hyph(word, &mut offsets): where a word may break, e.g.
//                                    spell::Hyphenator::hyphenate), inside the word that does
//                                    not fit; hyphen: draw a hyphen after the line.
//                                    fits(a, b, hyphen): a..b (and a hyphen) fits
//   char_props(c) -> u32             the packed properties (tests, other tables)
//
// Indices are byte offsets into UTF-8 text; an index inside a character is moved to that
// character's start.  Rules: UAX #29 GB3..GB13 and WB3..WB16 of Unicode 15.0 (no GB9c: that came
// with 15.1); UAX #14 LB2..LB31 with the pair rules of LB25 (numbers), LB30 for East Asian
// parentheses and LB30b for emoji modifiers.  Grapheme and word scans around i start at the
// nearest place before i where no rule joins or looks across (usually right there; at the
// latest the paragraph start, the last '\n' before i) and words end at the next such place,
// so the cost does not grow with the length of a line.

use alloc::vec::Vec;

use crate::segment_data::{RUN_PROPS, RUN_START};

// Grapheme_Cluster_Break (bits 0..3; 0 Other)
const G_CR: u32 = 1;
const G_LF: u32 = 2;
const G_CONTROL: u32 = 3;
const G_EXTEND: u32 = 4;
const G_ZWJ: u32 = 5;
const G_RI: u32 = 6;
const G_PREPEND: u32 = 7;
const G_SPACINGMARK: u32 = 8;
const G_L: u32 = 9;
const G_V: u32 = 10;
const G_T: u32 = 11;
const G_LV: u32 = 12;
const G_LVT: u32 = 13;

// Word_Break (bits 4..8; 0 Other)
const W_CR: u32 = 1;
const W_LF: u32 = 2;
const W_NEWLINE: u32 = 3;
const W_EXTEND: u32 = 4;
const W_ZWJ: u32 = 5;
const W_RI: u32 = 6;
const W_FORMAT: u32 = 7;
const W_KATAKANA: u32 = 8;
const W_HL: u32 = 9;
const W_ALETTER: u32 = 10;
const W_SQ: u32 = 11;
const W_DQ: u32 = 12;
const W_MIDNUMLET: u32 = 13;
const W_MIDLETTER: u32 = 14;
const W_MIDNUM: u32 = 15;
const W_NUMERIC: u32 = 16;
const W_EXTENDNUMLET: u32 = 17;
const W_WSEG: u32 = 18;

// Line_Break (bits 9..14; AI, SG, XX, SA and CJ resolved by the generator: LB1)
const L_BK: u32 = 1;
const L_CR: u32 = 2;
const L_LF: u32 = 3;
const L_CM: u32 = 4;
const L_NL: u32 = 5;
const L_WJ: u32 = 7;
const L_ZW: u32 = 8;
const L_GL: u32 = 9;
const L_SP: u32 = 10;
const L_ZWJ: u32 = 11;
const L_B2: u32 = 12;
const L_BA: u32 = 13;
const L_BB: u32 = 14;
const L_HY: u32 = 15;
const L_CB: u32 = 16;
const L_CL: u32 = 17;
const L_CP: u32 = 18;
const L_EX: u32 = 19;
const L_IN: u32 = 20;
const L_NS: u32 = 21;
const L_OP: u32 = 22;
const L_QU: u32 = 23;
const L_IS: u32 = 24;
const L_NU: u32 = 25;
const L_PO: u32 = 26;
const L_PR: u32 = 27;
const L_SY: u32 = 28;
const L_AL: u32 = 30;
const L_EB: u32 = 32;
const L_EM: u32 = 33;
const L_H2: u32 = 34;
const L_H3: u32 = 35;
const L_HL: u32 = 36;
const L_ID: u32 = 37;
const L_JL: u32 = 38;
const L_JV: u32 = 39;
const L_JT: u32 = 40;
const L_RI: u32 = 41;

const P_EXTPICT: u32 = 1 << 15;
const P_WIDE: u32 = 1 << 16;
const P_ALNUM: u32 = 1 << 17;
const P_SPACE: u32 = 1 << 18;
const P_UNASSIGNED: u32 = 1 << 19;

/// word_kind: a word (a letter or number in it)
pub const WK_WORD: u32 = 0;
/// word_kind: white space only
pub const WK_SPACE: u32 = 1;
/// word_kind: anything else (punctuation, symbols)
pub const WK_PUNCT: u32 = 2;

/// the packed properties of a character (see segment_data.rs)
pub fn char_props(c: char) -> u32 {
    let cp = c as u32;
    let i = RUN_START.partition_point(|&s| s <= cp);
    RUN_PROPS[i.saturating_sub(1)]
}

fn gcb(p: u32) -> u32 {
    p & 15
}
fn wb(p: u32) -> u32 {
    p >> 4 & 31
}
fn lbc(p: u32) -> u32 {
    p >> 9 & 63
}

// i moved back to a character start
fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// the start of the paragraph of i (after the last '\n' before i)
fn para_start(s: &str, i: usize) -> usize {
    s.as_bytes()[..i].iter().rposition(|&b| b == b'\n').map_or(0, |k| k + 1)
}

// ------------------------------------------------------------------ grapheme clusters

// the characters with properties pp (before) and p stay in one cluster: ri regional indicators
// in a row before p, emoji the state of GB11 (2: ExtPict Extend* ZWJ before p)
fn joins(pp: u32, p: u32, ri: u32, emoji: u8) -> bool {
    match (gcb(pp), gcb(p)) {
        (G_CR, G_LF) => true,                                              // GB3
        (G_CONTROL | G_CR | G_LF, _) | (_, G_CONTROL | G_CR | G_LF) => false, // GB4, GB5
        (G_L, G_L | G_V | G_LV | G_LVT) => true,                           // GB6
        (G_LV | G_V, G_V | G_T) => true,                                   // GB7
        (G_LVT | G_T, G_T) => true,                                        // GB8
        (_, G_EXTEND | G_ZWJ) => true,                                     // GB9
        (_, G_SPACINGMARK) => true,                                        // GB9a
        (G_PREPEND, _) => true,                                            // GB9b
        _ if p & P_EXTPICT != 0 && emoji == 2 => true,                     // GB11
        (G_RI, G_RI) => ri % 2 == 1,                                       // GB12, GB13
        _ => false,                                                        // GB999
    }
}

// the cluster boundaries of s after `from` (a boundary) in order: at(b) for each (s.len() last),
// true stops
fn grapheme_walk(s: &str, from: usize, at: &mut dyn FnMut(usize) -> bool) {
    let mut prev: Option<u32> = None;
    let mut ri = 0u32; // regional indicators in a row before this character
    let mut emoji = 0u8; // 1: ExtPict Extend*, 2: ExtPict Extend* ZWJ
    for (k, c) in s[from..].char_indices() {
        let i = from + k;
        let p = char_props(c);
        let g = gcb(p);
        if let Some(pp) = prev {
            if !joins(pp, p, ri, emoji) && at(i) {
                return;
            }
        }
        // the state after this character
        ri = if g == G_RI { ri + 1 } else { 0 };
        emoji = if p & P_EXTPICT != 0 {
            1
        } else if emoji != 0 && g == G_EXTEND {
            1
        } else if emoji == 1 && g == G_ZWJ {
            2
        } else {
            0
        };
        prev = Some(p);
    }
    if from < s.len() {
        at(s.len());
    }
}

// a cluster boundary at or before i that a scan may start from: back from i the first place
// whose two characters stay apart in every state (GB11 only after a ZWJ, GB12 only between
// regional indicators), usually i itself; a scan from there sees the boundaries a scan from
// the paragraph start sees (a scan from any boundary does: the state there is the same)
fn cluster_start(s: &str, i: usize) -> usize {
    let mut k = floor(s, i);
    while k > 0 && k < s.len() {
        let (Some(a), Some(c)) = (s[..k].chars().next_back(), s[k..].chars().next()) else { break };
        let pp = char_props(a);
        if !joins(pp, char_props(c), 1, if gcb(pp) == G_ZWJ { 2 } else { 0 }) {
            break;
        }
        k -= a.len_utf8();
    }
    k
}

/// the end of the grapheme cluster that starts at or holds i (s.len() at the end)
pub fn grapheme_next(s: &str, i: usize) -> usize {
    let i = floor(s, i);
    if i >= s.len() {
        return s.len();
    }
    cluster_after(s, cluster_start(s, i), i)
}

/// the end of the grapheme cluster that starts at i, a cluster boundary (grapheme_next without
/// looking back: walks from boundary to boundary)
pub fn cluster_end(s: &str, i: usize) -> usize {
    let i = floor(s, i);
    let Some(c) = s.get(i..).and_then(|r| r.chars().next()) else { return s.len() };
    let n = i + c.len_utf8();
    // the common case: ASCII after a character that does not hold on to what follows (only a
    // Prepend character or CR before LF join an ASCII character)
    let pc = char_props(c);
    match s.as_bytes().get(n) {
        None => n,
        Some(&b) if b < 0x80 && gcb(pc) != G_PREPEND && !(gcb(pc) == G_CR && b == b'\n') => n,
        _ => cluster_after(s, i, i),
    }
}

// the first cluster boundary after i, walking from boundary `from` <= i
fn cluster_after(s: &str, from: usize, i: usize) -> usize {
    let mut end = s.len();
    grapheme_walk(s, from, &mut |b| {
        end = b;
        b > i
    });
    end
}

/// the start of the grapheme cluster before i (0 at the start)
pub fn grapheme_prev(s: &str, i: usize) -> usize {
    let i = floor(s, i);
    if i == 0 {
        return 0;
    }
    let from = cluster_start(s, i - 1);
    let mut last = from;
    grapheme_walk(s, from, &mut |b| {
        if b < i {
            last = b;
        }
        b >= i
    });
    last
}

/// i is a cluster boundary
pub fn is_grapheme_boundary(s: &str, i: usize) -> bool {
    if i == 0 || i >= s.len() {
        return i <= s.len();
    }
    if !s.is_char_boundary(i) {
        return false;
    }
    let from = cluster_start(s, i);
    let mut hit = from == i;
    if !hit {
        grapheme_walk(s, from, &mut |b| {
            hit = b == i;
            b >= i
        });
    }
    hit
}

/// every cluster boundary of s (0 and s.len() included)
pub fn graphemes(s: &str) -> Vec<usize> {
    let mut b = alloc::vec![0];
    grapheme_walk(s, 0, &mut |x| {
        b.push(x);
        false
    });
    b
}

/// the cluster a..b is an emoji sequence (pictographs, flags, keycaps, ZWJ sequences)
pub fn pictographic_cluster(s: &str, a: usize, b: usize) -> bool {
    let t = &s[floor(s, a)..floor(s, b)];
    t.chars().count() > 1
        && t.chars().any(|c| {
            let p = char_props(c);
            p & P_EXTPICT != 0 || gcb(p) == G_RI || gcb(p) == G_ZWJ || c == '\u{20E3}' || c == '\u{FE0F}'
        })
}

// ------------------------------------------------------------------ words

fn ignorable(w: u32) -> bool {
    matches!(w, W_EXTEND | W_FORMAT | W_ZWJ)
}

fn ahletter(w: u32) -> bool {
    w == W_ALETTER || w == W_HL
}

fn midnumletq(w: u32) -> bool {
    w == W_MIDNUMLET || w == W_SQ
}

/// every word boundary of s (0 and s.len() included)
pub fn word_bounds(s: &str) -> Vec<usize> {
    let ch: Vec<(usize, u32)> = s.char_indices().map(|(i, c)| (i, char_props(c))).collect();
    let n = ch.len();
    let w = |k: usize| wb(ch[k].1);
    // the non-ignorable character before index k (WB4), and the one after k
    let before = |k: usize| -> Option<usize> { (0..k).rev().find(|&j| !ignorable(w(j))) };
    let after = |k: usize| -> Option<usize> { (k + 1..n).find(|&j| !ignorable(w(j))) };
    let mut out = alloc::vec![0];
    for k in 1..n {
        let a = w(k - 1);
        let b = w(k);
        let keep = if a == W_CR && b == W_LF {
            true // WB3
        } else if matches!(a, W_NEWLINE | W_CR | W_LF) || matches!(b, W_NEWLINE | W_CR | W_LF) {
            false // WB3a, WB3b
        } else if a == W_ZWJ && ch[k].1 & P_EXTPICT != 0 {
            true // WB3c
        } else if a == W_WSEG && b == W_WSEG {
            true // WB3d
        } else if ignorable(b) {
            true // WB4
        } else {
            // the rules see the characters without Extend / Format / ZWJ
            let p1 = before(k);
            let x = p1.map_or(a, w);
            let x0 = p1.and_then(before).map(w); // the one before x
            let y = b;
            let y2 = after(k).map(w); // the one after y
            let ri_odd = || {
                let mut c = 0;
                let mut j = p1;
                while let Some(jj) = j {
                    if w(jj) != W_RI {
                        break;
                    }
                    c += 1;
                    j = before(jj);
                }
                c % 2 == 1
            };
            (ahletter(x) && ahletter(y))                                                      // WB5
                || (ahletter(x) && (y == W_MIDLETTER || midnumletq(y)) && y2.is_some_and(ahletter)) // WB6
                || (x0.is_some_and(ahletter) && (x == W_MIDLETTER || midnumletq(x)) && ahletter(y)) // WB7
                || (x == W_HL && y == W_SQ)                                                   // WB7a
                || (x == W_HL && y == W_DQ && y2 == Some(W_HL))                               // WB7b
                || (x0 == Some(W_HL) && x == W_DQ && y == W_HL)                               // WB7c
                || (x == W_NUMERIC && y == W_NUMERIC)                                         // WB8
                || (ahletter(x) && y == W_NUMERIC)                                            // WB9
                || (x == W_NUMERIC && ahletter(y))                                            // WB10
                || (x0 == Some(W_NUMERIC) && (x == W_MIDNUM || midnumletq(x)) && y == W_NUMERIC) // WB11
                || (x == W_NUMERIC && (y == W_MIDNUM || midnumletq(y)) && y2 == Some(W_NUMERIC)) // WB12
                || (x == W_KATAKANA && y == W_KATAKANA)                                       // WB13
                || ((ahletter(x) || matches!(x, W_NUMERIC | W_KATAKANA | W_EXTENDNUMLET)) && y == W_EXTENDNUMLET) // WB13a
                || (x == W_EXTENDNUMLET && (ahletter(y) || matches!(y, W_NUMERIC | W_KATAKANA))) // WB13b
                || (x == W_RI && y == W_RI && ri_odd())                                       // WB15, WB16
        };
        if !keep {
            out.push(ch[k].0);
        }
    }
    if s.is_empty() {
        return out;
    }
    out.push(s.len());
    out
}

/// the kind of the segment a..b (WK_WORD, WK_SPACE, WK_PUNCT)
pub fn word_kind(s: &str, a: usize, b: usize) -> u32 {
    let t = &s[floor(s, a)..floor(s, b)];
    let mut space = !t.is_empty();
    for c in t.chars() {
        let p = char_props(c);
        // letters, numbers and the joiners of a word (_, ' in can't) make a word
        if p & P_ALNUM != 0 || wb(p) == W_EXTENDNUMLET || matches!(lbc(p), L_ID | L_H2 | L_H3 | L_JL | L_JV | L_JT) {
            return WK_WORD;
        }
        if p & P_SPACE == 0 && !ignorable(wb(p)) {
            space = false;
        }
    }
    if space { WK_SPACE } else { WK_PUNCT }
}

// a word boundary at k (a character start inside s) that no rule looks across: neither
// character is ignored (WB4); a line break on one side, or Other or white space next to
// something else -- every rule that keeps two characters together or looks past one needs
// letters, digits, joiners, quotes or regional indicators on both sides
fn word_hard(s: &str, k: usize) -> bool {
    let (Some(a), Some(b)) = (s[..k].chars().next_back(), s[k..].chars().next()) else { return true };
    let (x, y) = (wb(char_props(a)), wb(char_props(b)));
    if ignorable(x) || ignorable(y) {
        return false;
    }
    if matches!(x, W_NEWLINE | W_CR | W_LF) || matches!(y, W_NEWLINE | W_CR | W_LF) {
        return !(x == W_CR && y == W_LF);
    }
    x == 0 || y == 0 || (x == W_WSEG) != (y == W_WSEG)
}

// how far word_window looks for a boundary no rule crosses before it takes the paragraph
const WORD_REACH: usize = 256;

// the word segments around i: (bounds, offset) between boundaries no rule looks across (a word
// scan from there finds the bounds a scan of the whole paragraph finds), else the paragraph
fn para_words(s: &str, i: usize) -> (Vec<usize>, usize) {
    let mut a = i;
    let mut n = 0;
    while a > 0 && !word_hard(s, a) && n < WORD_REACH {
        a = floor(s, a - 1);
        n += 1;
    }
    if a > 0 && !word_hard(s, a) {
        a = para_start(s, i);
    }
    let mut e = i + s[i..].chars().next().map_or(0, |c| c.len_utf8());
    n = 0;
    while e < s.len() && !word_hard(s, e) && n < WORD_REACH {
        e += s[e..].chars().next().map_or(1, |c| c.len_utf8());
        n += 1;
    }
    if e < s.len() && !word_hard(s, e) {
        e = s.as_bytes()[i..].iter().position(|&b| b == b'\n').map_or(s.len(), |k| i + k + 1);
    }
    (word_bounds(&s[a..e]).into_iter().map(|x| x + a).collect(), a)
}

/// the word segment that holds i (at the end of the text: the last one)
pub fn word_at(s: &str, i: usize) -> (usize, usize) {
    let i = floor(s, i);
    if s.is_empty() {
        return (0, 0);
    }
    let i = if i >= s.len() { floor(s, s.len() - 1) } else { i };
    let (b, _) = para_words(s, i);
    let k = b.partition_point(|&x| x <= i);
    let start = b[k.saturating_sub(1)];
    let end = b.get(k).copied().unwrap_or(s.len());
    (start, end)
}

/// Ctrl+Right: the start of the next word after i (the rest of this word and the spaces after
/// it skipped)
pub fn word_next(s: &str, i: usize) -> usize {
    let mut p = floor(s, i);
    if p >= s.len() {
        return s.len();
    }
    let (_, e) = word_at(s, p);
    p = e;
    while p < s.len() {
        let (a, e) = word_at(s, p);
        if word_kind(s, a, e) != WK_SPACE || s[a..e].contains('\n') {
            break;
        }
        p = e;
    }
    p
}

/// Ctrl+Left: the start of the word before i (spaces before i skipped)
pub fn word_prev(s: &str, i: usize) -> usize {
    let mut p = floor(s, i);
    while p > 0 {
        let (a, e) = word_at(s, p - 1);
        if word_kind(s, a, e) != WK_SPACE || s[a..e].contains('\n') {
            return a;
        }
        p = a;
    }
    0
}

// ------------------------------------------------------------------ line breaks (UAX #14)

/// the break opportunities of s: (index where a line may start, mandatory); the end of the
/// text is the last entry (mandatory)
pub fn line_breaks(s: &str) -> Vec<(usize, bool)> {
    let ch: Vec<(usize, u32)> = s.char_indices().map(|(i, c)| (i, char_props(c))).collect();
    let n = ch.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    // LB9 / LB10: combining marks and ZWJ take the class of their base; alone they are AL
    let mut cls: Vec<u32> = Vec::with_capacity(n);
    let mut attached: Vec<bool> = Vec::with_capacity(n);
    for k in 0..n {
        let c = lbc(ch[k].1);
        if matches!(c, L_CM | L_ZWJ) {
            if let Some(&base) = cls.last() {
                if !matches!(base, L_BK | L_CR | L_LF | L_NL | L_SP | L_ZW) {
                    cls.push(base);
                    attached.push(true);
                    continue;
                }
            }
            cls.push(L_AL);
            attached.push(false);
            continue;
        }
        cls.push(c);
        attached.push(false);
    }
    let wide = |k: usize| ch[k].1 & P_WIDE != 0;
    let mut ri = 0u32; // regional indicators in a row before k (attached marks skipped)
    let mut sp_before: Option<u32> = None; // the class before a run of spaces ending at k-1
    for k in 1..n {
        let a = cls[k - 1];
        let b = cls[k];
        let raw_a = lbc(ch[k - 1].1);
        // the class before the spaces (LB8, LB14 .. LB17)
        if a == L_SP {
            if k >= 2 && cls[k - 2] != L_SP {
                sp_before = Some(cls[k - 2]);
            } else if k < 2 {
                sp_before = None;
            }
        } else {
            sp_before = None;
        }
        if cls[k - 1] == L_RI && !attached[k - 1] {
            ri += 1;
        } else if !attached[k - 1] {
            ri = 0;
        }
        // mandatory breaks (LB4, LB5)
        let mandatory = match a {
            L_BK | L_NL | L_LF => true,
            L_CR => b != L_LF,
            _ => false,
        };
        if mandatory {
            out.push((ch[k].0, true));
            continue;
        }
        let brk = if a == L_CR && b == L_LF {
            false // LB5
        } else if matches!(b, L_BK | L_CR | L_LF | L_NL) {
            false // LB6
        } else if b == L_SP || b == L_ZW {
            false // LB7
        } else if a == L_ZW || (a == L_SP && sp_before == Some(L_ZW)) {
            true // LB8
        } else if raw_a == L_ZWJ {
            false // LB8a
        } else if attached[k] {
            false // LB9
        } else if a == L_WJ || b == L_WJ {
            false // LB11
        } else if a == L_GL {
            false // LB12
        } else if b == L_GL && !matches!(a, L_SP | L_BA | L_HY) {
            false // LB12a
        } else if matches!(b, L_CL | L_CP | L_EX | L_IS | L_SY) {
            false // LB13
        } else if a == L_OP || (a == L_SP && sp_before == Some(L_OP)) {
            false // LB14
        } else if b == L_OP && (a == L_QU || (a == L_SP && sp_before == Some(L_QU))) {
            false // LB15
        } else if b == L_NS && (matches!(a, L_CL | L_CP) || (a == L_SP && matches!(sp_before, Some(L_CL | L_CP)))) {
            false // LB16
        } else if b == L_B2 && (a == L_B2 || (a == L_SP && sp_before == Some(L_B2))) {
            false // LB17
        } else if a == L_SP {
            true // LB18
        } else if a == L_QU || b == L_QU {
            false // LB19
        } else if a == L_CB || b == L_CB {
            true // LB20
        } else if matches!(b, L_BA | L_HY | L_NS) || a == L_BB {
            false // LB21
        } else if matches!(a, L_HY | L_BA) && k >= 2 && cls[k - 2] == L_HL {
            false // LB21a
        } else if a == L_SY && b == L_HL {
            false // LB21b
        } else if b == L_IN {
            false // LB22
        } else if (matches!(a, L_AL | L_HL) && b == L_NU) || (a == L_NU && matches!(b, L_AL | L_HL)) {
            false // LB23
        } else if (a == L_PR && matches!(b, L_ID | L_EB | L_EM)) || (matches!(a, L_ID | L_EB | L_EM) && b == L_PO) {
            false // LB23a
        } else if (matches!(a, L_PR | L_PO) && matches!(b, L_AL | L_HL)) || (matches!(a, L_AL | L_HL) && matches!(b, L_PR | L_PO)) {
            false // LB24
        } else if matches!(
            (a, b),
            (L_CL | L_CP | L_NU, L_PO | L_PR) | (L_PO | L_PR, L_OP | L_NU) | (L_HY | L_IS | L_NU | L_SY, L_NU)
        ) {
            false // LB25 (the pair rules)
        } else if (a == L_JL && matches!(b, L_JL | L_JV | L_H2 | L_H3))
            || (matches!(a, L_JV | L_H2) && matches!(b, L_JV | L_JT))
            || (matches!(a, L_JT | L_H3) && b == L_JT)
        {
            false // LB26
        } else if (matches!(a, L_JL | L_JV | L_JT | L_H2 | L_H3) && b == L_PO) || (a == L_PR && matches!(b, L_JL | L_JV | L_JT | L_H2 | L_H3)) {
            false // LB27
        } else if matches!(a, L_AL | L_HL) && matches!(b, L_AL | L_HL) {
            false // LB28
        } else if a == L_IS && matches!(b, L_AL | L_HL) {
            false // LB29
        } else if (matches!(a, L_AL | L_HL | L_NU) && b == L_OP && !wide(k)) || (a == L_CP && !wide(k - 1) && matches!(b, L_AL | L_HL | L_NU)) {
            false // LB30
        } else if a == L_RI && b == L_RI && ri % 2 == 1 {
            false // LB30a
        } else if (a == L_EB || (ch[k - 1].1 & P_EXTPICT != 0 && ch[k - 1].1 & P_UNASSIGNED != 0)) && b == L_EM {
            false // LB30b
        } else {
            true // LB31
        };
        if brk {
            out.push((ch[k].0, false));
        }
    }
    out.push((s.len(), true));
    out
}

/// where a line starting at pos ends inside pos..pe (pos < end <= pe; pe when pos == pe): the
/// whole rest when it fits, else the last break opportunity whose text before it (without
/// trailing spaces) fits, else grapheme clusters while they fit (at least one); a mandatory
/// break inside ends the line there.  fits(a, b): the text a..b fits.
pub fn wrap_end(s: &str, pos: usize, pe: usize, fits: &mut dyn FnMut(usize, usize) -> bool) -> usize {
    let pe = floor(s, pe);
    if pos >= pe {
        return pe;
    }
    let seg = &s[pos..pe];
    let ops = line_breaks(seg);
    // a mandatory break before the end
    let hard = ops.iter().find(|o| o.1 && pos + o.0 < pe).map(|o| pos + o.0);
    let limit = hard.unwrap_or(pe);
    let visible = |e: usize| {
        let mut v = e;
        while v > pos {
            let c = s[..v].chars().next_back().unwrap_or('a');
            if char_props(c) & P_SPACE == 0 {
                break;
            }
            v -= c.len_utf8();
        }
        v
    };
    if fits(pos, visible(limit)) {
        return limit;
    }
    let mut best = pos;
    for &(o, _) in &ops {
        let e = pos + o;
        if e > limit || e == pos {
            continue;
        }
        if !fits(pos, visible(e)) {
            break;
        }
        best = e;
    }
    if best > pos {
        return best;
    }
    // one long piece: grapheme clusters while they fit (at least one)
    let mut best = grapheme_next(s, pos).min(limit);
    while best < limit {
        let n = grapheme_next(s, best).min(limit);
        if !fits(pos, n) {
            break;
        }
        best = n;
    }
    best
}

/// wrap_end with hyphenation: where a line starting at pos ends inside pos..pe, and whether
/// it ends with a hyphen (to be drawn after its text).  As wrap_end, and: an opportunity
/// after a soft hyphen (U+00AD) counts with the hyphen's width; when the next word does not
/// fit, hyph(word, &mut offsets) gives the places where that whole word (UAX #29) may break
/// (byte offsets inside it, ascending) and the line ends at the last one that fits with a
/// hyphen -- also inside a single word too long for a line, before graphemes are cut.  Words
/// with a digit or a soft hyphen of their own are never hyphenated.  fits(a, b, hyphen):
/// the text a..b fits (with a hyphen after it).
pub fn wrap_end_hyph(
    s: &str,
    pos: usize,
    pe: usize,
    fits: &mut dyn FnMut(usize, usize, bool) -> bool,
    mut hyph: Option<&mut dyn FnMut(&str, &mut Vec<usize>)>,
) -> (usize, bool) {
    let pe = floor(s, pe);
    if pos >= pe {
        return (pe, false);
    }
    let seg = &s[pos..pe];
    let ops = line_breaks(seg);
    let hard = ops.iter().find(|o| o.1 && pos + o.0 < pe).map(|o| pos + o.0);
    let limit = hard.unwrap_or(pe);
    let visible = |e: usize| {
        let mut v = e;
        while v > pos {
            let c = s[..v].chars().next_back().unwrap_or('a');
            if char_props(c) & P_SPACE == 0 {
                break;
            }
            v -= c.len_utf8();
        }
        v
    };
    let shy = |e: usize| s[..e].ends_with('\u{AD}');
    if fits(pos, visible(limit), false) {
        return (limit, false);
    }
    // the last opportunity that fits (after a soft hyphen: with the hyphen); the piece after
    // it ends at the next opportunity
    let mut best = pos;
    let mut best_h = false;
    let mut next = limit;
    for &(o, _) in &ops {
        let e = pos + o;
        if e > limit || e == pos {
            continue;
        }
        let h = shy(e);
        if fits(pos, visible(e), h) {
            best = e;
            best_h = h;
        } else if !h {
            next = e;
            break;
        }
    }
    if next <= best {
        next = ops.iter().map(|o| pos + o.0).find(|&e| e > best && e <= limit).unwrap_or(limit);
    }
    // hyphenation inside the words of the piece best..next
    if let Some(h) = hyph.as_mut() {
        let mut words: Vec<(usize, usize)> = Vec::new();
        let mut q = best;
        while q < next {
            let (a, b) = word_at(s, q);
            if b <= q {
                break;
            }
            let w = &s[a..b];
            if word_kind(s, a, b) == WK_WORD && !w.contains('\u{AD}') && !w.chars().any(|c| c.is_numeric()) {
                words.push((a, b));
            }
            q = b;
        }
        let mut offs = Vec::new();
        for &(a, b) in words.iter().rev() {
            offs.clear();
            h(&s[a..b], &mut offs);
            for &o in offs.iter().rev() {
                let e = a + o;
                if e > pos && e > best && e < b && e <= next && s.is_char_boundary(e) && fits(pos, e, true) {
                    return (e, true);
                }
            }
        }
    }
    if best > pos {
        return (best, best_h);
    }
    // one long piece: grapheme clusters while they fit (at least one)
    let mut end = grapheme_next(s, pos).min(limit);
    while end < limit {
        let n = grapheme_next(s, end).min(limit);
        if !fits(pos, n, false) {
            break;
        }
        end = n;
    }
    (end, false)
}

/// the Unicode version of the tables
pub fn unicode_version() -> &'static str {
    crate::segment_data::UNICODE_VERSION
}
