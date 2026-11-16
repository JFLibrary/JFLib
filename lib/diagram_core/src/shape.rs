// shape -- shapes and their tree on a page or in a master
//
//   ShapeData { header, sheet, text, foreign, data, extras }   one shape without its children
//   ShapeHeader                     the attributes: Type, ID-based references (Master,
//                                   MasterShape, LineStyle, FillStyle, TextStyle), Name, NameU,
//                                   Del and what else stood there
//   ShapeKind                       Shape, Group, Foreign, Guide
//   StyleRefs { line, fill, text }  the style IDs of a shape, page sheet or style (based on)
//   Text / TextItem                 the shape text: characters between cp, pp, tp and fld marks
//   ForeignData                     picture data of a Foreign shape (inline or a package part)
//   ShapeTree                       the shapes of a page or master: slots, IDs, z-order
//     tree.insert(parent, id, data) -> slot     (untracked; loaders build trees with it)
//     tree.get(slot) / by_id(id) / top() / children(slot) / parent(slot) / iter()
//
// A shape lives in a slot of its tree; the slot is its handle (ShapeRef) and never changes,
// even when the shape is deleted and the deletion undone.  Slots are not reused.  The file's
// shape ID is unique per page or master (by_id, a BTreeMap: thousands of shapes, no linear
// search).  Children are kept in z-order, the last one on top.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::names::Category;
use crate::sheet::{Extras, Sheet};
use crate::strings::{Strings, Sym};

/// the shape type (attribute Type)
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum ShapeKind {
    /// Shape: a shape with geometry
    #[default]
    Shape,
    /// Group: has subshapes
    Group,
    /// Foreign: shows its ForeignData (a picture, a metafile, an OLE object's image)
    Foreign,
    /// Guide: a guide line or guide point, not shown
    Guide,
    /// no Type attribute (the deleted subshapes of an instance)
    Unspecified,
    /// a type the crate does not know, as written
    Other(Sym),
}

impl ShapeKind {
    /// the kind of a Type attribute
    pub fn parse(text: &str, strings: &mut Strings) -> ShapeKind {
        match text {
            "Shape" => ShapeKind::Shape,
            "Group" => ShapeKind::Group,
            "Foreign" => ShapeKind::Foreign,
            "Guide" => ShapeKind::Guide,
            t => ShapeKind::Other(strings.intern(t)),
        }
    }

    /// the Type attribute (None: Unspecified or Other, whose Sym holds it)
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            ShapeKind::Shape => Some("Shape"),
            ShapeKind::Group => Some("Group"),
            ShapeKind::Foreign => Some("Foreign"),
            ShapeKind::Guide => Some("Guide"),
            _ => None,
        }
    }
}

/// the style IDs a sheet inherits from (LineStyle, FillStyle, TextStyle); for a style these
/// are the styles it is based on
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct StyleRefs {
    /// LineStyle
    pub line: Option<u32>,
    /// FillStyle
    pub fill: Option<u32>,
    /// TextStyle
    pub text: Option<u32>,
}

impl StyleRefs {
    /// all three the same style
    pub const fn all(id: u32) -> StyleRefs {
        StyleRefs { line: Some(id), fill: Some(id), text: Some(id) }
    }

    /// the style of a category
    pub fn get(&self, cat: Category) -> Option<u32> {
        match cat {
            Category::Line => self.line,
            Category::Fill => self.fill,
            Category::Text => self.text,
        }
    }

    /// whether `id` is one of the three
    pub fn uses(&self, id: u32) -> bool {
        self.line == Some(id) || self.fill == Some(id) || self.text == Some(id)
    }
}

/// the attributes of a shape
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ShapeHeader {
    /// Type
    pub kind: ShapeKind,
    /// Name: the language-dependent name (shown); None: derived (Shape::name)
    pub name: Option<Sym>,
    /// NameU: the universal name (key for formulas and matching); None: derived
    pub name_u: Option<Sym>,
    /// IsCustomName as written
    pub is_custom_name: Option<bool>,
    /// IsCustomNameU as written
    pub is_custom_name_u: Option<bool>,
    /// Master: the ID of the master this shape is an instance of
    pub master: Option<u32>,
    /// MasterShape: the ID of the shape in the master that this subshape inherits from
    pub master_shape: Option<u32>,
    /// LineStyle, FillStyle, TextStyle
    pub styles: StyleRefs,
    /// Del="1": the subshape the master passes on is deleted here
    pub del: bool,
    /// OriginalID as written
    pub original_id: Option<u32>,
    /// UniqueID as written
    pub unique_id: Option<Sym>,
}

/// one item of a shape text
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextItem {
    /// characters
    Chars(Box<str>),
    /// cp IX: from here on the Character row IX applies
    Cp(u32),
    /// pp IX: from here on the Paragraph row IX applies
    Pp(u32),
    /// tp IX: from here on the Tabs row IX applies
    Tp(u32),
    /// fld IX: the text field of Field row IX
    Field(u32),
    /// an element the crate does not know, as XML text
    Other(Sym),
}

/// the text of a shape (element Text)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Text {
    /// characters and marks in file order
    pub items: Vec<TextItem>,
    /// unknown attributes of the Text element
    pub extras: Extras,
}

impl Text {
    /// a text of plain characters
    pub fn plain(text: &str) -> Text {
        Text { items: alloc::vec![TextItem::Chars(Box::from(text))], extras: Extras::new() }
    }

    /// the characters without marks (fields left out)
    pub fn chars(&self) -> String {
        let mut s = String::new();
        for item in &self.items {
            if let TextItem::Chars(c) = item {
                s.push_str(c);
            }
        }
        s
    }
}

/// where the picture of a Foreign shape is
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum ForeignPayload {
    /// none
    #[default]
    None,
    /// inline, base64 as written
    Inline(Box<str>),
    /// a part of the package (its name, e.g. /visio/media/image1.png)
    Part(Sym),
}

/// the picture data of a Foreign shape (element ForeignData)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ForeignData {
    /// the attributes in file order (ForeignType, CompressionType, ObjectWidth ...)
    pub attrs: Vec<(Sym, Sym)>,
    /// the data
    pub payload: ForeignPayload,
    /// unknown children
    pub extras: Extras,
}

/// one shape without its subshapes
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ShapeData {
    /// the attributes
    pub header: ShapeHeader,
    /// the ShapeSheet: only what the shape sets itself, the rest is inherited
    pub sheet: Sheet,
    /// the text; None: inherited from the master shape (or none)
    pub text: Option<Box<Text>>,
    /// the picture of a Foreign shape
    pub foreign: Option<Box<ForeignData>>,
    /// Data1, Data2, Data3 as written
    pub data: [Option<Sym>; 3],
    /// unknown items of the Shape element
    pub extras: Extras,
}

impl ShapeData {
    /// a shape of this kind with nothing set
    pub fn new(kind: ShapeKind) -> ShapeData {
        ShapeData { header: ShapeHeader { kind, ..ShapeHeader::default() }, ..ShapeData::default() }
    }
}

/// (crate) a shape in its tree
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Node {
    pub(crate) id: u32,
    pub(crate) parent: Option<u32>,
    pub(crate) children: Vec<u32>,
    pub(crate) data: ShapeData,
}

// the first ID a new tree hands out
const FIRST_ID: u32 = 4;

/// why a shape could not be put into a tree
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TreeError {
    /// another shape of the tree has this ID
    DuplicateId(u32),
    /// the parent slot holds no shape
    NoParent,
}

/// the shapes of a page or a master
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ShapeTree {
    slots: Vec<Option<Box<Node>>>,
    by_id: BTreeMap<u32, u32>,
    top: Vec<u32>,
    // the next ID handed out: above every ID the tree ever had (u64: past u32::MAX none is left)
    next_id: u64,
}

impl ShapeTree {
    /// an empty tree; new shapes get IDs from 4 on ([MS-VSDX] ShapeSheet_Type: an ID is at least 4)
    pub fn new() -> ShapeTree {
        ShapeTree { next_id: FIRST_ID as u64, ..ShapeTree::default() }
    }

    /// add a shape with its file ID under `parent` (None: top level), on top of its siblings
    /// -> its slot (untracked: no undo step; loaders build trees with it)
    pub fn insert(&mut self, parent: Option<u32>, id: u32, data: ShapeData) -> Result<u32, TreeError> {
        if self.by_id.contains_key(&id) {
            return Err(TreeError::DuplicateId(id));
        }
        if let Some(p) = parent {
            if self.node(p).is_none() {
                return Err(TreeError::NoParent);
            }
        }
        let slot = self.slots.len() as u32;
        self.slots.push(Some(Box::new(Node { id, parent, children: Vec::new(), data })));
        self.by_id.insert(id, slot);
        self.next_id = self.next_id.max(id as u64 + 1);
        match parent {
            Some(p) => self.node_mut(p).expect("checked").children.push(slot),
            None => self.top.push(slot),
        }
        Ok(slot)
    }

    /// the shape in a slot
    pub fn get(&self, slot: u32) -> Option<&ShapeData> {
        self.node(slot).map(|n| &n.data)
    }

    /// the file ID of the shape in a slot
    pub fn id(&self, slot: u32) -> Option<u32> {
        self.node(slot).map(|n| n.id)
    }

    /// the slot of the shape with this file ID
    pub fn by_id(&self, id: u32) -> Option<u32> {
        self.by_id.get(&id).copied()
    }

    /// the top-level shapes in z-order (bottom first)
    pub fn top(&self) -> &[u32] {
        &self.top
    }

    /// the subshapes of a shape in z-order
    pub fn children(&self, slot: u32) -> &[u32] {
        self.node(slot).map_or(&[], |n| n.children.as_slice())
    }

    /// the group a shape belongs to (None: top level or no shape)
    pub fn parent(&self, slot: u32) -> Option<u32> {
        self.node(slot).and_then(|n| n.parent)
    }

    /// the number of shapes (all levels)
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// whether the tree has no shapes
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// the ID the next new shape gets (None: the tree has had the ID 4294967295, no new IDs)
    pub fn next_id(&self) -> Option<u32> {
        u32::try_from(self.next_id.max(FIRST_ID as u64)).ok()
    }

    /// every shape slot, each group before its subshapes, in z-order
    pub fn iter(&self) -> TreeIter<'_> {
        let mut stack: Vec<u32> = self.top.iter().rev().copied().collect();
        stack.shrink_to_fit();
        TreeIter { tree: self, stack }
    }

    /// whether `slot` is `ancestor` or lies inside it
    pub fn is_within(&self, mut slot: u32, ancestor: u32) -> bool {
        loop {
            if slot == ancestor {
                return true;
            }
            match self.parent(slot) {
                Some(p) => slot = p,
                None => return false,
            }
        }
    }

    pub(crate) fn node(&self, slot: u32) -> Option<&Node> {
        self.slots.get(slot as usize).and_then(|s| s.as_deref())
    }

    pub(crate) fn node_mut(&mut self, slot: u32) -> Option<&mut Node> {
        self.slots.get_mut(slot as usize).and_then(|s| s.as_deref_mut())
    }

    pub(crate) fn data_mut(&mut self, slot: u32) -> Option<&mut ShapeData> {
        self.node_mut(slot).map(|n| &mut n.data)
    }

    // a new slot for a shape made by an edit (None: no IDs left; IDs are never handed out twice)
    pub(crate) fn alloc(&mut self) -> Option<(u32, u32)> {
        let id = self.next_id()?;
        self.next_id = id as u64 + 1;
        let slot = self.slots.len() as u32;
        self.slots.push(None);
        Some((slot, id))
    }

    // the list a shape sits in (its parent's children or the top level)
    pub(crate) fn siblings_mut(&mut self, parent: Option<u32>) -> Option<&mut Vec<u32>> {
        match parent {
            Some(p) => self.node_mut(p).map(|n| &mut n.children),
            None => Some(&mut self.top),
        }
    }

    pub(crate) fn siblings(&self, parent: Option<u32>) -> &[u32] {
        match parent {
            Some(p) => self.children(p),
            None => &self.top,
        }
    }

    // take a shape and its subshapes out (each node keeps its parent and children) -> pre-order
    pub(crate) fn take_subtree(&mut self, slot: u32) -> Vec<(u32, Box<Node>)> {
        let mut out = Vec::new();
        let mut stack = alloc::vec![slot];
        while let Some(s) = stack.pop() {
            if let Some(node) = self.slots.get_mut(s as usize).and_then(|x| x.take()) {
                self.by_id.remove(&node.id);
                stack.extend(node.children.iter().rev());
                out.push((s, node));
            }
        }
        out
    }

    // put nodes taken by take_subtree back into their slots
    pub(crate) fn put_subtree(&mut self, nodes: Vec<(u32, Box<Node>)>) {
        for (s, node) in nodes {
            let s = s as usize;
            if self.slots.len() <= s {
                self.slots.resize_with(s + 1, || None);
            }
            self.by_id.insert(node.id, s as u32);
            self.next_id = self.next_id.max(node.id as u64 + 1);
            self.slots[s] = Some(node);
        }
    }
}

/// the slots of a tree, groups before their subshapes (ShapeTree::iter)
pub struct TreeIter<'a> {
    tree: &'a ShapeTree,
    stack: Vec<u32>,
}

impl Iterator for TreeIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let slot = self.stack.pop()?;
        self.stack.extend(self.tree.children(slot).iter().rev());
        Some(slot)
    }
}
