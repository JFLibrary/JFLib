// page -- pages, masters and styles: the containers of sheets and shapes
//
//   Page { header, sheet, shapes, connects, theme, extras, contents_extras, part }
//   PageHeader { id, name, name_u, background, back_page, sheet_styles, attrs ... }
//   Master { header, sheet, icon, shapes, connects, extras, contents_extras, part }
//   MasterHeader { id, name, name_u, base_id, unique_id, sheet_styles, attrs ... }
//   Style { header, sheet, extras }  StyleHeader { id, name, name_u, based_on, attrs ... }
//   Connect { from_sheet, from_cell, from_part, to_sheet, to_cell, to_part }
//   attr(&attrs, name) -> Option<Sym>   one of the attributes kept as written
//
// A foreground page names its background page by the background's page ID (back_page); a
// background page can have one of its own.  A master's content is a page of its own with
// shapes; a stencil is a document of masters without pages.  The attributes the model works
// with are fields, every other attribute stays in `attrs` in file order (ViewScale,
// ViewCenterX, Prompt, IconSize, PatternFlags ...), read with attr().
// Connect rows are the file's record of glue (CN-15): which cell of which shape (FromSheet,
// FromCell, FromPart) is glued to which part of which other shape (ToSheet, ToCell, ToPart);
// the sheets are shape IDs of the same page or master.

use alloc::vec::Vec;

use crate::shape::{ShapeTree, StyleRefs};
use crate::sheet::{Extras, Sheet};
use crate::strings::Sym;

/// the value of an attribute kept as written
pub fn attr(attrs: &[(Sym, Sym)], name: Sym) -> Option<Sym> {
    attrs.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

/// a Connect entry
#[derive(Clone, PartialEq, Debug)]
pub struct Connect {
    /// FromSheet: the ID of the glued shape (a connector, a callout)
    pub from_sheet: u32,
    /// FromCell: its glued cell (BeginX, EndX, Controls.Row_1 ...)
    pub from_cell: Option<Sym>,
    /// FromPart as written
    pub from_part: Option<i32>,
    /// ToSheet: the ID of the shape it is glued to
    pub to_sheet: u32,
    /// ToCell: the cell glued to (Connections.X1, PinX ...)
    pub to_cell: Option<Sym>,
    /// ToPart as written
    pub to_part: Option<i32>,
    /// unknown items
    pub extras: Extras,
}

/// the attributes of a page (and of its PageSheet element)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PageHeader {
    /// ID: unique among the pages of the document
    pub id: u32,
    /// Name: the language-dependent name ("Zeichenblatt-1")
    pub name: Option<Sym>,
    /// NameU: the universal name ("Page-1")
    pub name_u: Option<Sym>,
    /// IsCustomName as written
    pub is_custom_name: Option<bool>,
    /// IsCustomNameU as written
    pub is_custom_name_u: Option<bool>,
    /// Background="1": a background page
    pub background: bool,
    /// BackPage: the ID of this page's background page
    pub back_page: Option<u32>,
    /// LineStyle, FillStyle, TextStyle of the PageSheet element
    pub sheet_styles: StyleRefs,
    /// the other attributes of the Page element in file order
    pub attrs: Vec<(Sym, Sym)>,
}

/// a foreground or background page
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Page {
    /// the attributes
    pub header: PageHeader,
    /// the page's ShapeSheet (size, scale, layout, print, layers in the Layer section)
    pub sheet: Sheet,
    /// the shapes on the page
    pub shapes: ShapeTree,
    /// the glue records
    pub connects: Vec<Connect>,
    /// the theme of the page (an index into Document::themes)
    pub theme: Option<u32>,
    /// unknown items of the Page element (in pages.xml)
    pub extras: Extras,
    /// unknown items of the PageContents element (the page's own part)
    pub contents_extras: Extras,
    /// the name of the page's part as read (None: a new page)
    pub part: Option<Sym>,
}

impl Page {
    /// an empty page with this header
    pub fn new(header: PageHeader) -> Page {
        Page { header, shapes: ShapeTree::new(), ..Page::default() }
    }
}

/// the attributes of a master (and of its PageSheet element)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MasterHeader {
    /// ID: unique among the masters of the document
    pub id: u32,
    /// Name: the language-dependent name
    pub name: Option<Sym>,
    /// NameU: the universal name; with BaseID the key for matching masters (LG-04, MS-02)
    pub name_u: Option<Sym>,
    /// IsCustomName as written
    pub is_custom_name: Option<bool>,
    /// IsCustomNameU as written
    pub is_custom_name_u: Option<bool>,
    /// BaseID: the GUID of the master's origin
    pub base_id: Option<Sym>,
    /// UniqueID: the GUID of this master's version
    pub unique_id: Option<Sym>,
    /// LineStyle, FillStyle, TextStyle of the PageSheet element
    pub sheet_styles: StyleRefs,
    /// the other attributes of the Master element in file order (Prompt, Hidden, IconSize ...)
    pub attrs: Vec<(Sym, Sym)>,
}

/// a master: a template shape with its own page
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Master {
    /// the attributes
    pub header: MasterHeader,
    /// the master page's ShapeSheet
    pub sheet: Sheet,
    /// the icon as written (base64 text of the Icon element)
    pub icon: Option<Sym>,
    /// the master's shapes; the first top-level shape is the master shape of instances
    pub shapes: ShapeTree,
    /// glue records inside the master
    pub connects: Vec<Connect>,
    /// unknown items of the Master element (in masters.xml)
    pub extras: Extras,
    /// unknown items of the MasterContents element
    pub contents_extras: Extras,
    /// the name of the master's part as read
    pub part: Option<Sym>,
}

impl Master {
    /// an empty master with this header
    pub fn new(header: MasterHeader) -> Master {
        Master { header, shapes: ShapeTree::new(), ..Master::default() }
    }

    /// the slot of the shape instances inherit from (the first top-level shape)
    pub fn master_shape(&self) -> Option<u32> {
        self.shapes.top().first().copied()
    }
}

/// the attributes of a style sheet
#[derive(Clone, PartialEq, Debug, Default)]
pub struct StyleHeader {
    /// ID: unique among the styles (0 is "No Style")
    pub id: u32,
    /// Name: the language-dependent name
    pub name: Option<Sym>,
    /// NameU: the universal name
    pub name_u: Option<Sym>,
    /// IsCustomName as written
    pub is_custom_name: Option<bool>,
    /// IsCustomNameU as written
    pub is_custom_name_u: Option<bool>,
    /// LineStyle, FillStyle, TextStyle: the styles this one is based on
    pub based_on: StyleRefs,
    /// the other attributes in file order
    pub attrs: Vec<(Sym, Sym)>,
}

/// a style sheet (Formatvorlage)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Style {
    /// the attributes
    pub header: StyleHeader,
    /// its cells
    pub sheet: Sheet,
    /// unknown items of the StyleSheet element
    pub extras: Extras,
}
