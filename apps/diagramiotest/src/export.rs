// export -- the exports' tests (JD12): PDF read by pdftest's independent reader, SVG read back
// by the library's SVG reader and compared with the pixels, PNG and JPEG (golden images, round
// trips, transparency), the page selection and the shapes' links
//
//   run(h)                  group "export"
//   write_golden(dir)       --write-export-golden DIR: tests/golden/export-<name>.png anew (look
//                           at them first)
//   write_exports(dir, c)   --write-exports DIR [--corpus DIR]: every self-built drawing and the
//                           local files as PDF, SVG and PNG (to open in other readers, to
//                           compare with Visio; nothing of the local files is committed)
//
// The PDF checks parse every file with apps/pdftest/src/reader.rs (strict: xref, objects,
// streams, content grammar, resources, ToUnicode) and look for what the drawing must give:
// pages in the page's size, a bookmark per page, shadings and patterns for gradients and
// hatches, pictures (JPEG passed through), embedded fonts whose ToUnicode maps hold the texts,
// links to an address and to another page; the same document twice gives the same bytes.  The
// SVG of each drawing parses with lib/img_svg and draws (without text) close to the PNG export
// of the same page; its structure holds the gradients, patterns, pictures, links and shape data.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::render::Options;
use diagram_core::{Document, PageRef};
use diagram_io::vsdx;
use diagram_view::export::{self, pdf as epdf, raster, svg, LinkTarget, Pages};
use diagram_view::PageView;
use gfx::Surface;

use crate::pdfreader::{self, Obj, Pdf};
use crate::render::{find_up, fonts, png};
use crate::{drawings, out, H};

/// pixels per inch of the golden export images
const DPI: f64 = 48.0;
/// pixels per inch of the SVG comparison (0.75 pt lines two pixels wide: the rasterizers agree
/// on more than a hairline's coverage)
const SVG_DPI: f64 = 96.0;

fn load(bytes: &[u8]) -> Option<Document> {
    let mut lim = vsdx::Limits::VSDX;
    lim.zip.ratio = 2000;
    vsdx::read(bytes, &lim).ok().map(|l| l.doc)
}

/// the drawings of the export tests: render's and the links drawing
fn docs() -> Vec<(&'static str, Option<Document>)> {
    let mut v: Vec<(&'static str, Option<Document>)> = drawings::all().into_iter().map(|(n, p)| (n, load(&p.bytes()))).collect();
    v.push(("links", load(&drawings::links().bytes())));
    v
}

fn page_named(doc: &Document, name: &str) -> Option<PageRef> {
    doc.pages().find(|p| export::page_name(doc, *p) == name)
}

// ---------------------------------------------------------------- PDF through the reader

/// every dictionary of the file: the objects (streams: their dictionary) and the dictionaries
/// inside them, in arrays too (an annotation's action, an inline annotation)
fn dicts(pdf: &Pdf) -> Vec<&pdfreader::Dict> {
    fn walk<'a>(o: &'a Obj, out: &mut Vec<&'a pdfreader::Dict>, depth: u32) {
        if depth > 16 {
            return;
        }
        match o {
            Obj::Dict(d) | Obj::Stream(d, _) => {
                out.push(d);
                for (_, v) in &d.0 {
                    walk(v, out, depth + 1);
                }
            }
            Obj::Array(a) => {
                for v in a {
                    walk(v, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for o in pdf.objects.values() {
        walk(o, &mut out, 0);
    }
    out
}

fn int_of(pdf: &Pdf, d: &pdfreader::Dict, key: &str) -> Option<i64> {
    pdf.lookup(d, key).and_then(Obj::as_int)
}

/// the text every ToUnicode map of the file maps to
fn unicode_text(pdf: &Pdf) -> String {
    let mut all = String::new();
    for d in dicts(pdf) {
        let Some(tu) = d.get("ToUnicode") else { continue };
        let Ok(data) = pdf.stream_data(pdf.resolve(tu)) else { continue };
        if let Ok(m) = pdfreader::cmap_map(&data) {
            for s in m.values() {
                all.push_str(s);
            }
        }
    }
    all
}

fn pdf_checks(h: &mut H, name: &str, doc: &Document) {
    let pages = export::foreground_pages(doc);
    let mut fonts = fonts();
    let opts = epdf::PdfOptions { pages: pages.clone(), info: pdf::Info { title: Some(format!("Test {name}")), ..Default::default() }, ..Default::default() };
    let t0 = plat::time_us();
    let a = epdf::write(doc, &mut fonts, &opts);
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    let b = epdf::write(doc, &mut fonts, &opts);
    let Ok(bytes) = a else {
        h.check(&format!("export/pdf/{name}/write"), false, format!("{a:?}"));
        return;
    };
    h.check(&format!("export/pdf/{name}/write"), true, format!("{} bytes in {ms} ms", bytes.len()));
    h.check(&format!("export/pdf/{name}/deterministic"), b.as_ref().is_ok_and(|b| *b == bytes), String::from("the same bytes twice"));
    let pdf = match pdfreader::parse(&bytes) {
        Ok(p) => p,
        Err(e) => {
            h.check(&format!("export/pdf/{name}/parse"), false, e);
            return;
        }
    };
    let all = pdf.check_all();
    h.check(&format!("export/pdf/{name}/reader"), all.is_ok(), format!("{all:?}"));
    let pp = pdf.pages().unwrap_or_default();
    h.check(&format!("export/pdf/{name}/pages"), pp.len() == pages.len(), format!("{} PDF pages for {} pages", pp.len(), pages.len()));
    // the first page's MediaBox: the page in points
    if let (Some(&p0), Some(&page)) = (pp.first(), pages.first()) {
        let mut fonts = crate::render::fonts();
        let v = PageView::new(doc, page, &mut fonts, &Options::default());
        let (w, hh) = (v.drawing().width * 72.0, v.drawing().height * 72.0);
        let mb: Vec<f64> = pdf.inherited(p0, "MediaBox").and_then(Obj::as_array).map(|a| a.iter().filter_map(|o| pdf.resolve(o).as_num()).collect()).unwrap_or_default();
        let ok = mb.len() == 4 && mb[0].abs() < 1e-6 && mb[1].abs() < 1e-6 && (mb[2] - w).abs() < 0.01 && (mb[3] - hh).abs() < 0.01;
        h.check(&format!("export/pdf/{name}/mediabox"), ok, format!("{mb:?} for {w:.2} x {hh:.2} pt"));
    }
    // a bookmark per page, named after it
    let titles: Vec<String> = dicts(&pdf).iter().filter(|d| d.has("Parent") && d.has("Title")).filter_map(|d| pdf.lookup(d, "Title").and_then(Obj::as_str).map(|s| String::from_utf8_lossy(s).into_owned())).collect();
    let want: Vec<String> = pages.iter().map(|p| export::page_name(doc, *p)).collect();
    let mut sorted = titles.clone();
    sorted.sort();
    let mut want_sorted = want.clone();
    want_sorted.sort();
    h.check(&format!("export/pdf/{name}/bookmarks"), sorted == want_sorted, format!("{titles:?} (want {want:?})"));
    let text = unicode_text(&pdf);
    let ds = dicts(&pdf);
    let embedded = ds.iter().filter(|d| d.has("FontFile2") || d.has("FontFile3")).count();
    match name {
        "fills" => {
            let axial = ds.iter().filter(|d| int_of(&pdf, d, "ShadingType") == Some(2)).count();
            let radial = ds.iter().filter(|d| int_of(&pdf, d, "ShadingType") == Some(3)).count();
            let tiling = ds.iter().filter(|d| int_of(&pdf, d, "PatternType") == Some(1)).count();
            h.check(&format!("export/pdf/{name}/shadings"), axial >= 1 && radial >= 1, format!("{axial} axial, {radial} radial (the gradients; old patterns 25..40 may add more)"));
            // hatches: tiling patterns, or (lib/pdf's way) the area clipped and lines stroked in it
            let clips = pp.first().and_then(|p| pdf.page_contents(*p).ok()).and_then(|c| pdfreader::content_ops(&c).ok()).map_or(0, |ops| ops.iter().filter(|(_, op)| op == b"W" || op == b"W*").count());
            h.check(&format!("export/pdf/{name}/hatches"), tiling >= 6 || clips >= 6, format!("{tiling} tiling patterns, {clips} clips for six hatched boxes"));
            h.check(&format!("export/pdf/{name}/text"), text.contains("Schatten") || ["S", "c", "h", "a", "t", "e", "n"].iter().all(|c| text.contains(c)), format!("ToUnicode text {text:?}"));
        }
        "pictures" => {
            let images: Vec<&&pdfreader::Dict> = ds.iter().filter(|d| pdf.lookup(d, "Subtype").and_then(Obj::as_name) == Some(b"Image") && pdf.lookup(d, "ColorSpace").and_then(Obj::as_name) == Some(b"DeviceRGB")).collect();
            let dct = images.iter().filter(|d| pdf.lookup(d, "Filter").and_then(Obj::as_name) == Some(b"DCTDecode")).count();
            h.check(&format!("export/pdf/{name}/images"), images.len() >= 2 && dct == 1, format!("{} images, {dct} JPEG passed through (the PNG used three times is embedded once)", images.len()));
        }
        "text" => {
            let need = ["normal", "fett", "kursiv", "rot", "gross", "Serif", "durch", "Courier", "TxtAngle", "gedreht", "senkrecht", "Beschriftung", "zwei"];
            let chars: BTreeSet<char> = need.iter().flat_map(|w| w.chars()).collect();
            let missing: Vec<char> = chars.iter().copied().filter(|c| !text.contains(*c)).collect();
            h.check(&format!("export/pdf/{name}/fonts"), embedded >= 3, format!("{embedded} embedded font programs (sans, serif, mono)"));
            h.check(&format!("export/pdf/{name}/tounicode"), missing.is_empty(), format!("characters missing from ToUnicode: {missing:?}"));
        }
        "pages" => {
            h.check(&format!("export/pdf/{name}/background"), text.contains('H') && ["H", "i", "n", "t", "e", "r", "g", "u", "d"].iter().all(|c| text.contains(c)), format!("the background page's text on the pages: {text:?}"));
        }
        "links" => {
            let uris: Vec<String> = ds.iter().filter_map(|d| d.get("URI").and_then(|o| pdf.resolve(o).as_str())).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
            let gotos = ds.iter().filter(|d| d.type_is("Annot") && (d.has("Dest") || pdf.lookup(d, "A").and_then(Obj::as_dict).is_some_and(|a| a.has("D")))).count();
            h.check(&format!("export/pdf/{name}/uri"), uris == ["https://example.org/jfd"], format!("{uris:?}"));
            h.check(&format!("export/pdf/{name}/page-links"), gotos == 2, format!("{gotos} links to pages (Start -> Ziel, Ziel -> Start)"));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- SVG read back

/// the image's RGB smoothed over 3 x 3 pixels (two rasterizers place thin lines' coverage a
/// little differently; what must agree is where things are and their colours)
fn smooth(sf: &Surface) -> Vec<[i32; 3]> {
    let (w, h) = (sf.w.max(0) as usize, sf.h.max(0) as usize);
    let px = |x: usize, y: usize, k: usize| sf.px[y * sf.stride as usize + x * 4 + k] as i32;
    let mut out = alloc::vec![[0i32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0i32; 3];
            let mut n = 0;
            for yy in y.saturating_sub(1)..(y + 2).min(h) {
                for xx in x.saturating_sub(1)..(x + 2).min(w) {
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += px(xx, yy, k);
                    }
                    n += 1;
                }
            }
            out[y * w + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
    out
}

/// pixels whose smoothed colours differ by more than `far` in a channel, the pixels compared
/// (those outside the `masked` rectangles x0, y0, x1, y1)
fn diff_smooth(a: &Surface, b: &Surface, far: i32, masked: &[[i32; 4]]) -> Option<(u64, u64)> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let (sa, sb) = (smooth(a), smooth(b));
    let w = a.w.max(1) as usize;
    let (mut n, mut all) = (0u64, 0u64);
    for (i, (x, y)) in sa.iter().zip(&sb).enumerate() {
        let (px, py) = ((i % w) as i32, (i / w) as i32);
        if masked.iter().any(|m| px >= m[0] && px < m[2] && py >= m[1] && py < m[3]) {
            continue;
        }
        all += 1;
        if (0..3).any(|k| (x[k] - y[k]).abs() > far) {
            n += 1;
        }
    }
    Some((n, all))
}

/// (pixels differing by more than `far` in a channel, all pixels) of two images (alpha ignored,
/// PARGB taken as it is: the SVG has an opaque background)
fn diff(a: &Surface, b: &Surface, far: u32) -> Option<(u64, u64)> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let mut n = 0u64;
    for y in 0..a.h as usize {
        let (ra, rb) = (&a.px[y * a.stride as usize..], &b.px[y * b.stride as usize..]);
        for x in 0..a.w as usize {
            let m = (0..3).map(|k| (ra[x * 4 + k] as i32 - rb[x * 4 + k] as i32).unsigned_abs()).max().unwrap_or(0);
            if m > far {
                n += 1;
            }
        }
    }
    Some((n, a.w as u64 * a.h as u64))
}

fn svg_checks(h: &mut H, name: &str, doc: &Document) {
    let mut fonts = fonts();
    for (k, page) in export::foreground_pages(doc).into_iter().enumerate() {
        let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
        let so = svg::SvgOptions { shape_data: true, title: export::page_name(doc, page), ..Default::default() };
        let text = svg::write(doc, v.drawing(), &so);
        let tag = format!("export/svg/{name}-{}", k + 1);
        let parsed = img_svg::Svg::parse(text.as_bytes());
        let Ok(s) = parsed else {
            h.check(&format!("{tag}/parse"), false, format!("img_svg refuses it: {:?}", parsed.err()));
            continue;
        };
        h.check(&format!("{tag}/parse"), !s.truncated(), format!("{} elements, {} bytes", s.element_count(), text.len()));
        // the SVG drawn by img_svg (no text hook: texts left out) against the PNG export
        let ro = raster::RasterOptions { dpi: SVG_DPI, region: None, background: raster::Background::Page };
        let Some(px) = raster::render(&mut v, &ro) else { continue };
        let runs: usize = v.drawing().items.iter().map(|i| if let diagram_core::render::ItemKind::Text(t) = &i.kind { t.runs.len() } else { 0 }).sum();
        let texts = text.matches("<text ").count();
        h.check(&format!("{tag}/texts"), texts <= runs && (runs == 0) == (texts == 0), format!("{texts} <text> for {runs} runs (blank runs left out)"));
        // texts are left out of the comparison (img_svg draws them only through a text hook), so
        // are hatches (img_svg draws <pattern> as its fallback colour; the structure check below
        // looks for them) and the pictures' edges (two resamplers clamp outer pixels differently)
        let hgt = v.drawing().height;
        let to_px = |b: [f64; 4], m: f64| [((b[0] * SVG_DPI) - m) as i32, (((hgt - b[3]) * SVG_DPI) - m) as i32, ((b[2] * SVG_DPI) + m) as i32 + 1, (((hgt - b[1]) * SVG_DPI) + m) as i32 + 1];
        let hatch = |i: &diagram_core::render::Item| matches!(&i.kind, diagram_core::render::ItemKind::Fill { paint: diagram_core::render::Paint::Hatch { .. }, .. });
        let mut masked: Vec<[i32; 4]> = v.drawing().items.iter().filter(|i| i.role == diagram_core::render::Role::Text || hatch(i)).map(|i| to_px(i.bbox, 4.0)).collect();
        for it in &v.drawing().items {
            if let diagram_core::render::ItemKind::Image(_) = &it.kind {
                let r = to_px(it.bbox, 0.0);
                let e = 0.04 * SVG_DPI as f64;
                let e = e as i32;
                masked.extend([[r[0] - e, r[1] - e, r[2] + e, r[1] + e], [r[0] - e, r[3] - e, r[2] + e, r[3] + e], [r[0] - e, r[1] - e, r[0] + e, r[3] + e], [r[2] - e, r[1] - e, r[2] + e, r[3] + e]]);
            }
        }
        let mut o = img_svg::Opts::default();
        match s.rasterize(px.w, px.h, &mut o) {
            Ok(img) => {
                if plat::getenv("JD12_DUMP").is_some() {
                    plat::write_file(&format!("build/jd12/svg-{name}-{}.png", k + 1), &png(&img));
                    plat::write_file(&format!("build/jd12/px-{name}-{}.png", k + 1), &png(&px));
                    let mut d = Surface::new(px.w, px.h, gfx::FMT_XRGB);
                    for i in 0..d.px.len().min(img.px.len()).min(px.px.len()) {
                        d.px[i] = 255 - ((img.px[i] as i32 - px.px[i] as i32).unsigned_abs() * 4).min(255) as u8;
                    }
                    plat::write_file(&format!("build/jd12/diff-{name}-{}.png", k + 1), &png(&d));
                }
                let (far, all) = diff_smooth(&img, &px, 24, &masked).unwrap_or((u64::MAX, 1));
                let limit = all / 200;
                h.check(&format!("{tag}/pixels"), far <= limit, format!("{far} of {all} pixels off by more than 24 after smoothing (at most {limit}; texts, hatches and picture edges left out)"));
            }
            Err(e) => h.check(&format!("{tag}/pixels"), false, format!("img_svg rasterize: {e}")),
        }
        match name {
            "fills" => h.check(&format!("{tag}/paints"), text.contains("<linearGradient") && text.contains("<radialGradient") && text.matches("<pattern").count() >= 6 && text.contains("fill-opacity=\"0.502\""), String::from("linear and radial gradients, six hatch patterns, 50 % fills")),
            "pictures" => h.check(&format!("{tag}/images"), text.matches("data:image/png;base64,").count() == 4 && text.matches("data:image/jpeg;base64,").count() == 1 && text.matches("clip-path=").count() == 6, String::from("4 PNG and 1 JPEG pictures, each clipped, the EMF as a placeholder")),
            "links" if k == 0 => {
                let ok = text.contains("xlink:href=\"https://example.org/jfd\"") && text.contains("xlink:href=\"#page-2\"") && text.contains("data-prop-kosten=\"42\"") && text.contains("data-prop-ort=\"Halle &amp; Lager\"") && text.contains("xlink:title=\"Beispiel\"");
                h.check(&format!("{tag}/links"), ok, String::from("a link to the address and to page 2, shape data as attributes"));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- PNG, JPEG

/// a pixel (straight ARGB or XRGB) as 0xAARRGGBB
fn pixel(sf: &Surface, x: i32, y: i32) -> u32 {
    let i = (y * sf.stride + x * 4) as usize;
    let p = &sf.px[i..i + 4];
    (p[3] as u32) << 24 | (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32
}

fn raster_checks(h: &mut H, docs: &[(&'static str, Option<Document>)], golden: Option<&str>) {
    let mut fonts = fonts();
    for (name, doc) in docs {
        let Some(doc) = doc else { continue };
        if !["fills", "pictures", "text"].contains(name) {
            continue;
        }
        let page = export::foreground_pages(doc)[0];
        let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
        let ro = raster::RasterOptions { dpi: DPI, ..Default::default() };
        let Some(img) = raster::render(&mut v, &ro) else {
            h.check(&format!("export/png/{name}/render"), false, String::from("renders"));
            continue;
        };
        let (w, hh) = raster::size(&v, &ro);
        h.check(&format!("export/png/{name}/size"), img.w as u32 == w && img.h as u32 == hh && (w as f64 - v.drawing().width * DPI).abs() < 1.0, format!("{w} x {hh} at {DPI} dpi"));
        let Some(dir) = golden else { continue };
        let path = format!("{dir}/export-{name}.png");
        let want = plat::read_file(&path).and_then(|b| img_png::decode_pargb(&b));
        let what = match want.as_ref().and_then(|g| diff(&img, g, 16)) {
            Some((far, all)) => (far <= all / 1000, format!("{far} pixels off by more than 16 (at most {})", all / 1000)),
            None => (false, format!("{path}: missing or another size (write it with --write-export-golden)")),
        };
        h.check(&format!("export/png/{name}/golden"), what.0, what.1);
        // round trips: PNG (pHYs), JPEG (density, close to the pixels)
        let p = raster::png(&img, 150.0).unwrap_or_default();
        let back = img_png::decode_pargb(&p);
        let phys = p.windows(4).position(|w| w == b"pHYs").and_then(|k| p.get(k + 4..k + 13)).map(|d| (u32::from_be_bytes([d[0], d[1], d[2], d[3]]), d[8]));
        h.check(&format!("export/png/{name}/roundtrip"), back.as_ref().and_then(|b| diff(b, &img, 0)).is_some_and(|d| d.0 == 0) && phys == Some((5906, 1)), format!("decodes to the same pixels, pHYs {phys:?}"));
        let j = raster::jpeg(&img, 92, 150.0, 0xFFFFFF).unwrap_or_default();
        let jb = img_jpeg::dec::decode_fl_max(&j, 0, 30000, 30000).ok();
        let close = jb.as_ref().and_then(|b| diff(b, &img, 40)).is_some_and(|(far, all)| far <= all / 50);
        h.check(&format!("export/jpeg/{name}/roundtrip"), close && j.get(13) == Some(&1) && j.get(14..16) == Some(&[0, 150]), format!("{} bytes, close to the pixels, 150 dpi in JFIF", j.len()));
    }
    // transparency: fills at 48 dpi without the page's colour
    let Some((_, Some(doc))) = docs.iter().find(|(n, _)| *n == "fills") else { return };
    let page = export::foreground_pages(doc)[0];
    let mut v = PageView::new(doc, page, &mut fonts, &Options::default());
    let ro = raster::RasterOptions { dpi: DPI, region: None, background: raster::Background::Transparent };
    let Some(img) = raster::render(&mut v, &ro) else { return };
    let at = |x: f64, y: f64| pixel(&img, (x * DPI) as i32, ((6.0 - y) * DPI) as i32);
    let (corner, half, solid) = (at(0.1, 0.1), at(0.6, 5.2), at(7.0, 1.9));
    let ok = corner >> 24 == 0 && (half >> 24).abs_diff(128) <= 3 && (half >> 16) & 255 >= 250 && solid >> 24 == 255 && solid & 0xFF_FFFF == 0x2E75B6;
    h.check("export/png/transparent", ok, format!("corner {corner:08x}, 50 % red {half:08x}, opaque #2E75B6 {solid:08x}"));
    // a region of the page at another resolution
    let ro = raster::RasterOptions { dpi: 100.0, region: Some([0.4, 4.6, 1.4, 5.4]), background: raster::Background::Page };
    let part = raster::render(&mut v, &ro);
    h.check("export/png/region", part.as_ref().is_some_and(|p| p.w == 100 && p.h == 80), format!("{:?}", part.as_ref().map(|p| (p.w, p.h))));
}

// ---------------------------------------------------------------- pages, links

fn selection_checks(h: &mut H, docs: &[(&'static str, Option<Document>)]) {
    let Some((_, Some(doc))) = docs.iter().find(|(n, _)| *n == "pages") else { return };
    let names = |v: Vec<PageRef>| v.iter().map(|p| export::page_name(doc, *p)).collect::<Vec<_>>();
    let bg = page_named(doc, "Hintergrund-1");
    let got = (
        names(export::pages(doc, &Pages::All)),
        names(export::pages(doc, &Pages::Range { from: 2, to: 9 })),
        names(export::pages(doc, &Pages::Range { from: 3, to: 4 })),
        bg.map(|b| names(export::pages(doc, &Pages::Current(b)))),
    );
    let want = (alloc::vec![String::from("Seite-1"), String::from("Massstab")], alloc::vec![String::from("Massstab")], Vec::<String>::new(), Some(alloc::vec![String::from("Hintergrund-1")]));
    h.check("export/pages", got == want, format!("{got:?}"));
    let Some((_, Some(doc))) = docs.iter().find(|(n, _)| *n == "links") else { return };
    let Some(start) = page_named(doc, "Start") else { return };
    let mut fonts = fonts();
    let v = PageView::new(doc, start, &mut fonts, &Options::default());
    let links = export::links(doc, v.drawing());
    let ziel = page_named(doc, "Ziel");
    let ok = links.len() == 2
        && links.iter().any(|l| l.target == LinkTarget::Uri(String::from("https://example.org/jfd")) && l.description == "Beispiel" && (l.bbox[0] - 0.5).abs() < 0.02 && (l.bbox[3] - 2.5).abs() < 0.02)
        && links.iter().any(|l| Some(&l.target) == ziel.map(LinkTarget::Page).as_ref());
    h.check("export/links", ok, format!("{links:?}"));
    h.check("export/page-by-name", export::page_by_name(doc, "ziel,200") == ziel && export::page_by_name(doc, "Start/Sheet.1") == Some(start) && export::page_by_name(doc, "nirgends").is_none(), String::from("sub-addresses name pages"));
}

/// group "export"
pub fn run(h: &mut H) {
    img_core::io::register(img_core::IMGF_PNG, Some(&img_png::CODEC));
    img_core::io::register(img_core::IMGF_JPEG, Some(&img_jpeg::CODEC));
    let docs = docs();
    for (name, doc) in &docs {
        let Some(doc) = doc else {
            h.check(&format!("export/{name}/load"), false, String::from("the drawing loads"));
            continue;
        };
        pdf_checks(h, name, doc);
        svg_checks(h, name, doc);
    }
    let golden = find_up("apps/diagramiotest/tests/golden");
    raster_checks(h, &docs, golden.as_deref());
    selection_checks(h, &docs);
}

/// --write-export-golden DIR
pub fn write_golden(dir: &str) -> i32 {
    let mut fonts = fonts();
    for (name, doc) in docs() {
        let Some(doc) = doc else { continue };
        if !["fills", "pictures", "text"].contains(&name) {
            continue;
        }
        let page = export::foreground_pages(&doc)[0];
        let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
        let Some(img) = raster::render(&mut v, &raster::RasterOptions { dpi: DPI, ..Default::default() }) else { return 1 };
        let path = format!("{dir}/export-{name}.png");
        if !plat::write_file(&path, &png(&img)) {
            out(&format!("error: cannot write {path}\n"));
            return 1;
        }
        out(&format!("wrote {path}\n"));
    }
    0
}

/// --write-exports DIR [--corpus DIR]
pub fn write_exports(dir: &str, corpus: Option<String>) -> i32 {
    img_core::io::register(img_core::IMGF_PNG, Some(&img_png::CODEC));
    img_core::io::register(img_core::IMGF_JPEG, Some(&img_jpeg::CODEC));
    let mut list: Vec<(String, Document)> = docs().into_iter().filter_map(|(n, d)| d.map(|d| (String::from(n), d))).collect();
    if let Some(src) = corpus.or_else(|| find_up("apps/JFDiagrammer/context/files")) {
        for f in ["Zeichnung2.vsdx", "Zeichnung 3.vsdx"] {
            if let Some(d) = plat::read_file(&format!("{src}/{f}")).and_then(|b| load(&b)) {
                list.push((String::from(f.rsplit_once('.').map_or(f, |x| x.0)), d));
            }
        }
    } else {
        out("no local corpus: only the self-built drawings\n");
    }
    let mut fonts = fonts();
    let mut status = 0;
    for (stem, doc) in &list {
        let pages = export::foreground_pages(doc);
        let t0 = plat::time_us();
        match epdf::write(doc, &mut fonts, &epdf::PdfOptions { pages: pages.clone(), info: pdf::Info { title: Some(stem.clone()), creator: Some(String::from("diagramiotest")), ..Default::default() }, ..Default::default() }) {
            Ok(b) => {
                let path = format!("{dir}/{stem}.pdf");
                if plat::write_file(&path, &b) {
                    out(&format!("wrote {path} ({} pages, {} bytes, {} ms)\n", pages.len(), b.len(), plat::time_us().wrapping_sub(t0) / 1000));
                } else {
                    status = 1;
                }
            }
            Err(e) => {
                out(&format!("error: {stem}.pdf: {e}\n"));
                status = 1;
            }
        }
        for (k, page) in pages.iter().enumerate() {
            let mut v = PageView::new(doc, *page, &mut fonts, &Options::default());
            let svg_text = svg::write(doc, v.drawing(), &svg::SvgOptions { title: export::page_name(doc, *page), ..Default::default() });
            let svg_path = format!("{dir}/{stem}-{}.svg", k + 1);
            let png_path = format!("{dir}/{stem}-{}.png", k + 1);
            let img = raster::render(&mut v, &raster::RasterOptions { dpi: 150.0, ..Default::default() });
            let ok = plat::write_file(&svg_path, svg_text.as_bytes()) && img.as_ref().and_then(|i| raster::png(i, 150.0)).is_some_and(|p| plat::write_file(&png_path, &p));
            out(&format!("wrote {svg_path} and {png_path}{}\n", if ok { "" } else { " (FAILED)" }));
            if !ok {
                status = 1;
            }
        }
    }
    status
}
