// vss_sample -- (JD17) a small binary stencil of our own: three masters "Kasten" (a rectangle),
// "Kreis" (an ellipse) and "Pfeil" (an arrow), each a shape with XForm, a line, a fill and a
// Geometry section whose rows carry formulas (Width*0.5, Height*1 ...), built with vsskit after
// what lib/diagram_io/src/vss found in Visio's files.  No Microsoft content: names, numbers and
// formulas are ours.  It is the fixture of JFDiagrammer's tests/binary_stencil.uis and the
// semantic check of the group vss.
//
//   diagramiotest --write-vss DIR    writes DIR/jd17-schablone.vss
//
// The rows (cells.rs): fixed fields, the byte 1, a mask of cells with a value of their own, then
// blocks [u32 length][2][cell][tokens .. 0xFE]; a token 0x75 names a cell of the shape (2 Width,
// 3 Height), 0x20 a number (a double), 0x05 multiplies.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::{DocKind, MasterRef};
use diagram_io::{vsdx, vss};

use crate::vsskit::{chunk, list_chunk, utf16z, Doc, Entry, Pack, Stream, SEP, T_MASTER, T_NAME, T_NAMES, T_NAME_INDEX, T_PAGE_SHEET, T_PROMPT, T_SHAPE, T_STENCILS, T_TRAILER};
use crate::H;

const MM: u8 = 0x46;

fn p32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

// a cell: unit byte and double
fn num(d: &mut Vec<u8>, unit: u8, v: f64) {
    d.push(unit);
    d.extend_from_slice(&v.to_le_bytes());
}

// a formula block: cell `ix` = cell `of` * k (Width*0.5); `of` 2 Width, 3 Height
fn times(ix: u8, of: u8, k: f64) -> Vec<u8> {
    let mut t = vec![0x75, of, 0, 0, 0x60, 0x20];
    t.extend_from_slice(&k.to_le_bytes());
    t.extend_from_slice(&[0x05, 0xFE]);
    let mut b = Vec::new();
    p32(&mut b, 6 + t.len() as u32);
    b.extend_from_slice(&[2, ix]);
    b.extend(t);
    b
}

// a geometry row of type `ty` with X, Y (and more numbers) as fractions of Width and Height
fn row(ty: u32, id: u32, w: f64, h: f64, cells: &[(f64, bool)]) -> Vec<u8> {
    let mut d = Vec::new();
    for (k, &(f, of_w)) in cells.iter().enumerate() {
        let _ = k;
        num(&mut d, MM, f * if of_w { w } else { h });
    }
    d.extend_from_slice(&[1, 0]);
    for (k, &(f, of_w)) in cells.iter().enumerate() {
        d.extend(times(k as u8, if of_w { 2 } else { 3 }, f));
    }
    let mut c = chunk(ty, id, 2, &d);
    c.extend_from_slice(&SEP);
    c
}

// the XForm row: pin in the middle, local pin by formula
fn xform(w: f64, h: f64) -> Vec<u8> {
    let mut d = Vec::new();
    for v in [w / 2.0, h / 2.0, w, h, w / 2.0, h / 2.0] {
        num(&mut d, MM, v);
    }
    num(&mut d, 0x51, 0.0);
    // FlipX, FlipY, ResizeMode; the byte 1; mask: PinX .. Height, Angle .. ResizeMode local
    d.extend_from_slice(&[0, 0, 0, 1, 0xCF, 0x03]);
    d.extend(times(4, 2, 0.5));
    d.extend(times(5, 3, 0.5));
    d
}

// a line of 0.5 mm in colour index 10 (dark blue) and a light fill (RGB), both local
fn line_fill() -> (Vec<u8>, Vec<u8>) {
    let mut l = Vec::new();
    num(&mut l, 0x32, 0.5 / 25.4);
    l.extend_from_slice(&[10, 0, 0, 0x80, 0]);
    l.push(1);
    num(&mut l, 0x40, 0.0);
    l.extend_from_slice(&[2, 0, 0, 0, 2, 0]);
    // the byte 1, mask: LineWeight, LineColor, LinePattern
    l.extend_from_slice(&[1, 0x07, 0, 0, 0]);
    let mut f = Vec::new();
    f.extend_from_slice(&[24, 0xDD, 0xE8, 0xF5, 0]);
    f.extend_from_slice(&[0, 0, 0, 0, 0]);
    f.push(1);
    f.extend_from_slice(&[0; 11]);
    num(&mut f, 0x3F, 0.0);
    num(&mut f, 0x3F, 0.0);
    num(&mut f, 0x50, 0.0);
    f.extend_from_slice(&1f64.to_le_bytes());
    // the byte 1, mask: FillForegnd, FillPattern
    f.extend_from_slice(&[1, 0x05, 0]);
    (l, f)
}

// the chunk stream of a master's shape (ID 5) of w x h inches with a Geometry section of `rows`
fn shape(w: f64, h: f64, rows: &[(u32, Vec<(f64, bool)>)]) -> Vec<u8> {
    let (line, fill) = line_fill();
    let geo_rows: Vec<u32> = (0..=rows.len() as u32).collect();
    let mut s = list_chunk(T_SHAPE, 0xFFFF_FFFF, 1, 0, &shape_head(), &[2]);
    s.extend_from_slice(&SEP);
    s.extend(list_chunk(0x68, 1, 1, 1, &[], &[0, 1, 2]));
    s.extend_from_slice(&SEP);
    for (id, (ty, d)) in [(0u32, (0x9B, xform(w, h))), (1, (0x85, line)), (2, (0x86, fill))] {
        s.extend(chunk(ty, id, 2, &d));
        s.extend_from_slice(&SEP);
    }
    s.extend(list_chunk(0x6C, 2, 1, 1, &[], &geo_rows));
    s.extend_from_slice(&SEP);
    // the section's own cells: nothing hidden, NoFill local
    s.extend(chunk(0x89, 0, 2, &[0, 1, 1]));
    s.extend_from_slice(&SEP);
    for (k, (ty, cells)) in rows.iter().enumerate() {
        s.extend(row(*ty, k as u32 + 1, w, h, cells));
    }
    s
}

// a shape's header: no parent, no master, no styles (the stencil has none: line and fill are
// the shape's own)
fn shape_head() -> Vec<u8> {
    let mut h = vec![0u8; 68];
    h[0] = 2;
    let ff = 0xFFFF_FFFFu32.to_le_bytes();
    for at in [6, 10, 14, 18, 22, 26, 30, 34, 38, 42, 46] {
        h[at..at + 4].copy_from_slice(&ff);
    }
    h
}

/// the three masters: (Name, NameU, prompt, width, height in inches, geometry rows)
fn masters() -> Vec<(&'static str, &'static str, &'static str, f64, f64, Vec<(u32, Vec<(f64, bool)>)>)> {
    let (x, y) = (true, false);
    let rect = vec![
        (0x8A, vec![(0.0, x), (0.0, y)]),
        (0x8B, vec![(1.0, x), (0.0, y)]),
        (0x8B, vec![(1.0, x), (1.0, y)]),
        (0x8B, vec![(0.0, x), (1.0, y)]),
        (0x8B, vec![(0.0, x), (0.0, y)]),
    ];
    // Ellipse: centre X, Y; A, B a point on the X axis; C, D one on the Y axis
    let ellipse = vec![(0x8F, vec![(0.5, x), (0.5, y), (1.0, x), (0.5, y), (0.5, x), (1.0, y)])];
    let arrow = vec![
        (0x8A, vec![(0.0, x), (0.3, y)]),
        (0x8B, vec![(0.6, x), (0.3, y)]),
        (0x8B, vec![(0.6, x), (0.0, y)]),
        (0x8B, vec![(1.0, x), (0.5, y)]),
        (0x8B, vec![(0.6, x), (1.0, y)]),
        (0x8B, vec![(0.6, x), (0.7, y)]),
        (0x8B, vec![(0.0, x), (0.7, y)]),
        (0x8B, vec![(0.0, x), (0.3, y)]),
    ];
    vec![
        ("Kasten", "Box", "Ein Rechteck mit hellblauer Füllung.", 40.0 / 25.4, 25.0 / 25.4, rect),
        ("Kreis", "Circle", "Eine Ellipse.", 30.0 / 25.4, 30.0 / 25.4, ellipse),
        ("Pfeil", "Arrow", "Ein Blockpfeil nach rechts.", 40.0 / 25.4, 20.0 / 25.4, arrow),
    ]
}

/// the bytes of the stencil file
pub fn file() -> Vec<u8> {
    let ms = masters();
    let mut names = Vec::new();
    for (n, nu, ..) in &ms {
        for s in [n, nu] {
            let mut b = Vec::new();
            p32(&mut b, 1);
            b.extend(utf16z(s));
            names.push(Stream::blob(T_NAME, &b).packed(Pack::Repeats).s());
        }
    }
    let mut items = Vec::new();
    let mut index = Vec::new();
    p32(&mut index, ms.len() as u32);
    for (k, (_, _, prompt, w, h, rows)) in ms.iter().enumerate() {
        let sheet = chunk(T_PAGE_SHEET, 0xFFFF_FFFF, 0, &[0u8; 16]);
        let m = Stream::list(
            T_MASTER,
            vec![
                Stream::chunks(T_PAGE_SHEET, &sheet).packed(Pack::Repeats).s(),
                Entry::Null,
                Stream::blob(T_PROMPT, &utf16z(prompt)).packed(Pack::Repeats).s(),
                Entry::Null,
                Entry::Null,
                Stream::chunks(T_SHAPE, &shape(*w, *h, rows)).packed(Pack::Repeats).s(),
            ],
        )
        .packed(Pack::Repeats);
        items.push(m.s());
        p32(&mut index, 2 * k as u32);
        p32(&mut index, 2 * k as u32 + 1);
        p32(&mut index, k as u32);
        index.push(0);
    }
    items.push(Stream::blob(T_NAME_INDEX, &index).s());
    let order: Vec<u32> = (0..ms.len() as u32).collect();
    let list = Stream::list(T_STENCILS, items).order(&order).packed(Pack::Repeats);
    let root = Stream::list(T_TRAILER, vec![Stream::list(T_NAMES, names).packed(Pack::Literals).s(), list.s()]);
    crate::vsskit::file(&Doc::new(root).build())
}

/// diagramiotest --write-vss DIR
pub fn write(dir: &str) -> i32 {
    let path = alloc::format!("{dir}/jd17-schablone.vss");
    if plat::write_file(&path, &file()) {
        crate::out(&alloc::format!("wrote {path}\n"));
        0
    } else {
        crate::out(&alloc::format!("error: cannot write {path}\n"));
        1
    }
}

/// the checks of the sample: it reads, its masters and their geometry formulas are there, and
/// it goes to .vssx and back unchanged
pub fn run(h: &mut H) {
    let bytes = file();
    h.check("vss/sample/sniff", vss::sniff(&bytes), String::new());
    let r = vss::read(&bytes, DocKind::Stencil, &vsdx::Limits::VSDX);
    let Ok(l) = r else {
        h.check("vss/sample/read", false, alloc::format!("{:?}", r.err()));
        return;
    };
    h.check("vss/sample/read", l.warnings.is_empty(), alloc::format!("{:?}", l.warnings));
    let doc = &l.doc;
    let ms: Vec<MasterRef> = doc.masters().collect();
    let names: Vec<String> = ms.iter().filter_map(|&m| doc.master(m).and_then(|x| x.header.name).map(|s| String::from(doc.str(s)))).collect();
    h.check("vss/sample/masters", names == ["Kasten", "Kreis", "Pfeil"], alloc::format!("{names:?}"));
    // the formulas of the geometry: the first LineTo of the box is (Width*1, Height*0), the
    // ellipse's centre (Width*0.5, Height*0.5); the local pin's formulas are there
    let xml = vss::to_vdx(&bytes, DocKind::Stencil, &vsdx::Limits::VSDX).map(|(x, _)| x).unwrap_or_default();
    let want = ["<LineTo IX='2'><X Unit='MM' F='Width*1'>", "<Y Unit='MM' F='Height*0'>0</Y>", "<Ellipse IX='1'><X Unit='MM' F='Width*0.5'>", "<LocPinX Unit='MM' F='Width*0.5'>"];
    let missing: Vec<&str> = want.iter().copied().filter(|w| !xml.contains(w)).collect();
    h.check("vss/sample/formulas", missing.is_empty(), alloc::format!("{missing:?}"));
    let w = vsdx::write(doc, &vsdx::WriteOptions::DEFAULT);
    let back = w.as_ref().ok().and_then(|b| vsdx::read(b, &vsdx::Limits::VSDX).ok());
    h.check("vss/sample/vssx", back.as_ref().is_some_and(|b| b.doc.masters().count() == 3), alloc::format!("{:?}", w.err()));
}
