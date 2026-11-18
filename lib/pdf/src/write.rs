// write -- a document as PDF 1.7 bytes: header, objects, a classic xref table, the trailer
//
// Objects in a fixed order: 1 catalog, 2 page tree, 3 document information, 4 XMP metadata,
// then per page its page object, content stream and link annotations, then the fonts the
// pages use (Type0, CIDFont, FontDescriptor, font program, ToUnicode), the images they use
// (with their soft masks), the shadings (gradients), the soft mask groups of gradients with
// varying alpha (form XObjects; their ExtGStates sit in the page resources like the alpha ones)
// and the outlines.  Fonts, images and shadings no page draws are left out.
// The file ID is a hash of the bytes before the xref table.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use img_core::deflate::{zlib_deflate_st, DeflateState};

use crate::image::{png_filter, Pixels};
use crate::obj::{self, num, Fmt};
use crate::{Date, Dest, Document, Error, LinkTarget};

struct W {
    o: Vec<u8>,
    offs: Vec<usize>,
    st: Box<DeflateState>,
    level: u8,
}

impl W {
    fn begin(&mut self, n: u32) {
        self.offs[n as usize] = self.o.len();
        let _ = write!(Fmt(&mut self.o), "{n} 0 obj\n");
    }

    fn end(&mut self) {
        self.o.extend_from_slice(b"\nendobj\n");
    }

    /// a dictionary object
    fn dict(&mut self, n: u32, body: &[u8]) {
        self.begin(n);
        self.o.extend_from_slice(b"<< ");
        self.o.extend_from_slice(body);
        self.o.extend_from_slice(b" >>");
        self.end();
    }

    /// a stream object of data as it is (dict holds its filter, if any)
    fn stream_raw(&mut self, n: u32, dict: &[u8], data: &[u8]) {
        self.begin(n);
        self.o.extend_from_slice(b"<< ");
        self.o.extend_from_slice(dict);
        if !dict.is_empty() {
            self.o.push(b' ');
        }
        let _ = write!(Fmt(&mut self.o), "/Length {} >>\nstream\n", data.len());
        self.o.extend_from_slice(data);
        self.o.extend_from_slice(b"\nendstream");
        self.end();
    }

    /// a stream object, FlateDecode unless the level is 0
    fn stream(&mut self, n: u32, dict: &[u8], data: &[u8]) {
        if self.level == 0 {
            self.stream_raw(n, dict, data);
            return;
        }
        let z = zlib_deflate_st(Some(&mut self.st), data, self.level as i32);
        let mut d = dict.to_vec();
        if !d.is_empty() {
            d.push(b' ');
        }
        d.extend_from_slice(b"/Filter /FlateDecode");
        self.stream_raw(n, &d, &z);
    }
}

/// page / font / image object numbers
struct Plan {
    page: Vec<u32>,
    content: Vec<u32>,
    annots: Vec<Vec<u32>>,
    fonts: BTreeMap<usize, u32>,
    images: BTreeMap<usize, (u32, Option<u32>)>,
    shadings: BTreeMap<usize, u32>,
    masks: BTreeMap<usize, u32>,
    outline_root: u32,
    outlines: Vec<u32>,
    size: u32,
}

const CATALOG: u32 = 1;
const PAGES: u32 = 2;
const INFO: u32 = 3;
const METADATA: u32 = 4;

pub(crate) fn write(doc: &Document) -> Result<Vec<u8>, Error> {
    if doc.pages.is_empty() {
        return Err(Error::Invalid("a document without pages"));
    }
    let check = |d: &Dest| -> Result<(), Error> {
        let ok = match *d {
            Dest::Fit(p) => (p.0 as usize) < doc.pages.len(),
            Dest::Xyz { page, left, top, zoom } => (page.0 as usize) < doc.pages.len() && left.is_finite() && top.is_finite() && zoom.is_none_or(|z| z.is_finite() && z > 0.0),
        };
        if ok { Ok(()) } else { Err(Error::Invalid("a destination on a page the document does not have")) }
    };
    for p in &doc.pages {
        for l in &p.links {
            if let LinkTarget::Dest(d) = &l.target {
                check(d)?;
            }
        }
    }
    for o in &doc.outlines {
        check(&o.dest)?;
    }
    // the font programs first: a damaged font fails before anything is written
    let used_fonts: BTreeSet<usize> = doc.pages.iter().flat_map(|p| p.fonts.iter().copied()).collect();
    let used_images: BTreeSet<usize> = doc.pages.iter().flat_map(|p| p.images.iter().copied()).collect();
    let used_masks: BTreeSet<usize> = doc.pages.iter().flat_map(|p| p.masks.iter().copied()).collect();
    let used_shadings: BTreeSet<usize> = doc.pages.iter().flat_map(|p| p.shadings.iter().copied()).chain(used_masks.iter().map(|&m| doc.masks[m].0)).collect();
    let mut programs = BTreeMap::new();
    for &f in &used_fonts {
        programs.insert(f, doc.fonts[f].program()?);
    }

    let mut next = 5u32;
    let mut take = |k: u32| {
        let n = next;
        next += k;
        n
    };
    let mut plan = Plan { page: vec![], content: vec![], annots: vec![], fonts: BTreeMap::new(), images: BTreeMap::new(), shadings: BTreeMap::new(), masks: BTreeMap::new(), outline_root: 0, outlines: vec![], size: 0 };
    for p in &doc.pages {
        plan.page.push(take(1));
        plan.content.push(take(1));
        plan.annots.push((0..p.links.len()).map(|_| take(1)).collect());
    }
    for &f in &used_fonts {
        plan.fonts.insert(f, take(5));
    }
    for &i in &used_images {
        let n = take(1);
        let m = doc.images[i].alpha.as_ref().map(|_| take(1));
        plan.images.insert(i, (n, m));
    }
    for &s in &used_shadings {
        plan.shadings.insert(s, take(1));
    }
    for &m in &used_masks {
        plan.masks.insert(m, take(1));
    }
    if !doc.outlines.is_empty() {
        plan.outline_root = take(1);
        plan.outlines = (0..doc.outlines.len()).map(|_| take(1)).collect();
    }
    plan.size = next;

    let mut w = W { o: Vec::new(), offs: vec![0; plan.size as usize], st: DeflateState::new(), level: doc.level };
    w.o.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // catalog
    let mut b = Vec::new();
    b.extend_from_slice(b"/Type /Catalog /Pages 2 0 R /Metadata 4 0 R");
    if !doc.outlines.is_empty() {
        b.extend_from_slice(b" /Outlines ");
        obj::r(&mut b, plan.outline_root);
        b.extend_from_slice(b" /PageMode /UseOutlines");
    }
    if let Some(l) = &doc.lang {
        b.extend_from_slice(b" /Lang ");
        obj::text(&mut b, l);
    }
    if doc.info.title.is_some() {
        b.extend_from_slice(b" /ViewerPreferences << /DisplayDocTitle true >>");
    }
    w.dict(CATALOG, &b);

    // page tree
    let mut b = Vec::new();
    b.extend_from_slice(b"/Type /Pages /Kids [");
    for (i, &n) in plan.page.iter().enumerate() {
        if i > 0 {
            b.push(b' ');
        }
        obj::r(&mut b, n);
    }
    let _ = write!(Fmt(&mut b), "] /Count {}", doc.pages.len());
    w.dict(PAGES, &b);

    // information and metadata
    w.dict(INFO, &info_dict(doc));
    let xmp = xmp(doc);
    w.stream_raw(METADATA, b"/Type /Metadata /Subtype /XML", &xmp);

    // pages
    for (i, p) in doc.pages.iter().enumerate() {
        let mut b = Vec::new();
        b.extend_from_slice(b"/Type /Page /Parent 2 0 R /MediaBox ");
        obj::nums(&mut b, &[0.0, 0.0, p.width, p.height]);
        b.extend_from_slice(b" /Resources <<");
        if !p.fonts.is_empty() {
            b.extend_from_slice(b" /Font <<");
            for &f in &p.fonts {
                let _ = write!(Fmt(&mut b), " /F{f} ");
                obj::r(&mut b, plan.fonts[&f]);
            }
            b.extend_from_slice(b" >>");
        }
        if !p.images.is_empty() {
            b.extend_from_slice(b" /XObject <<");
            for &im in &p.images {
                let _ = write!(Fmt(&mut b), " /Im{im} ");
                obj::r(&mut b, plan.images[&im].0);
            }
            b.extend_from_slice(b" >>");
        }
        if !p.gstates.is_empty() || !p.masks.is_empty() {
            b.extend_from_slice(b" /ExtGState <<");
            for &g in &p.gstates {
                let (fa, sa) = doc.alphas[g];
                let _ = write!(Fmt(&mut b), " /G{g} << /Type /ExtGState /ca ");
                num(&mut b, fa as f64 / 1000.0);
                b.extend_from_slice(b" /CA ");
                num(&mut b, sa as f64 / 1000.0);
                b.extend_from_slice(b" >>");
            }
            for &m in &p.masks {
                let _ = write!(Fmt(&mut b), " /M{m} << /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G ");
                obj::r(&mut b, plan.masks[&m]);
                b.extend_from_slice(b" >> >>");
            }
            b.extend_from_slice(b" >>");
        }
        if !p.shadings.is_empty() {
            b.extend_from_slice(b" /Shading <<");
            for &s in &p.shadings {
                let _ = write!(Fmt(&mut b), " /Sh{s} ");
                obj::r(&mut b, plan.shadings[&s]);
            }
            b.extend_from_slice(b" >>");
        }
        b.extend_from_slice(b" >> /Contents ");
        obj::r(&mut b, plan.content[i]);
        if !plan.annots[i].is_empty() {
            b.extend_from_slice(b" /Annots [");
            for (k, &n) in plan.annots[i].iter().enumerate() {
                if k > 0 {
                    b.push(b' ');
                }
                obj::r(&mut b, n);
            }
            b.push(b']');
        }
        if p.transparency {
            b.extend_from_slice(b" /Group << /Type /Group /S /Transparency /CS /DeviceRGB >>");
        }
        w.dict(plan.page[i], &b);
        w.stream(plan.content[i], b"", &p.content);
        for (k, l) in p.links.iter().enumerate() {
            let mut b = Vec::new();
            b.extend_from_slice(b"/Type /Annot /Subtype /Link /Rect ");
            obj::nums(&mut b, &l.rect);
            b.extend_from_slice(b" /Border [0 0 0] /F 4 /P ");
            obj::r(&mut b, plan.page[i]);
            match &l.target {
                LinkTarget::Uri(u) => {
                    b.extend_from_slice(b" /A << /Type /Action /S /URI /URI ");
                    obj::literal(&mut b, &obj::uri_bytes(u));
                    b.extend_from_slice(b" >>");
                }
                LinkTarget::Dest(d) => {
                    b.extend_from_slice(b" /Dest ");
                    dest(&mut b, d, &plan);
                }
            }
            w.dict(plan.annots[i][k], &b);
        }
    }

    // fonts
    for (&f, &n) in &plan.fonts {
        let fd = &doc.fonts[f];
        let base = fd.base_font();
        let mut b = Vec::new();
        b.extend_from_slice(b"/Type /Font /Subtype /Type0 /BaseFont ");
        obj::name(&mut b, &base);
        b.extend_from_slice(b" /Encoding /Identity-H /DescendantFonts [");
        obj::r(&mut b, n + 1);
        b.extend_from_slice(b"] /ToUnicode ");
        obj::r(&mut b, n + 4);
        w.dict(n, &b);

        let mut b = Vec::new();
        b.extend_from_slice(if fd.cff { b"/Type /Font /Subtype /CIDFontType0 /BaseFont " } else { b"/Type /Font /Subtype /CIDFontType2 /BaseFont " });
        obj::name(&mut b, &base);
        b.extend_from_slice(b" /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor ");
        obj::r(&mut b, n + 2);
        b.extend_from_slice(b" /W ");
        b.extend_from_slice(&fd.widths());
        if !fd.cff {
            b.extend_from_slice(b" /CIDToGIDMap /Identity");
        }
        w.dict(n + 1, &b);

        let mut b = Vec::new();
        fd.descriptor(&mut b);
        b.extend_from_slice(if fd.cff { b" /FontFile3 " } else { b" /FontFile2 " });
        obj::r(&mut b, n + 3);
        w.dict(n + 2, &b);

        let prog = &programs[&f];
        let dict = if fd.cff { Vec::from(&b"/Subtype /CIDFontType0C"[..]) } else {
            let mut d = Vec::new();
            let _ = write!(Fmt(&mut d), "/Length1 {}", prog.len());
            d
        };
        w.stream(n + 3, &dict, prog);
        w.stream(n + 4, b"", &fd.to_unicode());
    }

    // images
    for (&i, &(n, mask)) in &plan.images {
        let img = &doc.images[i];
        let mut b = Vec::new();
        let _ = write!(Fmt(&mut b), "/Type /XObject /Subtype /Image /Width {} /Height {} /BitsPerComponent 8", img.w, img.h);
        if img.interpolate {
            b.extend_from_slice(b" /Interpolate true");
        }
        if let Some(m) = mask {
            b.extend_from_slice(b" /SMask ");
            obj::r(&mut b, m);
        }
        match &img.px {
            Pixels::Jpeg { data, comps, invert, transform } => {
                b.extend_from_slice(match comps {
                    1 => b" /ColorSpace /DeviceGray",
                    3 => b" /ColorSpace /DeviceRGB",
                    _ => b" /ColorSpace /DeviceCMYK",
                });
                if *invert {
                    b.extend_from_slice(b" /Decode [1 0 1 0 1 0 1 0]");
                }
                b.extend_from_slice(b" /Filter /DCTDecode");
                if let Some(t) = transform {
                    let _ = write!(Fmt(&mut b), " /DecodeParms << /ColorTransform {t} >>");
                }
                w.stream_raw(n, &b, data);
            }
            Pixels::Raw { colors, data } => {
                b.extend_from_slice(if *colors == 1 { b" /ColorSpace /DeviceGray" } else { b" /ColorSpace /DeviceRGB" });
                samples(&mut w, n, b, data, img.w, *colors);
            }
        }
        if let (Some(m), Some(a)) = (mask, img.alpha.as_ref()) {
            let mut b = Vec::new();
            let _ = write!(Fmt(&mut b), "/Type /XObject /Subtype /Image /Width {} /Height {} /BitsPerComponent 8 /ColorSpace /DeviceGray", img.w, img.h);
            if img.interpolate {
                b.extend_from_slice(b" /Interpolate true");
            }
            samples(&mut w, m, b, a, img.w, 1);
        }
    }

    // shadings, and the soft mask groups painting the gray ones: their form space is the user
    // space at the time of the page's gs (no /Matrix), where the gray shading has its coordinates
    for (&s, &n) in &plan.shadings {
        w.dict(n, &doc.shadings[s]);
    }
    for (&m, &n) in &plan.masks {
        let (g, bbox) = doc.masks[m];
        let mut b = Vec::new();
        b.extend_from_slice(b"/Type /XObject /Subtype /Form /BBox ");
        obj::nums(&mut b, &bbox);
        b.extend_from_slice(b" /Group << /Type /Group /S /Transparency /CS /DeviceGray >> /Resources << /Shading <<");
        let _ = write!(Fmt(&mut b), " /Sh{g} ");
        obj::r(&mut b, plan.shadings[&g]);
        b.extend_from_slice(b" >> >>");
        let mut content = Vec::new();
        let _ = write!(Fmt(&mut content), "/Sh{g} sh\n");
        w.stream(n, &b, &content);
    }

    // outlines
    if !doc.outlines.is_empty() {
        let items = &doc.outlines;
        let n = items.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut top = Vec::new();
        for (i, it) in items.iter().enumerate() {
            match it.parent {
                Some(p) => children[p].push(i),
                None => top.push(i),
            }
        }
        // descendants shown when an item is open; children come after their parents
        let mut visible = vec![0i64; n];
        for i in (0..n).rev() {
            let v = 1 + if items[i].open { visible[i] } else { 0 };
            if let Some(p) = items[i].parent {
                visible[p] += v;
            }
        }
        let root_count: i64 = top.iter().map(|&t| 1 + if items[t].open { visible[t] } else { 0 }).sum();
        let mut b = Vec::new();
        b.extend_from_slice(b"/Type /Outlines /First ");
        obj::r(&mut b, plan.outlines[top[0]]);
        b.extend_from_slice(b" /Last ");
        obj::r(&mut b, plan.outlines[*top.last().unwrap_or(&top[0])]);
        let _ = write!(Fmt(&mut b), " /Count {root_count}");
        w.dict(plan.outline_root, &b);
        let siblings = |i: usize| -> &Vec<usize> {
            match items[i].parent {
                Some(p) => &children[p],
                None => &top,
            }
        };
        for (i, it) in items.iter().enumerate() {
            let mut b = Vec::new();
            b.extend_from_slice(b"/Title ");
            obj::text(&mut b, &it.title);
            b.extend_from_slice(b" /Parent ");
            obj::r(&mut b, it.parent.map_or(plan.outline_root, |p| plan.outlines[p]));
            let sib = siblings(i);
            let at = sib.iter().position(|&s| s == i).unwrap_or(0);
            if at > 0 {
                b.extend_from_slice(b" /Prev ");
                obj::r(&mut b, plan.outlines[sib[at - 1]]);
            }
            if at + 1 < sib.len() {
                b.extend_from_slice(b" /Next ");
                obj::r(&mut b, plan.outlines[sib[at + 1]]);
            }
            if let (Some(&f), Some(&l)) = (children[i].first(), children[i].last()) {
                b.extend_from_slice(b" /First ");
                obj::r(&mut b, plan.outlines[f]);
                b.extend_from_slice(b" /Last ");
                obj::r(&mut b, plan.outlines[l]);
                let c = if it.open { visible[i] } else { -visible[i] };
                let _ = write!(Fmt(&mut b), " /Count {c}");
            }
            b.extend_from_slice(b" /Dest ");
            dest(&mut b, &it.dest, &plan);
            w.dict(plan.outlines[i], &b);
        }
    }

    // xref and trailer
    let id_a = obj::hash64(1, &w.o);
    let id_b = obj::hash64(2, &w.o);
    let xref = w.o.len();
    let _ = write!(Fmt(&mut w.o), "xref\n0 {}\n0000000000 65535 f \n", plan.size);
    for n in 1..plan.size as usize {
        let _ = write!(Fmt(&mut w.o), "{:010} 00000 n \n", w.offs[n]);
    }
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&id_a.to_be_bytes());
    id[8..].copy_from_slice(&id_b.to_be_bytes());
    let _ = write!(Fmt(&mut w.o), "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R /ID [", plan.size);
    obj::hex(&mut w.o, &id);
    w.o.push(b' ');
    obj::hex(&mut w.o, &id);
    let _ = write!(Fmt(&mut w.o), "] >>\nstartxref\n{xref}\n%%EOF\n");
    Ok(w.o)
}

/// an image's or soft mask's samples: FlateDecode with PNG predictors (level 0: as they are)
fn samples(w: &mut W, n: u32, mut dict: Vec<u8>, data: &[u8], width: u32, colors: u8) {
    if w.level == 0 {
        w.stream_raw(n, &dict, data);
        return;
    }
    let filtered = png_filter(data, width as usize, colors as usize);
    let z = zlib_deflate_st(Some(&mut w.st), &filtered, w.level as i32);
    let _ = write!(Fmt(&mut dict), " /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {width} >>");
    w.stream_raw(n, &dict, &z);
}

fn dest(b: &mut Vec<u8>, d: &Dest, plan: &Plan) {
    b.push(b'[');
    obj::r(b, plan.page[d.page().0 as usize]);
    match *d {
        Dest::Fit(_) => b.extend_from_slice(b" /Fit]"),
        Dest::Xyz { left, top, zoom, .. } => {
            b.extend_from_slice(b" /XYZ ");
            num(b, left);
            b.push(b' ');
            num(b, top);
            b.push(b' ');
            match zoom {
                Some(z) => num(b, z),
                None => b.extend_from_slice(b"null"),
            }
            b.push(b']');
        }
    }
}

fn date_fields(d: &Date) -> (u16, u8, u8, u8, u8, u8, char, i16, i16) {
    let off = d.offset_minutes.clamp(-1439, 1439);
    let sign = if off < 0 { '-' } else { '+' };
    (d.year.min(9999), d.month.clamp(1, 12), d.day.clamp(1, 31), d.hour.min(23), d.minute.min(59), d.second.min(59), sign, off.abs() / 60, off.abs() % 60)
}

/// D:YYYYMMDDHHmmSS+HH'mm'
fn pdf_date(d: &Date) -> Vec<u8> {
    let (y, mo, da, h, mi, s, sign, oh, om) = date_fields(d);
    let mut b = Vec::new();
    let _ = write!(Fmt(&mut b), "D:{y:04}{mo:02}{da:02}{h:02}{mi:02}{s:02}{sign}{oh:02}'{om:02}'");
    b
}

/// YYYY-MM-DDThh:mm:ss+hh:mm
fn xmp_date(d: &Date) -> Vec<u8> {
    let (y, mo, da, h, mi, s, sign, oh, om) = date_fields(d);
    let mut b = Vec::new();
    let _ = write!(Fmt(&mut b), "{y:04}-{mo:02}-{da:02}T{h:02}:{mi:02}:{s:02}{sign}{oh:02}:{om:02}");
    b
}

fn producer(doc: &Document) -> &str {
    doc.info.producer.as_deref().unwrap_or("JFLib")
}

fn info_dict(doc: &Document) -> Vec<u8> {
    let i = &doc.info;
    let mut b = Vec::new();
    for (key, v) in [("Title", &i.title), ("Author", &i.author), ("Subject", &i.subject), ("Keywords", &i.keywords), ("Creator", &i.creator)] {
        if let Some(v) = v {
            let _ = write!(Fmt(&mut b), "/{key} ");
            obj::text(&mut b, v);
            b.push(b' ');
        }
    }
    b.extend_from_slice(b"/Producer ");
    obj::text(&mut b, producer(doc));
    for (key, v) in [("CreationDate", &i.created), ("ModDate", &i.modified)] {
        if let Some(d) = v {
            let _ = write!(Fmt(&mut b), " /{key} ");
            obj::literal(&mut b, &pdf_date(d));
        }
    }
    b
}

fn xml(b: &mut Vec<u8>, s: &str) {
    for c in s.chars() {
        match c {
            '&' => b.extend_from_slice(b"&amp;"),
            '<' => b.extend_from_slice(b"&lt;"),
            '>' => b.extend_from_slice(b"&gt;"),
            '"' => b.extend_from_slice(b"&quot;"),
            // XML 1.0 has no other control characters
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => {
                let mut buf = [0u8; 4];
                b.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

fn xmp(doc: &Document) -> Vec<u8> {
    let i = &doc.info;
    let mut b = Vec::new();
    b.extend_from_slice(b"<?xpacket begin=\"\xEF\xBB\xBF\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n<dc:format>application/pdf</dc:format>\n");
    let alt = |b: &mut Vec<u8>, tag: &str, v: &str| {
        let _ = write!(Fmt(b), "<{tag}><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        xml(b, v);
        let _ = write!(Fmt(b), "</rdf:li></rdf:Alt></{tag}>\n");
    };
    let simple = |b: &mut Vec<u8>, tag: &str, v: &[u8]| {
        let _ = write!(Fmt(b), "<{tag}>");
        b.extend_from_slice(v);
        let _ = write!(Fmt(b), "</{tag}>\n");
    };
    if let Some(t) = &i.title {
        alt(&mut b, "dc:title", t);
    }
    if let Some(a) = &i.author {
        b.extend_from_slice(b"<dc:creator><rdf:Seq><rdf:li>");
        xml(&mut b, a);
        b.extend_from_slice(b"</rdf:li></rdf:Seq></dc:creator>\n");
    }
    if let Some(s) = &i.subject {
        alt(&mut b, "dc:description", s);
    }
    if let Some(l) = &doc.lang {
        b.extend_from_slice(b"<dc:language><rdf:Bag><rdf:li>");
        xml(&mut b, l);
        b.extend_from_slice(b"</rdf:li></rdf:Bag></dc:language>\n");
    }
    if let Some(k) = &i.keywords {
        let mut v = Vec::new();
        xml(&mut v, k);
        simple(&mut b, "pdf:Keywords", &v);
    }
    let mut v = Vec::new();
    xml(&mut v, producer(doc));
    simple(&mut b, "pdf:Producer", &v);
    if let Some(c) = &i.creator {
        let mut v = Vec::new();
        xml(&mut v, c);
        simple(&mut b, "xmp:CreatorTool", &v);
    }
    if let Some(d) = &i.created {
        simple(&mut b, "xmp:CreateDate", &xmp_date(d));
    }
    if let Some(d) = &i.modified {
        simple(&mut b, "xmp:ModifyDate", &xmp_date(d));
        simple(&mut b, "xmp:MetadataDate", &xmp_date(d));
    }
    b.extend_from_slice(b"</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>");
    b
}
