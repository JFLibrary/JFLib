// dxf -- DXF import (G6e2; docs/RENDER2D.md "DXF"): the ASCII format of AutoCAD R12 to 2018 read into a data model
// that needs no GPU: the header's variables, the tables of layers, linetypes, text styles, block records and
// dimension styles, blocks with their entities, and the entities of the drawing.  lib/render2d puts it into a scene
// (render2d::dxf); a converter or a test reads the same without a device.
//
//   dxf::load(bytes, &Limits) -> Result<Document>        the whole file in memory, every entity kept
//   dxf::read(&mut source, &Limits, &mut sink) -> Result<Stats>   streaming: the source hands out the file in pieces,
//        the sink gets the header, the tables, each block when it is complete and each entity of ENTITIES as it is
//        read (nothing of ENTITIES is kept: a file of a gigabyte loads in bounded memory)
//   mtext::parse(text, height) -> Mtext                   MTEXT's inline format codes as paragraphs of runs
//   specials(text) -> Vec<Special>                       TEXT's %% codes (degree, diameter, under- and overline)
//   aci_rgb(index), font_substitute(name)                AutoCAD's colour index, which repository font stands in
//
// Binary DXF is refused (Error::Binary).  Files are untrusted input: every count, length and size is bounded
// (Limits), a malformed value is skipped with a note, a group code that is no number ends the file with an error;
// nothing nests deeper than POLYLINE / VERTEX and INSERT / ATTRIB, so there is no recursion.  Strings before AutoCAD
// 2007 are in the header's code page ($DWGCODEPAGE, codepages.rs), from AC1021 on UTF-8; \U+XXXX is decoded
// everywhere, %% codes and MTEXT's codes where the text is drawn (specials, mtext).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

mod codepages;
mod fonts;
pub mod mtext;
mod pairs;
mod read;
mod strings;

pub use fonts::{font_substitute, Family, Substitute, SAMPLE};
pub use strings::{decode_unicode_escapes, specials, Special};

/// what went wrong; the text says where
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// a binary DXF ("AutoCAD Binary DXF"): not read (decision 31: the ASCII format)
    Binary,
    /// not a DXF: a group code that is no number, a pair cut off (the line in the text)
    Invalid(String),
    /// a Limits value exceeded
    Limit(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::Binary => write!(f, "DXF: a binary DXF, only the ASCII format is read"),
            Error::Invalid(s) => write!(f, "DXF: invalid: {s}"),
            Error::Limit(s) => write!(f, "DXF: over a limit: {s}"),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// the bounds a file must stay in (Limits::DEFAULT; raise them for large files you trust more)
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// bytes of one line (a group code or a value)
    pub line_bytes: usize,
    /// group pairs of one entity (with its vertices and attributes) and the bytes of their values
    pub entity_pairs: usize,
    pub entity_bytes: usize,
    /// points of one entity: vertices, control points, knots, a hatch's edges and its pattern's dashes
    pub entity_points: usize,
    /// entities kept: all of a Document's, or the blocks' while streaming
    pub kept_entities: u64,
    /// points kept (as entity_points, summed over what is kept)
    pub kept_points: u64,
    /// blocks, distinct names (layers, linetypes, styles, blocks), entries of all tables
    pub blocks: usize,
    pub names: usize,
    pub table_entries: usize,
}

impl Limits {
    pub const DEFAULT: Limits = Limits {
        line_bytes: 1 << 20,
        entity_pairs: 1 << 24,
        entity_bytes: 1 << 28,
        entity_points: 1 << 24,
        kept_entities: 1 << 26,
        kept_points: 1 << 28,
        blocks: 1 << 20,
        names: 1 << 22,
        table_entries: 1 << 20,
    };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// a point (x, y, z)
pub type P3 = [f64; 3];

/// a name of the file (a layer, linetype, text style, block, an entity's kind): an index into Names; names that differ
/// only in the case of ASCII letters are one (AutoCAD's rule)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Name(pub u32);

/// the names of a file, in the order they were first seen (Name(0) is the layer "0")
#[derive(Clone, Debug, Default)]
pub struct Names {
    list: Vec<String>,
    map: BTreeMap<Vec<u8>, u32>,
}

impl Names {
    /// the name's text as the file first spelled it
    pub fn get(&self, n: Name) -> &str {
        self.list.get(n.0 as usize).map_or("", |s| s.as_str())
    }

    /// a name that is already known
    pub fn find(&self, s: &str) -> Option<Name> {
        self.map.get(&key(s.as_bytes())[..]).map(|&i| Name(i))
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub(crate) fn intern(&mut self, s: &str, limit: usize) -> Result<Name> {
        let k = key(s.as_bytes());
        if let Some(&i) = self.map.get(&k) {
            return Ok(Name(i));
        }
        if self.list.len() >= limit {
            return Err(Error::Limit(alloc::format!("more than {limit} distinct names")));
        }
        let i = self.list.len() as u32;
        self.list.push(String::from(s));
        self.map.insert(k, i);
        Ok(Name(i))
    }
}

/// a name's key: ASCII letters in upper case
fn key(s: &[u8]) -> Vec<u8> {
    s.iter().map(|b| b.to_ascii_uppercase()).collect()
}

/// an entity's colour (62, 420): by layer, by block, an index of AutoCAD's palette (1 to 255; aci_rgb) or a true
/// colour 0xRRGGBB (420 wins over 62)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    ByLayer,
    ByBlock,
    Aci(u8),
    Rgb(u32),
}

/// an entity's linetype (6): by layer (also when absent), by block, or a linetype of the table ("CONTINUOUS" is one)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Linetype {
    #[default]
    ByLayer,
    ByBlock,
    Named(Name),
}

/// a lineweight (370): by layer, by block, the drawing's default ($LWDEFAULT), or hundredths of a millimetre
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lineweight {
    #[default]
    ByLayer,
    ByBlock,
    Default,
    Mm100(u16),
}

/// a transparency (440): by layer, by block, or an alpha (255: opaque, 0: invisible)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Transparency {
    #[default]
    ByLayer,
    ByBlock,
    Alpha(u8),
}

/// what every entity has
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Common {
    /// the entity's kind as the file names it (LINE, LWPOLYLINE, ...)
    pub kind: Name,
    /// 5 (0: none)
    pub handle: u64,
    /// 8 (the layer "0" when absent)
    pub layer: Name,
    pub linetype: Linetype,
    pub color: Color,
    pub lineweight: Lineweight,
    /// 48 (1 when absent)
    pub ltscale: f64,
    pub transparency: Transparency,
    /// 210 (0, 0, 1 when absent): the normal of the entity's object coordinate system (OCS)
    pub extrusion: P3,
    /// 39
    pub thickness: f64,
    /// 60 = 1
    pub invisible: bool,
    /// 67 = 1: in paper space
    pub paper: bool,
}

impl Default for Common {
    fn default() -> Common {
        Common { kind: Name(0), handle: 0, layer: Name(0), linetype: Linetype::ByLayer, color: Color::ByLayer, lineweight: Lineweight::ByLayer, ltscale: 1.0,
                 transparency: Transparency::ByLayer, extrusion: [0.0, 0.0, 1.0], thickness: 0.0, invisible: false, paper: false }
    }
}

/// an entity: what all have, and its kind's geometry
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    pub common: Common,
    pub kind: Kind,
}

/// a vertex of a polyline: its point (OCS for 2D polylines, WCS for 3D), the bulge of the segment that starts here (the
/// tangent of a quarter of its arc's angle, positive: counter-clockwise), the widths at its start and end
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vertex {
    pub at: P3,
    pub bulge: f64,
    pub start_width: f64,
    pub end_width: f64,
}

/// LWPOLYLINE, and POLYLINE with its VERTEXes (2D and 3D; meshes are Kind::Mesh)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Polyline {
    pub vertices: Vec<Vertex>,
    /// 70 bit 1
    pub closed: bool,
    /// 70 bit 128: the linetype's pattern runs on across the vertices (else it starts anew at each)
    pub plinegen: bool,
    /// a 3D polyline (POLYLINE 70 bit 8): points in WCS, no bulges, no widths
    pub three_d: bool,
    /// 43 (LWPOLYLINE) or 40 / 41 of POLYLINE: the width where a vertex has none of its own
    pub const_width: f64,
    /// 38 (LWPOLYLINE) or 30 of POLYLINE's point: the OCS z
    pub elevation: f64,
}

/// POLYLINE as a polyface mesh (70 bit 64: faces of up to four vertices, an edge invisible where its index is
/// negative) or a polygon mesh (70 bit 16: m by n vertices, closed in m or n)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mesh {
    pub vertices: Vec<P3>,
    /// polyface: per face its vertex indices from 1 (negative: the edge from it is invisible; 0: none)
    pub faces: Vec<[i32; 4]>,
    /// polygon mesh: its size and whether it closes
    pub m: u32,
    pub n: u32,
    pub closed_m: bool,
    pub closed_n: bool,
}

/// SPLINE (and the spline of a HELIX): degree, knots, control points with weights, fit points
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Spline {
    pub degree: u32,
    /// 70: 1 closed, 2 periodic, 4 rational, 8 planar, 16 linear
    pub flags: u32,
    pub knots: Vec<f64>,
    pub control: Vec<P3>,
    /// one per control point, or empty (all 1)
    pub weights: Vec<f64>,
    pub fit: Vec<P3>,
    pub start_tangent: Option<P3>,
    pub end_tangent: Option<P3>,
}

/// TEXT, ATTRIB and ATTDEF
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Text {
    /// 1, decoded (\U+XXXX; the %% codes stay: specials())
    pub text: String,
    /// 7 (Standard when absent)
    pub style: Option<Name>,
    /// 10, 11 (OCS)
    pub at: P3,
    pub at2: Option<P3>,
    /// 40
    pub height: f64,
    /// 50 (degrees)
    pub rotation: f64,
    /// 41, 51 (degrees): None when absent (the style's)
    pub width: Option<f64>,
    pub oblique: Option<f64>,
    /// 71: 2 backward, 4 upside down (None: the style's)
    pub generation: Option<u32>,
    /// 72 (0 left, 1 center, 2 right, 3 aligned, 4 middle, 5 fit), 73 (0 baseline, 1 bottom, 2 middle, 3 top)
    pub halign: u32,
    pub valign: u32,
    /// ATTRIB and ATTDEF: the tag (2) and the flags (70: 1 invisible, 2 constant, 4 verify, 8 preset)
    pub tag: Option<String>,
    pub attr_flags: u32,
    /// ATTRIB / ATTDEF of several lines (R2018: an MTEXT inside them)
    pub mtext: Option<alloc::boxed::Box<MText>>,
}

/// MTEXT
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MText {
    /// 3 ... and 1, joined, decoded (\U+XXXX); the format codes stay (mtext::parse)
    pub text: String,
    pub style: Option<Name>,
    /// 10 (WCS)
    pub at: P3,
    /// 40, 41 (the reference rectangle's width: 0, no wrapping), 46 (its height, for columns)
    pub height: f64,
    pub width: f64,
    pub rect_height: f64,
    /// 71 (1 top left to 9 bottom right), 72 (1 left to right, 3 top to bottom, 5 by style)
    pub attach: u32,
    pub direction: u32,
    /// 11 (WCS): the x axis' direction; else 50 (radians, in the OCS)
    pub x_dir: Option<P3>,
    pub rotation: f64,
    /// 73 (1 at least, 2 exactly), 44 (the factor of the single spacing)
    pub spacing_style: u32,
    pub spacing: f64,
    /// 90 (1 a background fill, 2 in the background colour, 16 a text frame), 63 / 421 its colour, 45 how far
    /// beyond the text (a factor of the height)
    pub bg_flags: u32,
    pub bg_color: Color,
    pub bg_scale: f64,
    /// R2018's columns (75 type: 0 none, 1 static, 2 dynamic; 76 count; 48 width; 49 gutter; 50 heights)
    pub column_type: u32,
    pub column_count: u32,
    pub column_width: f64,
    pub column_gutter: f64,
    pub column_heights: Vec<f64>,
}

/// INSERT (also MINSERT's array) with its ATTRIBs
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Insert {
    /// 2
    pub block: Name,
    /// 10 (OCS), 41 42 43, 50 (degrees)
    pub at: P3,
    pub scale: P3,
    pub rotation: f64,
    /// 70, 71 (0 read as 1), 44, 45
    pub columns: u32,
    pub rows: u32,
    pub spacing: [f64; 2],
    /// the ATTRIBs after it (each an Entity of Kind::Text)
    pub attribs: Vec<Entity>,
}

/// an edge of a hatch's boundary path (angles in degrees, as the file has them)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Edge {
    Line { a: [f64; 2], b: [f64; 2] },
    Arc { center: [f64; 2], radius: f64, start: f64, end: f64, ccw: bool },
    /// major: the end of the major axis relative to the centre; ratio: minor over major
    Ellipse { center: [f64; 2], major: [f64; 2], ratio: f64, start: f64, end: f64, ccw: bool },
    Spline { degree: u32, rational: bool, periodic: bool, knots: Vec<f64>, control: Vec<[f64; 2]>, weights: Vec<f64>, fit: Vec<[f64; 2]> },
    /// a polyline path: its vertices (x, y, bulge), closed
    Poly { vertices: Vec<[f64; 3]>, closed: bool },
}

/// a hatch's boundary path: its flags (92: 1 external, 2 polyline, 4 derived, 8 textbox, 16 outermost) and edges
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Path {
    pub flags: u32,
    pub edges: Vec<Edge>,
}

/// a line family of a hatch pattern as the file stores it, already scaled and turned: angle (53, degrees), a base
/// point (43, 44), the offset to the next line (45, 46) and dashes (49: positive on, negative off, 0 a dot)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PatternLine {
    pub angle: f64,
    pub base: [f64; 2],
    pub offset: [f64; 2],
    pub dashes: Vec<f64>,
}

/// HATCH
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Hatch {
    /// 2, 70 (solid fill), 75 (0 normal, 1 outer, 2 ignore islands), 52, 41, 77
    pub pattern: String,
    pub solid: bool,
    pub style: u32,
    pub angle: f64,
    pub scale: f64,
    pub double: bool,
    /// 30: the OCS z of the boundary
    pub elevation: f64,
    pub paths: Vec<Path>,
    pub lines: Vec<PatternLine>,
    /// 450 = 1: a gradient (its two colours, 63/421 after 453)
    pub gradient: bool,
    pub gradient_colors: Vec<Color>,
}

/// SOLID, TRACE (the outline 10, 11, 13, 12 in OCS) and 3DFACE (10, 11, 12, 13 in WCS; 70: invisible edges)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Quad {
    pub points: [P3; 4],
    pub face: bool,
    pub invisible_edges: u32,
}

/// what draws through an anonymous block: DIMENSION (and its kinds ARC_DIMENSION, LARGE_RADIAL_DIMENSION) and
/// ACAD_TABLE (2: the block; 10 its insertion point for a table)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlockRef {
    pub block: Option<Name>,
    pub at: P3,
    /// ACAD_TABLE's 11: its horizontal direction (WCS)
    pub x_dir: Option<P3>,
}

/// LEADER: its vertices (WCS), an arrowhead (71), a spline path (72), the dimension style (3)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Leader {
    pub vertices: Vec<P3>,
    pub arrow: bool,
    pub spline: bool,
    pub dimstyle: Option<Name>,
}

/// WIPEOUT (and IMAGE's frame, read alike): insertion point, the u and v vectors of a pixel, the size in pixels, the
/// clip boundary in pixel coordinates (71: 1 a rectangle of two corners, 2 a polygon)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Wipeout {
    pub at: P3,
    pub u: P3,
    pub v: P3,
    pub size: [f64; 2],
    pub clip_type: u32,
    pub clip: Vec<[f64; 2]>,
}

/// an entity's geometry
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Line { a: P3, b: P3 },
    /// POINT: WCS, 50 the angle of its symbol (degrees)
    Point { at: P3, angle: f64 },
    /// CIRCLE and ARC (OCS centre; angles in degrees, counter-clockwise from start to end)
    Circle { center: P3, radius: f64 },
    Arc { center: P3, radius: f64, start: f64, end: f64 },
    /// ELLIPSE: WCS centre, the major axis' end relative to it, minor / major, the parameters (radians)
    Ellipse { center: P3, major: P3, ratio: f64, start: f64, end: f64 },
    Spline(Spline),
    Polyline(Polyline),
    Mesh(Mesh),
    Text(Text),
    MText(MText),
    Insert(Insert),
    Hatch(Hatch),
    Quad(Quad),
    BlockRef(BlockRef),
    Leader(Leader),
    Wipeout(Wipeout),
    /// RAY (from `at`) and XLINE (both ways)
    Ray { at: P3, dir: P3, both: bool },
    /// what this reader does not read (its kind in Common::kind): counted, not drawn
    Other,
}

/// a layer of the table
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub name: Name,
    /// 62 (its magnitude) or 420
    pub color: Color,
    /// 62 negative: off; 70 bit 1: frozen, bit 4: locked; 290: plotted
    pub on: bool,
    pub frozen: bool,
    pub locked: bool,
    pub plot: bool,
    /// 6 (None: CONTINUOUS)
    pub linetype: Option<Name>,
    /// 370 (Default when absent)
    pub lineweight: Lineweight,
    /// the XDATA AcCmTransparency (1071): an alpha
    pub transparency: Option<u8>,
}

/// an element of a linetype: a dash (positive), a gap (negative) or a dot (0); a shape or text of a complex linetype
/// is kept as a flag (74) and its text
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dash {
    pub length: f64,
    pub complex: u32,
    pub text: Option<String>,
}

/// a linetype of the table
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LinetypeDef {
    pub name: Name,
    pub description: String,
    pub dashes: Vec<Dash>,
    /// 40
    pub total: f64,
}

/// a text style of the table
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StyleDef {
    pub name: Name,
    /// 3: the font file (txt, simplex.shx, arial.ttf, ...), 4 the big font file
    pub font: String,
    pub bigfont: String,
    /// the TrueType family the XDATA "ACAD" names (1000), if any
    pub family: Option<String>,
    /// 40 (0: none fixed), 41, 50 (degrees)
    pub height: f64,
    pub width: f64,
    pub oblique: f64,
    /// 70: 1 a shape file (not a text font), 4 vertical; 71: 2 backward, 4 upside down
    pub flags: u32,
    pub generation: u32,
}

/// a block record of the table
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlockRecord {
    pub name: Name,
    pub handle: u64,
    /// 70: the insertion units
    pub units: i32,
}

/// a dimension style (what a LEADER's arrow needs)
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DimStyle {
    pub name: Name,
    /// 41 DIMASZ, 40 DIMSCALE
    pub arrow_size: f64,
    pub scale: f64,
}

/// the tables
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Tables {
    pub layers: Vec<Layer>,
    pub linetypes: Vec<LinetypeDef>,
    pub styles: Vec<StyleDef>,
    pub block_records: Vec<BlockRecord>,
    pub dimstyles: Vec<DimStyle>,
}

impl Tables {
    pub fn layer(&self, n: Name) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == n)
    }

    pub fn linetype(&self, n: Name) -> Option<&LinetypeDef> {
        self.linetypes.iter().find(|l| l.name == n)
    }

    pub fn style(&self, n: Name) -> Option<&StyleDef> {
        self.styles.iter().find(|l| l.name == n)
    }

    pub fn dimstyle(&self, n: Name) -> Option<&DimStyle> {
        self.dimstyles.iter().find(|l| l.name == n)
    }
}

/// the header's variables this reader uses
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    /// $ACADVER (AC1009 R12 ... AC1032 2018; empty without a header), $DWGCODEPAGE
    pub version: String,
    pub codepage: String,
    /// $INSUNITS (0 unitless, 1 inch, 4 mm, 6 m, ...)
    pub insunits: i32,
    /// $LTSCALE, $CELTSCALE
    pub ltscale: f64,
    /// $LWDISPLAY (None: not in the file), $LWDEFAULT (hundredths of a mm)
    pub lwdisplay: Option<bool>,
    pub lwdefault: u16,
    /// $PDMODE, $PDSIZE
    pub pdmode: i32,
    pub pdsize: f64,
    /// $EXTMIN, $EXTMAX
    pub extmin: Option<P3>,
    pub extmax: Option<P3>,
    /// $DIMSCALE, $DIMASZ, $TEXTSIZE, $MIRRTEXT
    pub dimscale: f64,
    pub dimasz: f64,
    pub textsize: f64,
    pub mirrtext: bool,
    /// whether the file had a HEADER section
    pub present: bool,
}

impl Default for Header {
    fn default() -> Header {
        Header { version: String::new(), codepage: String::new(), insunits: 0, ltscale: 1.0, lwdisplay: None, lwdefault: 25, pdmode: 0, pdsize: 0.0, extmin: None, extmax: None,
                 dimscale: 1.0, dimasz: 0.18, textsize: 0.2, mirrtext: false, present: false }
    }
}

/// a block: its name, base point, flags (70: 1 anonymous, 2 attributes, 4 xref, 8 xref overlay, 16 external,
/// 32 resolved, 64 referenced), entities
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub name: Name,
    pub base: P3,
    pub flags: u32,
    /// 1: the file an xref names
    pub xref: String,
    pub entities: Vec<Entity>,
    /// *Paper_Space...: a layout's
    pub paper: bool,
}

/// where an entity marker was
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Where {
    Entities,
    Blocks,
}

/// what was read
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// entity markers per section and kind as the reader took them (VERTEX, ATTRIB, SEQEND, BLOCK, ENDBLK count too)
    pub kinds: BTreeMap<(Where, String), u64>,
    /// entities of ENTITIES in paper space (67 = 1)
    pub paper: u64,
    /// bytes and lines read, group pairs
    pub bytes: u64,
    pub lines: u64,
    pub pairs: u64,
    /// sections seen
    pub sections: Vec<String>,
    /// what was skipped or guessed, each once (at most 64)
    pub notes: Vec<String>,
}

impl Stats {
    pub(crate) fn note(&mut self, s: String) {
        if self.notes.len() < 64 && !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    /// the count of a kind in a section
    pub fn count(&self, w: Where, kind: &str) -> u64 {
        self.kinds.get(&(w, String::from(kind))).copied().unwrap_or(0)
    }
}

/// what a streaming read hands out, in the file's order: the header (also an empty one, before anything else), the
/// tables (once, when the TABLES section ends, or empty before the first block or entity), each block when its ENDBLK
/// is read, each entity of ENTITIES (paper space too: Common::paper)
pub trait Sink {
    fn header(&mut self, _h: &Header, _names: &Names) {}
    fn tables(&mut self, _t: &Tables, _names: &Names) {}
    fn block(&mut self, _b: Block, _names: &Names) {}
    fn entity(&mut self, _e: Entity, _names: &Names) {}
}

/// a whole file
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub header: Header,
    pub tables: Tables,
    pub blocks: Vec<Block>,
    pub entities: Vec<Entity>,
    pub names: Names,
    pub stats: Stats,
}

impl Document {
    /// a name's text
    pub fn name(&self, n: Name) -> &str {
        self.names.get(n)
    }

    /// a block by its name
    pub fn block(&self, n: Name) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == n)
    }
}

struct Collect {
    doc: Document,
}

impl Sink for Collect {
    fn header(&mut self, h: &Header, _: &Names) {
        self.doc.header = h.clone();
    }
    fn tables(&mut self, t: &Tables, _: &Names) {
        self.doc.tables = t.clone();
    }
    fn block(&mut self, b: Block, _: &Names) {
        self.doc.blocks.push(b);
    }
    fn entity(&mut self, e: Entity, _: &Names) {
        self.doc.entities.push(e);
    }
}

/// a whole file in memory into a Document (every entity kept: Limits::kept_entities and kept_points bound them)
pub fn load(bytes: &[u8], limits: &Limits) -> Result<Document> {
    let mut at = 0usize;
    let mut src = |buf: &mut [u8]| {
        let n = buf.len().min(bytes.len() - at);
        buf[..n].copy_from_slice(&bytes[at..at + n]);
        at += n;
        n
    };
    let mut c = Collect { doc: Document::default() };
    let (stats, names) = read::run(&mut src, limits, &mut c, true)?;
    c.doc.stats = stats;
    c.doc.names = names;
    Ok(c.doc)
}

/// a file read piece by piece: `source` fills a buffer and says how many bytes it put there (0: the end); the sink
/// gets what was read in the file's order.  Only the tables and the blocks are kept (Limits bound them); the entities
/// of ENTITIES go to the sink one by one
pub fn read(source: &mut dyn FnMut(&mut [u8]) -> usize, limits: &Limits, sink: &mut dyn Sink) -> Result<(Stats, Names)> {
    read::run(source, limits, sink, false)
}

/// whether bytes begin like a binary DXF
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(b"AutoCAD Binary DXF")
}

/// AutoCAD's colour index (ACI) as 0xRRGGBB: 1 red, 2 yellow, 3 green, 4 cyan, 5 blue, 6 magenta, 7 white (black on a
/// light background: the caller decides), 8 and 9 greys, 10 to 249 24 hues of 15 degrees, each in five brightnesses
/// (255, 204, 153, 127, 76) and each of those mixed half with white of that brightness, 250 to 255 greys; 0 and 256
/// (by block, by layer) are white
pub fn aci_rgb(i: u8) -> u32 {
    let rgb = |r: u32, g: u32, b: u32| (r << 16) | (g << 8) | b;
    match i {
        1 => rgb(255, 0, 0),
        2 => rgb(255, 255, 0),
        3 => rgb(0, 255, 0),
        4 => rgb(0, 255, 255),
        5 => rgb(0, 0, 255),
        6 => rgb(255, 0, 255),
        8 => rgb(128, 128, 128),
        9 => rgb(192, 192, 192),
        250 => rgb(51, 51, 51),
        251 => rgb(80, 80, 80),
        252 => rgb(105, 105, 105),
        253 => rgb(130, 130, 130),
        254 => rgb(190, 190, 190),
        10..=249 => {
            let k = (i - 10) as u32;
            let (hue, level) = (k / 10, k % 10);
            let v = [255.0, 204.0, 153.0, 127.0, 76.0][(level / 2) as usize];
            // the hue at full saturation and brightness 1, in steps of 15 degrees
            let h = hue as f64 * 15.0 / 60.0;
            let sector = h as u32;
            let f = h - sector as f64;
            let (r, g, b) = match sector {
                0 => (1.0, f, 0.0),
                1 => (1.0 - f, 1.0, 0.0),
                2 => (0.0, 1.0, f),
                3 => (0.0, 1.0 - f, 1.0),
                4 => (f, 0.0, 1.0),
                _ => (1.0, 0.0, 1.0 - f),
            };
            let ch = |c: f64| {
                let full = c * v;
                let c = if level % 2 == 1 { full + 0.5 * (v - full) } else { full };
                c as u32
            };
            rgb(ch(r), ch(g), ch(b))
        }
        _ => rgb(255, 255, 255),
    }
}
