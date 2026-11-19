// sheet -- the ShapeSheet store: sheets of single cells and sections of rows (decision V1)
//
//   Sheet { cells, sections, triggers, extras }    a shape's, page's, master's, style's or the
//                                                  document's cells
//   Section { name, index, del, cells, rows, ... }  Geometry1..n (index), Character, User ...
//   Row { key, kind, del, cells, ... }             RowKey::Index(ix) or RowKey::Named(name)
//   CellAddr::single(c) / section(s, c) / row(s, r, c)   where a cell sits in a sheet
//   SecKey { name, index }                         a section: Geometry has index 0, 1, 2 ...,
//                                                  every other section index 0
//   sheet.cell(name) / section(key) / find(addr)   plain lookups, no inheritance (inherit.rs)
//   Extras / Extra / Place / ExtraItem             what the file had and the model does not
//                                                  know (DF-03), at the place it stood
//
// Rows of the sections User, Property, Actions, Control, Hyperlink and ActionTag are named (N),
// the others indexed (IX); IX may be missing in the file, then the row's position counts
// (`ix_written` remembers which).  `del` is the file's Del="1": the row or section a master shape
// or style would pass on is deleted here.  Cells keep the file's order.
//
// Extras: diagram_io fills them while reading (JD3) and writes them back at their place (JD5):
// unknown attributes of the element, unknown child elements and text after the n-th known
// child, what stood inside a cell (RefBy, text) by the cell's position, and what stood inside
// the Shapes and Connects lists by the entries before it.  An element is kept as the file
// wrote it; a namespace prefix in it is declared on the part's root (kept with the root's
// attributes).  The model never looks into them.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::cell::Cell;
use crate::strings::Sym;

// ---- extras (DF-03)

/// where an unknown item stood
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Place {
    /// an attribute of the element itself
    Element,
    /// a child after this many known children (0: before the first)
    AfterChild(u32),
    /// inside the element's cell at this position (attributes, RefBy, text)
    InCell(u32),
    /// inside the element's Shapes child (a page's, master's or group's), after this many
    /// Shape elements; an attribute here is one of the Shapes element itself
    InShapes(u32),
    /// inside the element's Connects child, after this many Connect elements; an attribute
    /// here is one of the Connects element itself
    InConnects(u32),
}

/// an unknown attribute, element or text
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExtraItem {
    /// name="value" (the name with its prefix as written)
    Attribute {
        /// the attribute name
        name: Sym,
        /// its value
        value: Sym,
    },
    /// an element with everything inside it, as XML text
    Element(Sym),
    /// character data
    Text(Sym),
}

/// one unknown item and its place
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Extra {
    /// where it stood
    pub place: Place,
    /// what it was
    pub item: ExtraItem,
}

/// the unknown items of one element (nothing allocated while there are none)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Extras(Option<Box<Vec<Extra>>>);

impl Extras {
    /// none
    pub const fn new() -> Self {
        Extras(None)
    }

    /// add one item
    pub fn push(&mut self, extra: Extra) {
        self.0.get_or_insert_with(Default::default).push(extra);
    }

    /// the items in the order they were added
    pub fn items(&self) -> &[Extra] {
        self.0.as_deref().map_or(&[], |v| v.as_slice())
    }

    /// whether there are none
    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }
}

// ---- triggers

/// an update trigger (Trigger N="..." with its RefBy children kept as XML)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Trigger {
    /// the trigger name (N)
    pub name: Sym,
    /// its children as XML text (Sym::EMPTY: none)
    pub inner: Sym,
}

// ---- keys and addresses

/// how a row is identified in its section
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum RowKey {
    /// IX (rows of Geometry, Character, Connection ...)
    Index(u32),
    /// N (rows of User, Property, Hyperlink ...)
    Named(Sym),
}

/// how a section is identified in its sheet: Geometry by index, every other section index 0
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct SecKey {
    /// the section name (N)
    pub name: Sym,
    /// IX of the section (0-based; only Geometry has several)
    pub index: u32,
}

impl SecKey {
    /// a section that exists once per sheet
    pub const fn one(name: Sym) -> SecKey {
        SecKey { name, index: 0 }
    }
}

/// where a cell sits in a sheet
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct CellAddr {
    /// None: a single cell of the sheet
    pub section: Option<SecKey>,
    /// None: a cell of the section itself (Geometry's NoFill ...)
    pub row: Option<RowKey>,
    /// the cell name
    pub cell: Sym,
}

impl CellAddr {
    /// a single cell (PinX, Width ...)
    pub const fn single(cell: Sym) -> CellAddr {
        CellAddr { section: None, row: None, cell }
    }

    /// a cell of a section itself (Geometry1.NoFill)
    pub const fn section(section: SecKey, cell: Sym) -> CellAddr {
        CellAddr { section: Some(section), row: None, cell }
    }

    /// a cell of a row (Geometry1.X2, Prop.Name.Value)
    pub const fn row(section: SecKey, row: RowKey, cell: Sym) -> CellAddr {
        CellAddr { section: Some(section), row: Some(row), cell }
    }
}

// ---- rows, sections, sheets

/// a row of a section
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// IX or N
    pub key: RowKey,
    /// whether IX stood in the file (false: the position gave the index)
    pub ix_written: bool,
    /// the language-dependent name (LocalName)
    pub local_name: Option<Sym>,
    /// the row type (T): MoveTo, LineTo ... in Geometry; None: inherited or none
    pub kind: Option<Sym>,
    /// Del="1": the inherited row is deleted here
    pub del: bool,
    /// the cells in file order
    pub cells: Vec<Cell>,
    /// update triggers of the row
    pub triggers: Vec<Trigger>,
    /// unknown items
    pub extras: Extras,
}

impl Row {
    /// an empty row
    pub fn new(key: RowKey) -> Row {
        Row {
            key,
            ix_written: matches!(key, RowKey::Index(_)),
            local_name: None,
            kind: None,
            del: false,
            cells: Vec::new(),
            triggers: Vec::new(),
            extras: Extras::new(),
        }
    }

    /// the cell named `name`
    pub fn cell(&self, name: Sym) -> Option<&Cell> {
        self.cells.iter().find(|c| c.name == name)
    }

    /// the position of the cell named `name`
    pub fn cell_pos(&self, name: Sym) -> Option<usize> {
        self.cells.iter().position(|c| c.name == name)
    }
}

/// a section of a sheet
#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    /// the section name (N)
    pub name: Sym,
    /// IX (0-based; counted when the file has none)
    pub index: u32,
    /// whether IX stood in the file
    pub ix_written: bool,
    /// Del="1": the inherited section is deleted here
    pub del: bool,
    /// cells of the section itself (Geometry: NoFill, NoLine, NoShow, NoSnap, NoQuickDrag)
    pub cells: Vec<Cell>,
    /// the rows in file order (indexed rows by rising IX)
    pub rows: Vec<Row>,
    /// update triggers of the section
    pub triggers: Vec<Trigger>,
    /// unknown items
    pub extras: Extras,
}

impl Section {
    /// an empty section
    pub fn new(key: SecKey) -> Section {
        Section {
            name: key.name,
            index: key.index,
            ix_written: false,
            del: false,
            cells: Vec::new(),
            rows: Vec::new(),
            triggers: Vec::new(),
            extras: Extras::new(),
        }
    }

    /// the section's key
    pub fn key(&self) -> SecKey {
        SecKey { name: self.name, index: self.index }
    }

    /// the row with this key
    pub fn row(&self, key: RowKey) -> Option<&Row> {
        self.rows.iter().find(|r| r.key == key)
    }

    /// the position of the row with this key
    pub fn row_pos(&self, key: RowKey) -> Option<usize> {
        self.rows.iter().position(|r| r.key == key)
    }

    /// the cell of the section itself named `name`
    pub fn cell(&self, name: Sym) -> Option<&Cell> {
        self.cells.iter().find(|c| c.name == name)
    }

    /// where a new indexed row with this index belongs (rows stay sorted by index)
    pub fn insert_pos(&self, key: RowKey) -> usize {
        match key {
            RowKey::Index(ix) => self.rows.iter().position(|r| matches!(r.key, RowKey::Index(i) if i > ix)).unwrap_or(self.rows.len()),
            RowKey::Named(_) => self.rows.len(),
        }
    }
}

/// a ShapeSheet: single cells and sections
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sheet {
    /// the single cells in file order
    pub cells: Vec<Cell>,
    /// the sections in file order
    pub sections: Vec<Section>,
    /// update triggers of the sheet
    pub triggers: Vec<Trigger>,
    /// unknown items
    pub extras: Extras,
}

/// what a sheet holds at an address, without inheritance
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Local<'a> {
    /// the cell is here
    Cell(&'a Cell),
    /// the row or section is here with Del="1"
    Deleted,
    /// nothing here: ask the parent
    Missing,
}

impl Sheet {
    /// an empty sheet
    pub fn new() -> Sheet {
        Sheet::default()
    }

    /// the single cell named `name`
    pub fn cell(&self, name: Sym) -> Option<&Cell> {
        self.cells.iter().find(|c| c.name == name)
    }

    /// the section with this key
    pub fn section(&self, key: SecKey) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == key.name && s.index == key.index)
    }

    /// the position of the section with this key
    pub fn section_pos(&self, key: SecKey) -> Option<usize> {
        self.sections.iter().position(|s| s.name == key.name && s.index == key.index)
    }

    /// the cell at `addr` in this sheet alone (no inheritance)
    pub fn find(&self, addr: &CellAddr) -> Local<'_> {
        let Some(sk) = addr.section else {
            return self.cell(addr.cell).map_or(Local::Missing, Local::Cell);
        };
        let Some(sec) = self.section(sk) else { return Local::Missing };
        if sec.del {
            return Local::Deleted;
        }
        let Some(rk) = addr.row else {
            return sec.cell(addr.cell).map_or(Local::Missing, Local::Cell);
        };
        let Some(row) = sec.row(rk) else { return Local::Missing };
        if row.del {
            return Local::Deleted;
        }
        row.cell(addr.cell).map_or(Local::Missing, Local::Cell)
    }

    /// whether the sheet holds no cells, sections or triggers
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.sections.is_empty() && self.triggers.is_empty()
    }
}
