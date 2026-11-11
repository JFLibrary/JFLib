// print -- printing, the OS-independent part: paper sizes, page layout, rows over white, the
// job as a one-page PDF  [src/plat/print.s, src/img/pdfwrite.s; docs/modules/PRINT.md]
//
//   print(job) -> 0 printed / ERR_CANCEL / ERR_UNSUPPORTED / ERR_IO / ERR_NOMEM / ERR_DIMS
//        the OS part: win_print.rs (PrintDlgExW + GDI), linux_print.rs (portal Print + PDF)
//   page_init(paper, flags) -> PPage         the paper PAPER_* in pt (PJF_LANDSCAPE swaps
//                                            width / height), no unprintable edge
//   layout(job, page) -> Place               the image rectangle and the printable area on
//                                            the page (pt, from the top-left)
//   row_white(dst, src, n, mode)             n pixels (B, G, R, A) -> n * 3 bytes composited
//                                            over white (PRW_* mode bits)
//   print_pdf(job, page) -> PDF bytes / ERR_*  the job laid out on page as a one-page PDF
//                                            (job.page_w / page_h set)
//   print_dpi(unit, dpu) -> f64              pixels per inch from a resolution in unit (0 none
//                                            -> 96, 1 inch, 2 cm)
//   pdf_write_image(src, opt) -> PDF bytes / ERR_*   one image on one PDF page (pdfwrite.s)
//   set_print_deflate(f)                     the zlib compressor of the PDF streams
//   PrintDoc (E9): a job of several pages
//        PrintDoc::begin(&job, pages) -> Result<PrintDoc, ERR_*>   the system's print dialog
//            (printer, paper, orientation, copies; a page range when pages > 0) where there
//            is one; job gives the title, paper, PJF_LANDSCAPE, margin, scale, dpi (its src
//            is not used)
//        d.page() -> PPage, d.copies(), d.wants(n)   the chosen page in pt, copies, whether
//            page n (from 1) is in the chosen range
//        d.add_image(&src) -> 0 / ERR_*   one page: the image laid out as layout() places it
//        d.pdf_native(), d.add_pdf(bytes)  the platform prints a PDF as it is (Linux)
//        d.end() -> 0 printed / ERR_*, d.abort()
//        Windows: PrintDlgExW + GDI, a StartPage per page (win_print.rs); Linux: the
//        portal's dialog, the pages as one PDF of several pages for Print (linux_print.rs);
//        test variant everywhere: jfgui_PRINT_TO=FILE (Windows: through the printer
//        jfgui_PRINT_PRINTER; elsewhere our PDF into FILE); else ERR_UNSUPPORTED
//
// Deviation (plat-shared): plat must not depend on the image crates, so pdfwrite.s is ported
// here and the one thing it took from img -- zlib_deflate -- comes through a hook.  An app
// sets it to img_core::deflate::zlib_deflate (then the PDF is byte-identical to the
// assembly's); without a hook the streams are zlib "stored" blocks (a valid, uncompressed
// PDF).  The source is a PrintSrc (rows of B, G, R, A bytes; the gfx::Surface fields)
// instead of a SURFACE pointer.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use rt::fx::{cvtd64, maxsd, minsd};

// ---- results (the ERR_* of jfgui.inc / plugin.inc that plat returns)
pub const ERR_NOMEM: i32 = -8;
pub const ERR_UNSUPPORTED: i32 = -9;
pub const ERR_DIMS: i32 = -12;
pub const ERR_IO: i32 = -13;
pub const ERR_CANCEL: i32 = -14;
pub const ERR_LIMIT: i32 = -15;

// ---- the source formats (FMT_* of jfgui.inc)
pub const PRINT_XRGB: u8 = 0; // alpha byte ignored
pub const PRINT_PARGB: u8 = 1; // premultiplied
pub const PRINT_ARGB: u8 = 3; // straight

// ---- PrintJob::flags
pub const PJF_LANDSCAPE: u32 = 1; // Querformat
pub const PJF_CENTER: u32 = 2; // centre in the printable area (else its top-left corner)
pub const PJF_SMASK: u32 = 4; // PDF only: straight colours + alpha as /SMask (no white)
// ---- PrintJob::scale
pub const PJS_FIT: u32 = 0; // as large as the printable area allows (aspect kept)
pub const PJS_DPI: u32 = 1; // original size: pixels / DPI inches
pub const PJS_PERCENT: u32 = 2; // PrintJob::percent of the original size
// ---- paper
pub const PAPER_A4: u32 = 0;
pub const PAPER_LETTER: u32 = 1;
pub const PAPER_A5: u32 = 2;
pub const PAPER_LEGAL: u32 = 3;
pub const PAPER_A3: u32 = 4;
pub const PAPER_N: u32 = 5;
// ---- row_white modes
pub const PRW_BGR: u32 = 1; // B, G, R byte order (GDI DIB) instead of R, G, B (PDF)
pub const PRW_PREMUL: u32 = 2; // source is premultiplied
pub const PRW_COPY: u32 = 4; // straight colours unchanged (no white; PDF with SMask)
pub const PRW_OPAQUE: u32 = 8; // alpha ignored (XRGB)

const TITLE_MAX: usize = 256; // code points of /Title
const PDF_LEVEL_DEF: i32 = 6;

/// the pixels of a print job: rows of 32-bit B, G, R, A (the fields of gfx::Surface)
#[derive(Clone, Copy)]
pub struct PrintSrc<'a> {
    pub px: &'a [u8],
    pub w: i32,
    pub h: i32,
    pub stride: i32,
    pub fmt: u8, // PRINT_ARGB (straight), PRINT_PARGB or PRINT_XRGB
}

/// one print job (PRINTJOB)
#[derive(Clone, Copy)]
pub struct PrintJob<'a> {
    pub src: Option<PrintSrc<'a>>,
    pub title: &'a str, // document name (spooler job, PDF /Title); "" = "Bild" (no PDF /Title)
    pub dpi_x: f64,     // image pixels per inch (<= 0: 96)
    pub dpi_y: f64,     // (<= 0: dpi_x)
    pub flags: u32,     // PJF_*
    pub scale: u32,     // PJS_*
    pub percent: u32,   // PJS_PERCENT: 1..10000 % of the size by DPI
    pub margin: u32,    // on every side, 1/100 mm (the printer's unprintable edge wins when larger)
    pub paper: u32,     // PAPER_* suggested to the dialog / used without one
    // out
    pub copies: u32,  // copies (informative, 0 = unknown)
    pub page_w: f64,  // page size in pt the job was laid out on
    pub page_h: f64,
}

impl<'a> PrintJob<'a> {
    pub fn new(src: Option<PrintSrc<'a>>, title: &'a str) -> PrintJob<'a> {
        PrintJob {
            src,
            title,
            dpi_x: 96.0,
            dpi_y: 96.0,
            flags: 0,
            scale: PJS_FIT,
            percent: 100,
            margin: 0,
            paper: PAPER_A4,
            copies: 0,
            page_w: 0.0,
            page_h: 0.0,
        }
    }
}

/// the page (PPAGE, pt = 1/72 inch)
#[derive(Clone, Copy, Default, Debug)]
pub struct PPage {
    pub w: f64, // page size (orientation applied)
    pub h: f64,
    pub l: f64, // unprintable edge left / top / right / bottom
    pub t: f64,
    pub r: f64,
    pub b: f64,
    pub token: u32, // print system cookie (portal token)
    pub flags: u32, // PJF_LANDSCAPE as chosen in the dialog
}

/// where the image goes (PLACE, pt from the page's top-left)
#[derive(Clone, Copy, Default, Debug)]
pub struct Place {
    pub x: f64, // image rectangle
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub ax: f64, // printable area (margins applied); the image is clipped to it
    pub ay: f64,
    pub aw: f64,
    pub ah: f64,
}

/// PDFOPT of pdf_write_image
#[derive(Clone, Copy, Default)]
pub struct PdfOpt<'a> {
    pub page_w: f64, // MediaBox in pt
    pub page_h: f64,
    pub x: f64, // image rectangle, pt from the page's top-left
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub cx: f64, // clip rectangle (cw <= 0: no clip)
    pub cy: f64,
    pub cw: f64,
    pub ch: f64,
    pub title: &'a str, // /Title ("" = none)
    pub flags: u32,     // PJF_SMASK
    pub level: i32,     // deflate level (<= 0: 6)
}

// PAPER_* portrait width, height in pt (mm * 72 / 25.4)
static PAPER_PT: [[f64; 2]; 5] = [
    [595.2755905511812, 841.8897637795276], // A4 210 x 297 mm
    [612.0, 792.0],                         // Letter 8.5 x 11 in
    [419.5275590551181, 595.2755905511812], // A5 148 x 210 mm
    [612.0, 1008.0],                        // Legal 8.5 x 14 in
    [841.8897637795276, 1190.551181102362], // A3 297 x 420 mm
];
const MM100_PT: f64 = 0.028346456692913385; // 1/100 mm in pt (72 / 2540)

/// the paper in pt, PJF_LANDSCAPE swaps; no unprintable edge  [print_page_init]
pub fn page_init(paper: u32, flags: u32) -> PPage {
    let p = PAPER_PT[if paper >= PAPER_N { 0 } else { paper as usize }];
    let (w, h) = if flags & PJF_LANDSCAPE != 0 { (p[1], p[0]) } else { (p[0], p[1]) };
    PPage { w, h, l: 0.0, t: 0.0, r: 0.0, b: 0.0, token: 0, flags: flags & PJF_LANDSCAPE }
}

/// pixels per inch from a document resolution: unit 0 none -> 96, 1 inch, 2 cm (* 2.54);
/// dpu <= 0 -> 96  [print_dpi]
pub fn print_dpi(unit: u32, dpu: f64) -> f64 {
    if unit < 1 || !(dpu > 0.0) {
        return 96.0;
    }
    if unit == 2 { dpu * 2.54 } else { dpu }
}

/// the image rectangle and the printable area on the page  [print_layout]
pub fn layout(job: &PrintJob, page: &PPage) -> Place {
    let dx = if job.dpi_x > 0.0 { job.dpi_x } else { 96.0 };
    let dy = if job.dpi_y > 0.0 { job.dpi_y } else { dx };
    let (pw, ph) = job.src.map_or((1, 1), |s| (s.w, s.h));
    let mut iw = pw as f64 * 72.0 / dx; // image size in pt
    let mut ih = ph as f64 * 72.0 / dy;
    let m = job.margin as f64 * MM100_PT;
    let mut pl = Place::default();
    pl.ax = maxsd(page.l, m);
    pl.aw = maxsd(page.w - maxsd(page.r, m) - pl.ax, 1.0);
    pl.ay = maxsd(page.t, m);
    pl.ah = maxsd(page.h - maxsd(page.b, m) - pl.ay, 1.0);
    if job.scale != PJS_DPI {
        let s = if job.scale == PJS_PERCENT {
            job.percent.clamp(1, 10000) as f64 / 100.0
        } else {
            minsd(pl.aw / iw, pl.ah / ih)
        };
        iw *= s;
        ih *= s;
    }
    pl.w = iw;
    pl.h = ih;
    pl.x = pl.ax;
    pl.y = pl.ay;
    if job.flags & PJF_CENTER != 0 {
        pl.x += (pl.aw - iw) * 0.5;
        pl.y += (pl.ah - ih) * 0.5;
    }
    pl
}

/// n pixels of src (B, G, R, A) -> n * 3 bytes of dst (R, G, B; PRW_BGR: B, G, R).  Straight:
/// c * a / 255 + 255 - a (rounded); PRW_PREMUL: c + 255 - a; PRW_COPY / PRW_OPAQUE: c
/// [print_row_white]
pub fn row_white(dst: &mut [u8], src: &[u8], n: usize, mode: u32) {
    for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)).take(n) {
        let (b, g, r) = (s[0] as u32, s[1] as u32, s[2] as u32);
        let a = s[3] as u32;
        let (b, g, r) = if mode & (PRW_OPAQUE | PRW_COPY) != 0 {
            (b, g, r)
        } else if mode & PRW_PREMUL != 0 {
            let k = 255 - a;
            ((b + k).min(255), (g + k).min(255), (r + k).min(255))
        } else {
            let w = (255 - a) * 255 + 128;
            let st = |c: u32| {
                let t = c * a + w;
                (t + (t >> 8)) >> 8
            };
            (st(b), st(g), st(r))
        };
        if mode & PRW_BGR != 0 {
            d[0] = b as u8;
            d[1] = g as u8;
            d[2] = r as u8;
        } else {
            d[0] = r as u8;
            d[1] = g as u8;
            d[2] = b as u8;
        }
    }
}

/// the job laid out on page as a one-page PDF (job.page_w / page_h set)  [print_pdf]
pub fn print_pdf(job: &mut PrintJob, page: &PPage) -> Result<Vec<u8>, i32> {
    let src = job.src.ok_or(ERR_DIMS)?;
    let pl = layout(job, page);
    job.page_w = page.w;
    job.page_h = page.h;
    let opt = PdfOpt {
        page_w: page.w,
        page_h: page.h,
        x: pl.x,
        y: pl.y,
        w: pl.w,
        h: pl.h,
        cx: pl.ax, // clip = printable area
        cy: pl.ay,
        cw: pl.aw,
        ch: pl.ah,
        title: job.title,
        flags: job.flags & PJF_SMASK,
        level: 0,
    };
    pdf_write_image(&src, &opt)
}

// ---------------------------------------------------------------- the zlib hook

/// a zlib compressor: (data, level) -> zlib stream (img_core::deflate::zlib_deflate)
pub type DeflateFn = fn(&[u8], i32) -> Vec<u8>;

static DEFLATE: AtomicUsize = AtomicUsize::new(0);

/// the compressor of the PDF streams (None: zlib stored blocks, no compression)
pub fn set_print_deflate(f: Option<DeflateFn>) {
    DEFLATE.store(f.map_or(0, |f| f as usize), Ordering::SeqCst);
}

fn deflate(data: &[u8], level: i32) -> Vec<u8> {
    let p = DEFLATE.load(Ordering::SeqCst);
    if p != 0 {
        // SAFETY: only set_print_deflate stores here, always a DeflateFn.
        let f: DeflateFn = unsafe { core::mem::transmute::<usize, DeflateFn>(p) };
        return f(data, level);
    }
    // zlib with stored blocks: header 78 01, blocks of at most 65535 bytes, Adler-32
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        out.push(chunks.peek().is_none() as u8);
        out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        out.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

// ---------------------------------------------------------------- the PDF writer  [pdfwrite.s]

// " a b" with 3 decimals each: millipoints rounded (cvtsd2si), "-" for negative  [pw_num]
fn pw_num(o: &mut String, v: f64) {
    let m = cvtd64(v * 1000.0);
    if m < 0 {
        o.push('-');
    }
    let u = m.unsigned_abs();
    let _ = write!(o, "{}.{:03}", u / 1000, u % 1000);
}

fn pw_nums(o: &mut String, a: f64, b: f64) {
    o.push(' ');
    pw_num(o, a);
    o.push(' ');
    pw_num(o, b);
}

/// " /Title <FEFF....>": UTF-16BE hex, at most TITLE_MAX code points  [pw_title]
fn pw_title(o: &mut String, s: &str) {
    o.push_str(" /Title <FEFF");
    for c in s.chars().take_while(|&c| c != '\0').take(TITLE_MAX) {
        let mut buf = [0u16; 2];
        for u in c.encode_utf16(&mut buf) {
            let _ = write!(o, "{:04X}", u);
        }
    }
    o.push('>');
}

// the image streams of a PDF page: RGB (deflated; over white unless PJF_SMASK keeps straight
// colours) and the alpha plane of the soft mask (PJF_SMASK and a straight source) -- the
// checks of pdf_write_image
fn pdf_image_data(src: &PrintSrc, flags: u32, level: i32) -> Result<(Vec<u8>, Option<Vec<u8>>), i32> {
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 || src.px.is_empty() {
        return Err(ERR_DIMS);
    }
    let mut mode = match src.fmt {
        PRINT_ARGB => 0,
        PRINT_PARGB => PRW_PREMUL,
        PRINT_XRGB => PRW_OPAQUE,
        _ => return Err(ERR_UNSUPPORTED),
    };
    if src.fmt == PRINT_ARGB && flags & PJF_SMASK != 0 {
        mode = PRW_COPY;
    }
    let level = if level <= 0 { PDF_LEVEL_DEF } else { level };
    let (wu, hu) = (w as usize, h as usize);
    // w and h size the buffers below: the rows must lie inside px
    if src.stride < w.saturating_mul(4) {
        return Err(ERR_DIMS);
    }
    let covered = wu.checked_mul(4).and_then(|rw| (hu - 1).checked_mul(src.stride as usize)?.checked_add(rw));
    if covered.is_none_or(|n| n > src.px.len()) {
        return Err(ERR_DIMS);
    }
    let stride = src.stride as isize;
    let row_of = |y: usize| -> &[u8] {
        let o = (y as isize * stride) as usize;
        src.px.get(o..o + wu * 4).unwrap_or(&[])
    };
    // RGB rows
    let rb = wu * 3;
    let mut raw = alloc::vec![0u8; rb * hu];
    for y in 0..hu {
        row_white(&mut raw[y * rb..(y + 1) * rb], row_of(y), wu, mode);
    }
    let rgb = deflate(&raw, level);
    drop(raw);
    // alpha plane for the soft mask
    let alpha = if mode == PRW_COPY {
        let mut a = Vec::with_capacity(wu * hu);
        for y in 0..hu {
            let r = row_of(y);
            for x in 0..wu {
                a.push(r.get(x * 4 + 3).copied().unwrap_or(0));
            }
        }
        Some(deflate(&a, level))
    } else {
        None
    };
    Ok((rgb, alpha))
}

// the content stream of a page: the clip (cw > 0) and the image Im1 at its rectangle
fn pdf_content(opt: &PdfOpt) -> String {
    let mut c = String::from("q\n");
    if opt.cw > 0.0 {
        pw_nums(&mut c, opt.cx, opt.page_h - opt.cy - opt.ch); // bottom of the clip in PDF space
        pw_nums(&mut c, opt.cw, opt.ch);
        c.push_str(" re W n\n");
    }
    pw_nums(&mut c, opt.w, 0.0);
    pw_nums(&mut c, 0.0, opt.h);
    pw_nums(&mut c, opt.x, opt.page_h - opt.y - opt.h);
    c.push_str(" cm\n/Im1 Do\nQ\n");
    c
}

// "N 0 obj\n<< /Type /XObject /Subtype /Image ..>>\nstream\n" + data + "\nendstream\nendobj\n"
fn pdf_image_obj(f: &mut Vec<u8>, n: usize, w: i32, h: i32, rgb: &[u8], smask: Option<usize>) {
    let mut s = String::new();
    let _ = write!(s, "{n} 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h}");
    s.push_str(" /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode");
    if let Some(m) = smask {
        let _ = write!(s, " /SMask {m} 0 R");
    }
    let _ = write!(s, " /Length {} >>\nstream\n", rgb.len());
    f.extend_from_slice(s.as_bytes());
    f.extend_from_slice(rgb);
    f.extend_from_slice(b"\nendstream\nendobj\n");
}

// the soft mask object N (alpha, DeviceGray)
fn pdf_mask_obj(f: &mut Vec<u8>, n: usize, w: i32, h: i32, a: &[u8]) {
    let mut s = String::new();
    let _ = write!(s, "{n} 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h}");
    let _ = write!(s, " /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", a.len());
    f.extend_from_slice(s.as_bytes());
    f.extend_from_slice(a);
    f.extend_from_slice(b"\nendstream\nendobj\n");
}

/// one image on one PDF 1.4 page: objects 1 Catalog, 2 Pages, 3 Page, 4 Contents, 5 Image
/// (/DeviceRGB 8 bit /FlateDecode, composited over white -- with PJF_SMASK and a straight
/// source: straight colours), 6 Info, [7 /SMask alpha]; classic xref  [pdf_write_image]
pub fn pdf_write_image(src: &PrintSrc, opt: &PdfOpt) -> Result<Vec<u8>, i32> {
    let (w, h) = (src.w, src.h);
    let (rgb, alpha) = pdf_image_data(src, opt.flags, opt.level)?;
    let nobj = if alpha.is_some() { 7 } else { 6 };
    let mut off = [0usize; 8];
    let mut f: Vec<u8> = Vec::with_capacity(rgb.len() + alpha.as_ref().map_or(0, |a| a.len()) + 4096);
    let mut s = String::new();
    f.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    off[1] = f.len();
    f.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    off[2] = f.len();
    f.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    // 3 Page
    off[3] = f.len();
    s.push_str("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0");
    pw_nums(&mut s, opt.page_w, opt.page_h);
    s.push_str("] /Resources << /XObject << /Im1 5 0 R >> /ProcSet [/PDF /ImageC] >> /Contents 4 0 R >>\nendobj\n");
    f.extend_from_slice(s.as_bytes());
    // 4 Contents
    let c = pdf_content(opt);
    off[4] = f.len();
    s.clear();
    let _ = write!(s, "4 0 obj\n<< /Length {} >>\nstream\n", c.len());
    f.extend_from_slice(s.as_bytes());
    f.extend_from_slice(c.as_bytes());
    f.extend_from_slice(b"\nendstream\nendobj\n");
    // 5 Image
    off[5] = f.len();
    pdf_image_obj(&mut f, 5, w, h, &rgb, (nobj == 7).then_some(7));
    // 6 Info
    off[6] = f.len();
    s.clear();
    s.push_str("6 0 obj\n<< /Producer (jfguiLib)");
    if !opt.title.is_empty() && !opt.title.starts_with('\0') {
        pw_title(&mut s, opt.title);
    }
    s.push_str(" >>\nendobj\n");
    f.extend_from_slice(s.as_bytes());
    // 7 SMask
    if let Some(a) = &alpha {
        off[7] = f.len();
        pdf_mask_obj(&mut f, 7, w, h, a);
    }
    // xref, trailer
    let startxref = f.len();
    s.clear();
    let _ = write!(s, "xref\n0 {}\n0000000000 65535 f \n", nobj + 1);
    for o in &off[1..=nobj] {
        let _ = write!(s, "{o:010} 00000 n \n");
    }
    let _ = write!(s, "trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{startxref}\n", nobj + 1);
    s.push_str("%%EOF\n");
    f.extend_from_slice(s.as_bytes());
    Ok(f)
}

// ---------------------------------------------------------------- jobs of several pages (E9)

// a PDF of several pages built page by page: 1 Catalog, 2 Pages, 3 Info, then per page Page,
// Contents, Image [, SMask]; the image streams are kept deflated until finish
#[derive(Default)]
pub(crate) struct PdfPages {
    // the objects from 4 on, each "N 0 obj .. endobj\n"
    objs: Vec<Vec<u8>>,
    pages: Vec<usize>,
}

impl PdfPages {
    // one page of page_w x page_h pt with the image placed as opt says
    fn add(&mut self, src: &PrintSrc, opt: &PdfOpt) -> Result<(), i32> {
        let (rgb, alpha) = pdf_image_data(src, opt.flags, opt.level)?;
        let n = 4 + self.objs.len(); // Page n, Contents n+1, Image n+2, SMask n+3
        let mut s = String::new();
        let _ = write!(s, "{n} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0");
        pw_nums(&mut s, opt.page_w, opt.page_h);
        let _ = write!(s, "] /Resources << /XObject << /Im1 {} 0 R >> /ProcSet [/PDF /ImageC] >> /Contents {} 0 R >>\nendobj\n", n + 2, n + 1);
        self.objs.push(s.into_bytes());
        let c = pdf_content(opt);
        let mut o = Vec::with_capacity(c.len() + 64);
        let mut s = String::new();
        let _ = write!(s, "{} 0 obj\n<< /Length {} >>\nstream\n", n + 1, c.len());
        o.extend_from_slice(s.as_bytes());
        o.extend_from_slice(c.as_bytes());
        o.extend_from_slice(b"\nendstream\nendobj\n");
        self.objs.push(o);
        let mut o = Vec::with_capacity(rgb.len() + 200);
        pdf_image_obj(&mut o, n + 2, src.w, src.h, &rgb, alpha.as_ref().map(|_| n + 3));
        self.objs.push(o);
        if let Some(a) = &alpha {
            let mut o = Vec::with_capacity(a.len() + 200);
            pdf_mask_obj(&mut o, n + 3, src.w, src.h, a);
            self.objs.push(o);
        }
        self.pages.push(n);
        Ok(())
    }

    // the whole file
    fn finish(&self, title: &str) -> Vec<u8> {
        let total: usize = self.objs.iter().map(|o| o.len()).sum();
        let mut f: Vec<u8> = Vec::with_capacity(total + 1024 + self.pages.len() * 16);
        let mut off = Vec::with_capacity(self.objs.len() + 4);
        f.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        off.push(f.len());
        f.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off.push(f.len());
        let mut s = String::from("2 0 obj\n<< /Type /Pages /Kids [");
        for (k, p) in self.pages.iter().enumerate() {
            let _ = write!(s, "{}{p} 0 R", if k > 0 { " " } else { "" });
        }
        let _ = write!(s, "] /Count {} >>\nendobj\n", self.pages.len());
        f.extend_from_slice(s.as_bytes());
        off.push(f.len());
        s.clear();
        s.push_str("3 0 obj\n<< /Producer (jfguiLib)");
        if !title.is_empty() && !title.starts_with('\0') {
            pw_title(&mut s, title);
        }
        s.push_str(" >>\nendobj\n");
        f.extend_from_slice(s.as_bytes());
        for o in &self.objs {
            off.push(f.len());
            f.extend_from_slice(o);
        }
        let startxref = f.len();
        s.clear();
        let _ = write!(s, "xref\n0 {}\n0000000000 65535 f \n", off.len() + 1);
        for o in &off {
            let _ = write!(s, "{o:010} 00000 n \n");
        }
        let _ = write!(s, "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{startxref}\n%%EOF\n", off.len() + 1);
        f.extend_from_slice(s.as_bytes());
        f
    }
}

// the platform's part of a PrintDoc (win_print.rs: the printer DC and its geometry; Linux /
// tests: the PDF and where it goes)
#[derive(Default)]
pub(crate) struct DocSys {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub hdc: isize,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub dpi: [f64; 2],
    #[cfg_attr(not(windows), allow(dead_code))]
    pub off: [i32; 2],
    #[cfg_attr(not(windows), allow(dead_code))]
    pub started: bool,
    // the PDF being built (platforms that print PDF documents, the test variant)
    pub pdf: Option<PdfPages>,
    // add_pdf's document
    pub raw: Option<Vec<u8>>,
    // the test variant's file (jfgui_PRINT_TO)
    pub to: Option<String>,
    // PrintDlgExW's page ranges (from, to; empty: every page)
    pub ranges: Vec<(u32, u32)>,
}

/// a print job of several pages (see the file header; E9)
pub struct PrintDoc {
    pub(crate) title: String,
    pub(crate) dpi_x: f64,
    pub(crate) dpi_y: f64,
    pub(crate) flags: u32,
    pub(crate) scale: u32,
    pub(crate) percent: u32,
    pub(crate) margin: u32,
    pub(crate) paper: u32,
    #[cfg_attr(not(windows), allow(dead_code))] // (the dialog's page range: Windows)
    pub(crate) pages: u32,
    pub(crate) page: PPage,
    pub(crate) copies: u32,
    pub(crate) added: u32,
    pub(crate) sys: DocSys,
}

impl PrintDoc {
    /// start a job: the system's print dialog where the platform has one (printer, paper,
    /// orientation, copies; with pages > 0 a page range of 1..=pages), never one in the test
    /// variant (jfgui_PRINT_TO).  job: the title (spooler, PDF /Title; "" = "Dokument"), paper,
    /// PJF_LANDSCAPE / PJF_CENTER / PJF_SMASK, margin, scale, percent, dpi -- its src is not
    /// used.  Err: ERR_CANCEL (the dialog was cancelled), ERR_UNSUPPORTED (no printing here,
    /// no window, the manifest forbids it), ERR_IO
    pub fn begin(job: &PrintJob, pages: u32) -> Result<PrintDoc, i32> {
        if !crate::policy::print_allowed() {
            return Err(ERR_UNSUPPORTED);
        }
        let title = if job.title.is_empty() || job.title.starts_with('\0') { "Dokument" } else { job.title };
        let mut d = PrintDoc {
            title: String::from(title),
            dpi_x: job.dpi_x,
            dpi_y: job.dpi_y,
            flags: job.flags,
            scale: job.scale,
            percent: job.percent,
            margin: job.margin,
            paper: job.paper,
            pages,
            page: page_init(job.paper, job.flags),
            copies: 0,
            added: 0,
            sys: DocSys::default(),
        };
        let r = crate::doc_begin(&mut d);
        if r != 0 {
            return Err(r);
        }
        Ok(d)
    }

    /// the page the job prints on (pt; the printer's unprintable edges; the orientation the
    /// dialog chose in flags): lay the pages out for it
    pub fn page(&self) -> PPage {
        self.page
    }

    /// the copies the dialog chose (informative: the printer makes them; 1 in the test variant)
    pub fn copies(&self) -> u32 {
        self.copies
    }

    /// page n (from 1) is in the range the dialog chose (always without a range)
    pub fn wants(&self, n: u32) -> bool {
        self.sys.ranges.is_empty() || self.sys.ranges.iter().any(|&(a, b)| n >= a && n <= b)
    }

    /// the pages added so far
    pub fn added(&self) -> u32 {
        self.added
    }

    /// the platform prints a PDF document as it is (add_pdf): Linux's portal and the test
    /// variant there; Windows and the others: images only
    pub fn pdf_native(&self) -> bool {
        self.sys.pdf.is_some()
    }

    // the job's settings with src for layout()
    fn job_of<'a>(&'a self, src: &PrintSrc<'a>) -> PrintJob<'a> {
        PrintJob {
            src: Some(*src),
            title: &self.title,
            dpi_x: self.dpi_x,
            dpi_y: self.dpi_y,
            flags: self.flags,
            scale: self.scale,
            percent: self.percent,
            margin: self.margin,
            paper: self.paper,
            copies: 0,
            page_w: self.page.w,
            page_h: self.page.h,
        }
    }

    /// one page: the image laid out on the page as layout() places it (the job's scale, dpi,
    /// margin, PJF_CENTER; clipped to the printable area) -> 0 / ERR_DIMS / ERR_UNSUPPORTED (a
    /// format or after add_pdf) / ERR_IO / ERR_NOMEM
    pub fn add_image(&mut self, src: &PrintSrc) -> i32 {
        if src.w <= 0 || src.h <= 0 || src.px.is_empty() {
            return ERR_DIMS;
        }
        if self.sys.raw.is_some() {
            return ERR_UNSUPPORTED;
        }
        let pl = layout(&self.job_of(src), &self.page);
        let r = if let Some(pdf) = self.sys.pdf.as_mut() {
            let opt = PdfOpt {
                page_w: self.page.w,
                page_h: self.page.h,
                x: pl.x,
                y: pl.y,
                w: pl.w,
                h: pl.h,
                cx: pl.ax,
                cy: pl.ay,
                cw: pl.aw,
                ch: pl.ah,
                title: "",
                flags: self.flags & PJF_SMASK,
                level: 0,
            };
            pdf.add(src, &opt).err().unwrap_or(0)
        } else {
            crate::doc_image(self, src, &pl)
        };
        if r == 0 {
            self.added += 1;
        }
        r
    }

    /// the whole job as a PDF of the app's own, printed as it is (only where pdf_native(),
    /// before any add_image) -> 0 / ERR_UNSUPPORTED
    pub fn add_pdf(&mut self, pdf: &[u8]) -> i32 {
        if !self.pdf_native() || self.added > 0 || self.sys.raw.is_some() || !pdf.starts_with(b"%PDF-") {
            return ERR_UNSUPPORTED;
        }
        self.sys.raw = Some(Vec::from(pdf));
        self.added = 1;
        0
    }

    /// finish: the job goes to the printer -> 0 printed / ERR_CANCEL (the portal's job was
    /// cancelled) / ERR_DIMS (no page) / ERR_IO
    pub fn end(mut self) -> i32 {
        if self.added == 0 {
            crate::doc_abort(&mut self);
            return ERR_DIMS;
        }
        crate::doc_end(&mut self)
    }

    /// drop the job (nothing is printed)
    pub fn abort(mut self) {
        crate::doc_abort(&mut self);
    }

    // (platforms) the PDF of the pages or add_pdf's document
    pub(crate) fn pdf_bytes(&self) -> Option<Vec<u8>> {
        if let Some(r) = &self.sys.raw {
            return Some(r.clone());
        }
        self.sys.pdf.as_ref().map(|p| p.finish(&self.title))
    }
}

// (platforms without a print system of their own) begin: the test variant writes our PDF
// into jfgui_PRINT_TO, else ERR_UNSUPPORTED
#[allow(dead_code)]
pub(crate) fn doc_begin_pdf_only(d: &mut PrintDoc) -> i32 {
    match crate::env("jfgui_PRINT_TO").filter(|v| !v.is_empty()) {
        Some(to) => {
            d.sys.to = Some(to);
            d.sys.pdf = Some(PdfPages::default());
            d.copies = 1;
            0
        }
        None => ERR_UNSUPPORTED,
    }
}

// (platforms) end of a PDF job of the test variant: the PDF into its file
#[allow(dead_code)]
pub(crate) fn doc_end_pdf_file(d: &mut PrintDoc) -> i32 {
    let (Some(to), Some(pdf)) = (d.sys.to.clone(), d.pdf_bytes()) else { return ERR_UNSUPPORTED };
    if crate::write_file_raw(&to, &pdf) { 0 } else { ERR_IO }
}
