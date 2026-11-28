// rcheck -- the check groups of lib/pdf's reader (pdf::read): files written here byte by byte
// (classic tables, cross-reference and object streams, updates, damage), the writer's sample
// read back, every filter, the encrypted fixtures of tests/read (made and checked against
// Poppler by tools/pdf_read_check.py --fixtures), text extraction, the document's navigation
// and forms, and hostile files and mutations (with a fixed seed: never a panic or a hang).
//
//   read_sample    the writer's sample: pages, sizes, information, outlines, links, text
//   read_syntax    strings, names, numbers; tables, streams, updates, hybrid and damaged files
//   read_filters   ASCIIHex, ASCII85, LZW, RunLength, Flate (raw, cut short), predictors, limits
//   read_crypt     RC4 40 / 128, AES 128 / 256 (revisions 2 to 6): user, owner, wrong passwords
//   read_text      positions and spacing, fonts (standard, Type 0, Type 3, Differences, built-in
//                  encodings), forms, inline images, invisible text, right-to-left runs
//   read_nav       outlines, named destinations, actions, annotations, page labels, fields,
//                  signatures, attachments, dates
//   read_hostile   loops, depth, bombs, huge numbers; mutated files
//   read_budget    files of a few MB that took minutes to hours or many GB (lib review of
//                  2026-11-27): each within 20 s, with what it should give

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use pdf::read::{parse_date, Action, Document, Error, FieldKind, Object, Options, Ref, View};

use crate::T;

pub const GROUPS: &[(&str, fn(&mut T))] = &[
    ("read_sample", read_sample),
    ("read_syntax", read_syntax),
    ("read_filters", read_filters),
    ("read_crypt", read_crypt),
    ("read_text", read_text),
    ("read_nav", read_nav),
    ("read_hostile", read_hostile),
    ("read_budget", read_budget),
];

// ------------------------------------------------------------------ files made here

/// objects by number, laid out with a table or with cross-reference streams
pub(crate) struct Mk {
    objs: BTreeMap<u32, Vec<u8>>,
}

pub(crate) fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// a stream object's body, uncompressed
pub(crate) fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    cat(&[format!("<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes(), data, b"\nendstream"])
}

/// a stream object's body, with FlateDecode
pub(crate) fn flate(dict: &str, data: &[u8]) -> Vec<u8> {
    stream(&format!("/Filter /FlateDecode {dict}"), &compress::zlib::compress(data, 6))
}

impl Mk {
    pub(crate) fn new() -> Mk {
        Mk { objs: BTreeMap::new() }
    }

    pub(crate) fn add(&mut self, n: u32, body: impl AsRef<[u8]>) -> &mut Mk {
        self.objs.insert(n, body.as_ref().to_vec());
        self
    }

    fn body(&self) -> (Vec<u8>, BTreeMap<u32, usize>) {
        let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offs = BTreeMap::new();
        for (n, b) in &self.objs {
            offs.insert(*n, out.len());
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(b);
            out.extend_from_slice(b"\nendobj\n");
        }
        (out, offs)
    }

    /// a classic cross-reference table; `trailer` without << >>
    pub(crate) fn table(&self, trailer: &str) -> Vec<u8> {
        let (mut out, offs) = self.body();
        let max = self.objs.keys().max().copied().unwrap_or(0);
        let x = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", max + 1).as_bytes());
        for n in 1..=max {
            out.extend_from_slice(match offs.get(&n) {
                Some(o) => format!("{o:010} 00000 n \n"),
                None => String::from("0000000000 65535 f \n"),
            }.as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} {trailer} >>\nstartxref\n{x}\n%%EOF\n", max + 1).as_bytes());
        out
    }

    /// a cross-reference stream (PDF 1.5); with `objstm` every object that is no stream goes
    /// into one object stream
    pub(crate) fn xref_stream(&self, trailer: &str, objstm: bool) -> Vec<u8> {
        let mut m = Mk { objs: BTreeMap::new() };
        let mut inside: BTreeMap<u32, usize> = BTreeMap::new();
        let max = self.objs.keys().max().copied().unwrap_or(0);
        let stm = max + 1;
        if objstm {
            let (mut head, mut data) = (String::new(), Vec::new());
            for (n, b) in &self.objs {
                if b.windows(7).any(|w| w == b"stream\n") {
                    m.objs.insert(*n, b.clone());
                    continue;
                }
                head.push_str(&format!("{n} {} ", data.len()));
                inside.insert(*n, inside.len());
                data.extend_from_slice(b);
                data.push(b'\n');
            }
            let all = cat(&[head.as_bytes(), &data]);
            m.objs.insert(stm, flate(&format!("/Type /ObjStm /N {} /First {}", inside.len(), head.len()), &all));
        } else {
            m.objs = self.objs.clone();
        }
        let (mut out, offs) = m.body();
        let xnum = stm + 1;
        let mut rows = vec![0u8, 0, 0, 0, 0, 0xFF, 0xFF];
        for n in 1..xnum {
            let mut row = [0u8; 7];
            if let Some(&i) = inside.get(&n) {
                row[0] = 2;
                row[1..5].copy_from_slice(&stm.to_be_bytes());
                row[5..7].copy_from_slice(&(i as u16).to_be_bytes());
            } else if let Some(&o) = offs.get(&n) {
                row[0] = 1;
                row[1..5].copy_from_slice(&(o as u32).to_be_bytes());
            }
            rows.extend_from_slice(&row);
        }
        let x = out.len();
        let mut row = [1u8, 0, 0, 0, 0, 0, 0];
        row[1..5].copy_from_slice(&(x as u32).to_be_bytes());
        rows.extend_from_slice(&row);
        let body = flate(&format!("/Type /XRef /W [1 4 2] /Size {} {trailer}", xnum + 1), &rows);
        out.extend_from_slice(format!("{xnum} 0 obj\n").as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(format!("\nendobj\nstartxref\n{x}\n%%EOF\n").as_bytes());
        out
    }
}

/// one page: catalog 1, pages 2, page 3, content 4, font 5 (Helvetica, WinAnsiEncoding) unless
/// `font` gives another; `res` adds to the resources
fn one_page(content: &[u8], font: Option<&[u8]>, res: &str) -> Mk {
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .add(3, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> {res} >> >>"))
        .add(4, flate("", content))
        .add(5, font.unwrap_or(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"));
    m
}

fn open(t: &mut T, name: &str, data: Vec<u8>) -> Option<Document> {
    t.ok(name, Document::open(data).map_err(|e| format!("{e}")))
}

fn text0(t: &mut T, name: &str, data: Vec<u8>) -> String {
    match open(t, name, data) {
        Some(d) => d.text(0).unwrap_or_default(),
        None => String::new(),
    }
}

/// the offset after the last startxref
fn startxref(d: &[u8]) -> usize {
    let at = d.windows(9).rposition(|w| w == b"startxref").unwrap_or(0);
    core::str::from_utf8(&d[at + 9..]).ok().and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// rounded to an integer (no std here)
fn rnd(v: f64) -> i64 {
    if v >= 0.0 { (v + 0.5) as i64 } else { (v - 0.5) as i64 }
}

fn rf(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

// ------------------------------------------------------------------ the sample

fn read_sample(t: &mut T) {
    let Some(bytes) = t.ok("sample", crate::sample::build().map_err(|e| format!("{e:?}"))) else { return };
    let Some(doc) = open(t, "open", bytes) else { return };
    t.eq("pages", doc.page_count(), 5);
    t.eq("version", doc.version(), (1, 7));
    t.check("not encrypted", !doc.is_encrypted() && doc.permissions().is_none());
    t.check("not repaired", !doc.was_repaired());
    let sizes: Vec<(i64, i64)> = doc.pages().iter().map(|p| p.size()).map(|(w, h)| (rnd(w * 100.0), rnd(h * 100.0))).collect();
    t.eq("sizes", sizes, vec![(59528, 84189), (59528, 84189), (79200, 61200), (28346, 28346), (59528, 84189)]);
    let info = doc.info();
    t.eq("title", info.title.as_deref(), Some("JFLib PDF – Beispiel"));
    t.eq("author", info.author.as_deref(), Some("JFLib"));
    t.eq("producer", info.producer.as_deref(), Some("JFLib"));
    t.eq("created", info.created.map(|d| (d.year, d.month, d.day, d.hour, d.offset_minutes)), Some((2026, 11, 16, 12, 60)));
    t.eq("language", doc.language().as_deref(), Some("de-DE"));
    t.check("XMP metadata", doc.metadata().is_some_and(|m| m.windows(9).any(|w| w == b"x:xmpmeta")));
    let o = doc.outlines();
    t.eq("outline: top items", o.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["1 Vektorgrafik", "2 Bilder, Text und Links", "3 Querformat (Letter, y nach oben)", "4 Kleine Seite (100 × 100 mm)", "5 Verläufe und Schraffuren"]);
    t.eq("outline: 1 has four children", o.first().map(|x| x.children.len()), Some(4));
    t.eq("outline: 2.2 Text > Arabisch", o.get(1).and_then(|x| x.children.get(1)).and_then(|x| x.children.first()).map(|x| x.title.as_str()), Some("Arabisch und Ersatzschrift"));
    t.check("outline: XYZ with zoom 2", matches!(o.get(1).and_then(|x| x.children.get(1)).and_then(|x| x.children.first()).and_then(|x| x.action.clone()), Some(Action::GoTo(d)) if d.page == 1 && matches!(d.view, View::Xyz { zoom: Some(z), .. } if z == 2.0)));
    t.check("outline: 1 is open", o.first().is_some_and(|x| x.open));
    let links = doc.links(1).unwrap_or_default();
    t.check("link: URI", links.iter().any(|(_, a)| *a == Action::Uri("https://github.com/JFLibrary/JFLib".into())));
    t.check("link: to page 3", links.iter().any(|(_, a)| matches!(a, Action::GoTo(d) if d.page == 2)));
    let all: String = (0..doc.page_count()).map(|i| doc.text(i).unwrap_or_default()).collect();
    for s in [
        "JFLib PDF – Vektor-PDF-Schreiber",
        "Größe, Übermaß, Ärger",
        "„Anführungen“",
        "3 × 4 ≥ 10 €",
        "Καλημέρα κόσμε",
        "Здравствуй, мир",
        "Pfiff",
        "office",
        "AVATAR",
        "Unsichtbarer Text",
        "AVOBFGJQ",
        "ABAB",
        "gedrehter Text",
        "100 × 100 mm",
        "https://github.com/JFLibrary/JFLib",
        "Verläufe und Schraffuren",
        "السلام عليكم",
        "fn main() { println!(\"Hallo\"); }",
    ] {
        t.check(&format!("text has {s:?}"), all.contains(s));
    }
    // the invisible text is there, and marked
    let p2 = doc.page_text(1).unwrap_or_default();
    t.check("invisible glyphs marked", p2.items.iter().any(|i| i.invisible) && p2.items.iter().any(|i| !i.invisible));
    t.check("page 2 fonts named", p2.fonts.iter().any(|f| f.contains("Inter")));
    // where the title stands: 22 pt Inter at x 50, y 70 from the top of an A4 page
    let p1 = doc.page_text(0).unwrap_or_default();
    if let Some(g) = p1.items.first() {
        t.check("first glyph at 50, 841.89 - 70", (g.x - 50.0).abs() < 0.01 && (g.y - (841.889_763_779_527_6 - 70.0)).abs() < 0.01);
        t.check("first glyph 22 pt", (g.size - 22.0).abs() < 0.01);
    }
    t.check("no page 6", doc.page(5).is_err() && doc.text(5) == Err(Error::NoPage));
}

// ------------------------------------------------------------------ syntax and structure

fn read_syntax(t: &mut T) {
    // strings, names, numbers
    let mut m = one_page(b"BT /F1 12 Tf 50 700 Td (Hallo) Tj ET", None, "");
    m.add(
        9,
        b"<< /S (a\\(b\\)c\\\\d\\n\\101\\0618 \\\nx) /P (a(b)c) /H <48 656C 6c6F 7> /N /A#20B#2fC /R 1.5 /M -.25 /I -17 /E [] /D << >> /B true /Z null /F false /X 4 0 R >>".as_slice(),
    );
    let Some(doc) = open(t, "objects", m.table("/Root 1 0 R")) else { return };
    let o = doc.get(rf(9));
    let d = o.as_dict().cloned().unwrap_or_default();
    t.eq("literal string escapes", d.get("S").and_then(Object::as_bytes), Some(b"a(b)c\\d\nA18 x".as_slice()));
    t.eq("balanced parentheses", d.get("P").and_then(Object::as_bytes), Some(b"a(b)c".as_slice()));
    t.eq("hex string, odd digit", d.get("H").and_then(Object::as_bytes), Some(b"Hellop".as_slice()));
    t.eq("name escapes", d.name("N"), Some(b"A B/C".as_slice()));
    t.eq("reals", (d.num("R"), d.num("M")), (Some(1.5), Some(-0.25)));
    t.eq("integer", d.int("I"), Some(-17));
    t.check("empty array and dict", matches!(d.get("E"), Some(Object::Array(a)) if a.is_empty()) && matches!(d.get("D"), Some(Object::Dict(x)) if x.is_empty()));
    t.check("booleans and null", d.get("B") == Some(&Object::Bool(true)) && d.get("F") == Some(&Object::Bool(false)) && d.get("Z") == Some(&Object::Null));
    t.eq("a reference", d.get("X").and_then(Object::as_ref), Some(rf(4)));
    t.check("a missing object is null", doc.get(rf(77)).is_null());
    t.eq("the text", doc.text(0).ok().as_deref(), Some("Hallo\n"));

    // the same document in every layout
    let pages = |n: usize| -> Mk {
        let mut m = Mk::new();
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 10 + 2 * i)).collect();
        m.add(1, "<< /Type /Catalog /Pages 2 0 R >>")
            .add(2, format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 500 700] /Resources << /Font << /F1 3 0 R >> >> >>", kids.join(" ")))
            .add(3, "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>")
            .add(4, "<< /Title (Layouts) >>");
        for i in 0..n {
            let p = 10 + 2 * i as u32;
            m.add(p, format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R >>", p + 1));
            m.add(p + 1, flate("", format!("BT /F1 10 Tf 40 600 Td (Seite {}) Tj ET", i + 1).as_bytes()));
        }
        m
    };
    let base = pages(3);
    for (name, data) in [
        ("table", base.table("/Root 1 0 R /Info 4 0 R")),
        ("xref stream", base.xref_stream("/Root 1 0 R /Info 4 0 R", false)),
        ("object stream", base.xref_stream("/Root 1 0 R /Info 4 0 R", true)),
    ] {
        let Some(doc) = open(t, name, data) else { continue };
        t.eq(&format!("{name}: pages"), doc.page_count(), 3);
        t.eq(&format!("{name}: inherited media box"), doc.page(2).map(|p| p.media_box).ok(), Some([0.0, 0.0, 500.0, 700.0]));
        t.eq(&format!("{name}: text of page 3"), doc.text(2).ok().as_deref(), Some("Seite 3\n"));
        t.eq(&format!("{name}: title"), doc.info().title.as_deref(), Some("Layouts"));
        t.check(&format!("{name}: not repaired"), !doc.was_repaired());
    }

    // an incremental update: a new title and a fourth page
    let old = base.table("/Root 1 0 R /Info 4 0 R");
    let prev_off = startxref(&old);
    let mut upd = old.clone();
    let mut offs = Vec::new();
    for (n, body) in [
        (4u32, b"<< /Title (Neu) >>".to_vec()),
        (20, b"<< /Type /Page /Parent 2 0 R /Contents 21 0 R >>".to_vec()),
        (21, flate("", b"BT /F1 10 Tf 40 600 Td (Seite 4) Tj ET")),
        (2, b"<< /Type /Pages /Kids [10 0 R 12 0 R 14 0 R 20 0 R] /Count 4 /MediaBox [0 0 500 700] /Resources << /Font << /F1 3 0 R >> >> >>".to_vec()),
    ] {
        offs.push((n, upd.len()));
        upd.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        upd.extend_from_slice(&body);
        upd.extend_from_slice(b"\nendobj\n");
    }
    let x = upd.len();
    upd.extend_from_slice(b"xref\n");
    for (n, o) in &offs {
        upd.extend_from_slice(format!("{n} 1\n{o:010} 00000 n \n").as_bytes());
    }
    upd.extend_from_slice(format!("trailer\n<< /Size 22 /Root 1 0 R /Info 4 0 R /Prev {prev_off} >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
    if let Some(doc) = open(t, "update", upd.clone()) {
        t.eq("update: pages", doc.page_count(), 4);
        t.eq("update: title", doc.info().title.as_deref(), Some("Neu"));
        t.eq("update: page 4", doc.text(3).ok().as_deref(), Some("Seite 4\n"));
        t.eq("update: page 1 from the first revision", doc.text(0).ok().as_deref(), Some("Seite 1\n"));
    }
    // /Prev pointing at itself
    let looped = {
        let mut u = old.clone();
        let x = u.len();
        u.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 22 /Root 1 0 R /Prev {x} >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
        u
    };
    if let Some(doc) = open(t, "prev loop", looped) {
        t.eq("prev loop: pages", doc.page_count(), 3);
    }

    // a hybrid file: the table lists the object stream, a cross-reference stream its objects
    let hybrid = {
        let xs = base.xref_stream("/Root 1 0 R /Info 4 0 R", true);
        // the cross-reference stream's offset, then a table after it that says XRefStm
        let at = xs.windows(9).rposition(|w| w == b"startxref").unwrap_or(0);
        let xoff = startxref(&xs);
        let mut h = xs[..at].to_vec();
        let x = h.len();
        h.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 30 /Root 1 0 R /Info 4 0 R /XRefStm {xoff} >>\nstartxref\n{x}\n%%EOF\n").as_bytes());
        h
    };
    if let Some(doc) = open(t, "hybrid", hybrid) {
        t.eq("hybrid: pages", doc.page_count(), 3);
        t.eq("hybrid: text", doc.text(1).ok().as_deref(), Some("Seite 2\n"));
    }

    // damage: shifted offsets (found again object by object), no table, a cut file, no trailer
    let good = base.table("/Root 1 0 R /Info 4 0 R");
    let shifted = cat(&[b"%PDF-1.7\n%junk junk\n", &good[9..]]);
    if let Some(doc) = open(t, "shifted offsets", shifted) {
        t.eq("shifted: pages", doc.page_count(), 3);
        t.eq("shifted: text", doc.text(2).ok().as_deref(), Some("Seite 3\n"));
    }
    let xat = good.windows(5).position(|w| w == b"xref\n").unwrap_or(good.len());
    for (name, data) in [("no table", cat(&[&good[..xat], b"%%EOF\n"])), ("cut", good[..xat - 30].to_vec()), ("no startxref", good[..good.len() - 20].to_vec())] {
        let Some(doc) = open(t, name, data) else { continue };
        t.check(&format!("{name}: repaired"), doc.was_repaired() || name == "no startxref");
        t.eq(&format!("{name}: pages"), doc.page_count(), 3);
        t.eq(&format!("{name}: text of page 2"), doc.text(1).ok().as_deref(), Some("Seite 2\n"));
    }
    // a scanned file finds the objects of its object streams and the catalog without a trailer
    let xs = base.xref_stream("/Root 1 0 R", true);
    let cut_at = xs.windows(4).rposition(|w| w == b"/XRe").unwrap_or(xs.len()).saturating_sub(12);
    if let Some(doc) = open(t, "object stream without its cross-reference", xs[..cut_at].to_vec()) {
        t.check("objstm scan: repaired", doc.was_repaired());
        t.eq("objstm scan: pages", doc.page_count(), 3);
        t.eq("objstm scan: text", doc.text(0).ok().as_deref(), Some("Seite 1\n"));
    }
    // a section that says it starts at 1 but holds object 0
    let one_off = {
        let s = String::from_utf8_lossy(&good).replacen("xref\n0 ", "xref\n1 ", 1);
        s.into_bytes()
    };
    if let Some(doc) = open(t, "subsection off by one", one_off) {
        t.eq("off by one: pages", doc.page_count(), 3);
    }
    // stream lengths: too long, too short, indirect, missing
    for (name, len) in [("too long", "/Length 999"), ("too short", "/Length 3"), ("indirect", "/Length 8 0 R"), ("missing", "")] {
        let content = b"BT /F1 12 Tf 50 700 Td (Laenge) Tj ET";
        let mut m = one_page(b"", None, "");
        m.add(4, cat(&[format!("<< {len} >>\nstream\n").as_bytes(), content, b"\nendstream"])).add(8, format!("{}", content.len()));
        let got = text0(t, name, m.table("/Root 1 0 R"));
        t.eq(&format!("stream length {name}"), got.as_str(), "Laenge\n");
    }
    // the page tree: inheritance, rotation, a crop box, a loop
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R 2 0 R 6 0 R 6 0 R] /Count 2 /Rotate 90 /MediaBox [0 0 300 400] >>")
        .add(3, "<< /Type /Pages /Parent 2 0 R /Kids [5 0 R] /Count 1 /CropBox [10 10 210 310] >>")
        .add(5, "<< /Type /Page /Parent 3 0 R /Rotate -90 >>")
        .add(6, "<< /Type /Page /Parent 2 0 R >>");
    if let Some(doc) = open(t, "page tree", m.table("/Root 1 0 R")) {
        t.eq("tree: pages (a loop and a page listed twice count once)", doc.page_count(), 2);
        t.eq("tree: crop box inherited", doc.page(0).map(|p| p.crop_box).ok(), Some([10.0, 10.0, 210.0, 310.0]));
        t.eq("tree: rotate -90 is 270", doc.page(0).map(|p| p.rotate).ok(), Some(270));
        t.eq("tree: size after rotation", doc.page(1).map(|p| p.size()).ok(), Some((400.0, 300.0)));
    }
    // the catalog's version beats the header's
    let mut m = one_page(b"", None, "");
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /Version /2.0 >>");
    if let Some(doc) = open(t, "version", m.table("/Root 1 0 R")) {
        t.eq("catalog version", doc.version(), (2, 0));
    }
    t.check("not a PDF", matches!(Document::open(b"hello, world".to_vec()), Err(Error::NotPdf)));
    t.check("empty", matches!(Document::open(Vec::new()), Err(Error::NotPdf)));
}

// ------------------------------------------------------------------ filters

/// PNG predictor rows (Predictor 10 + type) for `data` in rows of `cols` bytes
fn png_encode(data: &[u8], cols: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = vec![0u8; cols];
    for (r, row) in data.chunks(cols).enumerate() {
        let kind = (r % 5) as u8;
        out.push(kind);
        for i in 0..row.len() {
            let a = if i >= bpp { row[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let p = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                _ => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
            };
            out.push(row[i].wrapping_sub(p));
        }
        prev[..row.len()].copy_from_slice(row);
    }
    out
}

fn read_filters(t: &mut T) {
    let sample: Vec<u8> = (0..3000u32).map(|i| (i * 7 + i / 13) as u8).collect();
    let lzw_spec = [0x80u8, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
    let tiff8: Vec<u8> = {
        // 2 colours, 8 bits, 10 columns: differences per row
        let mut v = Vec::new();
        for row in sample[..400].chunks(20) {
            for i in 0..row.len() {
                v.push(if i >= 2 { row[i].wrapping_sub(row[i - 2]) } else { row[i] });
            }
        }
        v
    };
    let tiff16: Vec<u8> = {
        let mut v = Vec::new();
        for row in sample[..400].chunks(20) {
            let w: Vec<u16> = row.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            for i in 0..w.len() {
                let d = if i >= 1 { w[i].wrapping_sub(w[i - 1]) } else { w[i] };
                v.extend_from_slice(&d.to_be_bytes());
            }
        }
        v
    };
    let cut = {
        let z = compress::zlib::compress(&sample, 6);
        z[..z.len() / 2].to_vec()
    };
    let cases: Vec<(&str, Vec<u8>, Option<Vec<u8>>)> = vec![
        ("ASCIIHex", stream("/Filter /ASCIIHexDecode", b"48 65 6c6C\n6f 2>"), Some(b"Hello ".to_vec())),
        ("ASCII85", stream("/Filter /ASCII85Decode", b"87cURD]i,\"Ebo7~>"), Some(b"Hello World".to_vec())),
        ("ASCII85 z and <~", stream("/Filter /A85", b"<~z@:E^~>"), Some(b"\0\0\0\0abc".to_vec())),
        ("LZW (the example of ISO 32000)", stream("/Filter /LZWDecode", &lzw_spec), Some(b"-----A---B".to_vec())),
        ("RunLength", stream("/Filter /RunLengthDecode", &[2, b'a', b'b', b'c', 253, b'x', 128, b'z']), Some(b"abcxxxx".to_vec())),
        ("Flate", flate("", &sample), Some(sample.clone())),
        ("Flate, raw DEFLATE", stream("/Filter /FlateDecode", &compress::deflate::compress(&sample, 6)), Some(sample.clone())),
        ("Flate cut short gives the first part", stream("/Filter /FlateDecode", &cut), None),
        ("PNG predictors", flate("/DecodeParms << /Predictor 15 /Colors 3 /Columns 20 >>", &png_encode(&sample[..1200], 60, 3)), Some(sample[..1200].to_vec())),
        ("TIFF predictor, 8 bits", flate("/DecodeParms << /Predictor 2 /Colors 2 /Columns 10 >>", &tiff8), Some(sample[..400].to_vec())),
        ("TIFF predictor, 16 bits", flate("/DecodeParms << /Predictor 2 /Colors 1 /Columns 10 /BitsPerComponent 16 >>", &tiff16), Some(sample[..400].to_vec())),
        ("two filters", stream("/Filter [/ASCIIHexDecode /FlateDecode]", compress::zlib::compress(b"zwei Filter", 6).iter().map(|b| format!("{b:02x}")).collect::<String>().as_bytes()), Some(b"zwei Filter".to_vec())),
        ("filter by reference", stream("/Filter 8 0 R", b"414243>"), Some(b"ABC".to_vec())),
    ];
    for (name, body, want) in cases {
        let mut m = one_page(b"", None, "");
        m.add(7, body).add(8, "/ASCIIHexDecode");
        let Some(doc) = open(t, name, m.table("/Root 1 0 R")) else { continue };
        let Object::Stream(s) = doc.get(rf(7)) else {
            t.check(&format!("{name}: a stream"), false);
            continue;
        };
        let got = doc.decode(&s);
        match want {
            Some(w) => t.eq(name, got.ok(), Some(w)),
            None => t.check(name, got.as_ref().is_ok_and(|g| g.len() > 100 && sample.starts_with(g))),
        }
    }
    // limits: a bomb of zeros
    let bomb = flate("", &vec![0u8; 8 << 20]);
    let mut m = one_page(b"", None, "");
    m.add(7, bomb);
    let small = Options { max_stream: 1 << 20, ..Options::default() };
    if let Some(doc) = t.ok("bomb: open", Document::open_with(m.table("/Root 1 0 R"), &small).map_err(|e| format!("{e}"))) {
        if let Object::Stream(s) = doc.get(rf(7)) {
            t.check("bomb: refused at max_stream", matches!(doc.decode(&s), Err(Error::Limit(_))));
        }
    }
    // an image filter: the chain stops before it
    let mut m = one_page(b"", None, "");
    m.add(7, stream("/Filter [/ASCIIHexDecode /DCTDecode] /DecodeParms [null << /ColorTransform 0 >>]", b"FFD8FFD9>"));
    if let Some(doc) = open(t, "image filter", m.table("/Root 1 0 R")) {
        if let Object::Stream(s) = doc.get(rf(7)) {
            t.check("image filter: decode refuses", matches!(doc.decode(&s), Err(Error::Unsupported(_))));
            let r = doc.decode_until_image(&s);
            t.check("image filter: the JPEG and its parameters", matches!(&r, Ok((d, Some((n, p)))) if d == &[0xFF, 0xD8, 0xFF, 0xD9] && n == "DCTDecode" && p.int("ColorTransform") == Some(0)));
        }
    }
}

// ------------------------------------------------------------------ encryption

pub(crate) const FIX: &[(&str, &[u8], u8, bool)] = &[
    ("r2_rc4_40", include_bytes!("../tests/read/r2_rc4_40.pdf"), 2, true),
    ("r3_rc4_128", include_bytes!("../tests/read/r3_rc4_128.pdf"), 3, true),
    ("r3_rc4_128_no_user_pw", include_bytes!("../tests/read/r3_rc4_128_no_user_pw.pdf"), 3, false),
    ("r4_rc4_crypt_filter", include_bytes!("../tests/read/r4_rc4_crypt_filter.pdf"), 4, true),
    ("r4_aes_128", include_bytes!("../tests/read/r4_aes_128.pdf"), 4, true),
    ("r5_aes_256", include_bytes!("../tests/read/r5_aes_256.pdf"), 5, true),
    ("r6_aes_256", include_bytes!("../tests/read/r6_aes_256.pdf"), 6, true),
    ("r6_aes_256_no_user_pw", include_bytes!("../tests/read/r6_aes_256_no_user_pw.pdf"), 6, false),
];

fn read_crypt(t: &mut T) {
    let with = |data: &[u8], pw: &str| Document::open_with(data.to_vec(), &Options { password: pw.into(), ..Options::default() });
    for &(name, data, rev, user) in FIX {
        let none = Document::open(data.to_vec());
        if user {
            t.check(&format!("{name}: no password refused"), matches!(none, Err(Error::Password)));
        } else {
            t.check(&format!("{name}: opens without a password"), none.as_ref().is_ok_and(|d| !d.opened_as_owner()));
        }
        t.check(&format!("{name}: a wrong password refused"), matches!(with(data, "falsch"), Err(Error::Password)));
        for (pw, owner) in [("user", false), ("owner", true)] {
            if pw == "user" && !user {
                continue;
            }
            let Some(doc) = t.ok(&format!("{name}: {pw}"), with(data, pw).map_err(|e| format!("{e}"))) else { continue };
            let n = format!("{name} as {pw}");
            t.check(&format!("{n}: encrypted"), doc.is_encrypted());
            t.eq(&format!("{n}: revision"), doc.encryption_revision(), Some(rev));
            t.eq(&format!("{n}: owner"), doc.opened_as_owner(), owner);
            t.eq(&format!("{n}: permissions"), doc.permissions(), Some(-1340i32 as u32));
            t.eq(&format!("{n}: title (an encrypted string)"), doc.info().title, Some(format!("Verschlüsselt R{rev}")));
            t.eq(&format!("{n}: outline"), doc.outlines().first().map(|o| o.title.clone()), Some("Kapitel eins".into()));
            t.check(&format!("{n}: text (encrypted streams)"), doc.text(0).is_ok_and(|s| s.contains("Grüße aus dem Lesetest")));
            t.check(&format!("{n}: page 2"), doc.text(1).is_ok_and(|s| s.contains("„Anführungszeichen“")));
            t.check(&format!("{n}: metadata"), doc.metadata().is_some_and(|m| m.windows(9).any(|w| w == b"Metadaten")));
        }
    }
}

// ------------------------------------------------------------------ text

fn read_text(t: &mut T) {
    // the widths of Helvetica without /Widths: H 722, a 556, l 222, o 556, space 278
    let doc = Document::open(one_page(b"BT /F1 10 Tf 100 700 Td (Hallo Welt) Tj ET", None, "").table("/Root 1 0 R")).ok();
    if let Some(p) = doc.as_ref().and_then(|d| d.page_text(0).ok()) {
        let xs: Vec<i64> = p.items.iter().map(|i| rnd(i.x * 100.0)).collect();
        t.eq("standard widths: positions", xs.get(..7).map(<[i64]>::to_vec), Some(vec![10000, 10722, 11278, 11500, 11722, 12278, 12556]));
        t.eq("standard widths: the text", p.text(), String::from("Hallo Welt\n"));
    } else {
        t.check("standard widths: read", false);
    }
    let cases: Vec<(&str, &[u8], &str)> = vec![
        ("TJ: a large gap is a space, kerning is not", b"BT /F1 10 Tf 50 700 Td [(Wort)-300(zw)-40(ei)80(.)] TJ ET".as_slice(), "Wort zwei.\n"),
        ("Td: two lines", b"BT /F1 10 Tf 50 700 Td (eins) Tj 0 -14 Td (zwei) Tj ET".as_slice(), "eins\nzwei\n"),
        ("T* and '", b"BT /F1 10 Tf 14 TL 50 700 Td (a) Tj T* (b) Tj (c) ' ET".as_slice(), "a\nb\nc\n"),
        ("Tc: spaced letters stay one word", b"BT /F1 10 Tf 1 Tc 50 700 Td (gesperrt) Tj ET".as_slice(), "gesperrt\n"),
        ("Tw: word spacing", b"BT /F1 10 Tf 5 Tw 50 700 Td (ein Satz) Tj ET".as_slice(), "ein Satz\n"),
        ("Ts: a raised word stays on the line", b"BT /F1 10 Tf 50 700 Td (x) Tj 3 Ts (2) Tj 0 Ts ( ist) Tj ET".as_slice(), "x2 ist\n"),
        ("Tz: horizontal scaling", b"BT /F1 10 Tf 50 Tz 50 700 Td (eng) Tj ( gestellt) Tj ET".as_slice(), "eng gestellt\n"),
        ("fake bold: drawn twice", b"BT /F1 10 Tf 50 700 Td (fett) Tj ET BT /F1 10 Tf 50.2 700 Td (fett) Tj ET".as_slice(), "fett\n"),
        ("cm and q / Q", b"q 2 0 0 2 0 0 cm BT /F1 10 Tf 25 350 Td (gross) Tj ET Q BT /F1 10 Tf 50 680 Td (klein) Tj ET".as_slice(), "gross\nklein\n"),
        ("a space glyph without a gap is no space", b"BT /F1 10 Tf -2.7 Tw 50 700 Td (zu sammen) Tj ET".as_slice(), "zusammen\n"),
        ("Tm: a turned line", b"BT /F1 10 Tf 0 1 -1 0 300 300 Tm (aufwaerts) Tj ET".as_slice(), "aufwaerts\n"),
        ("inline image, then text", b"BI /W 2 /H 2 /CS /G /BPC 8 ID \x00EI\xff\x80 EI Q (x) BT /F1 10 Tf 50 700 Td (danach) Tj ET".as_slice(), "danach\n"),
        ("WinAnsi: umlauts and the euro", b"BT /F1 10 Tf 50 700 Td (\xc4\xd6\xdc\xe4\xf6\xfc\xdf \x80 \x93x\x94) Tj ET".as_slice(), "ÄÖÜäöüß € “x”\n"),
    ];
    for (name, content, want) in cases {
        let got = text0(t, name, one_page(content, None, "").table("/Root 1 0 R"));
        t.eq(name, got.as_str(), want);
    }
    // invisible text
    if let Some(doc) = Document::open(one_page(b"BT /F1 10 Tf 3 Tr 50 700 Td (unsichtbar) Tj ET", None, "").table("/Root 1 0 R")).ok() {
        let p = doc.page_text(0).unwrap_or_default();
        t.check("render mode 3: invisible", !p.items.is_empty() && p.items.iter().all(|i| i.invisible));
        t.eq("render mode 3: still text", p.text(), String::from("unsichtbar\n"));
    }
    // Differences and glyph names
    let font = b"<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Demo /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Adieresis /uni20AC /u1F600 /f_i /a.sc /.notdef /afii10017 /Omega] >> /FirstChar 0 /LastChar 255 >>";
    let got = text0(t, "differences", one_page(b"BT /F1 10 Tf 50 700 Td (ABCDEFGH) Tj ET", Some(font), "").table("/Root 1 0 R"));
    // .notdef has no text: its room is a gap, as in Poppler
    t.eq("Differences: glyph names", got.as_str(), "Ä€😀fia А\u{2126}\n");
    // the Symbol font's own encoding
    let sym = b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>";
    let got = text0(t, "symbol", one_page(b"BT /F1 10 Tf 50 700 Td (abg \xb3 p) Tj ET", Some(sym), "").table("/Root 1 0 R"));
    t.eq("Symbol", got.as_str(), "αβγ ≥ π\n");
    // the built-in encoding of an embedded Type 1 program
    let prog = b"%!PS-AdobeFont-1.0: Demo 001\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /Euro put\ndup 66 /germandbls put\nreadonly def\ncurrentfile eexec\n\x8f\x00\x01";
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (AB) Tj ET", Some(b"<< /Type /Font /Subtype /Type1 /BaseFont /Demo /FontDescriptor 6 0 R >>"), "");
    m.add(6, "<< /Type /FontDescriptor /FontName /Demo /Flags 4 /FontFile 7 0 R >>").add(7, stream("/Length1 150 /Length2 3 /Length3 0", prog));
    let got = text0(t, "type1", m.table("/Root 1 0 R"));
    t.eq("Type 1 built-in encoding", got.as_str(), "€ß\n");
    // a composite font with a ToUnicode CMap: ranges, arrays, a ligature
    let tu = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n2 beginbfrange <0001> <0003> <0041> <0010> <0011> [<00DF> <D83DDE00>] endbfrange\n1 beginbfchar <0020> <0066006C> endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end";
    let mut m = one_page(b"BT /F1 20 Tf 50 700 Td <0001000200030010> Tj <00110020> Tj ET", Some(b"<< /Type /Font /Subtype /Type0 /BaseFont /Comp /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"), "");
    m.add(6, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Comp /DW 500 /W [1 3 600 16 [700]] >>").add(7, flate("", tu));
    if let Some(doc) = open(t, "type0", m.table("/Root 1 0 R")) {
        let p = doc.page_text(0).unwrap_or_default();
        t.eq("Type 0: text", p.text(), String::from("ABCß😀fl\n"));
        let adv: Vec<i64> = p.items.iter().map(|i| rnd(i.dx * 10.0)).collect();
        t.eq("Type 0: widths from /W and /DW", adv, vec![120, 120, 120, 140, 100, 100]);
    }
    // vertical writing (Identity-V): the pen goes down
    let mut m = one_page(b"BT /F1 20 Tf 300 700 Td <00010002> Tj ET", Some(b"<< /Type /Font /Subtype /Type0 /BaseFont /Vert /Encoding /Identity-V /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"), "");
    m.add(6, "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Vert /DW2 [880 -1000] >>").add(7, flate("", b"begincmap 1 beginbfrange <0001> <0002> <4E00> endbfrange endcmap"));
    if let Some(doc) = open(t, "vertical", m.table("/Root 1 0 R")) {
        let p = doc.page_text(0).unwrap_or_default();
        t.check("vertical: one below the other", p.items.len() == 2 && (p.items[1].y - (p.items[0].y - 20.0)).abs() < 1e-6 && p.items[0].x == p.items[1].x);
        t.eq("vertical: text", p.text(), String::from("一丁\n"));
    }
    // a Type 3 font: widths in glyph space through the font matrix
    let t3 = b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [0.01 0 0 0.01 0 0] /CharProcs << /a 8 0 R >> /Encoding << /Differences [97 /a /b] >> /FirstChar 97 /LastChar 98 /Widths [50 80] /Resources << >> >>";
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (abab) Tj ET", Some(t3), "");
    m.add(8, stream("", b"50 0 d0 0 0 50 50 re f"));
    if let Some(doc) = open(t, "type3", m.table("/Root 1 0 R")) {
        let p = doc.page_text(0).unwrap_or_default();
        t.eq("Type 3: advances", p.items.iter().map(|i| rnd(i.dx * 100.0)).collect::<Vec<_>>(), vec![500, 800, 500, 800]);
        t.eq("Type 3: text", p.text(), String::from("abab\n"));
    }
    // form XObjects: their matrix and resources, one that draws itself
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (vorher) Tj ET /X1 Do /X1 Do", None, "/XObject << /X1 6 0 R >>");
    m.add(6, flate("/Type /XObject /Subtype /Form /BBox [0 0 600 800] /Matrix [1 0 0 1 0 -100] /Resources << /Font << /G 5 0 R >> /XObject << /X1 6 0 R >> >>", b"BT /G 10 Tf 50 700 Td (im Formular) Tj ET /X1 Do"));
    if let Some(doc) = open(t, "form", m.table("/Root 1 0 R")) {
        let p = doc.page_text(0).unwrap_or_default();
        // drawn twice at the same place: the text once
        t.eq("form: text", p.text(), String::from("vorher\nim Formular\n"));
        t.eq("form: both drawings are glyphs", p.items.iter().filter(|i| i.text == "F").count(), 2);
        t.check("form: its matrix", p.items.iter().any(|i| i.text == "i" && (i.y - 600.0).abs() < 1e-6));
    }
    // Hebrew drawn left to right (visual order) reads right to left; numbers keep their order
    let heb = b"<< /Type /Font /Subtype /Type0 /BaseFont /Heb /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>";
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td <000100020003000400050006000700080009> Tj ET", Some(heb), "");
    // "ab 12 " and the Hebrew letters of "לום" in visual order (final mem first)
    m.add(6, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Heb /DW 500 >>").add(
        7,
        flate("", b"begincmap 9 beginbfchar <0001> <0061> <0002> <0062> <0003> <0020> <0004> <0031> <0005> <0032> <0006> <0020> <0007> <05DD> <0008> <05D5> <0009> <05DC> endbfchar endcmap"),
    );
    let got = text0(t, "hebrew", m.table("/Root 1 0 R"));
    t.eq("right to left: reading order", got.as_str(), "ab 12 לום\n");
}

// ------------------------------------------------------------------ navigation and forms

fn read_nav(t: &mut T) {
    let mut m = Mk::new();
    let kids: Vec<String> = (0..6).map(|i| format!("{} 0 R", 10 + i)).collect();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /Outlines 30 0 R /Dests 40 0 R /Names << /Dests 41 0 R /EmbeddedFiles 50 0 R >> /PageLabels 3 0 R /AcroForm << /Fields [60 0 R 61 0 R 62 0 R 63 0 R 64 0 R] >> /Lang (de-CH) >>")
        .add(2, format!("<< /Type /Pages /Kids [{}] /Count 6 /MediaBox [0 0 400 400] >>", kids.join(" ")))
        .add(3, "<< /Nums [0 << /S /r >> 2 << /S /D /P (A-) /St 3 >> 4 << /S /A >> 5 << /P (Anhang) >>] >>")
        .add(4, "<< /Title <FEFF00DC0062006500720073006900630068007400> /CreationDate (D:2026112312) /ModDate (D:20261123143005Z) /Author (garbage date) /Subject (x) >>");
    for i in 0..6u32 {
        m.add(10 + i, if i == 0 { String::from("<< /Type /Page /Parent 2 0 R /Annots [20 0 R 21 0 R 22 0 R 65 0 R 66 0 R] >>") } else { String::from("<< /Type /Page /Parent 2 0 R >>") });
    }
    m.add(20, "<< /Type /Annot /Subtype /Link /Rect [100 50 10 20] /A << /S /URI /URI (https://example.org/a b) >> >>")
        .add(21, "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (kapitel) >>")
        .add(22, "<< /Type /Annot /Subtype /Text /Rect [0 0 20 20] /Contents (Eine Notiz) /T (Erika) /M (D:20260101120000+02'00') >>")
        // the outline: A (open) with A1, A2; B -> named dest; C -> a loop back to A
        .add(30, "<< /Type /Outlines /First 31 0 R /Last 34 0 R /Count 4 >>")
        .add(31, "<< /Title (A) /Parent 30 0 R /Next 32 0 R /First 35 0 R /Last 36 0 R /Count 2 /Dest [12 0 R /XYZ 10 20 1.5] >>")
        .add(32, "<< /Title (B) /Parent 30 0 R /Prev 31 0 R /Next 33 0 R /Dest /alt >>")
        .add(33, "<< /Title (C) /Parent 30 0 R /Prev 32 0 R /Next 31 0 R /A << /S /Named /N /NextPage >> >>")
        .add(35, "<< /Title (A1) /Parent 31 0 R /Next 36 0 R /A << /S /GoTo /D (kapitel) >> >>")
        .add(36, "<< /Title (A2) /Parent 31 0 R /Prev 35 0 R /Count -1 /A << /S /GoToR /F (andere.pdf) /D [4 /Fit] >> /First 37 0 R >>")
        .add(37, "<< /Title (A2a) /Parent 36 0 R /Dest [15 0 R /FitH 300] >>")
        .add(40, "<< /alt [13 0 R /Fit] >>")
        // a name tree with two kids and limits
        .add(41, "<< /Kids [42 0 R 43 0 R] >>")
        .add(42, "<< /Limits [(anfang) (ende)] /Names [(anfang) [10 0 R /Fit] (ende) [11 0 R /Fit]] >>")
        .add(43, "<< /Limits [(kapitel) (zz)] /Names [(kapitel) << /D [14 0 R /FitR 1 2 3 4] >> (zz) [15 0 R /Fit]] >>")
        // attachments
        .add(50, "<< /Names [(rechnung.xml) 51 0 R] >>")
        .add(51, "<< /Type /Filespec /F (rechnung.xml) /UF <FEFF006600610063007400750072002D0078002D0072006500630068006E0075006E0067002E0078006D006C> /Desc (Die Rechnung) /AFRelationship /Data /EF << /F 52 0 R >> >>")
        .add(52, flate("/Type /EmbeddedFile /Subtype /text#2Fxml /Params << /Size 19 /ModDate (D:20261123) >>", b"<rechnung>1</rechnung>"))
        // fields: text, checkbox, radio with two widgets, combo box with options, signature
        .add(60, "<< /FT /Tx /T (name) /V (Erika Mustermann) /Ff 2 /Rect [0 0 100 20] /P 10 0 R >>")
        .add(61, "<< /FT /Btn /T (zustimmung) /V /Yes /Rect [0 30 10 40] >>")
        .add(62, "<< /FT /Btn /T (farbe) /Ff 49152 /V /rot /Kids [65 0 R 66 0 R] >>")
        .add(65, "<< /Type /Annot /Subtype /Widget /Parent 62 0 R /Rect [0 50 10 60] /AS /rot >>")
        .add(66, "<< /Type /Annot /Subtype /Widget /Parent 62 0 R /Rect [20 50 30 60] /AS /Off >>")
        .add(63, "<< /T (adresse) /Kids [67 0 R] >>")
        .add(67, "<< /FT /Ch /T (land) /Parent 63 0 R /Ff 131073 /Opt [[(de) (Deutschland)] (Schweiz)] /V (Deutschland) >>")
        .add(64, "<< /FT /Sig /T (unterschrift) /V 68 0 R >>")
        .add(68, "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /Name (Erika) /Reason (Zustimmung) /M (D:20261123120000+01'00') /ByteRange [0 10 20 30] /Contents <3082> >>");
    let Some(doc) = open(t, "navigation", m.table("/Root 1 0 R /Info 4 0 R")) else { return };
    // information
    let info = doc.info();
    t.eq("title in UTF-16", info.title.as_deref(), Some("Übersicht"));
    t.eq("a date of year, month, day, hour", info.created.map(|d| (d.year, d.month, d.day, d.hour, d.minute, d.offset_minutes)), Some((2026, 11, 23, 12, 0, 0)));
    t.eq("a date in UTC", info.modified.map(|d| (d.hour, d.minute, d.second, d.offset_minutes)), Some((14, 30, 5, 0)));
    t.eq("language", doc.language().as_deref(), Some("de-CH"));
    t.eq("parse_date: offset", parse_date("D:20260601083000-05'30'").map(|d| d.offset_minutes), Some(-330));
    t.check("parse_date: garbage", parse_date("gestern").is_none() && parse_date("D:20261399").is_none());
    // labels
    let labels: Vec<Option<String>> = (0..6).map(|i| doc.page_label(i)).collect();
    t.eq("page labels", labels, vec![Some("i".into()), Some("ii".into()), Some("A-3".into()), Some("A-4".into()), Some("A".into()), Some("Anhang".into())]);
    // outline
    let o = doc.outlines();
    t.eq("outline titles (the loop ends)", o.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["A", "B", "C"]);
    t.check("outline: A open at XYZ on page 3", o.first().is_some_and(|a| a.open && matches!(&a.action, Some(Action::GoTo(d)) if d.page == 2 && d.view == View::Xyz { left: Some(10.0), top: Some(20.0), zoom: Some(1.5) })));
    t.check("outline: B through /Dests", matches!(o.get(1).and_then(|b| b.action.clone()), Some(Action::GoTo(d)) if d.page == 3 && d.view == View::Fit));
    t.eq("outline: C is a named action", o.get(2).and_then(|c| c.action.clone()), Some(Action::Named("NextPage".into())));
    let a = o.first().map(|a| a.children.clone()).unwrap_or_default();
    t.check("outline: A1 through the name tree", matches!(a.first().and_then(|x| x.action.clone()), Some(Action::GoTo(d)) if d.page == 4 && d.view == View::FitR([1.0, 2.0, 3.0, 4.0])));
    t.eq("outline: A2 to another file", a.get(1).and_then(|x| x.action.clone()), Some(Action::GoToFile { file: "andere.pdf".into(), page: Some(4) }));
    t.check("outline: A2 closed, A2a below", a.get(1).is_some_and(|x| !x.open && x.children.len() == 1));
    t.eq("named_dest", doc.named_dest("zz").map(|d| d.page), Some(5));
    t.eq("named_dest: none", doc.named_dest("nichts"), None);
    // annotations
    let an = doc.annotations(0).unwrap_or_default();
    t.eq("annotation kinds", an.iter().map(|x| x.subtype.as_str()).collect::<Vec<_>>(), vec!["Link", "Link", "Text", "Widget", "Widget"]);
    t.eq("link rectangle normalized", an.first().map(|x| x.rect), Some([10.0, 20.0, 100.0, 50.0]));
    t.check("note: text, author, date", an.get(2).is_some_and(|x| x.contents.as_deref() == Some("Eine Notiz") && x.author.as_deref() == Some("Erika") && x.modified.is_some_and(|d| d.offset_minutes == 120)));
    let links = doc.links(0).unwrap_or_default();
    t.eq("links", links.iter().map(|l| l.1.clone()).collect::<Vec<_>>(), vec![Action::Uri("https://example.org/a b".into()), Action::GoTo(pdf::read::Dest { page: 4, view: View::FitR([1.0, 2.0, 3.0, 4.0]) })]);
    // fields
    let f = doc.fields();
    t.eq("field names", f.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["name", "zustimmung", "farbe", "adresse.land", "unterschrift"]);
    let get = |n: &str| f.iter().find(|x| x.name == n).cloned();
    t.check("text field", get("name").is_some_and(|x| x.kind == FieldKind::Text && x.value == ["Erika Mustermann"] && x.required && !x.read_only && x.widgets == [(Some(0), [0.0, 0.0, 100.0, 20.0])]));
    t.check("check box", get("zustimmung").is_some_and(|x| x.kind == FieldKind::CheckBox && x.value == ["Yes"]));
    t.check("radio: two widgets on page 1", get("farbe").is_some_and(|x| x.kind == FieldKind::Radio && x.value == ["rot"] && x.widgets.len() == 2 && x.widgets.iter().all(|w| w.0 == Some(0))));
    t.check("combo box: options as shown", get("adresse.land").is_some_and(|x| x.kind == FieldKind::ComboBox && x.options == ["Deutschland", "Schweiz"] && x.value == ["Deutschland"] && x.read_only));
    t.check(
        "signature as stated",
        get("unterschrift").and_then(|x| x.signature).is_some_and(|s| s.sub_filter == "adbe.pkcs7.detached" && s.reason.as_deref() == Some("Zustimmung") && s.byte_range == [(0, 10), (20, 30)] && s.contents == [0x30, 0x82] && !s.covers_file),
    );
    // attachments
    let at = doc.attachments();
    t.eq("attachments", at.len(), 1);
    if let Some(x) = at.first() {
        t.eq("attachment: name (/UF)", x.name.as_str(), "factur-x-rechnung.xml");
        t.eq("attachment: MIME type", x.mime.as_deref(), Some("text/xml"));
        t.eq("attachment: description, relationship, size", (x.description.as_deref(), x.relationship.as_deref(), x.size), (Some("Die Rechnung"), Some("Data"), Some(19)));
        t.eq("attachment: data", x.stream.as_ref().and_then(|s| doc.decode(s).ok()), Some(b"<rechnung>1</rechnung>".to_vec()));
    }
}

// ------------------------------------------------------------------ hostile files

/// open and read everything there is; nothing may panic or hang
fn read_all(data: Vec<u8>) -> bool {
    let Ok(doc) = Document::open_with(data, &Options { max_stream: 8 << 20, ..Options::default() }) else { return false };
    let _ = doc.info();
    let _ = doc.metadata();
    let _ = doc.outlines();
    let _ = doc.fields();
    let _ = doc.attachments();
    for i in 0..doc.page_count().min(8) {
        let _ = doc.text(i);
        let _ = doc.annotations(i);
        let _ = doc.page_label(i);
    }
    true
}

fn read_hostile(t: &mut T) {
    let deep = cat(&[b"<< /A ", &b"[".repeat(5000), &b"]".repeat(5000), b" >>"]);
    let mut cases: Vec<(&str, Mk)> = Vec::new();
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (ok) Tj ET", None, "");
    m.add(9, deep);
    cases.push(("nesting 5000 deep", m));
    let mut m = one_page(b"", None, "");
    m.add(9, "9 0 R").add(1, "<< /Type /Catalog /Pages 2 0 R /Outlines 9 0 R /Names 9 0 R /AcroForm << /Fields [9 0 R] >> >>");
    cases.push(("a reference to itself", m));
    let mut m = one_page(b"", None, "");
    m.add(4, "<< /Length 4 0 R >>\nstream\nBT ET\nendstream");
    cases.push(("a length that is its own stream", m));
    let mut m = one_page(b"", None, "");
    m.add(4, "<< /Length 8 0 R >>\nstream\nBT ET\nendstream").add(8, "<< /Length 4 0 R >>\nstream\n1\nendstream");
    cases.push(("lengths in a loop", m));
    // ten forms, each drawing the next 16 times: 16^10 drawings unless the work is counted
    let mut m = one_page(b"/N Do", None, "/XObject << /N 20 0 R >>");
    for k in 0..10u32 {
        let body = if k == 9 { String::from("BT /F1 1 Tf (z) Tj ET") } else { "/N Do ".repeat(16) };
        m.add(20 + k, flate(&format!("/Subtype /Form /Resources << /XObject << /N {} 0 R >> /Font << /F1 5 0 R >> >>", 21 + k), body.as_bytes()));
    }
    cases.push(("forms that draw each other", m));
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (x) Tj ET", Some(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 9223372036854775807 /Widths [1 2] /Encoding << /Differences [9223372036854775807 /a /b] >> >>"), "");
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /D /St 9223372036854775807 >>] >> >>");
    cases.push(("huge numbers", m));
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td <0001> Tj ET", Some(b"<< /Type /Font /Subtype /Type0 /Encoding 6 0 R /DescendantFonts [<< /W [0 [1 2 3] 4294967295 4294967295 5] >>] /ToUnicode 6 0 R >>"), "");
    m.add(6, stream("", b"begincmap 1 begincodespacerange <> <> endcodespacerange 1 beginbfrange <FFFFFFFF> <00000000> <0041> <00000000> <FFFFFFFF> <0041> endbfrange endcmap"));
    cases.push(("odd CMaps", m));
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 3 0 R >> /Outlines << /First 4 0 R >> >>")
        .add(2, "<< /Type /Pages /Kids [2 0 R 5 0 R] /Count 99999999999999999999 >>")
        .add(3, "<< /Kids [3 0 R] /Limits [(a) (z)] >>")
        .add(4, "<< /Title (x) /Next 4 0 R /First 4 0 R /Dest (m) >>")
        .add(5, "<< /Type /Page /Parent 2 0 R /Annots [6 0 R] >>")
        .add(6, "<< /Subtype /Link /Dest (m) /Rect [0 0 1 1] >>");
    cases.push(("loops in trees", m));
    let mut m = one_page(b"", None, "");
    m.add(7, flate("/Type /ObjStm /N 2 /First 8", b"8 0 7 3 << /A 7 0 R >> 7 0 R"));
    cases.push(("an object stream that holds itself", m));
    for (name, m) in cases {
        for (layout, data) in [("table", m.table("/Root 1 0 R")), ("streams", m.xref_stream("/Root 1 0 R", true))] {
            read_all(data);
            t.check(&format!("{name} ({layout}): read through"), true);
        }
    }
    // the deep object reads as null or as far as it goes, the self reference as null
    let mut m = one_page(b"", None, "");
    m.add(9, "9 0 R");
    if let Ok(doc) = Document::open(m.table("/Root 1 0 R")) {
        t.check("self reference resolves to null", doc.resolve(&Object::Ref(rf(9))).is_null());
    }
    // mutations of the files of the other groups, with a fixed seed (the RC4 and AES-128 ones:
    // each try of an AES-256 password costs milliseconds, read_crypt has those)
    let mut seeds: Vec<Vec<u8>> = FIX.iter().filter(|f| f.2 <= 4).map(|f| f.1.to_vec()).collect();
    if let Ok(s) = crate::sample::build() {
        seeds.push(s);
    }
    let mut m = one_page(b"BT /F1 10 Tf 50 700 Td (Hallo) Tj [(a)-300(b)] TJ ET BI /W 1 /H 1 /BPC 8 /CS /G ID x EI", None, "");
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /Outlines << /First 9 0 R >> /PageLabels << /Nums [0 << /S /R >>] >> >>").add(9, "<< /Title (o) /Dest [3 0 R /Fit] >>");
    seeds.push(m.table("/Root 1 0 R"));
    seeds.push(m.xref_stream("/Root 1 0 R", true));
    let tokens: &[&[u8]] = &[b"obj", b"endobj", b"<<", b">>", b"[", b"]", b"(", b")", b"stream\n", b"endstream", b" 0 R", b"xref", b"trailer", b"/Length", b"%%EOF", b"\x00", b"999999999999", b"-1"];
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut rnd = move |n: usize| -> usize {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n.max(1) as u64) as usize
    };
    let (mut opened, mut total) = (0, 0);
    for round in 0..1600 {
        let base = &seeds[round % seeds.len()];
        let mut d = base.clone();
        for _ in 0..1 + rnd(6) {
            if d.is_empty() {
                break;
            }
            let at = rnd(d.len());
            match rnd(5) {
                0 => d[at] = rnd(256) as u8,
                1 => d[at] ^= 1 << rnd(8),
                2 => {
                    let tk = tokens[rnd(tokens.len())];
                    d.splice(at..at, tk.iter().copied());
                }
                3 => d.truncate(at),
                _ => {
                    let len = rnd(64).min(d.len() - at);
                    let piece = d[at..at + len].to_vec();
                    let to = rnd(d.len());
                    d.splice(to..to, piece);
                }
            }
        }
        total += 1;
        if read_all(d) {
            opened += 1;
        }
    }
    t.check(&format!("{total} mutated files read without a panic ({opened} opened)"), total == 1600 && opened > 400);
}

// ------------------------------------------------------------------ read_budget

/// a case of the budgets: done within 20 s (each took minutes to hours before, or many GB)
pub(crate) fn timed<R>(t: &mut T, name: &str, f: impl FnOnce() -> R) -> R {
    let t0 = plat::time_us();
    let r = f();
    let ms = plat::time_us().saturating_sub(t0) / 1000;
    t.check(&format!("{name}: within 20 s ({ms} ms)"), ms < 20_000);
    r
}

/// a cross-reference stream: rows of (type, field 2 of 4 bytes, field 3 of 2 bytes)
fn xref_rows(rows: &[(u8, u32, u16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows.len() * 7);
    for &(k, f2, f3) in rows {
        out.push(k);
        out.extend_from_slice(&f2.to_be_bytes());
        out.extend_from_slice(&f3.to_be_bytes());
    }
    out
}

/// an object stream: its objects (number, body) at the offsets its header gives, `pad` spaces
/// after them
fn objstm(objs: &[(u32, Vec<u8>)], pad: usize) -> Vec<u8> {
    let (mut head, mut data) = (String::new(), Vec::new());
    for (n, b) in objs {
        head.push_str(&format!("{n} {} ", data.len()));
        data.extend_from_slice(b);
        data.push(b'\n');
    }
    data.resize(data.len() + pad, b' ');
    flate(&format!("/Type /ObjStm /N {} /First {}", objs.len(), head.len()), &cat(&[head.as_bytes(), &data]))
}

/// the objects of `m` laid out, then a cross-reference stream of `rows` (its own row last, at
/// number `xnum`)
fn with_xref_stream(m: &Mk, rows: Vec<(u8, u32, u16)>, xnum: u32, trailer: &str) -> Vec<u8> {
    let (mut out, offs) = m.body();
    let x = out.len();
    let mut rows = rows;
    for r in rows.iter_mut() {
        if r.0 == 1 {
            r.1 = offs.get(&r.1).copied().unwrap_or(0) as u32;
        }
    }
    rows.push((1, x as u32, 0));
    let body = flate(&format!("/Type /XRef /W [1 4 2] /Size {} {trailer}", xnum + 1), &xref_rows(&rows));
    out.extend_from_slice(format!("{xnum} 0 obj\n").as_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(format!("\nendobj\nstartxref\n{x}\n%%EOF\n").as_bytes());
    out
}

fn read_budget(t: &mut T) {
    // a title of a million language escapes (each one was found and cut out of the string again)
    let mut title = String::from("<FEFF");
    for _ in 0..1_000_000 {
        title.push_str("001B001B");
    }
    title.push_str("0041>");
    let mut m = one_page(b"", None, "");
    m.add(6, format!("<< /Title {title} >>"));
    if let Some(doc) = open(t, "escapes: opens", m.table("/Root 1 0 R /Info 6 0 R")) {
        let got = timed(t, "a million language escapes", || doc.info().title);
        t.eq("language escapes are dropped", got.as_deref(), Some("A"));
    }

    // an inline image, then 300 000 candidates for its end, each with a hex string that runs on
    // to the last one (each was read to its end for each candidate)
    let content = cat(&[b"BI /W 1 /H 1 /BPC 8 /CS /G ID ", &b"\nEI <".repeat(300_000), b">x BT /F1 10 Tf 50 700 Td (after) Tj ET"]);
    if let Some(doc) = open(t, "EI candidates: opens", one_page(&content, None, "").table("/Root 1 0 R")) {
        let text = timed(t, "300 000 EI candidates", || doc.text(0).unwrap_or_default());
        t.check(&format!("the image ends at its first EI: {text:?}"), text.contains("after"));
    }

    // a form drawn a thousand times that draws a megabyte of white space a thousand times: no
    // operator in it, a terabyte to read
    let mut m = one_page(&b"/A Do ".repeat(1000), None, "/XObject << /A 20 0 R >>");
    m.add(20, flate("/Subtype /Form /Resources << /XObject << /B 21 0 R >> >>", &b"/B Do ".repeat(1000)));
    m.add(21, flate("/Subtype /Form", &vec![b' '; 1 << 20]));
    if let Some(doc) = open(t, "white space forms: opens", m.table("/Root 1 0 R")) {
        timed(t, "a megabyte of white space drawn a million times (text)", || doc.text(0).ok());
    }

    // a damaged file of strings inside strings: the repair read each object to the end of the file
    let n = 150_000;
    let mut d = b"%PDF-1.7\n".to_vec();
    for k in 1..=n {
        d.extend_from_slice(format!("{k} 0 obj (").as_bytes());
    }
    d.resize(d.len() + n, b')');
    let r = timed(t, "150 000 objects inside each other's strings, repaired", || Document::open(d).err());
    t.check("the strings inside strings are no document", r.is_some());

    // 4000 sections that each name one cross-reference stream of 16 MB (decoded for each, its 16
    // million free entries kept)
    let base = one_page(b"BT /F1 10 Tf 50 700 Td (ok) Tj ET", None, "").table("/Root 1 0 R");
    let mut prev = startxref(&base);
    let mut d = base;
    let s = d.len();
    d.extend_from_slice(b"99 0 obj\n");
    d.extend_from_slice(&flate("/Type /XRef /W [1 0 0] /Size 16777216", &vec![0u8; 16 << 20]));
    d.extend_from_slice(b"\nendobj\n");
    for _ in 0..4000 {
        let at = d.len();
        d.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {prev} /XRefStm {s} >>\n").as_bytes());
        prev = at;
    }
    d.extend_from_slice(format!("startxref\n{prev}\n%%EOF\n").as_bytes());
    let doc = timed(t, "4000 sections on one cross-reference stream", || Document::open(d));
    if let Some(doc) = t.ok("4000 sections: opens", doc.map_err(|e| format!("{e}"))) {
        t.check("4000 sections: not repaired, the text there", !doc.was_repaired() && doc.text(0).unwrap_or_default().contains("ok"));
    }

    // 400 000 pages in one object stream whose cross-reference names index 0 for each (each was
    // looked up through the whole index)
    let n = 400_000u32;
    let (stm, xnum) = (n + 3, n + 4);
    let kids: String = (3..n + 3).map(|k| format!("{k} 0 R ")).collect();
    let mut objs = vec![(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()), (2, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes())];
    for k in 3..n + 3 {
        objs.push((k, b"<< /Type /Page /Parent 2 0 R >>".to_vec()));
    }
    let mut m = Mk::new();
    m.add(stm, objstm(&objs, 0));
    let mut rows = vec![(0u8, 0u32, 0xFFFFu16)];
    rows.extend((1..n + 3).map(|_| (2, stm, 0)));
    rows.push((1, stm, 0));
    let d = with_xref_stream(&m, rows, xnum, "/Root 1 0 R");
    let doc = timed(t, "400 000 objects with a wrong index", || Document::open(d).map(|d| d.page_count()));
    t.eq("400 000 objects with a wrong index: the pages", doc.ok(), Some(n as usize));

    // a form field whose name of 100 KB 50 kids inherit: the names and values all fields give
    // stay within max_stream (a million kids were a terabyte)
    let big = "x".repeat(100_000);
    let kids: String = (11..61).map(|k| format!("{k} 0 R ")).collect();
    let mut m = one_page(b"", None, "");
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R] >> >>").add(10, format!("<< /T ({big}) /Kids [{kids}] >>"));
    for k in 11..61 {
        m.add(k, format!("<< /T (f{k}) /FT /Tx /Rect [0 0 1 1] >>"));
    }
    if let Some(doc) = t.ok("field names: opens", Document::open_with(m.table("/Root 1 0 R"), &Options { max_stream: 1 << 20, ..Options::default() }).map_err(|e| format!("{e}"))) {
        let f = doc.fields();
        let bytes: usize = f.iter().map(|x| x.name.len()).sum();
        t.check(&format!("field names: {} fields of {bytes} bytes, within max_stream", f.len()), !f.is_empty() && f.len() < 50 && bytes <= 1 << 20);
        t.check("field names: the first in order", f.first().is_some_and(|x| x.name.len() == 100_004 && x.name.ends_with(".f11")));
    }

    // 3000 pages inheriting a direct /Resources of 3000 entries (a copy for each page)
    let xo: String = (0..3000).map(|k| format!("/X{k} 99 0 R ")).collect();
    let kids: String = (3..3003).map(|k| format!("{k} 0 R ")).collect();
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>").add(2, format!("<< /Type /Pages /Kids [{kids}] /Count 3000 /Resources << /XObject << {xo} >> >> >>"));
    for k in 3..3003 {
        m.add(k, "<< /Type /Page /Parent 2 0 R >>");
    }
    if let Some(doc) = open(t, "inherited resources: opens", m.table("/Root 1 0 R")) {
        let last = doc.pages().last().and_then(|p| p.resources.get("XObject")).and_then(Object::as_dict).map(|d| d.len());
        t.eq("inherited resources: every page has them", (doc.page_count(), last), (3000, Some(3000)));
    }

    // object streams of 3 MB each with max_stream 4 MB: older ones are let go and read again
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>").add(2, "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>");
    let mut rows = vec![(0u8, 0u32, 0xFFFFu16), (1, 1, 0), (1, 2, 0)];
    for k in 0..3u32 {
        m.add(10 + k, objstm(&[(3 + k, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents {} 0 R /Resources << /Font << /F1 {} 0 R >> >> >>", 20 + k, 6 + k).into_bytes()), (6 + k, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec())], 3 << 20));
        m.add(20 + k, flate("", format!("BT /F1 10 Tf 50 700 Td (page {k}) Tj ET").as_bytes()));
    }
    // 3 4 5 in streams 10 11 12, 6 7 8 there too, 10 11 12 and 20 21 22 at offsets
    rows.extend([(2, 10, 0), (2, 11, 0), (2, 12, 0), (2, 10, 1), (2, 11, 1), (2, 12, 1), (0, 0, 0)]);
    rows.extend([(1, 10, 0), (1, 11, 0), (1, 12, 0)]);
    rows.extend((13..20).map(|_| (0u8, 0u32, 0u16)));
    rows.extend([(1, 20, 0), (1, 21, 0), (1, 22, 0)]);
    let d = with_xref_stream(&m, rows, 23, "/Root 1 0 R");
    if let Some(doc) = t.ok("object streams let go: opens", Document::open_with(d, &Options { max_stream: 4 << 20, ..Options::default() }).map_err(|e| format!("{e}"))) {
        let texts: Vec<String> = (0..3).map(|i| doc.text(i).unwrap_or_default()).collect();
        t.check(&format!("object streams let go and read again: {texts:?}"), (0..3).all(|i| texts[i].contains(&format!("page {i}"))));
    }
}
