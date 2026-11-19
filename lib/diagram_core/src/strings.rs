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
//
// The texts come from files, which are untrusted: the index is an ordered map, so no choice of
// texts makes a lookup slower than log n comparisons (a hash table with a fixed hash can be
// flooded with texts that all fall on one slot).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
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

/// the string table of a document
#[derive(Clone)]
pub struct Strings {
    texts: Vec<Arc<str>>,
    index: BTreeMap<Arc<str>, u32>,
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
        let mut s = Strings { texts: Vec::new(), index: BTreeMap::new() };
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
        let id = self.texts.len() as u32;
        let t: Arc<str> = Arc::from(text);
        self.texts.push(t.clone());
        self.index.insert(t, id);
        Sym(id)
    }

    /// the handle of `text` when the table has it
    pub fn get(&self, text: &str) -> Option<Sym> {
        self.index.get(text).map(|&id| Sym(id))
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
}
