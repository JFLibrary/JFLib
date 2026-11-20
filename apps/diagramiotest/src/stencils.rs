// stencils -- the stencil build tool (JD9, diagram_io::stencil): the start set of
// apps/JFDiagrammer/stencils/standard in both measures, a stencil from the SVG fixtures, and
// sources with faults
//
//   stencils/build      both measures build; write -> read -> write: the same model, the same
//                       bytes; two builds give the same bytes
//   stencils/masters    per master: NameU, names and QuickInfos in five languages
//                       (instance::MasterTexts), keywords and ShapeKeywords, the quick flag, the
//                       Master attributes, GUIDs, geometry with formulas and values, controls with
//                       translated tips, connection points, the icon (32x32, 4 bits, 766 bytes);
//                       metric 40 mm against US 1.5 in, other BaseIDs per measure
//   stencils/engine     the read-back stencil recalculated: nothing changes, nothing is kept
//   stencils/place      every master dropped into a new drawing (instance::import_master,
//                       instance::place in French, Engine::recalc): Name, NameU, "Name.ID" for
//                       the second drop, the arrow's translated tip, the text box's text
//   stencils/svg        tests/stencil_server.svg (two formats: a group of two subshapes) and
//                       tests/stencil_cloud.svg (one format: the master shape's own geometry)
//   stencils/errors     faults with file and line: a missing language, an unknown section, an
//                       unknown function, a missing Width, a missing master file, NameU twice,
//                       a reference into nothing, a cell text without all languages
//
//   diagramiotest --write-stencils DIR   jfd_standard_m.vssx, jfd_standard_u.vssx, the SVG test
//                       stencil jfd_svgtest_m.vssx and standard_dropped.vsdx (every master dropped
//                       on a drawing the way JFDiagrammer does it), for a look in Visio

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::formula::{Engine, Outcome};
use diagram_core::instance::{self, MasterTexts};
use diagram_core::{attr, CellAddr, DocKind, Document, Formula, MasterRef, Owner, RowKey, SecKey, ShapeKind, SheetRef, Unit, Value};
use diagram_io::stencil::{self, Built, SourceError};
use diagram_io::vsdx::{self, Limits, Measure, WriteOptions};

use crate::dump::{dump, first_difference};
use crate::{out, H};

/// the start set (apps/JFDiagrammer/stencils/standard)
pub(crate) const STANDARD: &[(&str, &str)] = &[
    ("stencil.txt", include_str!("../../JFDiagrammer/stencils/standard/stencil.txt")),
    ("rectangle.master", include_str!("../../JFDiagrammer/stencils/standard/rectangle.master")),
    ("rounded_rectangle.master", include_str!("../../JFDiagrammer/stencils/standard/rounded_rectangle.master")),
    ("ellipse.master", include_str!("../../JFDiagrammer/stencils/standard/ellipse.master")),
    ("triangle.master", include_str!("../../JFDiagrammer/stencils/standard/triangle.master")),
    ("diamond.master", include_str!("../../JFDiagrammer/stencils/standard/diamond.master")),
    ("arrow.master", include_str!("../../JFDiagrammer/stencils/standard/arrow.master")),
    ("textbox.master", include_str!("../../JFDiagrammer/stencils/standard/textbox.master")),
    // (JD13) the rest of SH-006
    ("square.master", include_str!("../../JFDiagrammer/stencils/standard/square.master")),
    ("circle.master", include_str!("../../JFDiagrammer/stencils/standard/circle.master")),
    ("right_triangle.master", include_str!("../../JFDiagrammer/stencils/standard/right_triangle.master")),
    ("pentagon.master", include_str!("../../JFDiagrammer/stencils/standard/pentagon.master")),
    ("hexagon.master", include_str!("../../JFDiagrammer/stencils/standard/hexagon.master")),
    ("heptagon.master", include_str!("../../JFDiagrammer/stencils/standard/heptagon.master")),
    ("octagon.master", include_str!("../../JFDiagrammer/stencils/standard/octagon.master")),
    ("decagon.master", include_str!("../../JFDiagrammer/stencils/standard/decagon.master")),
    ("can.master", include_str!("../../JFDiagrammer/stencils/standard/can.master")),
    ("parallelogram.master", include_str!("../../JFDiagrammer/stencils/standard/parallelogram.master")),
    ("trapezoid.master", include_str!("../../JFDiagrammer/stencils/standard/trapezoid.master")),
    ("cross.master", include_str!("../../JFDiagrammer/stencils/standard/cross.master")),
    ("chevron.master", include_str!("../../JFDiagrammer/stencils/standard/chevron.master")),
    ("cube.master", include_str!("../../JFDiagrammer/stencils/standard/cube.master")),
    ("star_4.master", include_str!("../../JFDiagrammer/stencils/standard/star_4.master")),
    ("star_5.master", include_str!("../../JFDiagrammer/stencils/standard/star_5.master")),
    ("star_6.master", include_str!("../../JFDiagrammer/stencils/standard/star_6.master")),
    ("star_7.master", include_str!("../../JFDiagrammer/stencils/standard/star_7.master")),
    ("star_16.master", include_str!("../../JFDiagrammer/stencils/standard/star_16.master")),
    ("star_24.master", include_str!("../../JFDiagrammer/stencils/standard/star_24.master")),
    ("star_32.master", include_str!("../../JFDiagrammer/stencils/standard/star_32.master")),
    ("rect_snip_1.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_1.master")),
    ("rect_snip_same.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_same.master")),
    ("rect_snip_diagonal.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_diagonal.master")),
    ("rect_round_1.master", include_str!("../../JFDiagrammer/stencils/standard/rect_round_1.master")),
    ("rect_round_same.master", include_str!("../../JFDiagrammer/stencils/standard/rect_round_same.master")),
    ("rect_round_diagonal.master", include_str!("../../JFDiagrammer/stencils/standard/rect_round_diagonal.master")),
    ("rect_snip_round.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_round.master")),
    ("rect_snip_corners.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_corners.master")),
    ("rect_snip_3.master", include_str!("../../JFDiagrammer/stencils/standard/rect_snip_3.master")),
    ("rect_round_3.master", include_str!("../../JFDiagrammer/stencils/standard/rect_round_3.master")),
    ("frame.master", include_str!("../../JFDiagrammer/stencils/standard/frame.master")),
    ("frame_corner.master", include_str!("../../JFDiagrammer/stencils/standard/frame_corner.master")),
    ("l_shape.master", include_str!("../../JFDiagrammer/stencils/standard/l_shape.master")),
    ("diagonal_stripe.master", include_str!("../../JFDiagrammer/stencils/standard/diagonal_stripe.master")),
    ("plaque.master", include_str!("../../JFDiagrammer/stencils/standard/plaque.master")),
    ("donut.master", include_str!("../../JFDiagrammer/stencils/standard/donut.master")),
    ("no_symbol.master", include_str!("../../JFDiagrammer/stencils/standard/no_symbol.master")),
    ("circle_from_center.master", include_str!("../../JFDiagrammer/stencils/standard/circle_from_center.master")),
    ("parenthesis_left.master", include_str!("../../JFDiagrammer/stencils/standard/parenthesis_left.master")),
    ("parenthesis_right.master", include_str!("../../JFDiagrammer/stencils/standard/parenthesis_right.master")),
    ("brace_left.master", include_str!("../../JFDiagrammer/stencils/standard/brace_left.master")),
    ("brace_right.master", include_str!("../../JFDiagrammer/stencils/standard/brace_right.master")),
];

const SVG_STENCIL: &str = "\
stencil: SVG test
file: svgtest
name.de: SVG-Test
name.en: SVG test
name.es: Prueba SVG
name.it: Prova SVG
name.fr: Test SVG
category: network
masters: server, cloud
quick: 1
language: en\n";

const SERVER: &str = "\
master: Server
name.de: Server
name.en: Server
name.es: Servidor
name.it: Server
name.fr: Serveur
prompt.de: Ein Server
prompt.en: A server
prompt.es: Un servidor
prompt.it: Un server
prompt.fr: Un serveur
svg: server.svg

[Shape]
Width = 12 mm
Width.us = 0.5 in
Height = 16 mm
Height.us = 0.625 in\n";

const CLOUD: &str = "\
master: Cloud
name.de: Wolke
name.en: Cloud
name.es: Nube
name.it: Nuvola
name.fr: Nuage
prompt.de: Eine Wolke
prompt.en: A cloud
prompt.es: Una nube
prompt.it: Una nuvola
prompt.fr: Un nuage
svg: cloud.svg

[Shape]
Width = 32 mm
Width.us = 1.25 in
Height = 20 mm
Height.us = 0.75 in\n";

const SVG_FILES: &[(&str, &str)] = &[
    ("stencil.txt", SVG_STENCIL),
    ("server.master", SERVER),
    ("cloud.master", CLOUD),
    ("server.svg", include_str!("../tests/stencil_server.svg")),
    ("cloud.svg", include_str!("../tests/stencil_cloud.svg")),
];

// a small stencil for the faults; the line numbers below count in these texts
const T_STENCIL: &str = "stencil: T\nfile: t\nname.de: T\nname.en: T\nname.es: T\nname.it: T\nname.fr: T\ncategory: general\nmasters: box\n";
const T_BOX: &str = "\
master: Box
name.de: Kasten
name.en: Box
name.es: Caja
name.it: Scatola
name.fr: Boîte
prompt.de: p
prompt.en: p
prompt.es: p
prompt.it: p
prompt.fr: p

[Shape]
Width = 20 mm
Height = 10 mm

[Geometry]
MoveTo X = 0; Y = 0
LineTo X = Width; Y = Height\n";

fn lookup<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, t)| String::from(*t))
}

pub(crate) fn build_set(table: &[(&str, &str)], m: Measure) -> Result<(Built, Vec<u8>), SourceError> {
    let f = lookup(table);
    stencil::build_vssx(&f, m)
}

// a GUID as Visio writes them: {XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}, upper case
fn guid_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 38
        && b[0] == b'{'
        && b[37] == b'}'
        && b[1..37].iter().enumerate().all(|(i, &c)| if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { c.is_ascii_digit() || (b'A'..=b'F').contains(&c) })
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if clean.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::new();
    for q in clean.chunks(4) {
        let pad = q.iter().filter(|&&c| c == b'=').count();
        let mut n = 0u32;
        for &c in q {
            n = (n << 6) | if c == b'=' { 0 } else { val(c)? };
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

fn le16(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 2).map_or(0, |x| u16::from_le_bytes([x[0], x[1]]) as u32)
}

fn le32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

// an icon as Visio writes it: a 32x32 ICO with 16 colours and a mask -> (opaque pixels, transparent corners)
fn icon_check(b: &[u8]) -> Result<(u32, bool), String> {
    if b.len() != 766 {
        return Err(format!("{} bytes, not 766", b.len()));
    }
    let head = [le16(b, 0), le16(b, 2), le16(b, 4), b[6] as u32, b[7] as u32, b[8] as u32, le16(b, 10), le16(b, 12), le32(b, 14), le32(b, 18)];
    if head != [0, 1, 1, 32, 32, 16, 1, 4, 744, 22] {
        return Err(format!("ICONDIR/ENTRY {head:?}"));
    }
    let bih = [le32(b, 22), le32(b, 26), le32(b, 30), le16(b, 34), le16(b, 36), le32(b, 38)];
    if bih != [40, 32, 64, 1, 4, 0] {
        return Err(format!("BITMAPINFOHEADER {bih:?}"));
    }
    if b[62..66] != [0, 0, 0, 0] {
        return Err(String::from("palette entry 0 is not black"));
    }
    let and = &b[766 - 128..];
    let transparent = |x: usize, y: usize| and[(31 - y) * 4 + x / 8] & (0x80 >> (x % 8)) != 0;
    let mut opaque = 0;
    for y in 0..32 {
        for x in 0..32 {
            if !transparent(x, y) {
                opaque += 1;
            }
        }
    }
    let corners = transparent(0, 0) && transparent(31, 0) && transparent(0, 31) && transparent(31, 31);
    Ok((opaque, corners))
}

pub(crate) fn read_back(h: &mut H, name: &str, bytes: &[u8]) -> Option<Document> {
    let l1 = match vsdx::read(bytes, &Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            h.check(name, false, format!("read: {e}"));
            return None;
        }
    };
    let b2 = vsdx::write(&l1.doc, &WriteOptions::DEFAULT).unwrap_or_default();
    let l2 = match vsdx::read(&b2, &Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            h.check(name, false, format!("read back: {e}"));
            return None;
        }
    };
    let (d1, d2) = (dump(&l1.doc), dump(&l2.doc));
    let what = if d1 == d2 { format!("{} bytes, {} warnings, model and bytes again the same", bytes.len(), l1.warnings.len()) } else { first_difference(&d1, &d2) };
    h.check(name, d1 == d2 && b2 == bytes && l1.warnings.is_empty(), what);
    Some(l1.doc)
}

pub(crate) fn sym_text(doc: &Document, v: Value) -> String {
    match v {
        Value::Text(t) => String::from(doc.str(t)),
        Value::Num(x) => format!("{x}"),
        Value::Absent => String::new(),
    }
}

pub(crate) fn master_shape(doc: &Document, m: MasterRef) -> Option<SheetRef> {
    let ms = doc.master(m)?.master_shape()?;
    Some(SheetRef::Shape(diagram_core::ShapeRef { owner: Owner::Master(m), slot: ms }))
}

pub(crate) fn cell(doc: &Document, sheet: SheetRef, sec: Option<(&str, u32)>, row: Option<RowKey>, name: &str) -> Option<diagram_core::Cell> {
    let c = doc.sym(name)?;
    let addr = match (sec, row) {
        (None, _) => CellAddr::single(c),
        (Some((s, ix)), None) => CellAddr::section(SecKey { name: doc.sym(s)?, index: ix }, c),
        (Some((s, ix)), Some(r)) => CellAddr::row(SecKey { name: doc.sym(s)?, index: ix }, r, c),
    };
    {
        let r = doc.resolve(sheet, &addr);
        r.cached.or(r.cell).copied()
    }
}

pub(crate) fn named(doc: &Document, n: &str) -> Option<RowKey> {
    doc.sym(n).map(RowKey::Named)
}

/// the group "stencils"
pub fn run(h: &mut H) {
    // ---- build: both measures, the round trip, deterministic
    let mut docs: Vec<Option<(Built, Document)>> = Vec::new();
    for m in [Measure::Metric, Measure::Us] {
        let tag = if m == Measure::Us { "us" } else { "metric" };
        match build_set(STANDARD, m) {
            Err(e) => {
                h.check(&format!("stencils/build/{tag}"), false, format!("{e}"));
                docs.push(None);
            }
            Ok((built, bytes)) => {
                h.check(&format!("stencils/build/{tag}"), built.doc.masters().count() == WANT.len() && built.vssx_name() == format!("jfd_standard_{}.vssx", &tag[..1]), format!("{} masters, {} bytes, {}", built.doc.masters().count(), bytes.len(), built.vssx_name()));
                let again = build_set(STANDARD, m).map(|(_, b)| b).unwrap_or_default();
                h.check(&format!("stencils/build/{tag}/deterministic"), again == bytes, format!("second build {} bytes, same: {}", again.len(), again == bytes));
                let back = read_back(h, &format!("stencils/build/{tag}/roundtrip"), &bytes);
                docs.push(back.map(|d| (built, d)));
            }
        }
    }
    let (Some(Some((metric_built, metric))), Some(Some((_, us)))) = (docs.first(), docs.get(1)) else { return };
    masters(h, metric, us);
    engine(h, metric);
    place(h, metric);
    let _ = metric_built;
    svg(h);
    errors(h);
}

/// per master of the standard set, in its order: NameU, French and German name, Geometry
/// sections, connection points, control points
type WantMaster = (&'static str, &'static str, &'static str, usize, usize, usize);
const WANT: [WantMaster; 50] = [
    ("Rectangle", "Rectangle", "Rechteck", 1, 4, 0),
    ("Rounded rectangle", "Rectangle arrondi", "Abgerundetes Rechteck", 1, 4, 1),
    ("Ellipse", "Ellipse", "Ellipse", 1, 4, 0),
    ("Triangle", "Triangle", "Dreieck", 1, 4, 1),
    ("Diamond", "Losange", "Raute", 1, 4, 0),
    ("Arrow", "Flèche", "Pfeil", 1, 2, 2),
    ("Text box", "Zone de texte", "Textfeld", 1, 0, 0),
    // (JD13) the rest of SH-006
    ("Square", "Carré", "Quadrat", 1, 4, 0),
    ("Circle", "Cercle", "Kreis", 1, 4, 0),
    ("Right triangle", "Triangle rectangle", "Rechtwinkliges Dreieck", 1, 3, 0),
    ("Pentagon", "Pentagone", "Fünfeck", 1, 4, 0),
    ("Hexagon", "Hexagone", "Sechseck", 1, 4, 0),
    ("Heptagon", "Heptagone", "Siebeneck", 1, 4, 0),
    ("Octagon", "Octogone", "Achteck", 1, 4, 0),
    ("Decagon", "Décagone", "Zehneck", 1, 4, 0),
    ("Can", "Boîte cylindrique", "Dose", 2, 4, 1),
    ("Parallelogram", "Parallélogramme", "Parallelogramm", 1, 4, 1),
    ("Trapezoid", "Trapèze", "Trapez", 1, 4, 1),
    ("Cross", "Croix", "Kreuz", 1, 4, 1),
    ("Chevron", "Chevron", "Chevron", 1, 4, 1),
    ("Cube", "Cube", "Würfel", 3, 4, 1),
    ("4-point star", "Étoile à 4 branches", "Stern mit 4 Zacken", 1, 4, 1),
    ("5-point star", "Étoile à 5 branches", "Stern mit 5 Zacken", 1, 4, 1),
    ("6-point star", "Étoile à 6 branches", "Stern mit 6 Zacken", 1, 4, 1),
    ("7-point star", "Étoile à 7 branches", "Stern mit 7 Zacken", 1, 4, 1),
    ("16-point star", "Étoile à 16 branches", "Stern mit 16 Zacken", 1, 4, 1),
    ("24-point star", "Étoile à 24 branches", "Stern mit 24 Zacken", 1, 4, 1),
    ("32-point star", "Étoile à 32 branches", "Stern mit 32 Zacken", 1, 4, 1),
    ("Snip single corner rectangle", "Rectangle, 1 coin coupé", "Rechteck, 1 Ecke abgeschnitten", 1, 4, 1),
    ("Snip same side corner rectangle", "Rectangle, coins du même côté coupés", "Rechteck, gleiche Seite abgeschnitten", 1, 4, 1),
    ("Snip diagonal corner rectangle", "Rectangle, coins diagonaux coupés", "Rechteck, diagonal abgeschnitten", 1, 4, 1),
    ("Round single corner rectangle", "Rectangle, 1 coin arrondi", "Rechteck, 1 Ecke abgerundet", 1, 4, 1),
    ("Round same side corner rectangle", "Rectangle, coins du même côté arrondis", "Rechteck, gleiche Seite abgerundet", 1, 4, 1),
    ("Round diagonal corner rectangle", "Rectangle, coins diagonaux arrondis", "Rechteck, diagonal abgerundet", 1, 4, 1),
    ("Snip and round single corner rectangle", "Rectangle, coin arrondi et coin coupé", "Rechteck, Ecke abgerundet und abgeschnitten", 1, 4, 2),
    ("Snip corners rectangle", "Rectangle, coins coupés", "Rechteck, abgeschnittene Ecken", 1, 4, 1),
    ("Snip three corners rectangle", "Rectangle, 3 coins coupés", "Rechteck, 3 Ecken abgeschnitten", 1, 4, 1),
    ("Round three corners rectangle", "Rectangle, 3 coins arrondis", "Rechteck, 3 Ecken abgerundet", 1, 4, 1),
    ("Frame", "Cadre", "Rahmen", 2, 4, 1),
    ("Frame corner", "Coin de cadre", "Rahmenecke", 2, 4, 2),
    ("L-shape", "Forme en L", "L-Form", 1, 4, 1),
    ("Diagonal stripe", "Bande diagonale", "Diagonaler Streifen", 1, 4, 1),
    ("Plaque", "Plaque", "Plakette", 1, 4, 1),
    ("Donut", "Anneau", "Donut", 2, 4, 1),
    ("No symbol", "Panneau d'interdiction", "Verbotszeichen", 3, 4, 1),
    // a 1-D shape from the centre to the rim: no connection points
    ("Circle from center", "Cercle à partir du centre", "Kreis vom Mittelpunkt", 1, 0, 0),
    ("Left parenthesis", "Parenthèse gauche", "Runde Klammer links", 1, 3, 1),
    ("Right parenthesis", "Parenthèse droite", "Runde Klammer rechts", 1, 3, 1),
    ("Left brace", "Accolade gauche", "Geschweifte Klammer links", 1, 3, 1),
    ("Right brace", "Accolade droite", "Geschweifte Klammer rechts", 1, 3, 1),
];

fn masters(h: &mut H, doc: &Document, us: &Document) {
    let ms: Vec<MasterRef> = doc.masters().collect();
    let us_ms: Vec<MasterRef> = us.masters().collect();
    let st = instance::StencilTexts::of(doc);
    h.check("stencils/masters/stencil", st.names == ["Standard-Shapes", "Basic Shapes", "Formas básicas", "Forme di base", "Formes de base"] && st.category == "general", format!("{:?} {}", st.names, st.category));
    let mut bases: Vec<String> = Vec::new();
    for (i, &m) in ms.iter().enumerate() {
        let Some(master) = doc.master(m) else { continue };
        let hd = &master.header;
        let t = MasterTexts::of(doc, m);
        let Some(&(nu, fr, de, want_geo, want_conn, want_ctl)) = WANT.get(i) else {
            h.check(&format!("stencils/masters/{}", t.name_u), false, format!("master {i} not in the expected list"));
            continue;
        };
        let tag = format!("stencils/masters/{nu}");
        let a = |n: &str| doc.sym(n).and_then(|s| attr(&hd.attrs, s)).map(|v| doc.str(v)).unwrap_or("-");
        h.check(&format!("{tag}/names"), t.name_u == nu && t.names[4] == fr && t.names[0] == de && t.prompts.iter().all(|p| !p.is_empty()) && t.keywords.iter().all(|k| !k.is_empty()), format!("{} {:?} prompts ok {}", t.name_u, t.names, t.prompts.iter().all(|p| !p.is_empty())));
        let name = hd.name.map(|s| doc.str(s)).unwrap_or("");
        h.check(&format!("{tag}/attributes"),
            name == de && a("Prompt") == t.prompts[0] && hd.is_custom_name == Some(true) && hd.is_custom_name_u == Some(true)
                && [a("IconSize"), a("AlignName"), a("MatchByName"), a("IconUpdate"), a("PatternFlags"), a("Hidden"), a("MasterType")] == ["1", "2", "0", "1", "0", "0", "2"],
            format!("Name {name}, Prompt {}, IconSize {} AlignName {} MatchByName {} IconUpdate {} PatternFlags {} Hidden {} MasterType {}", a("Prompt"), a("IconSize"), a("AlignName"), a("MatchByName"), a("IconUpdate"), a("PatternFlags"), a("Hidden"), a("MasterType")));
        let kw = doc.sym("ShapeKeywords").and_then(|k| master.sheet.cell(k)).map(|c| sym_text(doc, c.value)).unwrap_or_default();
        h.check(&format!("{tag}/keywords"), kw == t.keywords[0] && !kw.is_empty(), format!("ShapeKeywords {kw:?}"));
        h.check(&format!("{tag}/quick"), t.quick == (i < 4), format!("quick {}", t.quick));
        let (base, uid) = (hd.base_id.map(|s| doc.str(s)).unwrap_or(""), hd.unique_id.map(|s| doc.str(s)).unwrap_or(""));
        let us_base = us_ms.get(i).and_then(|&u| us.master(u)).and_then(|x| x.header.base_id).map(|s| us.str(s)).unwrap_or("");
        h.check(&format!("{tag}/guids"), guid_ok(base) && guid_ok(uid) && base != uid && base != us_base && guid_ok(us_base) && !bases.iter().any(|b| b == base), format!("BaseID {base} UniqueID {uid} US {us_base}"));
        bases.push(String::from(base));
        // the icon
        let icon = master.icon.map(|s| doc.str(s)).and_then(base64_decode).unwrap_or_default();
        match icon_check(&icon) {
            Ok((opaque, corners)) => h.check(&format!("{tag}/icon"), opaque > 40 && opaque < 1024, format!("766 bytes, {opaque} opaque pixels, corners transparent {corners}")),
            Err(e) => h.check(&format!("{tag}/icon"), false, e),
        }
        // geometry: formulas with their values
        let Some(sheet) = master_shape(doc, m) else {
            h.check(&format!("{tag}/geometry"), false, String::from("no master shape"));
            continue;
        };
        let geo = doc.geometry(sheet);
        let rows = geo.first().map_or(0, |&g| doc.rows(sheet, g).len());
        let (mut exprs, mut fresh) = (0, true);
        if let Some(&g) = geo.first() {
            for r in doc.rows(sheet, g) {
                for n in ["X", "Y"] {
                    if let Some(c) = cell(doc, sheet, Some(("Geometry", g.index)), Some(r.key), n) {
                        if let Formula::Expr(_) = c.formula {
                            exprs += 1;
                        }
                        fresh &= !c.stale && matches!(c.value, Value::Num(_));
                    }
                }
            }
        }
        h.check(&format!("{tag}/geometry"), geo.len() == want_geo && rows >= 1 && exprs >= 1 && fresh, format!("{} sections, {rows} rows, {exprs} formulas, values cached {fresh}", geo.len()));
        // connection points
        let conn = doc.sym("Connection").map_or(0, |c| doc.rows(sheet, SecKey::one(c)).len());
        h.check(&format!("{tag}/connections"), conn == want_conn, format!("{conn} connection points"));
        // controls with translated tips
        let ctl = doc.sym("Control").map_or(0, |c| doc.rows(sheet, SecKey::one(c)).len());
        let tips_ok = t.translated.iter().filter(|(r, _)| r.starts_with("Control.") && r.ends_with(".Tip")).count() == want_ctl
            && t.translated.iter().all(|(r, v)| {
                let row = r.split('.').nth(1).unwrap_or("");
                let here = cell(doc, sheet, Some(("Control", 0)), named(doc, row), "Tip").map(|c| sym_text(doc, c.value)).unwrap_or_default();
                v.iter().all(|x| !x.is_empty()) && here == v[0]
            });
        h.check(&format!("{tag}/controls"), ctl == want_ctl && tips_ok, format!("{ctl} controls, translated {:?}", t.translated.iter().map(|(r, v)| format!("{r}={}", v[4])).collect::<Vec<_>>()));
    }
    // metric 40 mm against US 1.5 in (the rectangle)
    let width = |d: &Document| d.masters().next().and_then(|m| master_shape(d, m)).and_then(|s| cell(d, s, None, None, "Width"));
    match (width(doc), width(us)) {
        (Some(a), Some(b)) => {
            let ok = matches!(a.value, Value::Num(x) if (x - 40.0 / 25.4).abs() < 1e-12) && a.unit == Some(Unit::Millimeters)
                && matches!(b.value, Value::Num(x) if (x - 1.5).abs() < 1e-12) && b.unit == Some(Unit::Inches);
            h.check("stencils/masters/measure", ok, format!("metric {:?} {:?}, US {:?} {:?}", a.value, a.unit, b.value, b.unit));
        }
        _ => h.check("stencils/masters/measure", false, String::from("no Width")),
    }
    // the text box: style Text Only, its text
    if let Some(&tb) = ms.get(6) {
        let t = MasterTexts::of(doc, tb);
        let text = master_shape(doc, tb).and_then(|s| match s {
            SheetRef::Shape(r) => doc.shape(r).and_then(|x| x.text()).map(|x| x.chars()),
            _ => None,
        });
        h.check("stencils/masters/textbox", t.text.as_ref().is_some_and(|x| x[4] == "Texte" && x[0] == "Text") && text.as_deref() == Some("Text"), format!("{:?} {text:?}", t.text));
    }
}

fn engine(h: &mut H, doc: &Document) {
    let mut d = doc.clone();
    let res = Engine::new().calculate(&d);
    let kept: Vec<String> = res
        .cells
        .iter()
        .filter(|(_, _, o)| matches!(o, Outcome::Kept(_)) || matches!(o, Outcome::Value(v) if v.is_err()))
        .map(|(_, a, o)| format!("{} {o:?}", d.str(a.cell)))
        .collect();
    let mut tx = d.transaction("check");
    let written = res.write(&mut tx).unwrap_or(usize::MAX);
    drop(tx);
    h.check("stencils/engine", kept.is_empty() && written == 0 && res.cycles.is_empty(), format!("{} formula cells computed, {written} changed, kept {kept:?}", res.cells.len()));
}

fn place(h: &mut H, src: &Document) {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let Some(page) = doc.pages().next() else { return };
    let fr = instance::lang_index("fr").unwrap_or(4);
    let mut tx = doc.transaction("drop");
    let mut placed: Vec<(usize, diagram_core::ShapeRef)> = Vec::new();
    let mut fail = String::new();
    for (i, m) in src.masters().enumerate() {
        let r = instance::import_master(&mut tx, src, m).and_then(|local| {
            let a = instance::place(&mut tx, page, local, 1.0 + i as f64, 2.0, fr)?;
            let b = instance::place(&mut tx, page, local, 1.0 + i as f64, 4.0, fr)?;
            Ok((a, b))
        });
        match r {
            Ok((a, b)) => {
                placed.push((i, a));
                placed.push((i + 100, b));
            }
            Err(e) => fail = format!("{fail} {i}: {e:?}"),
        }
    }
    let sheets: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
    let rc = Engine::new().recalc(&mut tx, &sheets);
    let kept = rc.as_ref().map_or(usize::MAX, |r| r.kept.len());
    let _ = tx.commit();
    h.check("stencils/place/drop", fail.is_empty() && placed.len() == 2 * WANT.len() && kept == 0, format!("{} instances, recalc kept {kept}{fail}", placed.len()));
    for &(i, s) in &placed {
        let second = i >= 100;
        let i = i % 100;
        let Some(sh) = doc.shape(s) else { continue };
        let (name, name_u) = (sh.name(), sh.name_u());
        let id = sh.id();
        let (w_u, w_fr) = WANT.get(i).map_or(("", ""), |w| (w.0, w.1));
        let (want, want_u) = if second { (format!("{w_fr}.{id}"), format!("{w_u}.{id}")) } else { (String::from(w_fr), String::from(w_u)) };
        h.check(&format!("stencils/place/{w_u}{}", if second { "/second" } else { "" }), name == want && name_u == want_u, format!("Name {name:?} NameU {name_u:?}"));
    }
    // the arrow's tip in French, the text box's text
    if let Some(&(_, arrow)) = placed.iter().find(|(i, _)| *i == 5) {
        let tip = cell(&doc, SheetRef::Shape(arrow), Some(("Control", 0)), named(&doc, "Row_1"), "Tip").map(|c| sym_text(&doc, c.value)).unwrap_or_default();
        h.check("stencils/place/arrow-tip", tip == "Modifier la largeur du corps", format!("Tip {tip:?}"));
    }
    if let Some(&(_, tb)) = placed.iter().find(|(i, _)| *i == 6) {
        let text = doc.shape(tb).and_then(|s| s.text()).map(|t| t.chars()).unwrap_or_default();
        h.check("stencils/place/textbox-text", text == "Texte", format!("text {text:?}"));
    }
    match vsdx::write(&doc, &WriteOptions::DEFAULT).map(|b| vsdx::read(&b, &Limits::VSDX)) {
        Ok(Ok(l)) => h.check("stencils/place/save", l.warnings.is_empty() && l.doc.masters().count() == WANT.len(), format!("{} masters, {} warnings", l.doc.masters().count(), l.warnings.len())),
        Ok(Err(e)) => h.check("stencils/place/save", false, format!("read: {e}")),
        Err(e) => h.check("stencils/place/save", false, format!("write: {e}")),
    }
}

fn svg(h: &mut H) {
    let (built, bytes) = match build_set(SVG_FILES, Measure::Metric) {
        Ok(x) => x,
        Err(e) => {
            h.check("stencils/svg/build", false, format!("{e}"));
            return;
        }
    };
    h.check("stencils/svg/build", built.doc.masters().count() == 2, format!("{} masters, {} bytes", built.doc.masters().count(), bytes.len()));
    let Some(doc) = read_back(h, "stencils/svg/roundtrip", &bytes) else { return };
    let ms: Vec<MasterRef> = doc.masters().collect();
    // the build language is English
    let en = ms.first().and_then(|&m| doc.master(m)).and_then(|m| m.header.name).map(|s| doc.str(s)).unwrap_or("");
    h.check("stencils/svg/language", en == "Server", format!("Name {en:?}"));
    // a NURBSTo row's cells: X/Y formulas, E = NURBS(...) with U=NURBS
    let nurbs_of = |sheet: SheetRef| -> (usize, usize, bool) {
        let mut sections = 0;
        let mut nurbs = 0;
        let mut ok = true;
        for g in doc.geometry(sheet) {
            sections += 1;
            for r in doc.rows(sheet, g) {
                if r.kind.map(|k| doc.str(k)) == Some("NURBSTo") {
                    nurbs += 1;
                    let e = cell(&doc, sheet, Some(("Geometry", g.index)), Some(r.key), "E");
                    ok &= e.is_some_and(|c| c.unit == Some(Unit::Nurbs) && matches!(c.formula, Formula::Expr(_)) && sym_text(&doc, c.value).starts_with("NURBS(1, 3, 0, 0, "));
                    for (n, v) in [("A", 0.0), ("B", 1.0), ("C", 0.0), ("D", 1.0)] {
                        ok &= cell(&doc, sheet, Some(("Geometry", g.index)), Some(r.key), n).is_some_and(|c| c.value == Value::Num(v));
                    }
                }
            }
        }
        (sections, nurbs, ok)
    };
    // server: a group of two subshapes as large as the group
    if let Some(&server) = ms.first() {
        let group = doc.master(server).and_then(|m| m.master_shape()).map(|slot| diagram_core::ShapeRef { owner: Owner::Master(server), slot });
        let kind = group.and_then(|g| doc.shape(g)).map(|s| s.header().kind);
        let subs: Vec<diagram_core::ShapeRef> = group.and_then(|g| doc.shape(g)).map(|s| s.children().collect()).unwrap_or_default();
        let gw = group.and_then(|g| cell(&doc, SheetRef::Shape(g), None, None, "Width")).and_then(|c| c.value.num()).unwrap_or(0.0);
        let mut what = Vec::new();
        let mut ok = kind == Some(ShapeKind::Group) && subs.len() == 2;
        for (k, &sub) in subs.iter().enumerate() {
            let (sections, nurbs, nok) = nurbs_of(SheetRef::Shape(sub));
            let w = cell(&doc, SheetRef::Shape(sub), None, None, "Width");
            let wf = w.and_then(|c| c.formula.expr()).map(|f| String::from(doc.str(f))).unwrap_or_default();
            let wv = w.and_then(|c| c.value.num()).unwrap_or(-1.0);
            let fill = cell(&doc, SheetRef::Shape(sub), None, None, "FillForegnd").map(|c| sym_text(&doc, c.value)).unwrap_or_default();
            let want = if k == 0 { (1, 4, "#2E3A46") } else { (7, 4 * 2 + 5, "#6FA3E0") };
            ok &= sections == want.0 && nurbs >= want.1 && nok && wf.starts_with("Sheet.") && wf.ends_with("!Width") && (wv - gw).abs() < 1e-12 && fill == want.2;
            what.push(format!("sub {k}: {sections} sections, {nurbs} NURBSTo, Width {wf} = {wv}, fill {fill}"));
        }
        h.check("stencils/svg/server", ok, format!("{kind:?}, {}", what.join("; ")));
    }
    // cloud: one format, the master shape's own geometry; the open line as fill + line
    if let Some(&cloud) = ms.get(1) {
        let Some(sheet) = master_shape(&doc, cloud) else { return };
        let (sections, nurbs, nok) = nurbs_of(sheet);
        let flags: Vec<(bool, bool)> = doc
            .geometry(sheet)
            .iter()
            .map(|g| {
                let f = |n: &str| cell(&doc, sheet, Some(("Geometry", g.index)), None, n).is_some_and(|c| c.value == Value::Num(1.0));
                (f("NoFill"), f("NoLine"))
            })
            .collect();
        let lw = cell(&doc, sheet, None, None, "LineWeight");
        // stroke-width 2 in viewBox units, the viewBox 64 x 40 on 32 x 20 mm
        let want_lw = 2.0 * diagram_core::formula::math::sqrt((32.0 / 25.4 / 64.0) * (20.0 / 25.4 / 40.0));
        let lw_ok = lw.is_some_and(|c| c.unit == Some(Unit::Points) && matches!(c.value, Value::Num(x) if (x - want_lw).abs() < 1e-9));
        let kind = match sheet {
            SheetRef::Shape(r) => doc.shape(r).map(|s| s.header().kind),
            _ => None,
        };
        h.check("stencils/svg/cloud", kind == Some(ShapeKind::Shape) && sections == 3 && nurbs >= 6 && nok && flags == [(false, false), (false, true), (true, false)] && lw_ok,
            format!("{kind:?}, {sections} sections, {nurbs} NURBSTo, NoFill/NoLine {flags:?}, LineWeight {:?}", lw.map(|c| c.value)));
    }
    // the icons of the SVG masters are drawn
    let icons: Vec<String> = ms
        .iter()
        .map(|&m| {
            let icon = doc.master(m).and_then(|x| x.icon).map(|s| doc.str(s)).and_then(base64_decode).unwrap_or_default();
            match icon_check(&icon) {
                Ok((opaque, _)) => opaque.to_string(),
                Err(e) => e,
            }
        })
        .collect();
    h.check("stencils/svg/icons", icons.iter().all(|s| s.parse::<u32>().is_ok_and(|n| n > 40)), format!("opaque pixels {icons:?}"));
}

// a fault: the source set with one change must fail with this file, line and message part
fn fault(h: &mut H, name: &str, set: &[(&str, &str)], file: &str, line: u32, part: &str) {
    match build_set(set, Measure::Metric) {
        Ok(_) => h.check(&format!("stencils/errors/{name}"), false, String::from("built without error")),
        Err(e) => {
            let shown = format!("{e}");
            let ok = e.file == file && e.line == line && e.message.contains(part) && shown.starts_with(&format!("{file}:{line}: "));
            h.check(&format!("stencils/errors/{name}"), ok, shown);
        }
    }
}

fn errors(h: &mut H) {
    let ok_set: &[(&str, &str)] = &[("stencil.txt", T_STENCIL), ("box.master", T_BOX)];
    match build_set(ok_set, Measure::Metric) {
        Ok((b, _)) => h.check("stencils/errors/base", b.doc.masters().count() == 1, String::from("the base of the faults builds")),
        Err(e) => h.check("stencils/errors/base", false, format!("{e}")),
    }
    let with = |box_text: String| -> Vec<(String, String)> { alloc::vec![(String::from("stencil.txt"), String::from(T_STENCIL)), (String::from("box.master"), box_text)] };
    let refs = |v: &[(String, String)]| -> Vec<(String, String)> { v.to_vec() };
    let run = |h: &mut H, name: &str, set: Vec<(String, String)>, file: &str, line: u32, part: &str| {
        let r: Vec<(&str, &str)> = set.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        fault(h, name, &r, file, line, part);
    };
    run(h, "missing-language", with(T_BOX.replace("name.fr: Boîte\n", "")), "box.master", 2, "name.fr fehlt");
    run(h, "unknown-section", with(T_BOX.replace("[Geometry]", "[Geometrie]")), "box.master", 17, "unbekannter Abschnitt [Geometrie]");
    run(h, "unknown-function", with(T_BOX.replace("LineTo X = Width;", "LineTo X = FOO(Width);")), "box.master", 19, "unbekannte Funktion FOO");
    run(h, "missing-width", with(T_BOX.replace("Width = 20 mm\n", "")), "box.master", 1, "Width fehlt");
    run(h, "reference", with(T_BOX.replace("LineTo X = Width;", "LineTo X = User.Nope;")), "box.master", 19, "Bezug ins Leere");
    run(h, "typo", with(T_BOX.replace("LineTo X = Width;", "LineTo X = Widht*2;")), "box.master", 19, "Bezug ins Leere: Widht");
    run(h, "cell-language", with(format!("{T_BOX}\n[Controls]\nRow_1:  X = 0; Y = 0\n        Tip.de = \"Ziehen\"\n")), "box.master", 23, "Tip: Sprache en fehlt");
    run(h, "unknown-cell", with(T_BOX.replace("Height = 10 mm", "Height = 10 mm\nHieght = 3 mm")), "box.master", 16, "unbekannte Zelle Hieght");
    let missing = alloc::vec![(String::from("stencil.txt"), T_STENCIL.replace("masters: box", "masters: box, nix")), (String::from("box.master"), String::from(T_BOX))];
    run(h, "missing-master", refs(&missing), "stencil.txt", 9, "nix.master fehlt");
    let twice = alloc::vec![
        (String::from("stencil.txt"), T_STENCIL.replace("masters: box", "masters: box, box2")),
        (String::from("box.master"), String::from(T_BOX)),
        (String::from("box2.master"), String::from(T_BOX)),
    ];
    run(h, "nameu-twice", twice, "box2.master", 1, "NameU");
    let variant = with(T_BOX.replace("Width = 20 mm\n", "Width = 20 mm\nWidth.metric = 1 in\nWidth.metric = 2 in\n"));
    run(h, "variant-twice", variant, "box.master", 16, "Width.metric doppelt");
    // lang suffixes need all five, also in stencil.txt; an unknown category
    let st = alloc::vec![(String::from("stencil.txt"), T_STENCIL.replace("category: general", "category: allgemein")), (String::from("box.master"), String::from(T_BOX))];
    run(h, "category", st, "stencil.txt", 8, "unbekannte Kategorie allgemein");
    // no panic on odd sources
    let odd = ["", "\u{feff}", "[", "master:", "master: X\n[Shape]\nWidth = ;", "master: X\n[Geometry]\n  LineTo X = 1", "master: X\nname.xx: y", "master: X\n[User]\nA:\nA:"];
    let mut all_errors = true;
    for o in odd {
        let set = alloc::vec![(String::from("stencil.txt"), String::from(T_STENCIL)), (String::from("box.master"), String::from(o))];
        let r: Vec<(&str, &str)> = set.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        all_errors &= build_set(&r, Measure::Metric).is_err();
    }
    h.check("stencils/errors/odd", all_errors, format!("{} odd sources refused", odd.len()));
}

/// diagramiotest --write-stencils DIR: the start set in both measures and the SVG test stencil
pub fn write(dir: &str) -> i32 {
    let mut status = 0;
    for (set, m) in [(STANDARD, Measure::Metric), (STANDARD, Measure::Us), (SVG_FILES, Measure::Metric)] {
        match build_set(set, m) {
            Ok((built, bytes)) => {
                let path = format!("{dir}/{}", built.vssx_name());
                if plat::write_file(&path, &bytes) {
                    out(&format!("wrote {path} ({} KB)\n", bytes.len() / 1024));
                } else {
                    out(&format!("error: cannot write {path}\n"));
                    status = 1;
                }
            }
            Err(e) => {
                out(&format!("error: {e}\n"));
                status = 1;
            }
        }
    }
    // a drawing with an instance of every master of the start set, dropped as JFDiagrammer does
    // it (instance::import_master + place + Engine::recalc in one transaction)
    match dropped_drawing() {
        Ok(bytes) => {
            let path = format!("{dir}/standard_dropped.vsdx");
            if plat::write_file(&path, &bytes) {
                out(&format!("wrote {path} ({} KB)\n", bytes.len() / 1024));
            } else {
                out(&format!("error: cannot write {path}\n"));
                status = 1;
            }
        }
        Err(e) => {
            out(&format!("error: {e}\n"));
            status = 1;
        }
    }
    status
}

// the metric start set dropped onto a new drawing in a row, names in German (LANGS[0])
fn dropped_drawing() -> Result<Vec<u8>, String> {
    let (built, _) = build_set(STANDARD, Measure::Metric).map_err(|e| e.to_string())?;
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let masters: Vec<MasterRef> = built.doc.masters().collect();
    let mut tx = doc.transaction("drop");
    let mut x = 0.9;
    for m in masters {
        let im = instance::import_master(&mut tx, &built.doc, m).map_err(|e| format!("{e:?}"))?;
        let s = instance::place(&mut tx, page, im, x, 4.0, 0).map_err(|e| format!("{e:?}"))?;
        Engine::new().recalc(&mut tx, &[SheetRef::Shape(s)]).map_err(|e| format!("{e:?}"))?;
        x += 1.6;
    }
    tx.commit();
    vsdx::write(&doc, &WriteOptions::DEFAULT).map_err(|e| format!("{e:?}"))
}
