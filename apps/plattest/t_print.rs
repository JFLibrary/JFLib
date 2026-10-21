// t_print -- (plat-shared) printing without a dialog and without paper
//   plattest --test print     paper sizes, print_dpi, row_white against the exact rounding
//        for every (colour, alpha), the PDF writer's errors and its stored-zlib fallback
//        (inflated again), the headless guard (no jfgui_PRINT_TO: ERR_UNSUPPORTED, no
//        dialog, no bus), a job without pixels (ERR_DIMS)
//   plattest --test printlayout   layout() for a grid of jobs / pages, one line each (bit
//        patterns) -- tools/platshared/print_rs_check.py replays print_check.py's layout
//   plattest --test printjob --case N [--expect E]   one job of tools/print_check.py's CASES
//        through plat::print (its 37 x 23 straight RGBA test image, title "druck_test.png",
//        the resolution chain of PaintCraft for a PNG pHYs): with jfgui_PRINT_TO / _PRINTER /
//        _SMASK / _PORTAL set by the caller.  The zlib hook is img_core's deflate (the PDF is
//        byte-identical to PaintCraft's).  Prints "print: result R, copies C, page W x H".
//                     [ASMLIB/src/plat/print.s, img/pdfwrite.s; tools/print_check.py]

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::{
    ERR_DIMS, ERR_UNSUPPORTED, PAPER_A3, PAPER_A4, PAPER_N, PJF_CENTER, PJF_LANDSCAPE, PJS_DPI, PJS_FIT, PJS_PERCENT,
    PPage, PRINT_ARGB, PRINT_PARGB, PRINT_XRGB, PRW_BGR, PRW_COPY, PRW_PREMUL, PdfOpt, PrintJob, PrintSrc, layout,
    page_init, pdf_write_image, print_dpi, row_white,
};

use crate::T;

const IMG_W: i32 = 37;
const IMG_H: i32 = 23;

/// print_check.py make_pixels: straight B, G, R, A rows
fn make_pixels() -> Vec<u8> {
    let mut px = Vec::with_capacity((IMG_W * IMG_H * 4) as usize);
    let alpha = [255u32, 0, 128, 200, 37];
    for y in 0..IMG_H as u32 {
        for x in 0..IMG_W as u32 {
            let r = x * 255 / (IMG_W as u32 - 1);
            let g = y * 255 / (IMG_H as u32 - 1);
            let b = (x * 7 + y * 13) & 255;
            let a = alpha[((x + 2 * y) % 5) as usize];
            px.extend_from_slice(&[b as u8, g as u8, r as u8, a as u8]);
        }
    }
    px
}

/// (paper, landscape, scale, percent, center, margin mm, PNG pHYs ppm or 0)  print_check.CASES
const CASES: [(u32, bool, u32, u32, bool, u32, u32); 3] = [
    (0, false, PJS_FIT, 100, true, 5, 0),
    (1, true, PJS_PERCENT, 50, false, 0, 0),
    (2, false, PJS_DPI, 100, true, 12, 11811), // 300 dpi as a PNG stores it
];

/// the dpi PaintCraft derives from a PNG pHYs: img ppm_dpi (16.16) -> set_dpi (per cm) ->
/// print_dpi; no pHYs: 96
fn png_dpi(ppm: u32) -> f64 {
    if ppm == 0 {
        return print_dpi(0, 0.0);
    }
    let d16 = img_core::io::ppm_dpi(ppm);
    print_dpi(2, d16 as f64 / 65536.0 / 2.54)
}

fn job_of<'a>(src: &'a [u8], c: usize) -> PrintJob<'a> {
    let (paper, land, scale, pct, center, margin, ppm) = CASES[c];
    let mut j = PrintJob::new(Some(PrintSrc { px: src, w: IMG_W, h: IMG_H, stride: IMG_W * 4, fmt: PRINT_ARGB }), "druck_test.png");
    let dpi = png_dpi(ppm);
    j.dpi_x = dpi;
    j.dpi_y = dpi;
    j.flags = if land { PJF_LANDSCAPE } else { 0 } | if center { PJF_CENTER } else { 0 };
    j.scale = scale;
    j.percent = pct;
    j.margin = margin * 100;
    j.paper = paper;
    j
}

pub fn t_print(t: &mut T) {
    // ---- paper sizes, resolution
    let a4 = page_init(PAPER_A4, 0);
    let a3l = page_init(PAPER_A3, PJF_LANDSCAPE | PJF_CENTER);
    let bad = page_init(PAPER_N + 3, 0);
    t.check(a4.w == 595.2755905511812 && a4.h == 841.8897637795276 && a4.l == 0.0 && a4.flags == 0,
            "print: print_page_init A4 portrait (pt)");
    t.check(a3l.w == 1190.551181102362 && a3l.h == 841.8897637795276 && a3l.flags == PJF_LANDSCAPE,
            "print: A3 landscape swaps width / height, flags = PJF_LANDSCAPE only");
    t.check(bad.w == a4.w && bad.h == a4.h, "print: an unknown paper is A4");
    t.check(print_dpi(0, 300.0) == 96.0 && print_dpi(1, 300.0) == 300.0 && print_dpi(1, -1.0) == 96.0
            && print_dpi(2, 100.0) == 254.0 && print_dpi(3, 72.0) == 72.0, "print: print_dpi (none / inch / cm)");
    // ---- rows over white: the exact rounding of c * a / 255 + 255 - a
    let mut ok = true;
    let mut src = vec![0u8; 256 * 4];
    let mut dst = vec![0u8; 256 * 3];
    for a in 0..256u32 {
        for c in 0..256u32 {
            src[c as usize * 4..c as usize * 4 + 4].copy_from_slice(&[c as u8, (255 - c) as u8, (c / 2) as u8, a as u8]);
        }
        row_white(&mut dst, &src, 256, 0);
        for c in 0..256u32 {
            let want = |v: u32| ((v * a + 255 * (255 - a) + 127) / 255) as u8;
            let d = &dst[c as usize * 3..c as usize * 3 + 3];
            ok &= d == [want(c / 2), want(255 - c), want(c)];
        }
        row_white(&mut dst, &src, 256, PRW_BGR | PRW_PREMUL);
        for c in 0..256u32 {
            let want = |v: u32| (v + 255 - a).min(255) as u8;
            ok &= dst[c as usize * 3..c as usize * 3 + 3] == [want(c), want(255 - c), want(c / 2)];
        }
        row_white(&mut dst, &src, 256, PRW_COPY);
        for c in 0..256u32 {
            ok &= dst[c as usize * 3..c as usize * 3 + 3] == [(c / 2) as u8, (255 - c) as u8, c as u8];
        }
    }
    t.check(ok, "print: print_row_white for every colour / alpha (straight, premultiplied, copy)");
    // ---- the PDF writer
    let px = make_pixels();
    let opt = PdfOpt { page_w: 100.0, page_h: 200.0, x: 10.0, y: 20.0, w: 37.0, h: 23.0, title: "x", ..PdfOpt::default() };
    let empty = PrintSrc { px: &[], w: 0, h: 0, stride: 0, fmt: PRINT_ARGB };
    let a8 = PrintSrc { px: &px, w: IMG_W, h: IMG_H, stride: IMG_W * 4, fmt: 2 };
    t.check(pdf_write_image(&empty, &opt).err() == Some(ERR_DIMS), "print: pdf_write_image of an empty image -> ERR_DIMS");
    t.check(pdf_write_image(&a8, &opt).err() == Some(ERR_UNSUPPORTED), "print: pdf_write_image of A8 -> ERR_UNSUPPORTED");
    plat::set_print_deflate(None);
    let xrgb = PrintSrc { px: &px, w: IMG_W, h: IMG_H, stride: IMG_W * 4, fmt: PRINT_XRGB };
    let pdf = pdf_write_image(&xrgb, &opt).unwrap_or_default();
    let stream = pdf_stream(&pdf, b"5 0 obj");
    let raw = stream.and_then(|s| {
        let mut out = vec![0u8; (IMG_W * IMG_H * 3) as usize];
        let n = img_core::inflate::zlib_inflate(s, &mut out).ok()?;
        out.truncate(n);
        Some(out)
    });
    let want: Vec<u8> = px.chunks(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
    t.check(pdf.starts_with(b"%PDF-1.4\n") && pdf.ends_with(b"%%EOF\n") && raw.as_deref() == Some(&want[..]),
            "print: without a zlib hook the image is stored (and inflates to the pixels)");
    plat::set_print_deflate(Some(img_core::deflate::zlib_deflate));
    let parg = PrintSrc { px: &px, w: IMG_W, h: IMG_H, stride: IMG_W * 4, fmt: PRINT_PARGB };
    let pdf2 = pdf_write_image(&parg, &opt).unwrap_or_default();
    let n2 = pdf_stream(&pdf2, b"5 0 obj").map_or(0, |s| s.len());
    t.check(n2 > 0 && n2 < stream.map_or(0, |s| s.len()), "print: with img_core's deflate the stream is compressed");
    // ---- headless: no test variable -> unsupported, never a dialog / bus
    let mut job = job_of(&px, 0);
    job.copies = 9;
    let r = plat::print(&mut job);
    t.info(&format!("  (headless print -> {r})\n"));
    t.check(r == ERR_UNSUPPORTED && job.copies == 0, "print: headless without jfgui_PRINT_TO -> ERR_UNSUPPORTED");
    let mut nojob = PrintJob::new(None, "");
    t.check(plat::print(&mut nojob) == ERR_DIMS, "print: a job without pixels -> ERR_DIMS");
}

/// the stream of the object starting with head (by its /Length)
fn pdf_stream<'a>(pdf: &'a [u8], head: &[u8]) -> Option<&'a [u8]> {
    let o = pdf.windows(head.len()).position(|w| w == head)?;
    let rest = &pdf[o..];
    let l = rest.windows(8).position(|w| w == b"/Length ")? + 8;
    let mut n = 0usize;
    let mut k = l;
    while k < rest.len() && rest[k].is_ascii_digit() {
        n = n * 10 + (rest[k] - b'0') as usize;
        k += 1;
    }
    let s = rest.windows(7).position(|w| w == b"stream\n")? + 7;
    rest.get(s..s + n)
}

/// layout() for a grid of jobs and pages: "paper land scale pct center margin dpi l t r b ->
/// x y w h ax ay aw ah" with f64 bit patterns in hex
pub fn t_printlayout(_t: &mut T) {
    let px = make_pixels();
    let mut s = String::new();
    let dpis = [96.0, 300.0, 72.5, 0.0, -1.0];
    for paper in 0..PAPER_N {
        for land in [false, true] {
            for scale in [PJS_FIT, PJS_DPI, PJS_PERCENT] {
                for (pct, center, margin) in [(100u32, true, 5u32), (50, false, 0), (0, true, 12), (20000, false, 300)] {
                    for (k, &dpi) in dpis.iter().enumerate() {
                        let mut j = PrintJob::new(Some(PrintSrc { px: &px, w: IMG_W, h: IMG_H, stride: IMG_W * 4, fmt: PRINT_ARGB }), "");
                        j.dpi_x = dpi;
                        j.dpi_y = if k == 2 { 150.0 } else { dpi };
                        j.flags = if land { PJF_LANDSCAPE } else { 0 } | if center { PJF_CENTER } else { 0 };
                        j.scale = scale;
                        j.percent = pct;
                        j.margin = margin * 100;
                        j.paper = paper;
                        let mut pg = page_init(paper, j.flags);
                        if k == 1 {
                            // a printer's unprintable edge
                            pg = PPage { l: 12.5, t: 18.0, r: 9.25, b: 30.0, ..pg };
                        }
                        let p = layout(&j, &pg);
                        s.push_str(&format!(
                            "{paper} {} {scale} {pct} {} {margin} {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x} -> {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x}\n",
                            land as u32, center as u32, j.dpi_x.to_bits(), j.dpi_y.to_bits(), pg.w.to_bits(), pg.h.to_bits(),
                            pg.l.to_bits(), pg.t.to_bits(), pg.r.to_bits(), pg.b.to_bits(), p.x.to_bits(), p.y.to_bits(),
                            p.w.to_bits(), p.h.to_bits(), p.ax.to_bits(), p.ay.to_bits(), p.aw.to_bits(), p.ah.to_bits()
                        ));
                    }
                }
            }
        }
    }
    crate::out(&s);
}

/// one print_check case through plat::print (the environment chooses the variant)
pub fn t_printjob(t: &mut T) {
    let c = plat::arg_value("--case").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(CASES.len() - 1);
    let expect = plat::arg_value("--expect").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    plat::set_print_deflate(Some(img_core::deflate::zlib_deflate));
    let px = make_pixels();
    let mut job = job_of(&px, c);
    let r = plat::print(&mut job);
    crate::out(&format!("print: result {r}, copies {}, page {:.3} x {:.3}\n", job.copies, job.page_w, job.page_h));
    t.check(r == expect, &format!("printjob: case {c} -> {expect}"));
}
