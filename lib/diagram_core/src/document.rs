// document -- the document: pages, masters, styles, the document sheet, themes and parts
//
//   Document::new(DocKind::Drawing)        empty: no pages, masters or styles
//   doc.intern("PinX") / sym(..) / str(sym)   the document's string table (strings.rs)
//   doc.pages() / page(r) / page_by_id(id) / page_by_name_u(..)     (masters, styles alike)
//   doc.shape(r) -> Option<Shape>          a shape with its links: id, data, children,
//                                          parent, master shape, NameU and Name
//   doc.sheet(SheetRef) / sheet_styles(..)  any ShapeSheet of the document
//   doc.insert_page(page) / insert_master / insert_style      (untracked, for loaders)
//   doc.page_mut(r) / master_mut / style_mut / doc_sheet_mut  (untracked)
//   doc.unused_page_name("Page")           "Page-3": the first free number
//   doc.transaction(label)                 tracked edits with undo (edit.rs)
//
// Handles: PageRef, MasterRef and StyleRef are slots of the document like ShapeRef is a slot of
// its tree: stable for the document's life, also across undo, never reused.  The file's IDs
// (page ID, master ID, style ID, shape ID) are attributes of the objects; references inside
// the model use them as the file does (Master="2", LineStyle="3", BackPage="4", MasterShape),
// so a shape whose master is gone still says which one it was.
//
// Untracked access (the *_mut functions, the public fields) is for building a document --
// diagram_io while reading, tests -- and for data the edit API does not cover yet.  Changes
// made that way bypass the transactions, so an undo history recorded before them no longer
// fits the document: clear it.
//
// Names (LG-04): NameU is the universal key (formulas, matching masters, own shapes), Name what
// the user sees; a file's Name stays as read.  A shape without NameU is "Sheet.<ID>", an
// instance "<master's NameU>.<ID>"; without Name it shows its NameU.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::page::{Master, Page, Style};
use crate::shape::{Node, ShapeData, ShapeHeader, ShapeTree, StyleRefs, Text};
use crate::sheet::{Extras, Sheet};
use crate::strings::{Strings, Sym};

// ---- handles

/// a page of the document (a slot, stable across undo)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PageRef(pub(crate) u32);

/// a master of the document
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MasterRef(pub(crate) u32);

/// a style of the document
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StyleRef(pub(crate) u32);

/// what holds a shape tree
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Owner {
    /// the shapes of a page
    Page(PageRef),
    /// the shapes of a master
    Master(MasterRef),
}

/// a shape: its tree and its slot there
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ShapeRef {
    /// the page or master
    pub owner: Owner,
    /// the slot in its tree
    pub slot: u32,
}

/// any ShapeSheet of the document
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SheetRef {
    /// the document sheet
    Document,
    /// a style sheet
    Style(StyleRef),
    /// a page sheet
    Page(PageRef),
    /// a master's page sheet
    Master(MasterRef),
    /// a shape's sheet
    Shape(ShapeRef),
}

// ---- document parts

/// what the document is
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum DocKind {
    /// a drawing (.vsdx, .vsdm)
    #[default]
    Drawing,
    /// a template (.vstx, .vstm)
    Template,
    /// a stencil: masters without pages (.vssx, .vssm)
    Stencil,
}

/// the DocumentSettings element
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DocSettings {
    /// TopPage: the ID of the page shown first
    pub top_page: Option<u32>,
    /// DefaultTextStyle: the style ID new text gets
    pub default_text_style: Option<u32>,
    /// DefaultLineStyle
    pub default_line_style: Option<u32>,
    /// DefaultFillStyle
    pub default_fill_style: Option<u32>,
    /// DefaultGuideStyle
    pub default_guide_style: Option<u32>,
    /// the other attributes in file order
    pub attrs: Vec<(Sym, Sym)>,
    /// the child elements and their text in file order (GlueSettings, SnapSettings,
    /// SnapExtensions, SnapAngles, DynamicGridEnabled, ProtectStyles ...)
    pub items: Vec<(Sym, Sym)>,
    /// unknown items
    pub extras: Extras,
}

/// an entry of the old colour table (Colors): index -> RGB
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ColorEntry {
    /// IX
    pub ix: u32,
    /// RGB as written ("#000000")
    pub rgb: Sym,
}

/// a font of the document (FaceName), its attributes in file order (NameU, Panose ...)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FaceName {
    /// the attributes
    pub attrs: Vec<(Sym, Sym)>,
}

/// the document's own ShapeSheet (DocumentSheet)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DocSheet {
    /// LineStyle, FillStyle, TextStyle
    pub styles: StyleRefs,
    /// the other attributes in file order (Name, NameU ...)
    pub attrs: Vec<(Sym, Sym)>,
    /// the cells
    pub sheet: Sheet,
}

/// a theme part, kept as read (typed later, JD11)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Theme {
    /// the part name (/visio/theme/theme1.xml)
    pub part: Sym,
    /// its bytes
    pub data: Vec<u8>,
}

/// a package part the model keeps as bytes: media, document properties, windows, comments,
/// data recordsets, the thumbnail, a macro project
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Part {
    /// the part name
    pub name: Sym,
    /// its content type
    pub content_type: Sym,
    /// the type of the relationship that points to it
    pub rel_type: Sym,
    /// its bytes
    pub data: Vec<u8>,
}

/// a relationship of the package as read: which part (or the package) points to which target
/// with which type.  The model derives the ones it owns (pages, masters, themes, pictures)
/// from its own links when it writes; the list keeps every relationship so that parts it only
/// keeps as bytes (Part), external targets and ids that kept XML refers to (r:id) survive
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Relationship {
    /// the source part (`/visio/document.xml`), "" for the package itself
    pub source: Sym,
    /// the id (`rId3`), unique per source
    pub id: Sym,
    /// the type, a URI
    pub kind: Sym,
    /// the target as written: relative to the source, or a URI when external
    pub target: Sym,
    /// TargetMode="External": outside the package, never followed
    pub external: bool,
    /// the part an internal target names (`/visio/media/image1.png`); None when external
    pub part: Option<Sym>,
}

/// why an object could not be put into the document
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DocError {
    /// another object of the same kind has this ID
    DuplicateId(u32),
}

// ---- arenas of pages, masters, styles

pub(crate) trait HasId {
    fn id(&self) -> u32;
}

impl HasId for Page {
    fn id(&self) -> u32 {
        self.header.id
    }
}
impl HasId for Master {
    fn id(&self) -> u32 {
        self.header.id
    }
}
impl HasId for Style {
    fn id(&self) -> u32 {
        self.header.id
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Arena<T> {
    slots: Vec<Option<Box<T>>>,
    pub(crate) order: Vec<u32>,
    by_id: BTreeMap<u32, u32>,
    // the next ID handed out (u64: past u32::MAX none is left)
    next_id: u64,
}

impl<T: HasId> Arena<T> {
    fn new(first_id: u32) -> Self {
        Arena { slots: Vec::new(), order: Vec::new(), by_id: BTreeMap::new(), next_id: first_id as u64 }
    }

    pub(crate) fn get(&self, slot: u32) -> Option<&T> {
        self.slots.get(slot as usize).and_then(|s| s.as_deref())
    }

    pub(crate) fn get_mut(&mut self, slot: u32) -> Option<&mut T> {
        self.slots.get_mut(slot as usize).and_then(|s| s.as_deref_mut())
    }

    pub(crate) fn by_id(&self, id: u32) -> Option<u32> {
        self.by_id.get(&id).copied()
    }

    pub(crate) fn push(&mut self, item: T) -> Result<u32, DocError> {
        let id = item.id();
        if self.by_id.contains_key(&id) {
            return Err(DocError::DuplicateId(id));
        }
        let slot = self.slots.len() as u32;
        self.slots.push(Some(Box::new(item)));
        self.by_id.insert(id, slot);
        self.order.push(slot);
        self.next_id = self.next_id.max(id as u64 + 1);
        Ok(slot)
    }

    // a new empty slot and the next free ID (edits fill it with put; None: no IDs left)
    pub(crate) fn alloc(&mut self) -> Option<(u32, u32)> {
        let id = u32::try_from(self.next_id).ok()?;
        self.next_id = id as u64 + 1;
        let slot = self.slots.len() as u32;
        self.slots.push(None);
        Some((slot, id))
    }

    pub(crate) fn put(&mut self, slot: u32, pos: u32, item: Box<T>) -> bool {
        let s = slot as usize;
        if self.slots.len() <= s {
            self.slots.resize_with(s + 1, || None);
        }
        if self.slots[s].is_some() || self.by_id.contains_key(&item.id()) {
            return false;
        }
        self.by_id.insert(item.id(), slot);
        self.next_id = self.next_id.max(item.id() as u64 + 1);
        self.slots[s] = Some(item);
        let pos = (pos as usize).min(self.order.len());
        self.order.insert(pos, slot);
        true
    }

    pub(crate) fn take(&mut self, slot: u32) -> Option<(u32, Box<T>)> {
        let item = self.slots.get_mut(slot as usize)?.take()?;
        self.by_id.remove(&item.id());
        let pos = self.order.iter().position(|&s| s == slot).unwrap_or(self.order.len());
        if pos < self.order.len() {
            self.order.remove(pos);
        }
        Some((pos as u32, item))
    }

    // whether `id` may be the ID of the object in `slot` (no other object has it)
    pub(crate) fn id_free_for(&self, slot: u32, id: u32) -> bool {
        self.by_id.get(&id).is_none_or(|&s| s == slot)
    }

    // keep by_id right when an edit changes an object's ID (headers are swapped whole)
    pub(crate) fn reindex(&mut self, slot: u32, old_id: u32) {
        if let Some(item) = self.get(slot) {
            let id = item.id();
            if id != old_id {
                self.by_id.remove(&old_id);
                self.by_id.insert(id, slot);
                self.next_id = self.next_id.max(id as u64 + 1);
            }
        }
    }

    pub(crate) fn position(&self, slot: u32) -> Option<usize> {
        self.order.iter().position(|&s| s == slot)
    }

    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }
}

/// a drawing, template or stencil
#[derive(Clone, Debug)]
pub struct Document {
    pub(crate) strings: Strings,
    /// drawing, template or stencil
    pub kind: DocKind,
    /// whether the file is macro-enabled (.vsdm, .vssm, .vstm); macros are kept, never run
    pub macros: bool,
    /// DocumentSettings
    pub settings: DocSettings,
    /// the old colour table
    pub colors: Vec<ColorEntry>,
    /// the fonts
    pub faces: Vec<FaceName>,
    pub(crate) doc_sheet: DocSheet,
    pub(crate) pages: Arena<Page>,
    pub(crate) masters: Arena<Master>,
    pub(crate) styles: Arena<Style>,
    /// the theme parts (Page::theme indexes them)
    pub themes: Vec<Theme>,
    /// the other package parts, as bytes
    pub parts: Vec<Part>,
    /// every relationship of the package as read (see Relationship)
    pub rels: Vec<Relationship>,
    /// the attributes of the VisioDocument element
    pub attrs: Vec<(Sym, Sym)>,
    /// unknown items of the VisioDocument element (EventList, HeaderFooter, PublishSettings ...
    /// until they are typed)
    pub extras: Extras,
    /// unknown items of the Pages element
    pub pages_extras: Extras,
    /// unknown items of the Masters element
    pub masters_extras: Extras,
    /// unknown items of the Colors element (a ColorEntry with more than IX and RGB too)
    pub colors_extras: Extras,
    /// unknown items of the FaceNames element (a FaceName with children too)
    pub faces_extras: Extras,
    /// unknown items of the StyleSheets element
    pub styles_extras: Extras,
}

impl Default for Document {
    fn default() -> Self {
        Document::new(DocKind::Drawing)
    }
}

impl Document {
    /// an empty document of this kind
    pub fn new(kind: DocKind) -> Document {
        Document {
            strings: Strings::new(),
            kind,
            macros: false,
            settings: DocSettings::default(),
            colors: Vec::new(),
            faces: Vec::new(),
            doc_sheet: DocSheet::default(),
            pages: Arena::new(0),
            masters: Arena::new(1),
            styles: Arena::new(0),
            themes: Vec::new(),
            parts: Vec::new(),
            rels: Vec::new(),
            attrs: Vec::new(),
            extras: Extras::new(),
            pages_extras: Extras::new(),
            masters_extras: Extras::new(),
            colors_extras: Extras::new(),
            faces_extras: Extras::new(),
            styles_extras: Extras::new(),
        }
    }

    // ---- strings

    /// the string table
    pub fn strings(&self) -> &Strings {
        &self.strings
    }

    /// the string table to add to (loaders hand it to Value::parse, Formula::parse,
    /// ShapeKind::parse); it only grows, so every Sym stays valid
    pub fn strings_mut(&mut self) -> &mut Strings {
        &mut self.strings
    }

    /// the handle of a text, added when new
    pub fn intern(&mut self, text: &str) -> Sym {
        self.strings.intern(text)
    }

    /// the handle of a text the document already has
    pub fn sym(&self, text: &str) -> Option<Sym> {
        self.strings.get(text)
    }

    /// the text of a handle
    pub fn str(&self, sym: Sym) -> &str {
        self.strings.str(sym)
    }

    // ---- pages

    /// the pages in document order (foreground and background)
    pub fn pages(&self) -> impl Iterator<Item = PageRef> + '_ {
        self.pages.order.iter().map(|&s| PageRef(s))
    }

    /// the foreground pages in document order
    pub fn foreground_pages(&self) -> impl Iterator<Item = PageRef> + '_ {
        self.pages().filter(|&p| self.page(p).is_some_and(|pg| !pg.header.background))
    }

    /// the background pages in document order
    pub fn background_pages(&self) -> impl Iterator<Item = PageRef> + '_ {
        self.pages().filter(|&p| self.page(p).is_some_and(|pg| pg.header.background))
    }

    /// the number of pages
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// a page
    pub fn page(&self, r: PageRef) -> Option<&Page> {
        self.pages.get(r.0)
    }

    /// a page for untracked changes (see the module comment)
    pub fn page_mut(&mut self, r: PageRef) -> Option<&mut Page> {
        self.pages.get_mut(r.0)
    }

    /// the page with this page ID
    pub fn page_by_id(&self, id: u32) -> Option<PageRef> {
        self.pages.by_id(id).map(PageRef)
    }

    /// the page with this universal name
    pub fn page_by_name_u(&self, name_u: &str) -> Option<PageRef> {
        let sym = self.sym(name_u)?;
        self.pages().find(|&p| self.page(p).is_some_and(|pg| pg.header.name_u == Some(sym)))
    }

    /// the position of a page in document order
    pub fn page_position(&self, r: PageRef) -> Option<usize> {
        self.pages.position(r.0)
    }

    /// the background page of a page
    pub fn background_of(&self, r: PageRef) -> Option<PageRef> {
        self.page(r)?.header.back_page.and_then(|id| self.page_by_id(id))
    }

    /// add a page at the end (untracked)
    pub fn insert_page(&mut self, page: Page) -> Result<PageRef, DocError> {
        self.pages.push(page).map(PageRef)
    }

    /// "<prefix>-<n>" with the smallest n no page has as Name or NameU
    pub fn unused_page_name(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let name = format!("{prefix}-{n}");
            let taken = self.sym(&name).is_some_and(|s| {
                self.pages().any(|p| self.page(p).is_some_and(|pg| pg.header.name == Some(s) || pg.header.name_u == Some(s)))
            });
            if !taken {
                return name;
            }
            n += 1;
        }
    }

    // ---- masters

    /// the masters in document order
    pub fn masters(&self) -> impl Iterator<Item = MasterRef> + '_ {
        self.masters.order.iter().map(|&s| MasterRef(s))
    }

    /// a master
    pub fn master(&self, r: MasterRef) -> Option<&Master> {
        self.masters.get(r.0)
    }

    /// a master for untracked changes
    pub fn master_mut(&mut self, r: MasterRef) -> Option<&mut Master> {
        self.masters.get_mut(r.0)
    }

    /// the master with this master ID
    pub fn master_by_id(&self, id: u32) -> Option<MasterRef> {
        self.masters.by_id(id).map(MasterRef)
    }

    /// the master with this universal name
    pub fn master_by_name_u(&self, name_u: &str) -> Option<MasterRef> {
        let sym = self.sym(name_u)?;
        self.masters().find(|&m| self.master(m).is_some_and(|ms| ms.header.name_u == Some(sym)))
    }

    /// add a master at the end (untracked)
    pub fn insert_master(&mut self, master: Master) -> Result<MasterRef, DocError> {
        self.masters.push(master).map(MasterRef)
    }

    // ---- styles

    /// the styles in document order
    pub fn styles(&self) -> impl Iterator<Item = StyleRef> + '_ {
        self.styles.order.iter().map(|&s| StyleRef(s))
    }

    /// a style
    pub fn style(&self, r: StyleRef) -> Option<&Style> {
        self.styles.get(r.0)
    }

    /// a style for untracked changes
    pub fn style_mut(&mut self, r: StyleRef) -> Option<&mut Style> {
        self.styles.get_mut(r.0)
    }

    /// the style with this style ID
    pub fn style_by_id(&self, id: u32) -> Option<StyleRef> {
        self.styles.by_id(id).map(StyleRef)
    }

    /// the style with this universal name
    pub fn style_by_name_u(&self, name_u: &str) -> Option<StyleRef> {
        let sym = self.sym(name_u)?;
        self.styles().find(|&s| self.style(s).is_some_and(|st| st.header.name_u == Some(sym)))
    }

    /// add a style at the end (untracked)
    pub fn insert_style(&mut self, style: Style) -> Result<StyleRef, DocError> {
        self.styles.push(style).map(StyleRef)
    }

    // ---- document sheet

    /// the document's own ShapeSheet
    pub fn doc_sheet(&self) -> &DocSheet {
        &self.doc_sheet
    }

    /// the document sheet for untracked changes
    pub fn doc_sheet_mut(&mut self) -> &mut DocSheet {
        &mut self.doc_sheet
    }

    // ---- sheets

    /// any sheet of the document
    pub fn sheet(&self, r: SheetRef) -> Option<&Sheet> {
        match r {
            SheetRef::Document => Some(&self.doc_sheet.sheet),
            SheetRef::Style(s) => self.style(s).map(|x| &x.sheet),
            SheetRef::Page(p) => self.page(p).map(|x| &x.sheet),
            SheetRef::Master(m) => self.master(m).map(|x| &x.sheet),
            SheetRef::Shape(s) => self.tree(s.owner)?.get(s.slot).map(|d| &d.sheet),
        }
    }

    pub(crate) fn sheet_mut(&mut self, r: SheetRef) -> Option<&mut Sheet> {
        match r {
            SheetRef::Document => Some(&mut self.doc_sheet.sheet),
            SheetRef::Style(s) => self.styles.get_mut(s.0).map(|x| &mut x.sheet),
            SheetRef::Page(p) => self.pages.get_mut(p.0).map(|x| &mut x.sheet),
            SheetRef::Master(m) => self.masters.get_mut(m.0).map(|x| &mut x.sheet),
            SheetRef::Shape(s) => self.tree_mut(s.owner)?.data_mut(s.slot).map(|d| &mut d.sheet),
        }
    }

    /// the styles a sheet inherits from (a style: the styles it is based on)
    pub fn sheet_styles(&self, r: SheetRef) -> StyleRefs {
        match r {
            SheetRef::Document => self.doc_sheet.styles,
            SheetRef::Style(s) => self.style(s).map(|x| x.header.based_on).unwrap_or_default(),
            SheetRef::Page(p) => self.page(p).map(|x| x.header.sheet_styles).unwrap_or_default(),
            SheetRef::Master(m) => self.master(m).map(|x| x.header.sheet_styles).unwrap_or_default(),
            SheetRef::Shape(s) => self.shape(s).map(|x| x.header().styles).unwrap_or_default(),
        }
    }

    // ---- shapes

    /// the shape tree of a page or master
    pub fn tree(&self, owner: Owner) -> Option<&ShapeTree> {
        match owner {
            Owner::Page(p) => self.page(p).map(|x| &x.shapes),
            Owner::Master(m) => self.master(m).map(|x| &x.shapes),
        }
    }

    pub(crate) fn tree_mut(&mut self, owner: Owner) -> Option<&mut ShapeTree> {
        match owner {
            Owner::Page(p) => self.pages.get_mut(p.0).map(|x| &mut x.shapes),
            Owner::Master(m) => self.masters.get_mut(m.0).map(|x| &mut x.shapes),
        }
    }

    /// a shape and its links
    pub fn shape(&self, r: ShapeRef) -> Option<Shape<'_>> {
        let node = self.tree(r.owner)?.node(r.slot)?;
        Some(Shape { doc: self, r, node })
    }

    /// the shape with this file ID on a page or in a master
    pub fn shape_by_id(&self, owner: Owner, id: u32) -> Option<ShapeRef> {
        self.tree(owner)?.by_id(id).map(|slot| ShapeRef { owner, slot })
    }

    /// the top-level shapes of a page or master in z-order
    pub fn top_shapes(&self, owner: Owner) -> impl Iterator<Item = ShapeRef> + '_ {
        self.tree(owner).map_or(&[][..], |t| t.top()).iter().map(move |&slot| ShapeRef { owner, slot })
    }

    /// every shape of a page or master, groups before their subshapes
    pub fn all_shapes(&self, owner: Owner) -> impl Iterator<Item = ShapeRef> + '_ {
        self.tree(owner).into_iter().flat_map(move |t| t.iter().map(move |slot| ShapeRef { owner, slot }))
    }

    /// the shape `r` inherits from: for an instance its master's master shape, for a
    /// subshape of an instance the shape MasterShape names in the instance's master
    pub fn master_shape_of(&self, r: ShapeRef) -> Option<ShapeRef> {
        let tree = self.tree(r.owner)?;
        let node = tree.node(r.slot)?;
        if let Some(mid) = node.data.header.master {
            let m = self.master_by_id(mid)?;
            let slot = self.master(m)?.master_shape()?;
            return Some(ShapeRef { owner: Owner::Master(m), slot });
        }
        let msid = node.data.header.master_shape?;
        let mut up = node.parent;
        while let Some(p) = up {
            let pn = tree.node(p)?;
            if let Some(mid) = pn.data.header.master {
                let m = self.master_by_id(mid)?;
                let slot = self.master(m)?.shapes.by_id(msid)?;
                return Some(ShapeRef { owner: Owner::Master(m), slot });
            }
            up = pn.parent;
        }
        None
    }

    /// the master an instance (or a subshape of one) belongs to
    pub fn master_of(&self, r: ShapeRef) -> Option<MasterRef> {
        let tree = self.tree(r.owner)?;
        let mut cur = Some(r.slot);
        while let Some(s) = cur {
            let n = tree.node(s)?;
            if let Some(mid) = n.data.header.master {
                return self.master_by_id(mid);
            }
            if n.data.header.master_shape.is_none() {
                return None;
            }
            cur = n.parent;
        }
        None
    }

    /// whether any shape of the document is an instance of the master with this ID
    pub fn master_in_use(&self, id: u32) -> bool {
        let owners = self.pages().map(Owner::Page).chain(self.masters().map(Owner::Master));
        let owners: Vec<Owner> = owners.collect();
        owners.into_iter().any(|o| {
            self.tree(o).is_some_and(|t| t.iter().any(|s| t.get(s).is_some_and(|d| d.header.master == Some(id))))
        })
    }

    /// whether the master with ID `a` uses the master with ID `b` (itself or through instances
    /// in its shapes, at any depth)
    pub fn master_depends_on(&self, a: u32, b: u32) -> bool {
        let mut seen = alloc::collections::BTreeSet::new();
        let mut stack = alloc::vec![a];
        while let Some(id) = stack.pop() {
            if id == b {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            let Some(m) = self.master_by_id(id).and_then(|r| self.master(r)) else { continue };
            stack.extend(m.shapes.iter().filter_map(|s| m.shapes.get(s).and_then(|d| d.header.master)));
        }
        false
    }

    /// whether any sheet, style or setting of the document names the style with this ID
    pub fn style_in_use(&self, id: u32) -> bool {
        if self.doc_sheet.styles.uses(id) {
            return true;
        }
        let st = &self.settings;
        if [st.default_text_style, st.default_line_style, st.default_fill_style, st.default_guide_style].contains(&Some(id)) {
            return true;
        }
        if self.styles().any(|s| self.style(s).is_some_and(|x| x.header.id != id && x.header.based_on.uses(id))) {
            return true;
        }
        if self.pages().any(|p| self.page(p).is_some_and(|x| x.header.sheet_styles.uses(id)))
            || self.masters().any(|m| self.master(m).is_some_and(|x| x.header.sheet_styles.uses(id)))
        {
            return true;
        }
        let owners: Vec<Owner> = self.pages().map(Owner::Page).chain(self.masters().map(Owner::Master)).collect();
        owners.into_iter().any(|o| {
            self.tree(o).is_some_and(|t| t.iter().any(|s| t.get(s).is_some_and(|d| d.header.styles.uses(id))))
        })
    }
}

/// a shape with access to its document (Document::shape)
#[derive(Copy, Clone)]
pub struct Shape<'d> {
    doc: &'d Document,
    r: ShapeRef,
    node: &'d Node,
}

impl<'d> Shape<'d> {
    /// its handle
    pub fn r(&self) -> ShapeRef {
        self.r
    }

    /// its file ID (unique on its page or in its master)
    pub fn id(&self) -> u32 {
        self.node.id
    }

    /// everything it holds itself
    pub fn data(&self) -> &'d ShapeData {
        &self.node.data
    }

    /// its attributes
    pub fn header(&self) -> &'d ShapeHeader {
        &self.node.data.header
    }

    /// its own cells (without inheritance)
    pub fn sheet(&self) -> &'d Sheet {
        &self.node.data.sheet
    }

    /// as a sheet reference
    pub fn sheet_ref(&self) -> SheetRef {
        SheetRef::Shape(self.r)
    }

    /// the group it belongs to
    pub fn parent(&self) -> Option<ShapeRef> {
        self.node.parent.map(|slot| ShapeRef { owner: self.r.owner, slot })
    }

    /// its subshapes in z-order
    pub fn children(&self) -> impl Iterator<Item = ShapeRef> + 'd {
        let owner = self.r.owner;
        self.node.children.iter().map(move |&slot| ShapeRef { owner, slot })
    }

    /// the shape it inherits from (Document::master_shape_of)
    pub fn master_shape(&self) -> Option<ShapeRef> {
        self.doc.master_shape_of(self.r)
    }

    /// whether the master's subshape is deleted here (Del="1")
    pub fn is_deleted(&self) -> bool {
        self.node.data.header.del
    }

    /// its text, from the master shape when it has none of its own
    pub fn text(&self) -> Option<&'d Text> {
        let mut cur = Some(self.r);
        let mut depth = 0;
        while let (Some(r), true) = (cur, depth < 64) {
            let d = self.doc.tree(r.owner)?.get(r.slot)?;
            if let Some(t) = &d.text {
                return Some(t);
            }
            cur = self.doc.master_shape_of(r);
            depth += 1;
        }
        None
    }

    /// the universal name: NameU, else derived ("Sheet.7", "<master's NameU>.7")
    pub fn name_u(&self) -> String {
        if let Some(n) = self.node.data.header.name_u {
            return String::from(self.doc.str(n));
        }
        self.derived_name(true)
    }

    /// the shown name: Name, else NameU, else derived from the master's Name
    pub fn name(&self) -> String {
        let h = &self.node.data.header;
        if let Some(n) = h.name.or(h.name_u) {
            return String::from(self.doc.str(n));
        }
        self.derived_name(false)
    }

    fn derived_name(&self, universal: bool) -> String {
        let id = self.node.id;
        if let Some(m) = self.node.data.header.master.and_then(|mid| self.doc.master_by_id(mid)).and_then(|m| self.doc.master(m)) {
            let n = if universal { m.header.name_u.or(m.header.name) } else { m.header.name.or(m.header.name_u) };
            if let Some(n) = n {
                return format!("{}.{id}", self.doc.str(n));
            }
        }
        format!("Sheet.{id}")
    }
}
