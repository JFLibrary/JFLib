// ops::picture -- a picture file as a shape (RE-02 "Bild aus Datei", SP-09): its format, size
// and resolution read from its header, its bytes as a media part of the package, a Foreign
// shape that shows it
//
//   probe_picture(bytes) -> Option<PictureInfo>   PNG (IHDR, pHYs), JPEG (SOFn, JFIF density),
//                                       GIF (logical screen), BMP (DIB header, pixels per metre):
//                                       format, pixels, dots per inch (0: the file does not say)
//   info.size(per_inch, max) -> [w, h]  the size Visio gives it: pixels / dpi (96 when the file
//                                       does not say) in paper inches, times the drawing units
//                                       per paper inch, scaled down (proportionally) into `max`
//   new_picture(tx, eng, owner, bytes, format, centre, size) -> ShapeRef
//
// How Visio keeps a picture ([MS-VSDX] ForeignData, Foreign shapes): a shape of Type
// "Foreign" with a ForeignData element (ForeignType="Bitmap", CompressionType "PNG", "JPEG",
// "GIF" ...) whose Rel names a part of the package by an image relationship
// (/visio/media/imageN.ext; diagram_io::vsdx::write makes the relationship and the content type
// from Document::parts, the shape refers to the part by its name).  The picture fills the box
// ImgOffsetX/Y, ImgWidth/ImgHeight inside the shape, cut at the shape's box; Visio writes them
// as ImgOffsetX = ImgWidth*0, ImgOffsetY = ImgHeight*0, ImgWidth = Width*1, ImgHeight =
// Height*1, so the picture stretches with the shape.  The shape here also has a rectangle in
// Geometry (no fill) and LinePattern 0, so a border can be given later (Line format) without a
// line at first; Annahme: Visio's inserted pictures carry such a section too, their border is
// off until set.  A BMP is best converted to PNG by the caller (CompressionType "BMP" is
// written for it here, Annahme from [MS-VSDX]'s list of compression types).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Value};
use crate::document::{Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::names;
use crate::shape::{ForeignData, ForeignPayload, ShapeData, ShapeKind, StyleRefs};
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::strings::Sym;

/// the relationship type of a picture part
pub const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// the formats of picture files
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PictureFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
}

impl PictureFormat {
    /// the part name's extension
    pub fn extension(self) -> &'static str {
        match self {
            PictureFormat::Png => "png",
            PictureFormat::Jpeg => "jpeg",
            PictureFormat::Gif => "gif",
            PictureFormat::Bmp => "bmp",
        }
    }

    /// the part's content type
    pub fn content_type(self) -> &'static str {
        match self {
            PictureFormat::Png => "image/png",
            PictureFormat::Jpeg => "image/jpeg",
            PictureFormat::Gif => "image/gif",
            PictureFormat::Bmp => "image/bmp",
        }
    }

    /// ForeignData's CompressionType
    pub fn compression(self) -> &'static str {
        match self {
            PictureFormat::Png => "PNG",
            PictureFormat::Jpeg => "JPEG",
            PictureFormat::Gif => "GIF",
            PictureFormat::Bmp => "BMP",
        }
    }
}

/// what a picture file's header says
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct PictureInfo {
    pub format: PictureFormat,
    /// pixels
    pub w: u32,
    pub h: u32,
    /// dots per inch across and down (0: the file does not say)
    pub dpi: [f64; 2],
}

impl PictureInfo {
    /// the size of the picture's shape: its pixels at its resolution (96 dpi when the file does
    /// not say) in paper inches times `per_inch` (drawing units per paper inch), scaled down
    /// into `max` (width, height; 0: no bound) keeping its proportions
    pub fn size(&self, per_inch: f64, max: [f64; 2]) -> [f64; 2] {
        let dpi = |d: f64| if d.is_finite() && d >= 1.0 { d } else { 96.0 };
        let mut w = self.w as f64 / dpi(self.dpi[0]) * per_inch;
        let mut h = self.h as f64 / dpi(self.dpi[1]) * per_inch;
        for (k, lim) in max.iter().enumerate() {
            let cur = if k == 0 { w } else { h };
            if *lim > 0.0 && cur > *lim {
                let f = lim / cur;
                w *= f;
                h *= f;
            }
        }
        [w, h]
    }
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le32(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// the format, pixels and resolution of a picture file (None: no PNG, JPEG, GIF or BMP, or a
/// header that ends too soon or names no pixels)
pub fn probe_picture(b: &[u8]) -> Option<PictureInfo> {
    let info = if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        probe_png(b)
    } else if b.starts_with(&[0xFF, 0xD8]) {
        probe_jpeg(b)
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(PictureInfo { format: PictureFormat::Gif, w: le16(b, 6)?, h: le16(b, 8)?, dpi: [0.0; 2] })
    } else if b.starts_with(b"BM") {
        probe_bmp(b)
    } else {
        None
    }?;
    (info.w > 0 && info.h > 0).then_some(info)
}

fn probe_png(b: &[u8]) -> Option<PictureInfo> {
    if b.get(12..16)? != b"IHDR" {
        return None;
    }
    let (w, h) = (be32(b, 16)?, be32(b, 20)?);
    let mut dpi = [0.0; 2];
    // the chunks up to the pixels: pHYs in pixels per metre (unit 1)
    let mut at = 8;
    while let (Some(len), Some(kind)) = (be32(b, at), b.get(at + 4..at + 8)) {
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        if kind == b"pHYs" && len >= 9 && b.get(at + 16) == Some(&1) {
            dpi = [be32(b, at + 8)? as f64 * 0.0254, be32(b, at + 12)? as f64 * 0.0254];
        }
        at = at.checked_add(12 + len as usize)?;
    }
    Some(PictureInfo { format: PictureFormat::Png, w, h, dpi })
}

fn probe_jpeg(b: &[u8]) -> Option<PictureInfo> {
    let mut at = 2;
    let mut dpi = [0.0; 2];
    loop {
        // markers may be padded with FF bytes
        while b.get(at) == Some(&0xFF) && b.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *b.get(at)? != 0xFF {
            return None;
        }
        let m = *b.get(at + 1)?;
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            at += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return None;
        }
        let len = be16(b, at + 2)? as usize;
        let seg = at + 4;
        if m == 0xE0 && b.get(seg..seg + 5) == Some(b"JFIF\0") {
            // units 1: dots per inch, 2: dots per centimetre, 0: a ratio only
            let (unit, x, y) = (*b.get(seg + 7)?, be16(b, seg + 8)? as f64, be16(b, seg + 10)? as f64);
            dpi = match unit {
                1 => [x, y],
                2 => [x * 2.54, y * 2.54],
                _ => [0.0; 2],
            };
        }
        // SOF0 .. SOF15 except DHT (C4), JPG (C8), DAC (CC)
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            return Some(PictureInfo { format: PictureFormat::Jpeg, w: be16(b, seg + 3)?, h: be16(b, seg + 1)?, dpi });
        }
        at = seg.checked_add(len.checked_sub(2)?)?;
    }
}

fn probe_bmp(b: &[u8]) -> Option<PictureInfo> {
    let head = le32(b, 14)?;
    let (w, h, dpi) = if head == 12 {
        (le16(b, 18)? as i32, le16(b, 20)? as i32, [0.0; 2])
    } else {
        let (x, y) = if head >= 40 { (le32(b, 38)?, le32(b, 42)?) } else { (0, 0) };
        (le32(b, 18)?, le32(b, 22)?, [x.max(0) as f64 * 0.0254, y.max(0) as f64 * 0.0254])
    };
    // a negative height: rows top down
    Some(PictureInfo { format: PictureFormat::Bmp, w: w.unsigned_abs(), h: h.unsigned_abs(), dpi })
}

// the first free media part name of this extension: /visio/media/image1.png, image2 ...
fn free_name(tx: &Transaction, ext: &str) -> String {
    let taken: Vec<String> = tx.doc().parts.iter().map(|p| tx.doc().str(p.name).trim_start_matches('/').to_ascii_lowercase()).collect();
    let mut n = 1;
    loop {
        let name = format!("/visio/media/image{n}.{ext}");
        if !taken.contains(&name[1..].to_ascii_lowercase()) {
            return name;
        }
        n += 1;
    }
}

fn cform(tx: &mut Transaction, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (tx.intern(name), tx.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

/// a picture of `format` with these bytes as a Foreign shape of `size` (drawing units) around
/// `centre` on top of the owner's shapes; the bytes become a new media part -> the shape (an
/// empty size or no bytes: Invalid)
pub fn new_picture(tx: &mut Transaction, eng: &mut Engine, owner: Owner, bytes: &[u8], format: PictureFormat, centre: [f64; 2], size: [f64; 2]) -> Result<ShapeRef, EditError> {
    tx.doc().tree(owner).ok_or(EditError::NoPage)?;
    let [w, h] = size;
    if bytes.is_empty() || !(w > 1e-9 && h > 1e-9 && w.is_finite() && h.is_finite()) {
        return Err(EditError::Invalid);
    }
    let name = free_name(tx, format.extension());
    tx.add_part(&name, format.content_type(), REL_IMAGE, bytes.to_vec())?;
    let part = tx.intern(&name);
    let mut d = ShapeData::new(ShapeKind::Foreign);
    let st = tx.doc().settings.clone();
    let known = |id: Option<u32>| id.filter(|&i| tx.doc().style_by_id(i).is_some());
    d.header.styles = StyleRefs { line: known(st.default_line_style), fill: known(st.default_fill_style), text: known(st.default_text_style) };
    let num = |name: Sym, v: f64| Cell::constant(name, Value::Num(v), None);
    let mut cells = alloc::vec![
        num(names::PIN_X, centre[0]),
        num(names::PIN_Y, centre[1]),
        num(names::WIDTH, w),
        num(names::HEIGHT, h),
        cform(tx, "LocPinX", "Width*0.5", w * 0.5),
        cform(tx, "LocPinY", "Height*0.5", h * 0.5),
        num(names::ANGLE, 0.0),
        num(names::FLIP_X, 0.0),
        num(names::FLIP_Y, 0.0),
        cform(tx, "ImgOffsetX", "ImgWidth*0", 0.0),
        cform(tx, "ImgOffsetY", "ImgHeight*0", 0.0),
        cform(tx, "ImgWidth", "Width*1", w),
        cform(tx, "ImgHeight", "Height*1", h),
    ];
    cells.push(num(names::LINE_PATTERN, 0.0));
    d.sheet.cells = cells;
    let mut geo = Section::new(SecKey::one(names::GEOMETRY));
    geo.ix_written = true;
    for (n, v) in [(names::NO_FILL, 1.0), (names::NO_LINE, 0.0), (names::NO_SHOW, 0.0), (names::NO_SNAP, 0.0), (names::NO_QUICK_DRAG, 0.0)] {
        geo.cells.push(num(n, v));
    }
    for (i, (x, y)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)].into_iter().enumerate() {
        let mut r = Row::new(RowKey::Index(i as u32 + 1));
        r.kind = Some(if i == 0 { names::REL_MOVE_TO } else { names::REL_LINE_TO });
        r.cells = alloc::vec![num(names::X, x), num(names::Y, y)];
        geo.rows.push(r);
    }
    d.sheet.sections.push(geo);
    let (ft, fv, ct, cv) = (tx.intern("ForeignType"), tx.intern("Bitmap"), tx.intern("CompressionType"), tx.intern(format.compression()));
    d.foreign = Some(alloc::boxed::Box::new(ForeignData { attrs: alloc::vec![(ft, fv), (ct, cv)], payload: ForeignPayload::Part(part), extras: Default::default() }));
    let s = tx.add_shape_with(owner, None, d)?;
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocKind, Document, Page, PageHeader};

    // a PNG header: IHDR w x h, a pHYs of ppm pixels per metre
    fn png(w: u32, h: u32, ppm: Option<u32>) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend(13u32.to_be_bytes());
        b.extend(b"IHDR");
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        if let Some(p) = ppm {
            b.extend(9u32.to_be_bytes());
            b.extend(b"pHYs");
            b.extend(p.to_be_bytes());
            b.extend(p.to_be_bytes());
            b.extend([1, 0, 0, 0, 0]);
        }
        b.extend(0u32.to_be_bytes());
        b.extend(b"IEND");
        b
    }

    #[test]
    fn headers_tell_size_and_resolution() {
        let p = probe_picture(&png(300, 150, Some(5906))).unwrap();
        assert_eq!((p.format, p.w, p.h), (PictureFormat::Png, 300, 150));
        assert!((p.dpi[0] - 150.0).abs() < 0.1, "{:?}", p.dpi);
        assert_eq!(probe_picture(&png(4, 2, None)).unwrap().dpi, [0.0, 0.0]);
        // JPEG: JFIF at 72 dpi, then SOF0 640 x 480
        let mut j = alloc::vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        j.extend(b"JFIF\0");
        j.extend([1, 1, 1, 0, 72, 0, 72, 0, 0]);
        j.extend([0xFF, 0xC0, 0, 17, 8, 1, 0xE0, 2, 0x80, 3]);
        j.extend([0; 9]);
        let p = probe_picture(&j).unwrap();
        assert_eq!((p.format, p.w, p.h, p.dpi), (PictureFormat::Jpeg, 640, 480, [72.0, 72.0]));
        // GIF: the logical screen; BMP: a top-down DIB at 3780 pixels per metre (96 dpi)
        let mut g = b"GIF89a".to_vec();
        g.extend([10, 0, 20, 0, 0, 0, 0]);
        assert_eq!(probe_picture(&g).map(|p| (p.w, p.h)), Some((10, 20)));
        let mut bm = b"BM".to_vec();
        bm.extend([0; 12]);
        bm.extend(40i32.to_le_bytes());
        bm.extend(8i32.to_le_bytes());
        bm.extend((-6i32).to_le_bytes());
        bm.extend([1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        bm.extend(3780i32.to_le_bytes());
        bm.extend(3780i32.to_le_bytes());
        let p = probe_picture(&bm).unwrap();
        assert_eq!((p.format, p.w, p.h), (PictureFormat::Bmp, 8, 6));
        assert!((p.dpi[1] - 96.0).abs() < 0.1);
        // nothing known, a header that ends too soon, no pixels
        assert!(probe_picture(b"hello").is_none());
        assert!(probe_picture(&png(0, 5, None)).is_none());
        assert!(probe_picture(&j[..12]).is_none());
    }

    #[test]
    fn size_follows_the_resolution_and_fits() {
        let p = PictureInfo { format: PictureFormat::Png, w: 960, h: 480, dpi: [0.0; 2] };
        assert_eq!(p.size(1.0, [0.0, 0.0]), [10.0, 5.0]);
        // a page at 1:2 (two drawing units per paper inch), bounded to 8 wide
        assert_eq!(p.size(2.0, [8.0, 100.0]), [8.0, 4.0]);
        let q = PictureInfo { dpi: [192.0, 192.0], ..p };
        assert_eq!(q.size(1.0, [0.0, 2.0]), [4.0, 2.0]);
    }

    #[test]
    fn a_picture_is_a_foreign_shape_with_its_part() {
        let mut doc = Document::new(DocKind::Drawing);
        let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
        let owner = Owner::Page(page);
        let mut eng = Engine::new();
        let bytes = png(4, 2, None);
        let mut tx = doc.transaction("Bild");
        let s = new_picture(&mut tx, &mut eng, owner, &bytes, PictureFormat::Png, [3.0, 4.0], [2.0, 1.0]).unwrap();
        // a second one gets the next name
        let t = new_picture(&mut tx, &mut eng, owner, &bytes, PictureFormat::Png, [5.0, 4.0], [2.0, 1.0]).unwrap();
        let change = tx.commit().unwrap();
        assert_eq!(doc.parts.len(), 2);
        assert_eq!(doc.str(doc.parts[0].name), "/visio/media/image1.png");
        assert_eq!(doc.str(doc.parts[1].name), "/visio/media/image2.png");
        assert_eq!(doc.str(doc.parts[0].content_type), "image/png");
        assert_eq!(doc.parts[0].data, bytes);
        let sh = doc.shape(s).unwrap();
        assert_eq!(sh.header().kind, ShapeKind::Foreign);
        let fd = doc.tree(owner).unwrap().get(t.slot).unwrap().foreign.as_deref().unwrap();
        assert_eq!(fd.payload, ForeignPayload::Part(doc.sym("/visio/media/image2.png").unwrap()));
        let attr = |n: &str| fd.attrs.iter().find(|(k, _)| doc.str(*k) == n).map(|(_, v)| doc.str(*v));
        assert_eq!((attr("ForeignType"), attr("CompressionType")), (Some("Bitmap"), Some("PNG")));
        let get = |c: &str| doc.resolve(SheetRef::Shape(s), &crate::CellAddr::single(doc.sym(c).unwrap())).num().unwrap();
        assert_eq!((get("PinX"), get("Width"), get("ImgWidth"), get("ImgHeight"), get("LinePattern")), (3.0, 2.0, 2.0, 1.0, 0.0));
        // the display list draws it in the shape's box
        let d = crate::render::build(&doc, page, &mut crate::render::ApproxMeasure, &crate::render::Options::default());
        assert!(d.items.iter().any(|i| matches!(i.kind, crate::render::ItemKind::Image(_))));
        // undo takes the parts back
        let mut change = change;
        assert!(change.undo(&mut doc));
        assert!(doc.parts.is_empty() && doc.shape(s).is_none());
        assert!(change.redo(&mut doc));
        assert_eq!(doc.parts.len(), 2);
        // empty data or size
        let mut tx = doc.transaction("Bild");
        assert_eq!(new_picture(&mut tx, &mut eng, owner, &[], PictureFormat::Png, [0.0; 2], [1.0, 1.0]), Err(EditError::Invalid));
        assert_eq!(new_picture(&mut tx, &mut eng, owner, &bytes, PictureFormat::Png, [0.0; 2], [0.0, 1.0]), Err(EditError::Invalid));
    }
}
