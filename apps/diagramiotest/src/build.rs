// build -- own documents made through the model API for the writing tests and the Visio check:
// shapes with geometry and text, a 1-D connector glued to two shapes, masters and an instance,
// a picture.  Everything is made here (generic names, own values), nothing comes from a file.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    Cell, Connect, DocKind, Document, ForeignData, ForeignPayload, Formula, Master, MasterHeader, Owner, Part, Row, RowKey,
    SecKey, Section, ShapeData, ShapeKind, ShapeTree, StyleRefs, Text, TextItem, Unit, Value,
};
use diagram_io::vsdx::{self, Measure};

const MM: f64 = 1.0 / 25.4;

/// a cell with a value and, when given, a formula
fn c(doc: &mut Document, name: &str, v: f64, unit: Option<Unit>, f: Option<&str>) -> Cell {
    let n = doc.intern(name);
    let formula = f.map_or(Formula::Absent, |f| Formula::Expr(doc.intern(f)));
    Cell { formula, ..Cell::constant(n, Value::Num(v), unit) }
}

fn row(doc: &mut Document, ix: u32, kind: &str, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Index(ix));
    r.kind = Some(doc.intern(kind));
    r.cells = cells;
    r
}

/// the geometry of a rectangle or an ellipse filling the shape (formulas after Width, Height)
fn geometry(doc: &mut Document, w: f64, h: f64, ellipse: bool) -> Section {
    let name = doc.intern("Geometry");
    let mut s = Section::new(SecKey { name, index: 0 });
    s.ix_written = true;
    s.cells = alloc::vec![c(doc, "NoFill", 0.0, None, None), c(doc, "NoLine", 0.0, None, None), c(doc, "NoShow", 0.0, None, None), c(doc, "NoSnap", 0.0, None, None)];
    if ellipse {
        let cells = alloc::vec![
            c(doc, "X", w / 2.0, Some(Unit::Millimeters), Some("Width*0.5")),
            c(doc, "Y", h / 2.0, Some(Unit::Millimeters), Some("Height*0.5")),
            c(doc, "A", w, Some(Unit::Millimeters), Some("Width*1")),
            c(doc, "B", h / 2.0, Some(Unit::Millimeters), Some("Height*0.5")),
            c(doc, "C", w / 2.0, Some(Unit::Millimeters), Some("Width*0.5")),
            c(doc, "D", h, Some(Unit::Millimeters), Some("Height*1")),
        ];
        s.rows.push(row(doc, 1, "Ellipse", cells));
        return s;
    }
    let pts = [(0.0, 0.0, "MoveTo", "Width*0", "Height*0"), (w, 0.0, "LineTo", "Width*1", "Height*0"), (w, h, "LineTo", "Width*1", "Height*1"), (0.0, h, "LineTo", "Width*0", "Height*1"), (0.0, 0.0, "LineTo", "Geometry1.X1", "Geometry1.Y1")];
    for (i, (x, y, kind, fx, fy)) in pts.into_iter().enumerate() {
        let cells = alloc::vec![c(doc, "X", x, Some(Unit::Millimeters), Some(fx)), c(doc, "Y", y, Some(Unit::Millimeters), Some(fy))];
        s.rows.push(row(doc, i as u32 + 1, kind, cells));
    }
    s
}

/// a 2-D shape: pin, size, local pin, geometry, text
pub fn box_shape(doc: &mut Document, x: f64, y: f64, w: f64, h: f64, text: &str, ellipse: bool) -> ShapeData {
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = StyleRefs::all(1);
    d.sheet.cells = alloc::vec![
        c(doc, "PinX", x, Some(Unit::Millimeters), None),
        c(doc, "PinY", y, Some(Unit::Millimeters), None),
        c(doc, "Width", w, Some(Unit::Millimeters), None),
        c(doc, "Height", h, Some(Unit::Millimeters), None),
        c(doc, "LocPinX", w / 2.0, Some(Unit::Millimeters), Some("Width*0.5")),
        c(doc, "LocPinY", h / 2.0, Some(Unit::Millimeters), Some("Height*0.5")),
        c(doc, "Angle", 0.0, Some(Unit::Degrees), None),
        c(doc, "FlipX", 0.0, Some(Unit::Bool), None),
        c(doc, "FlipY", 0.0, Some(Unit::Bool), None),
        c(doc, "ResizeMode", 0.0, None, None),
    ];
    d.sheet.sections.push(geometry(doc, w, h, ellipse));
    if !text.is_empty() {
        d.text = Some(Box::new(Text::plain(text)));
    }
    d
}

/// a straight 1-D connector from p0 to p1 (horizontal or vertical), its ends glued to the shapes `from`
/// and `to` (BegTrigger / EndTrigger follow their moves)
pub fn connector(doc: &mut Document, from: u32, to: u32, p0: (f64, f64), p1: (f64, f64)) -> ShapeData {
    // straight across or up and down (no_std: no square roots needed)
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = dx.abs() + dy.abs();
    let angle = match (dx == 0.0, dy >= 0.0, dx >= 0.0) {
        (true, true, _) => core::f64::consts::FRAC_PI_2,
        (true, false, _) => -core::f64::consts::FRAC_PI_2,
        (false, _, true) => 0.0,
        (false, _, false) => core::f64::consts::PI,
    };
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = StyleRefs::all(2);
    let begin_trigger = alloc::format!("_XFTRIGGER(Sheet.{from}!EventXFMod)");
    let end_trigger = alloc::format!("_XFTRIGGER(Sheet.{to}!EventXFMod)");
    d.sheet.cells = alloc::vec![
        c(doc, "PinX", (p0.0 + p1.0) / 2.0, Some(Unit::Millimeters), Some("(BeginX+EndX)/2")),
        c(doc, "PinY", (p0.1 + p1.1) / 2.0, Some(Unit::Millimeters), Some("(BeginY+EndY)/2")),
        c(doc, "Width", len, Some(Unit::Millimeters), Some("SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)")),
        c(doc, "Height", 0.0, Some(Unit::Millimeters), None),
        c(doc, "LocPinX", len / 2.0, Some(Unit::Millimeters), Some("Width*0.5")),
        c(doc, "LocPinY", 0.0, Some(Unit::Millimeters), Some("Height*0.5")),
        c(doc, "Angle", angle, Some(Unit::Degrees), Some("ATAN2(EndY-BeginY,EndX-BeginX)")),
        c(doc, "BeginX", p0.0, Some(Unit::Millimeters), None),
        c(doc, "BeginY", p0.1, Some(Unit::Millimeters), None),
        c(doc, "EndX", p1.0, Some(Unit::Millimeters), None),
        c(doc, "EndY", p1.1, Some(Unit::Millimeters), None),
        c(doc, "EndArrow", 4.0, None, None),
        c(doc, "ObjType", 2.0, None, None),
    ];
    for (n, f) in [("BegTrigger", begin_trigger), ("EndTrigger", end_trigger)] {
        let name = doc.intern(n);
        let f = doc.intern(&f);
        d.sheet.cells.push(Cell { formula: Formula::Expr(f), ..Cell::constant(name, Value::Num(2.0), None) });
    }
    let name = doc.intern("Geometry");
    let mut g = Section::new(SecKey { name, index: 0 });
    g.ix_written = true;
    g.cells = alloc::vec![c(doc, "NoFill", 1.0, None, None), c(doc, "NoLine", 0.0, None, None)];
    let r1 = alloc::vec![c(doc, "X", 0.0, Some(Unit::Millimeters), Some("Width*0")), c(doc, "Y", 0.0, Some(Unit::Millimeters), Some("Height*0"))];
    let r2 = alloc::vec![c(doc, "X", len, Some(Unit::Millimeters), Some("Width*1")), c(doc, "Y", 0.0, Some(Unit::Millimeters), Some("Height*0"))];
    g.rows.push(row(doc, 1, "MoveTo", r1));
    g.rows.push(row(doc, 2, "LineTo", r2));
    d.sheet.sections.push(g);
    d
}

fn glue(doc: &mut Document, conn: u32, end: &str, part: i32, to: u32) -> Connect {
    Connect {
        from_sheet: conn,
        from_cell: Some(doc.intern(end)),
        from_part: Some(part),
        to_sheet: to,
        to_cell: Some(doc.intern("PinX")),
        to_part: Some(3),
        extras: Default::default(),
    }
}

/// a master with one shape (its master shape) and an icon left out (Visio draws one)
pub fn master(doc: &mut Document, id: u32, name: &str, ellipse: bool, guid: u32) -> Master {
    let n = doc.intern(name);
    let base = doc.intern(&alloc::format!("{{4A464C42-0000-4000-8000-{:012X}}}", 0x0005_0000 + guid));
    let unique = doc.intern(&alloc::format!("{{4A464C42-0000-4000-8000-{:012X}}}", 0x0006_0000 + guid));
    let mut m = Master::new(MasterHeader { id, name: Some(n), name_u: Some(n), base_id: Some(base), unique_id: Some(unique), sheet_styles: StyleRefs::all(0), ..MasterHeader::default() });
    let (w, h) = (40.0 * MM, 24.0 * MM);
    let pw = doc.intern("PageWidth");
    let ph = doc.intern("PageHeight");
    m.sheet.cells = alloc::vec![Cell::constant(pw, Value::Num(w), Some(Unit::Millimeters)), Cell::constant(ph, Value::Num(h), Some(Unit::Millimeters))];
    let mut d = box_shape(doc, w / 2.0, h / 2.0, w, h, "", ellipse);
    d.header.name_u = Some(n);
    d.header.name = Some(n);
    m.shapes = ShapeTree::new();
    let _ = m.shapes.insert(None, 5, d);
    m
}

/// a new drawing with own shapes: two boxes, an ellipse, two connectors glued between them,
/// an instance of a master of the document, a group, a picture in a part of its own
pub fn drawing(measure: Measure, picture: Option<Vec<u8>>) -> Document {
    let mut doc = vsdx::new_document(DocKind::Drawing, measure);
    let m = master(&mut doc, 2, "Box", false, 1);
    let _ = doc.insert_master(m);
    let Some(pr) = doc.pages().next() else { return doc };
    let a = box_shape(&mut doc, 60.0 * MM, 150.0 * MM, 50.0 * MM, 25.0 * MM, "Start", false);
    let b = box_shape(&mut doc, 160.0 * MM, 150.0 * MM, 50.0 * MM, 25.0 * MM, "Prüfen & weiter", false);
    let e = box_shape(&mut doc, 160.0 * MM, 70.0 * MM, 50.0 * MM, 30.0 * MM, "Ende", true);
    let c1 = connector(&mut doc, 4, 5, (85.0 * MM, 150.0 * MM), (135.0 * MM, 150.0 * MM));
    let c2 = connector(&mut doc, 5, 6, (160.0 * MM, 137.5 * MM), (160.0 * MM, 85.0 * MM));
    // an instance: Master='2', its own pin, the rest inherited
    let mut inst = ShapeData::new(ShapeKind::Shape);
    inst.header.master = Some(2);
    let px = doc.intern("PinX");
    let py = doc.intern("PinY");
    inst.sheet.cells = alloc::vec![Cell::constant(px, Value::Num(250.0 * MM), Some(Unit::Millimeters)), Cell::constant(py, Value::Num(150.0 * MM), Some(Unit::Millimeters))];
    inst.text = Some(Box::new(Text { items: alloc::vec![TextItem::Cp(0), TextItem::Chars(Box::from("Instanz"))], extras: Default::default() }));
    // a group of two small boxes
    let mut g = box_shape(&mut doc, 60.0 * MM, 60.0 * MM, 50.0 * MM, 30.0 * MM, "", false);
    g.header.kind = ShapeKind::Group;
    g.sheet.sections.clear();
    let g1 = box_shape(&mut doc, 12.5 * MM, 15.0 * MM, 20.0 * MM, 20.0 * MM, "A", false);
    let g2 = box_shape(&mut doc, 37.5 * MM, 15.0 * MM, 20.0 * MM, 20.0 * MM, "B", true);
    let pic = picture.map(|data| {
        let name = doc.intern("/visio/media/image1.png");
        let ct = doc.intern("image/png");
        doc.parts.push(Part { name, content_type: ct, rel_type: diagram_core::Sym::EMPTY, data });
        let mut d = box_shape(&mut doc, 250.0 * MM, 70.0 * MM, 20.0 * MM, 20.0 * MM, "", false);
        d.header.kind = ShapeKind::Foreign;
        d.sheet.sections.clear();
        let attrs = alloc::vec![(doc.intern("ForeignType"), doc.intern("Bitmap")), (doc.intern("CompressionType"), doc.intern("PNG"))];
        d.foreign = Some(Box::new(ForeignData { attrs, payload: ForeignPayload::Part(name), extras: Default::default() }));
        d
    });
    let connects = alloc::vec![glue(&mut doc, 7, "BeginX", 9, 4), glue(&mut doc, 7, "EndX", 12, 5), glue(&mut doc, 8, "BeginX", 9, 5), glue(&mut doc, 8, "EndX", 12, 6)];
    let Some(page) = doc.page_mut(pr) else { return doc };
    let t = &mut page.shapes;
    let _ = t.insert(None, 4, a);
    let _ = t.insert(None, 5, b);
    let _ = t.insert(None, 6, e);
    let _ = t.insert(None, 7, c1);
    let _ = t.insert(None, 8, c2);
    let _ = t.insert(None, 9, inst);
    if let Ok(gs) = t.insert(None, 10, g) {
        let _ = t.insert(Some(gs), 11, g1);
        let _ = t.insert(Some(gs), 12, g2);
    }
    if let Some(p) = pic {
        let _ = t.insert(None, 13, p);
    }
    page.connects = connects;
    let _ = Owner::Page(pr);
    doc
}

/// a new stencil with three own masters (box, ellipse, a second box)
pub fn stencil(measure: Measure) -> Document {
    let mut doc = vsdx::new_document(DocKind::Stencil, measure);
    for (id, name, ellipse) in [(2, "Kasten", false), (3, "Ellipse", true), (4, "Rahmen", false)] {
        let m = master(&mut doc, id, name, ellipse, id);
        let _ = doc.insert_master(m);
    }
    doc
}

/// a name for messages
pub fn kind_name(kind: DocKind) -> String {
    String::from(match kind {
        DocKind::Drawing => "drawing",
        DocKind::Template => "template",
        DocKind::Stencil => "stencil",
    })
}

// a CRC-32 (PNG chunks), bit by bit
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// a real 24x24 PNG (RGB, zlib with stored blocks): a Signalblau square with a white diagonal
pub fn png() -> Vec<u8> {
    const N: usize = 24;
    let mut raw = Vec::new();
    for y in 0..N {
        raw.push(0u8);
        for x in 0..N {
            let px: [u8; 3] = if x == y || x + 1 == y { [0xFF, 0xFF, 0xFF] } else { [0x1F, 0x5F, 0xA8] };
            raw.extend_from_slice(&px);
        }
    }
    // zlib: header, one stored block, Adler-32
    let mut z = alloc::vec![0x78u8, 0x01, 0x01];
    z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
    z.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = alloc::vec![0x89u8, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(N as u32).to_be_bytes());
    ihdr.extend_from_slice(&(N as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
