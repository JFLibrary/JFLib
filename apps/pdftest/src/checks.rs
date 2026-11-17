// checks -- the check groups of pdftest: each writes documents with lib/pdf and reads them back
// with reader.rs (strict: header, xref offsets, objects, references, stream lengths, filters,
// content grammar and resource names, ToUnicode CMaps), then compares what it finds with what was
// drawn.  Font programs are checked by a small sfnt / CFF reader of this file, against the
// original font files.
//
//   sample     the sample document (sample.rs): structure, pages, determinism, information
//   graphics   paths, paint, state written only when it changes, transforms, clipping, alpha
//   images     JPEG passed through, RGBA with SMask, gray, surfaces, predictors undone
//   truetype   subsets: tables, checksums, kept and emptied glyphs, composites, widths
//   cff        CFF subsets (name-keyed and CID-keyed), CFF2 refused
//   text       glyph positions (kerning in TJ), y-down text, ToUnicode, fallback, modes
//   links      URI and destination annotations, rectangles of turned links
//   outlines   the outline tree: First / Last / Prev / Next / Parent / Count
//   errors     refusals and damaged input (never a panic)

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use pdf::{Dest, Document, Error, FillRule, Image, LineCap, LineJoin, Matrix, Origin, Rgb, TextMode};

use crate::reader::{self, Obj, Pdf};
use crate::sample;
use crate::T;

pub const GROUPS: &[(&str, fn(&mut T))] = &[
    ("sample", sample_doc),
    ("graphics", graphics),
    ("images", images),
    ("truetype", truetype),
    ("cff", cff),
    ("text", text),
    ("links", links),
    ("outlines", outlines),
    ("errors", errors),
];

const MINI_TTF: &[u8] = include_bytes!("../../fonttest/tests/fixtures/mini.ttf");
const MINI_TTC: &[u8] = include_bytes!("../../fonttest/tests/fixtures/mini.ttc");
const CFF2_OTF: &[u8] = include_bytes!("../../fonttest/tests/cff2/jfcff2.otf");

// ------------------------------------------------------------------ helpers

/// write, parse strictly, check everything
fn read_back(t: &mut T, name: &str, doc: &Document) -> Option<(Vec<u8>, Pdf)> {
    let bytes = t.ok(&format!("{name}: write"), doc.write().map_err(|e| format!("{e}")))?;
    let pdf = t.ok(&format!("{name}: parse"), reader::parse(&bytes))?;
    t.ok(&format!("{name}: check_all"), pdf.check_all())?;
    Some((bytes, pdf))
}

/// the operations of page i
fn ops(t: &mut T, pdf: &Pdf, page: usize) -> Vec<(Vec<Obj>, Vec<u8>)> {
    let Some(pages) = t.ok("pages", pdf.pages()) else { return vec![] };
    let Some(&p) = pages.get(page) else {
        t.check("page exists", false);
        return vec![];
    };
    let Some(data) = t.ok("page contents", pdf.page_contents(p)) else { return vec![] };
    t.ok("content ops", reader::content_ops(&data)).unwrap_or_default()
}

fn content(t: &mut T, pdf: &Pdf, page: usize) -> String {
    let pages = pdf.pages().unwrap_or_default();
    let data = pages.get(page).and_then(|&p| pdf.page_contents(p).ok()).unwrap_or_default();
    let s = String::from_utf8_lossy(&data).into_owned();
    t.check("content is text", !s.is_empty());
    s
}

fn count_op(ops: &[(Vec<Obj>, Vec<u8>)], op: &str) -> usize {
    ops.iter().filter(|(_, o)| o == op.as_bytes()).count()
}

fn nums(args: &[Obj]) -> Vec<f64> {
    args.iter().filter_map(Obj::as_num).collect()
}

fn page_dict<'a>(pdf: &'a Pdf, page: usize) -> Option<&'a reader::Dict> {
    let p = *pdf.pages().ok()?.get(page)?;
    pdf.get(p)?.as_dict()
}

/// the resolved value of a key of a dict
fn val<'a>(pdf: &'a Pdf, d: &'a reader::Dict, key: &str) -> Option<&'a Obj> {
    pdf.lookup(d, key)
}

fn name_is(o: Option<&Obj>, n: &str) -> bool {
    o.is_some_and(|o| o.is_name(n))
}

fn new_page(doc: &mut Document) -> pdf::PageId {
    doc.add_page(200.0, 100.0)
}

/// every font dictionary of subtype Type0 -> (Type0 dict, CIDFont dict, FontDescriptor)
fn type0_fonts(pdf: &Pdf) -> Vec<(&reader::Dict, &reader::Dict, &reader::Dict)> {
    let mut out = Vec::new();
    for o in pdf.objects.values() {
        let Some(d) = o.as_dict() else { continue };
        if !name_is(pdf.lookup(d, "Subtype"), "Type0") {
            continue;
        }
        let Some(cid) = pdf.lookup(d, "DescendantFonts").and_then(Obj::as_array).and_then(|a| a.first()).map(|o| pdf.resolve(o)).and_then(Obj::as_dict) else { continue };
        let Some(desc) = pdf.lookup(cid, "FontDescriptor").and_then(Obj::as_dict) else { continue };
        out.push((d, cid, desc));
    }
    out
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

/// the tables of an sfnt (face of a collection) -> tag -> bytes
fn sfnt_tables(d: &[u8], face: usize) -> Option<BTreeMap<[u8; 4], &[u8]>> {
    let base = if d.get(..4) == Some(b"ttcf") { u32_at(d, 12 + 4 * face)? as usize } else { 0 };
    let n = u16_at(d, base + 4)? as usize;
    let mut m = BTreeMap::new();
    for i in 0..n {
        let r = base + 12 + 16 * i;
        let tag: [u8; 4] = d.get(r..r + 4)?.try_into().ok()?;
        let off = u32_at(d, r + 8)? as usize;
        let len = u32_at(d, r + 12)? as usize;
        m.insert(tag, d.get(off..off + len)?);
    }
    Some(m)
}

fn sum32(d: &[u8]) -> u32 {
    let mut s = 0u32;
    for c in d.chunks(4) {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        s = s.wrapping_add(u32::from_be_bytes(b));
    }
    s
}

/// glyph g's bytes in glyf (through loca)
fn glyph_bytes<'a>(t: &BTreeMap<[u8; 4], &'a [u8]>, g: usize) -> Option<&'a [u8]> {
    let head = t.get(b"head")?;
    let loca = t.get(b"loca")?;
    let glyf = t.get(b"glyf")?;
    let long = u16_at(head, 50)? != 0;
    let at = |i: usize| -> Option<usize> { if long { u32_at(loca, 4 * i).map(|v| v as usize) } else { u16_at(loca, 2 * i).map(|v| v as usize * 2) } };
    let (s, e) = (at(g)?, at(g + 1)?);
    if s >= e {
        return Some(&[]);
    }
    glyf.get(s..e)
}

/// a gid's advance from hmtx
fn hmtx_adv(t: &BTreeMap<[u8; 4], &[u8]>, g: usize) -> Option<u16> {
    let nhm = u16_at(t.get(b"hhea")?, 34)? as usize;
    u16_at(t.get(b"hmtx")?, 4 * g.min(nhm - 1))
}

/// the W array as gid -> width
fn w_array(pdf: &Pdf, cid: &reader::Dict) -> BTreeMap<u32, f64> {
    let mut m = BTreeMap::new();
    let Some(w) = pdf.lookup(cid, "W").and_then(Obj::as_array) else { return m };
    let mut i = 0;
    while i < w.len() {
        let Some(first) = w[i].as_int() else { break };
        match w.get(i + 1) {
            Some(Obj::Array(ws)) => {
                for (k, v) in ws.iter().enumerate() {
                    m.insert(first as u32 + k as u32, v.as_num().unwrap_or(-1.0));
                }
                i += 2;
            }
            Some(o) if o.as_int().is_some() => {
                let last = o.as_int().unwrap_or(first);
                let v = w.get(i + 2).and_then(Obj::as_num).unwrap_or(-1.0);
                for g in first..=last {
                    m.insert(g as u32, v);
                }
                i += 3;
            }
            _ => break,
        }
    }
    m
}

/// the glyph ids shown by TJ / Tj on a page, in order
fn shown_glyphs(ops: &[(Vec<Obj>, Vec<u8>)]) -> Vec<u16> {
    let mut g = Vec::new();
    let mut push = |s: &[u8]| {
        for c in s.chunks_exact(2) {
            g.push(u16::from_be_bytes([c[0], c[1]]));
        }
    };
    for (args, op) in ops {
        match op.as_slice() {
            b"TJ" => {
                for a in args.first().and_then(Obj::as_array).unwrap_or(&[]) {
                    if let Some(s) = a.as_str() {
                        push(s);
                    }
                }
            }
            b"Tj" => {
                if let Some(s) = args.first().and_then(Obj::as_str) {
                    push(s);
                }
            }
            _ => {}
        }
    }
    g
}

// ------------------------------------------------------------------ sample

fn sample_doc(t: &mut T) {
    let Some(a) = t.ok("sample builds", sample::build().map_err(|e| format!("{e}"))) else { return };
    let b = sample::build().unwrap_or_default();
    t.check("same input, same bytes", a == b);
    let Some(pdf) = t.ok("parse", reader::parse(&a)) else { return };
    t.ok("check_all", pdf.check_all());
    t.eq("version", pdf.version, (1, 7));
    let pages = pdf.pages().unwrap_or_default();
    t.eq("pages", pages.len(), 4);
    let sizes: Vec<Vec<f64>> = (0..pages.len()).map(|i| page_dict(&pdf, i).and_then(|d| val(&pdf, d, "MediaBox")).and_then(Obj::as_array).map(nums).unwrap_or_default()).collect();
    t.check("A4 portrait", sizes[0] == [0.0, 0.0, 595.27559, 841.88976]);
    t.check("Letter landscape", sizes[2] == [0.0, 0.0, 792.0, 612.0]);
    t.check("100 mm square", sizes[3] == [0.0, 0.0, 283.46457, 283.46457]);
    t.check("trailer ID of two 16-byte strings", pdf.trailer.get("ID").and_then(Obj::as_array).is_some_and(|a| a.len() == 2 && a.iter().all(|s| s.as_str().is_some_and(|s| s.len() == 16))));
    let root = pdf.root().ok();
    t.check("catalog /Lang", root.and_then(|r| val(&pdf, r, "Lang")).and_then(Obj::as_str) == Some(b"de-DE"));
    t.check("catalog /PageMode /UseOutlines", name_is(root.and_then(|r| val(&pdf, r, "PageMode")), "UseOutlines"));
    t.check("DisplayDocTitle", root.and_then(|r| val(&pdf, r, "ViewerPreferences")).and_then(Obj::as_dict).and_then(|v| v.get("DisplayDocTitle")).and_then(Obj::as_bool) == Some(true));
    // information: the title has an en dash -> UTF-16BE with BOM
    let info = pdf.trailer.get("Info").map(|o| pdf.resolve(o)).and_then(Obj::as_dict);
    let title = info.and_then(|i| i.get("Title")).and_then(Obj::as_str).unwrap_or(&[]);
    let mut want = vec![0xFE, 0xFF];
    for u in "JFLib PDF – Beispiel".encode_utf16() {
        want.extend_from_slice(&u.to_be_bytes());
    }
    t.check("Info /Title in UTF-16BE", title == want.as_slice());
    t.check("Info /CreationDate", info.and_then(|i| i.get("CreationDate")).and_then(Obj::as_str) == Some(b"D:20261116120000+01'00'"));
    t.check("Info /Producer", info.and_then(|i| i.get("Producer")).and_then(Obj::as_str) == Some(b"JFLib"));
    let xmp = root.and_then(|r| r.get("Metadata")).map(|m| pdf.stream_data(m).unwrap_or_default()).unwrap_or_default();
    let xs = String::from_utf8(xmp.clone()).unwrap_or_default();
    t.check("XMP is UTF-8 with the title", xs.contains("<rdf:li xml:lang=\"x-default\">JFLib PDF – Beispiel</rdf:li>"));
    t.check("XMP dates", xs.contains("<xmp:CreateDate>2026-11-16T12:00:00+01:00</xmp:CreateDate>") && xs.contains("<xmp:ModifyDate>2026-11-16T12:30:05+01:00</xmp:ModifyDate>"));
    t.check("XMP uncompressed", root.and_then(|r| val(&pdf, r, "Metadata")).and_then(Obj::as_stream).is_some_and(|(d, _)| d.get("Filter").is_none()));
    // every content stream and font program compressed
    let flate = pdf.objects.values().filter_map(Obj::as_stream).filter(|(d, _)| name_is(d.get("Filter"), "FlateDecode")).count();
    t.check("streams compressed", flate >= 12);
    t.eq("Type0 fonts (5 used)", type0_fonts(&pdf).len(), 5);
    // the uncompressed variant reads the same
    let mut doc = sample::document().ok();
    if let Some(d) = doc.as_mut() {
        d.set_compression(0);
    }
    if let Some((raw, p2)) = doc.as_ref().and_then(|d| read_back(t, "level 0", d)) {
        t.check("level 0: no FlateDecode content", !raw.windows(12).any(|w| w == b"/FlateDecode") || p2.objects.values().filter_map(Obj::as_stream).all(|(d, _)| !name_is(d.get("Filter"), "FlateDecode")));
        t.check("level 0: larger", raw.len() > a.len());
    }
}

// ------------------------------------------------------------------ graphics

fn graphics(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    {
        let mut c = doc.canvas(p, Origin::BottomLeft);
        c.set_fill_color(Rgb::new(1.0, 0.0, 0.0));
        c.rect(10.0, 10.0, 20.0, 30.0);
        c.fill(FillRule::NonZero);
        c.rect(40.0, 10.0, 20.0, 30.0); // same colour: no second rg
        c.fill(FillRule::EvenOdd);
        c.set_stroke_color(Rgb::hex(0x00FF00));
        c.set_line_width(2.5);
        c.set_line_cap(LineCap::Round);
        c.set_line_join(LineJoin::Bevel);
        c.set_miter_limit(4.0);
        c.set_dash(&[3.0, 1.5], 0.5);
        c.move_to(0.1 + 0.2, -0.000_000_01);
        c.line_to(100.0, 1e7);
        c.curve_to(1.0, 2.0, 3.0, 4.0, 5.0, 6.0);
        c.close();
        c.stroke();
        c.set_fill_alpha(0.25);
        c.set_stroke_alpha(0.5);
        c.circle(50.0, 50.0, 10.0);
        c.fill_stroke(FillRule::NonZero);
        c.save();
        c.translate(10.0, 20.0);
        c.scale(2.0, 3.0);
        c.rect(0.0, 0.0, 1.0, 1.0);
        c.clip(FillRule::EvenOdd);
        c.set_fill_color(Rgb::BLUE_TEST);
        c.rect(0.0, 0.0, 5.0, 5.0);
        c.fill(FillRule::NonZero);
        c.restore();
        // after restore: red and alpha 0.25 are back, nothing new to write for them
        c.rect(1.0, 1.0, 1.0, 1.0);
        c.fill(FillRule::NonZero);
        // a path open while the transform changes stays where it was drawn
        c.move_to(10.0, 10.0);
        c.translate(5.0, 0.0);
        c.line_to(20.0, 10.0);
        c.stroke();
        // dash off again, invalid values ignored
        c.set_dash(&[-1.0, 2.0], 0.0);
        c.set_line_width(f64::NAN);
        c.move_to(0.0, 0.0);
        c.line_to(1.0, 1.0);
        c.stroke();
        c.save();
        c.save(); // left open: Drop closes it
    }
    let Some((_, pdf)) = read_back(t, "graphics", &doc) else { return };
    let o = ops(t, &pdf, 0);
    let s = content(t, &pdf, 0);
    t.eq("rg written once for red (and once for blue)", count_op(&o, "rg"), 2);
    t.check("re f then re f*", s.contains("10 10 20 30 re\nf\n") && s.contains("40 10 20 30 re\nf*\n"));
    t.check("stroke state", s.contains("0 1 0 RG\n2.5 w\n1 J\n2 j\n4 M\n[3 1.5] 0.5 d\n"));
    t.check("0.1 + 0.2 -> 0.3, -1e-8 -> 0", s.contains("0.3 0 m\n"));
    t.check("1e7 without exponent", s.contains("100 10000000 l\n"));
    t.check("curve and close", s.contains("1 2 3 4 5 6 c\nh\nS\n"));
    t.check("fill_stroke B", count_op(&o, "B") == 1);
    t.check("transform cm", s.contains("1 0 0 1 10 20 cm\n2 0 0 3 0 0 cm\n"));
    t.check("clip W* n", s.contains("0 0 1 1 re\nW* n\n"));
    t.check("open path kept on transform", s.contains("5 10 m\n20 10 l\nS\n"));
    t.check("dash reset", s.contains("[] 0 d\n"));
    let q = count_op(&o, "q");
    t.eq("q and Q balanced", count_op(&o, "Q"), q);
    t.check("gs for alpha", count_op(&o, "gs") == 1);
    // ExtGState values and the transparency group
    let pd = page_dict(&pdf, 0);
    let gs = pd.and_then(|d| val(&pdf, d, "Resources")).and_then(Obj::as_dict).and_then(|r| val(&pdf, r, "ExtGState")).and_then(Obj::as_dict);
    let g0 = gs.and_then(|g| g.0.first()).map(|(_, v)| pdf.resolve(v)).and_then(Obj::as_dict);
    t.check("ExtGState ca 0.25 CA 0.5", g0.is_some_and(|g| g.get("ca").and_then(Obj::as_num) == Some(0.25) && g.get("CA").and_then(Obj::as_num) == Some(0.5)));
    t.check("transparency group", pd.and_then(|d| val(&pdf, d, "Group")).and_then(Obj::as_dict).is_some_and(|g| name_is(g.get("S"), "Transparency")));

    // y-down canvas: the flip comes first, text and images stay upright
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.rect(10.0, 10.0, 5.0, 5.0);
        c.fill(FillRule::NonZero);
        let m = c.ctm();
        t.check("TopLeft ctm", m == Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, 100.0));
        let d = c.dest(10.0, 20.0);
        t.check("dest through the flip", d == Dest::Xyz { page: p, left: 10.0, top: 80.0, zoom: None });
    }
    let Some((_, pdf)) = read_back(t, "top-left", &doc) else { return };
    let s = content(t, &pdf, 0);
    t.check("flip first", s.starts_with("q\n1 0 0 -1 0 100 cm\n"));
    // an untouched page and an unused font: an empty content stream, no font object
    let mut doc = Document::new();
    new_page(&mut doc);
    let _ = doc.add_font(sample::SERIF.to_vec(), 0);
    if let Some((_, pdf)) = read_back(t, "empty page", &doc) {
        t.eq("no font written", type0_fonts(&pdf).len(), 0);
    }
}

trait TestColors {
    const BLUE_TEST: Rgb;
}
impl TestColors for Rgb {
    const BLUE_TEST: Rgb = Rgb { r: 0.0, g: 0.0, b: 1.0 };
}

// ------------------------------------------------------------------ images

fn images(t: &mut T) {
    let jpeg = sample::test_jpeg(48, 32);
    t.check("test JPEG encoded", jpeg.len() > 100);
    let rgba = sample::test_rgba(16, 12);
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let Some(ji) = t.ok("jpeg", Image::jpeg(jpeg.clone()).map_err(|e| format!("{e}"))) else { return };
    t.eq("jpeg size", ji.size(), (48, 32));
    let j = doc.add_image(ji.interpolate(true));
    let r = doc.add_image(Image::rgba(16, 12, &rgba).unwrap_or_else(|_| Image::gray(1, 1, &[0]).unwrap()));
    // opaque RGBA and gray RGB lose their alpha / colour
    let opaque: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    let o = doc.add_image(Image::rgba(16, 12, &opaque).unwrap_or_else(|_| Image::gray(1, 1, &[0]).unwrap()));
    let grayish: Vec<u8> = (0..16 * 12).flat_map(|i| [i as u8, i as u8, i as u8]).collect();
    let g = doc.add_image(Image::rgb(16, 12, &grayish).unwrap_or_else(|_| Image::gray(1, 1, &[0]).unwrap()));
    // a premultiplied surface: un-premultiplied on the way in
    let mut sf = gfx::Surface::new(2, 1, gfx::FMT_PARGB);
    sf.px[..8].copy_from_slice(&[0, 0, 64, 128, 10, 20, 30, 255]); // B G R A
    let Some(si) = t.ok("surface", Image::from_surface(&sf).map_err(|e| format!("{e}"))) else { return };
    let s = doc.add_image(si);
    let unused = doc.add_image(Image::gray(1, 1, &[7]).unwrap_or_else(|_| Image::gray(1, 1, &[0]).unwrap()));
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.image(j, 10.0, 10.0, 48.0, 32.0);
        c.image(r, 60.0, 10.0, 16.0, 12.0);
        c.image(o, 80.0, 10.0, 16.0, 12.0);
        c.image(g, 100.0, 10.0, 16.0, 12.0);
        c.image_transform(s, Matrix::new(20.0, 0.0, 0.0, 10.0, 120.0, 10.0));
        c.image(j, 150.0, 50.0, 24.0, 16.0);
        let _ = unused;
    }
    let Some((_, pdf)) = read_back(t, "images", &doc) else { return };
    let xo = page_dict(&pdf, 0).and_then(|d| val(&pdf, d, "Resources")).and_then(Obj::as_dict).and_then(|r| val(&pdf, r, "XObject")).and_then(Obj::as_dict).cloned().unwrap_or_default();
    t.eq("XObjects (unused left out)", xo.len(), 5);
    let img = |i: usize| xo.get(&format!("Im{i}")).map(|o| pdf.resolve(o)).and_then(Obj::as_stream);
    // JPEG: the bytes as they were
    if let Some((d, raw)) = img(0) {
        t.check("JPEG DCTDecode", name_is(d.get("Filter"), "DCTDecode"));
        t.check("JPEG bytes unchanged", raw == jpeg.as_slice());
        t.check("JPEG RGB, 48 x 32", name_is(d.get("ColorSpace"), "DeviceRGB") && d.get("Width").and_then(Obj::as_int) == Some(48) && d.get("Height").and_then(Obj::as_int) == Some(32));
        t.check("Interpolate", d.get("Interpolate").and_then(Obj::as_bool) == Some(true));
    } else {
        t.check("JPEG XObject", false);
    }
    // RGBA: RGB samples and an SMask with the alpha, predictors undone by the reader
    if let Some((d, _)) = img(1) {
        let data = pdf.stream_data(xo.get("Im1").unwrap_or(&Obj::Null)).unwrap_or_default();
        let want: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        t.check("RGBA: RGB samples", data == want);
        t.check("RGBA: predictor 15", d.get("DecodeParms").and_then(Obj::as_dict).and_then(|p| p.get("Predictor")).and_then(Obj::as_int) == Some(15));
        let sm = d.get("SMask").unwrap_or(&Obj::Null);
        let alpha = pdf.stream_data(sm).unwrap_or_default();
        t.check("RGBA: SMask alpha", alpha == rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<u8>>());
        t.check("SMask DeviceGray", pdf.resolve(sm).as_stream().is_some_and(|(m, _)| name_is(m.get("ColorSpace"), "DeviceGray")));
    } else {
        t.check("RGBA XObject", false);
    }
    t.check("opaque RGBA: no SMask", img(2).is_some_and(|(d, _)| d.get("SMask").is_none()));
    t.check("gray RGB as DeviceGray", img(3).is_some_and(|(d, _)| name_is(d.get("ColorSpace"), "DeviceGray")));
    let gdata = pdf.stream_data(xo.get("Im3").unwrap_or(&Obj::Null)).unwrap_or_default();
    t.check("gray samples", gdata == (0..16 * 12).map(|i| i as u8).collect::<Vec<u8>>());
    let sdata = pdf.stream_data(xo.get("Im4").unwrap_or(&Obj::Null)).unwrap_or_default();
    t.check("surface un-premultiplied", sdata == [128, 0, 0, 30, 20, 10]);
    t.check("page with SMask: transparency group", page_dict(&pdf, 0).and_then(|d| val(&pdf, d, "Group")).is_some());
    let s = content(t, &pdf, 0);
    t.check("upright image in a y-down canvas", s.contains("q\n48 0 0 -32 10 42 cm\n/Im0 Do\nQ\n"));
    // JPEG kinds
    t.check("not a JPEG", matches!(Image::jpeg(vec![0, 1, 2, 3]), Err(Error::Image(_))));
    let mut prog = jpeg.clone();
    if let Some(i) = prog.windows(2).position(|w| w == [0xFF, 0xC0]) {
        prog[i + 1] = 0xC2;
        t.check("progressive JPEG taken", Image::jpeg(prog.clone()).is_ok());
        prog[i + 1] = 0xC9;
        t.check("arithmetic JPEG refused", matches!(Image::jpeg(prog), Err(Error::Unsupported(_))));
    }
    t.check("size mismatch refused", matches!(Image::rgb(4, 4, &[0; 47]), Err(Error::Image(_))));
    t.check("zero size refused", Image::gray(0, 4, &[]).is_err());
    t.check("alpha of the wrong size refused", Image::gray(2, 2, &[0; 4]).and_then(|i| i.with_alpha(vec![0; 3])).is_err());
}

// ------------------------------------------------------------------ truetype

fn truetype(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let Some(serif) = t.ok("add DejaVu Serif", doc.add_font(sample::SERIF.to_vec(), 0).map_err(|e| format!("{e}"))) else { return };
    let Some(mini) = t.ok("add mini.ttf", doc.add_font(MINI_TTF.to_vec(), 0).map_err(|e| format!("{e}"))) else { return };
    let Some(face1) = t.ok("add mini.ttc face 1", doc.add_font(MINI_TTC.to_vec(), 1).map_err(|e| format!("{e}"))) else { return };
    t.check("face 2 of mini.ttc refused", matches!(doc.add_font(MINI_TTC.to_vec(), 2), Err(Error::Font(_))));
    let m = doc.font_metrics(serif);
    t.check("metrics", m.is_some_and(|m| m.units_per_em == 2048 && m.ascent > 1500 && m.descent < 0 && m.cap_height > 1000));
    let text = "Hallo Welt";
    let shaped = doc.shape(&[serif], text);
    // mini.ttf: every glyph, composites among them
    let mini_n = sfnt_tables(MINI_TTF, 0).and_then(|t| u16_at(t.get(b"maxp")?, 4)).unwrap_or(0);
    let all: Vec<pdf::Glyph> = (1..mini_n as u32).map(|g| pdf::Glyph { font: 0, id: g, advance: 500, x: 0, y: 0, cluster: 0, end: 0 }).collect();
    let comp_gid = composite_gid(MINI_TTF);
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        let w = c.text(serif, 12.0, 10.0, 20.0, text);
        t.check("advance = text_width", (w - c.document().text_width(&[serif], 12.0, text)).abs() < 1e-9);
        // only the composite of mini.ttf: its components must come along
        if let Some(g) = comp_gid {
            c.glyphs(&[mini], 12.0, 10.0, 40.0, &[pdf::Glyph { font: 0, id: g, advance: 600, x: 0, y: 0, cluster: 0, end: 0 }], "");
        }
        c.text(face1, 12.0, 10.0, 60.0, "AV");
    }
    let Some((_, pdf)) = read_back(t, "truetype", &doc) else { return };
    t.eq("three fonts", type0_fonts(&pdf).len(), 3);
    let o = ops(t, &pdf, 0);
    let shown = shown_glyphs(&o);
    // the page's resource names are the document's font indices: F0 serif, F1 mini, F2 face 1
    let res = page_dict(&pdf, 0).and_then(|d| val(&pdf, d, "Resources")).and_then(Obj::as_dict).and_then(|r| val(&pdf, r, "Font")).and_then(Obj::as_dict).cloned().unwrap_or_default();
    for (key, orig, face) in [("F0", sample::SERIF, 0usize), ("F1", MINI_TTF, 0), ("F2", MINI_TTC, 1)] {
        let Some(t0) = res.get(key).map(|o| pdf.resolve(o)).and_then(Obj::as_dict) else {
            t.check(&format!("{key}: a font"), false);
            continue;
        };
        let Some(cid) = pdf.lookup(t0, "DescendantFonts").and_then(Obj::as_array).and_then(|a| a.first()).map(|o| pdf.resolve(o)).and_then(Obj::as_dict) else {
            t.check(&format!("{key}: DescendantFonts"), false);
            continue;
        };
        let Some(desc) = pdf.lookup(cid, "FontDescriptor").and_then(Obj::as_dict) else {
            t.check(&format!("{key}: FontDescriptor"), false);
            continue;
        };
        let base = pdf.lookup(t0, "BaseFont").and_then(Obj::as_name).unwrap_or(&[]);
        t.check("BaseFont TAG+name", base.len() > 7 && base[..6].iter().all(|c| c.is_ascii_uppercase()) && base[6] == b'+');
        t.check("Identity-H", name_is(pdf.lookup(t0, "Encoding"), "Identity-H"));
        t.check("CIDFontType2", name_is(pdf.lookup(cid, "Subtype"), "CIDFontType2"));
        t.check("CIDToGIDMap Identity", name_is(pdf.lookup(cid, "CIDToGIDMap"), "Identity"));
        t.check("CIDSystemInfo Adobe Identity", pdf.lookup(cid, "CIDSystemInfo").and_then(Obj::as_dict).is_some_and(|s| s.get("Ordering").and_then(Obj::as_str) == Some(b"Identity")));
        t.check("descriptor FontName = BaseFont", pdf.lookup(desc, "FontName").and_then(Obj::as_name) == Some(base));
        t.check("symbolic flag", pdf.lookup(desc, "Flags").and_then(Obj::as_int).is_some_and(|f| f & 4 != 0));
        let Some(ff) = desc.get("FontFile2") else {
            t.check(&format!("{key}: FontFile2"), false);
            continue;
        };
        let prog = pdf.stream_data(ff).unwrap_or_default();
        t.check("Length1", pdf.resolve(ff).as_stream().and_then(|(d, _)| d.get("Length1")).and_then(Obj::as_int) == Some(prog.len() as i64));
        let name = format!("{key} {}", String::from_utf8_lossy(base));
        check_sfnt(t, &name, &prog, orig, face, &pdf, cid, &shown);
        if key == "F1" {
            // the composite's components are kept
            if let (Some(g), Some(tabs)) = (comp_gid, sfnt_tables(&prog, 0)) {
                let comps = components(glyph_bytes(&tabs, g as usize).unwrap_or(&[]));
                t.check("composite has components", !comps.is_empty());
                t.check("components kept", comps.iter().all(|&c| glyph_bytes(&tabs, c as usize).is_some_and(|b| !b.is_empty())));
            }
        }
    }
    let _ = (all, shaped);
}

/// the first composite glyph of a TrueType font
fn composite_gid(font: &[u8]) -> Option<u32> {
    let t = sfnt_tables(font, 0)?;
    let n = u16_at(t.get(b"maxp")?, 4)? as usize;
    (0..n).find(|&g| glyph_bytes(&t, g).is_some_and(|b| b.len() >= 10 && (u16_at(b, 0).unwrap_or(0) as i16) < 0)).map(|g| g as u32)
}

/// a composite glyph's component ids
fn components(g: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    if g.len() < 10 || (u16_at(g, 0).unwrap_or(0) as i16) >= 0 {
        return out;
    }
    let mut p = 10;
    while let (Some(flags), Some(gid)) = (u16_at(g, p), u16_at(g, p + 2)) {
        out.push(gid);
        p += 4 + if flags & 1 != 0 { 4 } else { 2 } + if flags & 8 != 0 { 2 } else if flags & 0x40 != 0 { 4 } else if flags & 0x80 != 0 { 8 } else { 0 };
        if flags & 0x20 == 0 {
            break;
        }
    }
    out
}

/// an embedded TrueType subset against its original
fn check_sfnt(t: &mut T, name: &str, prog: &[u8], orig: &[u8], face: usize, pdf: &Pdf, cid: &reader::Dict, shown: &[u16]) {
    let (Some(st), Some(ot)) = (sfnt_tables(prog, 0), sfnt_tables(orig, face)) else {
        t.check(&format!("{name}: sfnt tables"), false);
        return;
    };
    t.check(&format!("{name}: version 1.0"), u32_at(prog, 0) == Some(0x0001_0000));
    for tag in [b"head", b"hhea", b"maxp", b"hmtx", b"loca", b"glyf", b"post", b"cmap"] {
        t.check(&format!("{name}: has {}", String::from_utf8_lossy(tag)), st.contains_key(tag));
    }
    t.check(&format!("{name}: no layout tables"), !st.contains_key(b"GSUB") && !st.contains_key(b"GPOS") && !st.contains_key(b"kern"));
    // table checksums and the whole-file checksum
    let n = u16_at(prog, 4).unwrap_or(0) as usize;
    let mut sums_ok = true;
    let mut aligned = true;
    for i in 0..n {
        let r = 12 + 16 * i;
        let tag: [u8; 4] = prog[r..r + 4].try_into().unwrap_or([0; 4]);
        let off = u32_at(prog, r + 8).unwrap_or(0) as usize;
        let len = u32_at(prog, r + 12).unwrap_or(0) as usize;
        aligned &= off % 4 == 0;
        let mut data = prog.get(off..off + len).unwrap_or(&[]).to_vec();
        if &tag == b"head" && data.len() >= 12 {
            data[8..12].fill(0);
        }
        sums_ok &= sum32(&data) == u32_at(prog, r + 4).unwrap_or(1);
    }
    t.check(&format!("{name}: table checksums"), sums_ok);
    t.check(&format!("{name}: tables 4-aligned"), aligned);
    t.check(&format!("{name}: checkSumAdjustment"), sum32(prog) == 0xB1B0_AFBA);
    let glyphs = u16_at(ot.get(b"maxp").unwrap_or(&&[][..]), 4).unwrap_or(0) as usize;
    t.eq(&format!("{name}: glyph count kept"), u16_at(st.get(b"maxp").unwrap_or(&&[][..]), 4).unwrap_or(0) as usize, glyphs);
    // kept glyphs are byte for byte the original's, the others empty; .notdef always kept
    let widths = w_array(pdf, cid);
    let upem = u16_at(ot.get(b"head").unwrap_or(&&[][..]), 18).unwrap_or(1000) as f64;
    let mut kept = 0;
    let mut same = true;
    for g in 0..glyphs {
        let a = glyph_bytes(&st, g).unwrap_or(&[]);
        if a.is_empty() {
            continue;
        }
        kept += 1;
        let b = glyph_bytes(&ot, g).unwrap_or(&[]);
        same &= a.len() >= b.len() && &a[..b.len()] == b && a[b.len()..].iter().all(|&z| z == 0);
    }
    t.check(&format!("{name}: kept glyphs unchanged"), same);
    t.check(&format!("{name}: few glyphs kept"), kept < glyphs || glyphs < 40);
    // every glyph shown of this font is kept and has the original width in W
    let mut widths_ok = true;
    for (&g, &w) in &widths {
        let want = hmtx_adv(&ot, g as usize).unwrap_or(0) as f64 * 1000.0 / upem;
        widths_ok &= (w - want).abs() < 0.01;
    }
    t.check(&format!("{name}: W = hmtx"), widths_ok && !widths.is_empty());
    let mine: Vec<u16> = shown.iter().copied().filter(|g| widths.contains_key(&(*g as u32))).collect();
    t.check(&format!("{name}: shown glyphs kept"), mine.iter().all(|&g| glyph_bytes(&st, g as usize).is_some_and(|b| !b.is_empty() || glyph_bytes(&ot, g as usize).is_some_and(|o| o.is_empty()))));
}

// ------------------------------------------------------------------ cff

/// a CFF INDEX at `at` -> (items, end)
fn cff_index(d: &[u8], at: usize) -> Option<(Vec<&[u8]>, usize)> {
    let count = u16_at(d, at)? as usize;
    if count == 0 {
        return Some((Vec::new(), at + 2));
    }
    let os = *d.get(at + 2)? as usize;
    let rd = |i: usize| -> Option<usize> {
        let mut v = 0usize;
        for k in 0..os {
            v = v << 8 | *d.get(at + 3 + i * os + k)? as usize;
        }
        Some(v)
    };
    let base = at + 3 + (count + 1) * os - 1;
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        items.push(d.get(base + rd(i)?..base + rd(i + 1)?)?);
    }
    Some((items, base + rd(count)?))
}

/// a CFF DICT -> operator (12 x as 1200 + x) -> integer operands
fn cff_dict(d: &[u8]) -> BTreeMap<u32, Vec<i64>> {
    let mut m = BTreeMap::new();
    let mut ops: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < d.len() {
        let b = d[i] as i64;
        match b {
            32..=246 => {
                ops.push(b - 139);
                i += 1;
            }
            247..=250 if i + 1 < d.len() => {
                ops.push((b - 247) * 256 + d[i + 1] as i64 + 108);
                i += 2;
            }
            251..=254 if i + 1 < d.len() => {
                ops.push(-(b - 251) * 256 - d[i + 1] as i64 - 108);
                i += 2;
            }
            28 if i + 2 < d.len() => {
                ops.push(i16::from_be_bytes([d[i + 1], d[i + 2]]) as i64);
                i += 3;
            }
            29 if i + 4 < d.len() => {
                ops.push(i32::from_be_bytes([d[i + 1], d[i + 2], d[i + 3], d[i + 4]]) as i64);
                i += 5;
            }
            30 => {
                i += 1;
                while i < d.len() && d[i] & 0x0F != 0x0F && d[i] >> 4 != 0x0F {
                    i += 1;
                }
                i += 1;
                ops.push(0);
            }
            12 if i + 1 < d.len() => {
                m.insert(1200 + d[i + 1] as u32, core::mem::take(&mut ops));
                i += 2;
            }
            0..=21 => {
                m.insert(b as u32, core::mem::take(&mut ops));
                i += 1;
            }
            _ => break,
        }
    }
    m
}

/// a bare CFF -> its charstrings (CharStrings INDEX through the Top DICT)
fn cff_charstrings(d: &[u8]) -> Option<(Vec<&[u8]>, BTreeMap<u32, Vec<i64>>)> {
    let hdr = *d.get(2)? as usize;
    let (_, after_name) = cff_index(d, hdr)?;
    let (tops, _) = cff_index(d, after_name)?;
    let top = cff_dict(tops.first()?);
    let cs = *top.get(&17)?.first()? as usize;
    Some((cff_index(d, cs)?.0, top))
}

fn cff(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let Some(mini) = t.ok("add mini.otf", doc.add_font(sample::MINI_OTF.to_vec(), 0).map_err(|e| format!("{e}"))) else { return };
    let Some(cidf) = t.ok("add cid.otf", doc.add_font(sample::CID_OTF.to_vec(), 0).map_err(|e| format!("{e}"))) else { return };
    t.check("CFF2 refused", matches!(doc.add_font(CFF2_OTF.to_vec(), 0), Err(Error::Unsupported(_))));
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.text(mini, 20.0, 10.0, 30.0, "AVO");
        c.text(cidf, 20.0, 10.0, 60.0, "B");
    }
    let Some((_, pdf)) = read_back(t, "cff", &doc) else { return };
    t.eq("two CFF fonts", type0_fonts(&pdf).len(), 2);
    let res = page_dict(&pdf, 0).and_then(|d| val(&pdf, d, "Resources")).and_then(Obj::as_dict).and_then(|r| val(&pdf, r, "Font")).and_then(Obj::as_dict).cloned().unwrap_or_default();
    for (key, is_cid) in [("F0", false), ("F1", true)] {
        let t0 = res.get(key).map(|o| pdf.resolve(o)).and_then(Obj::as_dict);
        let cid = t0.and_then(|d| pdf.lookup(d, "DescendantFonts")).and_then(Obj::as_array).and_then(|a| a.first()).map(|o| pdf.resolve(o)).and_then(Obj::as_dict);
        let desc = cid.and_then(|c| pdf.lookup(c, "FontDescriptor")).and_then(Obj::as_dict);
        let (Some(t0), Some(cid), Some(desc)) = (t0, cid, desc) else {
            t.check(&format!("{key}: Type0 font"), false);
            continue;
        };
        let base = format!("{key} {}", String::from_utf8_lossy(pdf.lookup(t0, "BaseFont").and_then(Obj::as_name).unwrap_or(&[])));
        t.check(&format!("{base}: CIDFontType0"), name_is(pdf.lookup(cid, "Subtype"), "CIDFontType0"));
        t.check(&format!("{base}: no CIDToGIDMap"), cid.get("CIDToGIDMap").is_none());
        let Some(ff) = desc.get("FontFile3") else {
            t.check(&format!("{base}: FontFile3"), false);
            continue;
        };
        t.check(&format!("{base}: CIDFontType0C"), pdf.resolve(ff).as_stream().is_some_and(|(d, _)| name_is(d.get("Subtype"), "CIDFontType0C")));
        let prog = pdf.stream_data(ff).unwrap_or_default();
        let orig_font = if is_cid { sample::CID_OTF } else { sample::MINI_OTF };
        let orig = sfnt_tables(orig_font, 0).and_then(|t| t.get(b"CFF ").map(|c| c.to_vec())).unwrap_or_default();
        let (Some((new_cs, new_top)), Some((old_cs, old_top))) = (cff_charstrings(&prog), cff_charstrings(&orig)) else {
            t.check(&format!("{base}: CFF parses"), false);
            continue;
        };
        t.eq(&format!("{base}: glyph count kept"), new_cs.len(), old_cs.len());
        let used: Vec<usize> = if is_cid { vec![0, 2] } else { vec![0, 1, 2, 3] };
        let mut ok = true;
        for (g, cs) in new_cs.iter().enumerate() {
            ok &= if used.contains(&g) { *cs == old_cs[g] } else { *cs == [14u8] };
        }
        t.check(&format!("{base}: used charstrings kept, the rest endchar"), ok);
        t.check(&format!("{base}: ROS kept"), new_top.contains_key(&1230) == old_top.contains_key(&1230));
        t.check(&format!("{base}: no UniqueID/XUID/Encoding"), !new_top.contains_key(&13) && !new_top.contains_key(&14) && !new_top.contains_key(&16));
        let widths = w_array(&pdf, cid);
        let hm = sfnt_tables(orig_font, 0).unwrap_or_default();
        t.check(&format!("{base}: W = hmtx"), !widths.is_empty() && widths.iter().all(|(&g, &w)| (w - hmtx_adv(&hm, g as usize).unwrap_or(0) as f64).abs() < 0.01));
    }
    // kerning from GPOS lands in TJ: A V in mini.otf, -70 units
    let o = ops(t, &pdf, 0);
    let tj = o.iter().find(|(_, op)| op == b"TJ").map(|(a, _)| a.clone()).unwrap_or_default();
    let arr = tj.first().and_then(Obj::as_array).unwrap_or(&[]);
    let adj: Vec<f64> = arr.iter().filter_map(Obj::as_num).collect();
    t.check("GPOS kerning as a TJ number (70)", adj.contains(&70.0));
}

// ------------------------------------------------------------------ text

fn text(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let q = doc.add_page(200.0, 100.0);
    let Ok(f) = sample::fonts(&mut doc) else {
        t.check("fonts", false);
        return;
    };
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.text(f.serif, 10.0, 5.0, 20.0, "fi");
        c.text_fallback(&[f.inter, f.mono], 10.0, 5.0, 40.0, "ab سلام");
        c.set_text_mode(TextMode::Invisible);
        c.text(f.serif, 10.0, 5.0, 60.0, "x");
        c.set_text_mode(TextMode::Fill);
        t.eq("empty text", c.text(f.serif, 10.0, 5.0, 70.0, ""), 0.0);
        t.eq("zero size", c.text(f.serif, 0.0, 5.0, 70.0, "a"), 0.0);
    }
    {
        let mut c = doc.canvas(q, Origin::BottomLeft);
        c.text(f.serif, 10.0, 5.0, 20.0, "y");
    }
    let shaped = doc.shape(&[f.serif], "fi");
    t.eq("fi is one ligature glyph", shaped.len(), 1);
    let mixed = doc.shape(&[f.inter, f.mono], "ab سلام");
    t.check("fallback used for Arabic", mixed.iter().any(|g| g.font == 1) && mixed.iter().any(|g| g.font == 0));
    let Some((_, pdf)) = read_back(t, "text", &doc) else { return };
    let o = ops(t, &pdf, 0);
    let tms: Vec<Vec<f64>> = o.iter().filter(|(_, op)| op == b"Tm").map(|(a, _)| nums(a)).collect();
    t.check("y-down text: Tm flips back", tms.first().is_some_and(|m| m == &[1.0, 0.0, 0.0, -1.0, 5.0, 20.0]));
    let o2 = ops(t, &pdf, 1);
    t.check("y-up text: Tm upright", o2.iter().any(|(a, op)| op == b"Tm" && nums(a) == [1.0, 0.0, 0.0, 1.0, 5.0, 20.0]));
    let tfs: Vec<&[u8]> = o.iter().filter(|(_, op)| op == b"Tf").filter_map(|(a, _)| a.first().and_then(Obj::as_name)).collect();
    t.check("fallback switches fonts", tfs.contains(&&b"F1"[..]) && tfs.contains(&&b"F2"[..]));
    t.check("invisible: 3 Tr", o.iter().any(|(a, op)| op == b"Tr" && a.first().and_then(Obj::as_int) == Some(3)));
    // right-to-left: the glyphs in visual order (cluster offsets falling), the lam-alef ligature
    // standing for both letters in logical order
    let ar = doc.shape(&[f.mono], "سلام");
    t.check("Arabic in visual order", ar.windows(2).all(|w| w[1].cluster < w[0].cluster));
    t.check("lam-alef ligature", ar.len() == 3);
    // ToUnicode: the ligature glyph stands for "fi", Arabic letters for themselves
    let fonts = type0_fonts(&pdf);
    let mut maps: BTreeMap<String, BTreeMap<u32, String>> = BTreeMap::new();
    for (t0, _, _) in &fonts {
        let base = String::from_utf8_lossy(pdf.lookup(t0, "BaseFont").and_then(Obj::as_name).unwrap_or(&[])).into_owned();
        let data = t0.get("ToUnicode").map(|u| pdf.stream_data(u).unwrap_or_default()).unwrap_or_default();
        if let Some(m) = t.ok(&format!("{base}: ToUnicode"), reader::cmap_map(&data)) {
            maps.insert(base, m);
        }
    }
    let serif = maps.iter().find(|(k, _)| k.contains("DejaVuSerif")).map(|(_, v)| v.clone()).unwrap_or_default();
    t.check("ligature maps to fi", shaped.first().is_some_and(|g| serif.get(&g.id).map(String::as_str) == Some("fi")));
    let mono = maps.iter().find(|(k, _)| k.contains("Mono")).map(|(_, v)| v.clone()).unwrap_or_default();
    let arabic: String = mono.values().cloned().collect();
    t.check("Arabic letters in ToUnicode", ['س', 'ل', 'ا', 'م'].iter().all(|c| arabic.contains(*c)));
    let lig = doc.shape(&[f.mono], "سلام").into_iter().find(|g| g.end - g.cluster == 4);
    t.check("lam-alef maps to both letters, logical order", lig.is_some_and(|g| mono.get(&g.id).map(String::as_str) == Some("لا")));
}

// ------------------------------------------------------------------ links

fn links(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let q = doc.add_page(300.0, 300.0);
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.link_uri(10.0, 10.0, 50.0, 20.0, "https://example.org/a b?ü=1");
        c.link_dest(10.0, 40.0, 50.0, 20.0, Dest::Fit(q));
        c.translate(100.0, 50.0);
        c.rotate(core::f64::consts::FRAC_PI_2);
        c.link_dest(0.0, 0.0, 40.0, 10.0, Dest::Xyz { page: p, left: 1.0, top: 2.0, zoom: Some(1.5) });
    }
    let Some((_, pdf)) = read_back(t, "links", &doc) else { return };
    let pages = pdf.pages().unwrap_or_default();
    let annots: Vec<&reader::Dict> = page_dict(&pdf, 0).and_then(|d| val(&pdf, d, "Annots")).and_then(Obj::as_array).unwrap_or(&[]).iter().filter_map(|o| pdf.resolve(o).as_dict()).collect();
    t.eq("three annotations", annots.len(), 3);
    let rect = |d: &reader::Dict| pdf.lookup(d, "Rect").and_then(Obj::as_array).map(nums).unwrap_or_default();
    if annots.len() == 3 {
        t.check("Link subtype", annots.iter().all(|a| name_is(pdf.lookup(a, "Subtype"), "Link")));
        t.check("no border", annots.iter().all(|a| pdf.lookup(a, "Border").and_then(Obj::as_array).map(nums) == Some(vec![0.0, 0.0, 0.0])));
        t.check("URI rect in page space", rect(annots[0]) == [10.0, 70.0, 60.0, 90.0]);
        let uri = annots[0].get("A").and_then(Obj::as_dict).and_then(|a| a.get("URI")).and_then(Obj::as_str).unwrap_or(&[]);
        t.check("URI percent-encoded", uri == b"https://example.org/a%20b?%C3%BC=1");
        let d1 = pdf.lookup(annots[1], "Dest").and_then(Obj::as_array).unwrap_or(&[]);
        t.check("Dest to page 2 /Fit", d1.len() == 2 && d1[0].as_ref_id().map(|r| r.0) == pages.get(1).copied() && d1[1].is_name("Fit"));
        t.check("turned link: bounding box", rect(annots[2]).iter().zip([90.0, 10.0, 100.0, 50.0]).all(|(a, b)| (a - b).abs() < 1e-6));
        let d2 = pdf.lookup(annots[2], "Dest").and_then(Obj::as_array).unwrap_or(&[]);
        t.check("XYZ dest", d2.len() == 5 && d2[1].is_name("XYZ") && nums(&d2[2..]) == [1.0, 2.0, 1.5]);
        t.check("/P back to the page", annots.iter().all(|a| a.get("P").and_then(Obj::as_ref_id).map(|r| r.0) == pages.first().copied()));
    }
    // a destination on a page the document lacks
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    doc.canvas(p, Origin::TopLeft).link_dest(0.0, 0.0, 1.0, 1.0, Dest::Fit(pdf::PageId(5)));
    t.check("dest to a missing page refused", matches!(doc.write(), Err(Error::Invalid(_))));
}

// ------------------------------------------------------------------ outlines

fn outlines(t: &mut T) {
    let mut doc = Document::new();
    let p = new_page(&mut doc);
    let a = doc.add_outline(None, "A", Dest::Fit(p));
    let a1 = doc.add_outline(Some(a), "A1", Dest::Fit(p));
    doc.add_outline(Some(a1), "A1a", Dest::Fit(p));
    doc.add_outline(Some(a), "A2", Dest::Fit(p));
    let b = doc.add_outline(None, "B – ü", Dest::Fit(p));
    doc.add_outline(Some(b), "B1", Dest::Fit(p));
    doc.add_outline(Some(b), "B2", Dest::Fit(p));
    doc.set_outline_open(b, false);
    doc.set_outline_open(a1, false);
    let Some((_, pdf)) = read_back(t, "outlines", &doc) else { return };
    let root = pdf.root().ok().and_then(|r| val(&pdf, r, "Outlines")).and_then(Obj::as_dict);
    let Some(root) = root else {
        t.check("Outlines", false);
        return;
    };
    // visible: A, A1, A2 (A1 closed hides A1a), B (closed) -> 4
    t.eq("root Count", root.get("Count").and_then(Obj::as_int), Some(4));
    fn dict_of<'a>(pdf: &'a Pdf, o: Option<&'a Obj>) -> Option<&'a reader::Dict> {
        o.map(|o| pdf.resolve(o)).and_then(Obj::as_dict)
    }
    let get = |o| dict_of(&pdf, o);
    let first = get(root.get("First"));
    let title = |d: Option<&reader::Dict>| d.and_then(|d| d.get("Title")).and_then(Obj::as_str).map(|s| s.to_vec()).unwrap_or_default();
    t.check("First = A", title(first) == b"A");
    t.eq("A Count (open: A1, A2)", first.and_then(|d| d.get("Count")).and_then(Obj::as_int), Some(2));
    let a1d = get(first.and_then(|d| d.get("First")));
    t.check("A First = A1", title(a1d) == b"A1");
    t.eq("A1 closed: Count -1", a1d.and_then(|d| d.get("Count")).and_then(Obj::as_int), Some(-1));
    let a2d = get(a1d.and_then(|d| d.get("Next")));
    t.check("A1 Next = A2", title(a2d) == b"A2");
    t.check("A2 Prev = A1", get(a2d.and_then(|d| d.get("Prev"))).map(|d| title(Some(d))) == Some(b"A1".to_vec()));
    t.check("A2 has no Next", a2d.is_some_and(|d| d.get("Next").is_none()));
    let bd = get(first.and_then(|d| d.get("Next")));
    let mut want = vec![0xFE, 0xFF];
    for u in "B – ü".encode_utf16() {
        want.extend_from_slice(&u.to_be_bytes());
    }
    t.check("B title UTF-16BE", title(bd) == want);
    t.eq("B closed: Count -2", bd.and_then(|d| d.get("Count")).and_then(Obj::as_int), Some(-2));
    t.check("Last = B", title(get(root.get("Last"))) == want);
    t.check("B Parent = root", bd.and_then(|d| d.get("Parent")).and_then(Obj::as_ref_id) == pdf.root().ok().and_then(|r| r.get("Outlines")).and_then(Obj::as_ref_id));
}

// ------------------------------------------------------------------ errors

fn errors(t: &mut T) {
    t.check("no pages refused", matches!(Document::new().write(), Err(Error::Invalid(_))));
    t.check("not a font", matches!(Document::new().add_font(vec![1, 2, 3, 4, 5, 6], 0), Err(Error::Font(_))));
    // a font whose fsType says "restricted": a copy of mini.ttf with OS/2 fsType = 2
    let mut restricted = MINI_TTF.to_vec();
    if let Some(off) = table_offset(&restricted, b"OS/2") {
        restricted[off + 8..off + 10].copy_from_slice(&2u16.to_be_bytes());
        t.check("restricted font refused", matches!(Document::new().add_font(restricted.clone(), 0), Err(Error::FontLicense(_))));
        restricted[off + 8..off + 10].copy_from_slice(&0x0204u16.to_be_bytes()); // print + no subsetting
        let mut doc = Document::new();
        let p = new_page(&mut doc);
        if let Some(f) = t.ok("no-subsetting font taken", doc.add_font(restricted, 0).map_err(|e| format!("{e}"))) {
            doc.canvas(p, Origin::TopLeft).text(f, 10.0, 1.0, 20.0, "A");
            if let Some((_, pdf)) = read_back(t, "no subsetting", &doc) {
                let prog = type0_fonts(&pdf).first().and_then(|(_, _, d)| d.get("FontFile2")).map(|f| pdf.stream_data(f).unwrap_or_default()).unwrap_or_default();
                let (Some(st), Some(ot)) = (sfnt_tables(&prog, 0), sfnt_tables(MINI_TTF, 0)) else {
                    t.check("no-subsetting sfnt", false);
                    return;
                };
                let n = u16_at(ot.get(b"maxp").unwrap_or(&&[][..]), 4).unwrap_or(0) as usize;
                t.check("no subsetting: every glyph kept", (0..n).all(|g| glyph_bytes(&st, g).map(<[u8]>::len) >= glyph_bytes(&ot, g).map(<[u8]>::len)));
            }
        }
    }
    // damaged fonts: cut at every length, bytes flipped -- refused or taken, never a panic, and a
    // taken one still writes (or refuses) cleanly
    let mut taken = 0;
    for cut in (0..MINI_TTF.len()).step_by(7) {
        let mut doc = Document::new();
        if let Ok(f) = doc.add_font(MINI_TTF[..cut].to_vec(), 0) {
            taken += 1;
            let p = new_page(&mut doc);
            doc.canvas(p, Origin::TopLeft).text(f, 10.0, 1.0, 20.0, "AV");
            let _ = doc.write();
        }
    }
    for i in (0..sample::MINI_OTF.len()).step_by(3) {
        let mut d = sample::MINI_OTF.to_vec();
        d[i] ^= 0x5A;
        let mut doc = Document::new();
        if let Ok(f) = doc.add_font(d, 0) {
            let p = new_page(&mut doc);
            doc.canvas(p, Origin::TopLeft).text(f, 10.0, 1.0, 20.0, "AVO");
            let _ = doc.write();
        }
        let mut d = MINI_TTF.to_vec();
        if i < d.len() {
            d[i] ^= 0xA5;
            let mut doc = Document::new();
            if let Ok(f) = doc.add_font(d, 0) {
                let p = new_page(&mut doc);
                doc.canvas(p, Origin::TopLeft).text(f, 10.0, 1.0, 20.0, "AVOH");
                let _ = doc.write();
            }
        }
    }
    t.check("damaged fonts: no panic", true);
    t.check("a cut font is refused", taken < MINI_TTF.len() / 7);
    // damaged JPEGs
    let jpeg = sample::test_jpeg(16, 16);
    for cut in 0..jpeg.len().min(400) {
        let _ = Image::jpeg(jpeg[..cut].to_vec());
    }
    t.check("cut JPEGs: no panic", true);
    // a canvas for a page that does not exist panics by contract; values that make no sense
    // are ignored
    let mut doc = Document::new();
    let p = doc.add_page(f64::NAN, -5.0);
    t.check("page sides clamped", doc.page_size(p) == Some((pdf::A4.0, pdf::PAGE_MIN)));
    {
        let mut c = doc.canvas(p, Origin::TopLeft);
        c.transform(Matrix::new(0.0, 0.0, 0.0, 0.0, 1.0, 1.0)); // singular: ignored
        c.image(pdf::ImageId::default_test(), 0.0, 0.0, 1.0, 1.0);
        c.restore(); // nothing saved: ignored
        c.set_fill_alpha(f64::INFINITY);
    }
    if let Some((_, pdf)) = read_back(t, "nonsense ignored", &doc) {
        let o = ops(t, &pdf, 0);
        t.eq("only the flip written as cm", count_op(&o, "cm"), 1);
    }
}

fn table_offset(d: &[u8], tag: &[u8; 4]) -> Option<usize> {
    let n = u16_at(d, 4)? as usize;
    (0..n).find(|i| d.get(12 + 16 * i..16 + 16 * i) == Some(&tag[..])).and_then(|i| u32_at(d, 12 + 16 * i + 8)).map(|v| v as usize)
}

trait ImageIdTest {
    fn default_test() -> Self;
}
impl ImageIdTest for pdf::ImageId {
    /// an id no document handed out (the API has no constructor; a fresh document's first image)
    fn default_test() -> pdf::ImageId {
        let mut d = Document::new();
        d.add_image(Image::gray(1, 1, &[0]).unwrap_or_else(|_| unreachable!()))
    }
}
