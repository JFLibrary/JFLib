// strings -- the document's string table: names, formulas and texts kept once, as Sym
//
//   Strings::new()                  "" is Sym::EMPTY, then the names the crate knows
//   s.intern("PinX") -> Sym         the string's handle (the same text -> the same Sym)
//   s.get("PinX") -> Option<Sym>    without adding it
//   s.str(sym) -> &str
//
// A drawing with thousands of shapes repeats a few hundred cell names and formulas many times
// (THEMEVAL(), GUARD(Width*0.5) ...); each text is kept once and a cell holds four-byte
// handles.  The table only grows: a Sym stays valid as long as its document, so undo steps can
// keep them.  A Sym belongs to one document; between documents texts go through str().

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

/// a text of a document's string table (cell and row names, formulas, values, raw XML)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Sym(pub(crate) u32);

impl Sym {
    /// the empty text
    pub const EMPTY: Sym = Sym(0);

    /// the position in the table (0 = the empty text)
    pub fn index(self) -> u32 {
        self.0
    }
}

const NONE: u32 = u32::MAX;

/// the string table of a document
#[derive(Clone)]
pub struct Strings {
    texts: Vec<Box<str>>,
    // open addressing over the indices of texts (NONE = free); power of two, at most half full
    slots: Vec<u32>,
}

fn hash(s: &str) -> u64 {
    // FNV-1a: short keys, no allocation, the same on every target
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl core::fmt::Debug for Strings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Strings({} texts)", self.texts.len())
    }
}

impl Default for Strings {
    fn default() -> Self {
        Strings::new()
    }
}

impl Strings {
    /// a table with the empty text and the names the crate itself refers to
    pub fn new() -> Self {
        let mut s = Strings { texts: Vec::new(), slots: vec![NONE; 1024] };
        s.intern("");
        for name in crate::names::KNOWN {
            s.intern(name);
        }
        s
    }

    /// the handle of `text`, added when it is new
    pub fn intern(&mut self, text: &str) -> Sym {
        if let Some(sym) = self.get(text) {
            return sym;
        }
        if (self.texts.len() + 1) * 2 > self.slots.len() {
            self.grow();
        }
        let id = self.texts.len() as u32;
        self.texts.push(Box::from(text));
        self.place(id);
        Sym(id)
    }

    /// the handle of `text` when the table has it
    pub fn get(&self, text: &str) -> Option<Sym> {
        let mask = self.slots.len() - 1;
        let mut i = hash(text) as usize & mask;
        loop {
            let id = self.slots[i];
            if id == NONE {
                return None;
            }
            if &*self.texts[id as usize] == text {
                return Some(Sym(id));
            }
            i = (i + 1) & mask;
        }
    }

    /// the text of a handle ("" for a handle of another, larger table)
    pub fn str(&self, sym: Sym) -> &str {
        self.texts.get(sym.0 as usize).map_or("", |t| t)
    }

    /// the number of texts in the table
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// whether the table holds nothing but the empty text and the known names
    pub fn is_empty(&self) -> bool {
        self.texts.len() <= crate::names::KNOWN.len() + 1
    }

    fn place(&mut self, id: u32) {
        let mask = self.slots.len() - 1;
        let mut i = hash(&self.texts[id as usize]) as usize & mask;
        while self.slots[i] != NONE {
            i = (i + 1) & mask;
        }
        self.slots[i] = id;
    }

    fn grow(&mut self) {
        let n = self.slots.len() * 2;
        self.slots = vec![NONE; n];
        for id in 0..self.texts.len() as u32 {
            self.place(id);
        }
    }
}
