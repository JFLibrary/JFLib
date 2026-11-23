// cells -- the sections, rows and cells of a binary object (shape, style, sheet) as the XML of
// Visio 2003-2010 writes them (vss/mod.rs says what the format is)
//
// A row chunk holds its cells in fixed fields, a byte 1, a mask with a bit per cell that holds a
// value of its own (no formula), and then blocks: [u32 length][u8 kind][u8 cell][...]; kind 2 is
// the cell's formula (formula.rs), the others (references between cells) are recomputed by
// whoever needs them and left out here.  A cell with a formula block is written with its formula,
// one with its mask bit with its value alone, any other is inherited and not written.  The cell
// numbers of masks and blocks follow the order of the XML schema; the fields' places are a table
// per chunk type (LAYOUTS).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::stream::{f64at, u16at, u32at};
use super::{formula, Bin, Obj, R};
use crate::vsdx::Error;

/// a shape's or style's references to its styles
#[derive(Clone, Copy, Default)]
pub(super) struct Refs {
    pub line: Option<u32>,
    pub fill: Option<u32>,
    pub text: Option<u32>,
}

const NONE: u32 = 0xFFFF_FFFF;

// the header of an object's list (the bytes between the list's lengths and its child IDs)
fn head<'s>(o: &Obj, s: &'s [u8]) -> &'s [u8] {
    o.c.list_parts(s).map_or(&[], |(h, _)| h)
}

/// the styles an object names in its header
pub(super) fn style_refs(o: &Obj, s: &[u8]) -> Refs {
    let h = head(o, s);
    let get = |at: usize| u32at(h, at).filter(|&v| v != NONE);
    match o.c.ty {
        0x47 | 0x48 | 0x4D | 0x4E => Refs { fill: get(26), line: get(34), text: get(42) },
        _ => Refs { fill: get(26), line: get(34), text: get(42) },
    }
}

pub(super) fn refs_attrs(out: &mut super::Out, r: Refs) {
    if let Some(v) = r.line {
        out.attr("LineStyle", &format!("{v}"));
    }
    if let Some(v) = r.fill {
        out.attr("FillStyle", &format!("{v}"));
    }
    if let Some(v) = r.text {
        out.attr("TextStyle", &format!("{v}"));
    }
}

/// the group a shape belongs to (0: none)
pub(super) fn shape_parent(o: &Obj, s: &[u8]) -> u32 {
    u32at(head(o, s), 2).unwrap_or(0)
}

/// the IDs of a group's subshapes in their order (its list of type 0x65)
pub(super) fn group_children(o: &Obj, s: &[u8]) -> Vec<u32> {
    let mut v = Vec::new();
    for k in o.kids.iter().filter(|k| k.c.ty == 0x65) {
        for r in &k.kids {
            if r.c.ty == 0x83 {
                if let Some(id) = u32at(r.c.data(s), 0) {
                    v.push(id);
                }
            }
        }
    }
    v
}

// ---- the table of row layouts

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum K {
    /// a unit byte and a double
    Num,
    /// a double without unit
    Dbl,
    /// a byte, 0 or 1
    Bool,
    /// a byte
    U8,
    /// two bytes
    U16,
    /// two bytes, signed
    I16,
    /// four bytes
    U32,
    /// four bytes, signed
    I32,
    /// a colour: index byte, R, G, B (the transparency byte after them is a cell of its own, Trans)
    Color,
    /// a transparency byte (0..255 -> 0..1)
    Trans,
    /// a font index (two bytes) as the font's ID
    Font,
}

/// one cell of a row: its name, kind, byte offset and its number in masks and blocks
#[derive(Clone, Copy)]
pub(super) struct Field {
    pub name: &'static str,
    pub k: K,
    pub at: usize,
    pub ix: u8,
}

const fn f(name: &'static str, k: K, at: usize, ix: u8) -> Field {
    Field { name, k, at, ix }
}

/// where a row type goes
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// a single-row section (XForm, Line ...): its element holds the cells
    Single,
    /// a row of a section of rows: element per row with IX
    Row,
    /// a named row (User, Prop, Act, Control ...): element per row with NameU
    Named,
    /// a row of a Geom section (MoveTo, LineTo ...)
    Geom,
    /// the cells of the Geom element itself
    GeomHead,
}

pub(super) struct Layout {
    pub ty: u32,
    pub elem: &'static str,
    pub place: Place,
    /// the bytes of the fixed fields (the byte 1 follows)
    pub fixed: usize,
    /// the bytes of the mask after the byte 1
    pub mask: usize,
    pub fields: &'static [Field],
}

use K::*;

pub(super) const LAYOUTS: &[Layout] = &[
    Layout {
        ty: 0x9B,
        elem: "XForm",
        place: Place::Single,
        fixed: 66,
        mask: 2,
        fields: &[
            f("PinX", Num, 0, 0),
            f("PinY", Num, 9, 1),
            f("Width", Num, 18, 2),
            f("Height", Num, 27, 3),
            f("LocPinX", Num, 36, 4),
            f("LocPinY", Num, 45, 5),
            f("Angle", Num, 54, 6),
            f("FlipX", Bool, 63, 7),
            f("FlipY", Bool, 64, 8),
            f("ResizeMode", U8, 65, 9),
        ],
    },
    Layout {
        ty: 0x8A,
        elem: "MoveTo",
        place: Place::Geom,
        fixed: 18,
        mask: 1,
        fields: &[f("X", Num, 0, 0), f("Y", Num, 9, 1)],
    },
    Layout {
        ty: 0x8B,
        elem: "LineTo",
        place: Place::Geom,
        fixed: 18,
        mask: 1,
        fields: &[f("X", Num, 0, 0), f("Y", Num, 9, 1)],
    },
];

fn layout(ty: u32) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.ty == ty)
}

/// the name the XML gives a unit code of the binary format
pub(super) fn unit_name(code: u8) -> Option<&'static str> {
    Some(match code {
        0x20 => return None,
        0x21 => "PER",
        0x3F => "DP",
        0x40 => "DL",
        0x41 => "IN",
        0x42 => "FT",
        0x45 => "CM",
        0x46 => "MM",
        0x47 => "M",
        0x50 => "DA",
        0x51 => "DEG",
        0x53 => "RAD",
        _ => return None,
    })
}

// a double as the XML writes it
pub(super) fn num(v: f64) -> String {
    if v.is_finite() {
        let s = format!("{v}");
        s
    } else {
        String::from("0")
    }
}

/// the value of a field as XML text and its unit
fn value(d: &[u8], fl: &Field, bin: &Bin) -> Option<(String, Option<&'static str>)> {
    let at = fl.at;
    Some(match fl.k {
        Num => {
            let u = *d.get(at)?;
            (num(f64at(d, at + 1)?), unit_name(u))
        }
        Dbl => (num(f64at(d, at)?), None),
        Bool => (format!("{}", (*d.get(at)? != 0) as u8), None),
        U8 => (format!("{}", *d.get(at)?), None),
        U16 => (format!("{}", u16at(d, at)?), None),
        I16 => (format!("{}", u16at(d, at)? as i16), None),
        U32 => (format!("{}", u32at(d, at)?), None),
        I32 => (format!("{}", u32at(d, at)? as i32), None),
        Color => {
            let ix = *d.get(at)?;
            let rgb = d.get(at + 1..at + 4)?;
            if ix == 0xFF {
                (format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]), None)
            } else {
                (format!("{ix}"), None)
            }
        }
        Trans => (num(*d.get(at)? as f64 / 255.0), None),
        Font => {
            let ix = u16at(d, at)?;
            let _ = bin;
            (format!("{ix}"), None)
        }
    })
}

/// the formula blocks of a row: (cell number, tokens)
fn blocks<'s>(d: &'s [u8], from: usize) -> Vec<(u8, &'s [u8])> {
    let mut v = Vec::new();
    let mut i = from;
    while i + 6 <= d.len() {
        let len = u32at(d, i).unwrap_or(0) as usize;
        if len < 6 || i + len > d.len() {
            break;
        }
        if d[i + 4] == 2 {
            v.push((d[i + 5], &d[i + 6..i + len]));
        }
        i += len;
    }
    v
}

struct NoNames;
impl formula::Names for NoNames {
    fn row(&self, _: Option<u32>, _: u8, _: u32) -> Option<String> {
        None
    }
}

/// write one row's cells; `local`: the mask says this cell is local
fn row_cells(bin: &mut Bin, l: &Layout, d: &[u8]) -> R<()> {
    let mask = d.get(l.fixed + 1..l.fixed + 1 + l.mask).unwrap_or(&[]);
    let bl = blocks(d, l.fixed + 1 + l.mask);
    for fl in l.fields {
        let formula = bl.iter().find(|b| b.0 == fl.ix).and_then(|b| formula::decode(b.1, &NoNames));
        let has_block = bl.iter().any(|b| b.0 == fl.ix);
        let local = mask.get(fl.ix as usize / 8).is_some_and(|m| m & (1 << (fl.ix % 8)) != 0);
        if !has_block && !local {
            continue;
        }
        let Some((v, u)) = value(d, fl, bin) else { continue };
        if has_block && formula.is_none() {
            bin.warn(String::from("formula"));
        }
        bin.out.cell(fl.name, &v, u, formula.as_deref())?;
    }
    Ok(())
}

/// the sections of an object
pub(super) fn sheet(bin: &mut Bin, o: &Obj, s: &[u8]) -> R<()> {
    let mut geom_ix = 0u32;
    for sec in &o.kids {
        match sec.c.ty {
            // the property list: single-row sections
            0x68 => {
                for r in &sec.kids {
                    let Some(l) = layout(r.c.ty) else {
                        bin.warn(format!("chunk 0x{:02X}", r.c.ty));
                        continue;
                    };
                    if l.place != Place::Single {
                        continue;
                    }
                    bin.out.raw("<")?;
                    bin.out.raw(l.elem)?;
                    bin.out.raw(">")?;
                    row_cells(bin, l, r.c.data(s))?;
                    bin.out.end(l.elem)?;
                }
            }
            // a Geometry section
            0x6C => {
                bin.out.open("Geom");
                bin.out.attr("IX", &format!("{geom_ix}"));
                bin.out.close()?;
                geom_ix += 1;
                let mut row = 1u32;
                for r in &sec.kids {
                    let Some(l) = layout(r.c.ty) else {
                        bin.warn(format!("chunk 0x{:02X}", r.c.ty));
                        continue;
                    };
                    if l.place == Place::GeomHead {
                        row_cells(bin, l, r.c.data(s))?;
                        continue;
                    }
                    bin.out.open(l.elem);
                    bin.out.attr("IX", &format!("{row}"));
                    bin.out.close()?;
                    row += 1;
                    row_cells(bin, l, r.c.data(s))?;
                    bin.out.end(l.elem)?;
                }
                bin.out.end("Geom")?;
            }
            _ => {}
        }
    }
    let _ = Error::Limit("cells");
    Ok(())
}
