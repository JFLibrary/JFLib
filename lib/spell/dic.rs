// dic -- the word list (.dic) of a Hunspell dictionary: an open-addressing hash table over one
// text arena (a slot holds 8 bits of the hash besides the entry: most misses never touch the
// text); homonyms (the same word with other flags) are chained.  Flag sets are stored once
// each (most words share a few).
//
// .dic lines: "word/flags morph..." -- the first line is the (approximate) count, "\/" is a
// slash inside the word, the morphological fields (after a tab, or after a blank followed by
// "xx:") are dropped, IGNORE characters are removed.  A mixed case word ("KdF", "iPhone") or an
// UPPER one with flags also gets a hidden Title case entry with ONLYUPCASE ("Kdf"), as in
// Hunspell: "KDF" is checked as "Kdf", "Kdf" typed is not a word (check.rs).  Only when the
// Title form is no word of its own: a hidden entry is dropped for a real one ("Hf" beside
// "HF/SM": "HFS" stays wrong), and a real one coming later takes its place.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::aff::{has, Aff, Flag, ONLYUPCASE};
use crate::check::{captype, lower, title, Cap};

const EMPTY: u32 = u32::MAX;
/// the entry bits of a slot (the top 8 bits: the hash's)
const IDX: u32 = 0x00FF_FFFF;

/// one word of the table
#[derive(Clone, Copy)]
struct Ent {
    off: u32,
    len: u32,
    set: u32,  // flag set index
    next: u32, // next homonym or EMPTY
}

/// the words and their flag sets
pub(crate) struct Words {
    text: String,
    ents: Vec<Ent>,
    slots: Vec<u32>, // hash tag << 24 | first entry of a chain; EMPTY
    sets: Vec<Vec<Flag>>,
    set_ids: BTreeMap<Vec<Flag>, u32>,
    /// some entry has a space ("a cappella")
    pub(crate) spaced: bool,
}

fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in s.as_bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h ^ (h >> 29)
}

impl Words {
    pub(crate) fn new() -> Words {
        let mut w = Words { text: String::new(), ents: Vec::new(), slots: Vec::new(), sets: Vec::new(), set_ids: BTreeMap::new(), spaced: false };
        w.set_id(Vec::new()); // set 0: no flags
        w.rehash(1024);
        w
    }

    fn set_id(&mut self, flags: Vec<Flag>) -> u32 {
        if let Some(&i) = self.set_ids.get(&flags) {
            return i;
        }
        let i = self.sets.len() as u32;
        self.sets.push(flags.clone());
        self.set_ids.insert(flags, i);
        i
    }

    fn rehash(&mut self, n: usize) {
        let n = n.next_power_of_two();
        self.slots = alloc::vec![EMPTY; n];
        let mask = n - 1;
        // newest first: a chain's head is its newest entry (`next` points to older ones)
        for i in (0..self.ents.len()).rev() {
            let e = self.ents[i];
            let w = &self.text[e.off as usize..(e.off + e.len) as usize];
            let (s, found, tag) = self.find_slot(w, mask);
            if !found {
                self.slots[s] = tag | i as u32;
            }
        }
    }

    // the slot of word w: (index, found, the tag bits of its hash)
    fn find_slot(&self, w: &str, mask: usize) -> (usize, bool, u32) {
        let h = hash(w);
        let tag = ((h >> 56) as u32) << 24;
        let mut s = h as usize & mask;
        loop {
            let v = self.slots[s];
            if v == EMPTY {
                return (s, false, tag);
            }
            if v & !IDX == tag {
                let e = self.ents[(v & IDX) as usize];
                if &self.text[e.off as usize..(e.off + e.len) as usize] == w {
                    return (s, true, tag);
                }
            }
            s = (s + 1) & mask;
        }
    }

    /// add a word with its flags (a homonym when the word is there with other flags)
    pub(crate) fn insert(&mut self, w: &str, flags: Vec<Flag>) {
        self.insert2(w, flags, false);
    }

    // add a word; hidden: a hidden Title entry (dropped when the word is there; a real word
    // replaces a hidden entry's flags)
    fn insert2(&mut self, w: &str, flags: Vec<Flag>, hidden: bool) {
        if w.is_empty() || w.len() > 0xFFFF || self.ents.len() >= IDX as usize {
            return;
        }
        let set = self.set_id(flags);
        let mask = self.slots.len() - 1;
        let (s, found, tag) = self.find_slot(w, mask);
        if found {
            if hidden {
                return;
            }
            // the same flags again: nothing to add; a hidden entry: the real word's flags now
            let mut i = self.slots[s] & IDX;
            while i != EMPTY {
                let e = &mut self.ents[i as usize];
                if e.set == set {
                    return;
                }
                if has(&self.sets[e.set as usize], ONLYUPCASE) {
                    e.set = set;
                    return;
                }
                i = e.next;
            }
        }
        let off = self.text.len() as u32;
        self.spaced |= w.contains(' ');
        self.text.push_str(w);
        let idx = self.ents.len() as u32;
        let next = if found { self.slots[s] & IDX } else { EMPTY };
        self.ents.push(Ent { off, len: w.len() as u32, set, next });
        self.slots[s] = tag | idx;
        if !found && self.ents.len() * 4 > self.slots.len() * 3 {
            self.rehash(self.slots.len() * 2);
        }
    }

    /// the flag sets of word w (one per homonym)
    pub(crate) fn get<'a>(&'a self, w: &str) -> Homonyms<'a> {
        let mask = self.slots.len() - 1;
        let (s, found, _) = self.find_slot(w, mask);
        Homonyms { words: self, i: if found { self.slots[s] & IDX } else { EMPTY } }
    }

    /// the number of the flag set `flags` (0: none of them; Hit::set)
    pub(crate) fn set_index(&self, flags: &[Flag]) -> u32 {
        self.set_ids.get(flags).copied().unwrap_or(0)
    }

    /// flag set i
    pub(crate) fn set(&self, i: u32) -> &[Flag] {
        self.sets.get(i as usize).map_or(&[], |s| s.as_slice())
    }

    /// every entry: (word, flags)
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &[Flag])> {
        self.ents.iter().map(move |e| (&self.text[e.off as usize..(e.off + e.len) as usize], self.sets[e.set as usize].as_slice()))
    }

    /// entry i of iter(): (word, flags)
    pub(crate) fn entry(&self, i: usize) -> (&str, &[Flag]) {
        let e = self.ents[i];
        (&self.text[e.off as usize..(e.off + e.len) as usize], self.sets[e.set as usize].as_slice())
    }

    pub(crate) fn len(&self) -> usize {
        self.ents.len()
    }
}

/// the flag sets of one word's homonyms
#[derive(Clone, Copy)]
pub(crate) struct Homonyms<'a> {
    words: &'a Words,
    i: u32,
}

impl<'a> Iterator for Homonyms<'a> {
    type Item = &'a [Flag];
    fn next(&mut self) -> Option<&'a [Flag]> {
        if self.i == EMPTY {
            return None;
        }
        let e = self.words.ents[self.i as usize];
        self.i = e.next;
        Some(self.words.sets[e.set as usize].as_slice())
    }
}

/// the word and the flag text of a .dic line (None: an empty line)
pub(crate) fn split_line(line: &str) -> Option<(String, &str)> {
    let mut line = line;
    // morphological fields: after a tab, or after a blank followed by "xx:"
    if let Some(t) = line.find('\t') {
        line = &line[..t];
    }
    let b = line.as_bytes();
    let mut cut = b.len();
    for i in 0..b.len() {
        if b[i] == b' ' && i + 3 < b.len() && b[i + 1].is_ascii_alphabetic() && b[i + 2].is_ascii_alphabetic() && b[i + 3] == b':' {
            cut = i;
            break;
        }
    }
    let line = line[..cut].trim_end();
    if line.is_empty() {
        return None;
    }
    // the first unescaped '/' starts the flags
    let mut word = String::new();
    let mut flags = "";
    let mut it = line.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '\\' && it.peek().is_some_and(|&(_, n)| n == '/') {
            word.push('/');
            it.next();
            continue;
        }
        if c == '/' && i > 0 {
            flags = &line[i + 1..];
            break;
        }
        word.push(c);
    }
    // a word without flags may still have trailing blanks + something unknown
    let word = String::from(word.trim_end());
    if word.is_empty() { None } else { Some((word, flags.trim())) }
}

/// read the lines of a decoded .dic file into `words`
pub(crate) fn load(words: &mut Words, aff: &Aff, text: &str) {
    let mut lines = text.lines();
    // the count line (skipped when it is one)
    let first = lines.next().unwrap_or("");
    let hint = first.trim().parse::<usize>().ok();
    if let Some(n) = hint {
        let want = (words.len() + n.min(4_000_000)) * 4 / 3 + 16;
        if want > words.slots.len() {
            words.rehash(want);
        }
    } else {
        add_line(words, aff, first);
    }
    for line in lines {
        add_line(words, aff, line);
    }
}

fn add_line(words: &mut Words, aff: &Aff, line: &str) {
    let Some((mut word, flags)) = split_line(line) else { return };
    if !aff.ignore.is_empty() {
        word.retain(|c| !aff.ignore.contains(&c));
    }
    let flags = if flags.is_empty() { Vec::new() } else { aff.flags(flags) };
    add(words, aff, &word, flags);
}

/// a word of the .dic (or added later) with its flags, and the hidden Title case entry of a
/// mixed case word or an UPPER one with flags (not for forbidden words)
pub(crate) fn add(words: &mut Words, aff: &Aff, w: &str, flags: Vec<Flag>) {
    let cap = captype(w, false);
    let hidden = (matches!(cap, Cap::Huh | Cap::HuhInit) || (cap == Cap::All && !flags.is_empty())) && !has(&flags, aff.forbidden);
    if hidden {
        let mut f2 = flags.clone();
        f2.push(ONLYUPCASE);
        f2.sort_unstable();
        f2.dedup();
        words.insert(w, flags);
        words.insert2(&title(&lower(w)), f2, true);
    } else {
        words.insert(w, flags);
    }
}
