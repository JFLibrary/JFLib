// rrender -- `pdftest --render FILE --out PREFIX [--page N | --last N] [--dpi D] [--password PW]`: pages
// drawn by pdf::read's renderer as binary PPM files PREFIX-<page>.ppm (pages counted from 1;
// every page without --page, the first N with --last), over white, for tools/pdf_render_check.py to compare with
// Poppler's pdftoppm.  Exit code 1 when the file does not open or a page fails.

use alloc::format;
use alloc::vec::Vec;
use pdf::read::{Document, Options, RenderOptions};

pub fn render(path: &str, out: &str, password: Option<alloc::string::String>) -> i32 {
    let Some(data) = plat::read_file(path) else {
        plat::write_stdout(format!("pdftest: cannot read {path}\n").as_bytes());
        return 1;
    };
    let opt = Options { password: password.unwrap_or_default(), ..Default::default() };
    let doc = match Document::open_with(data, &opt) {
        Ok(d) => d,
        Err(e) => {
            plat::write_stdout(format!("pdftest: {path}: {e}\n").as_bytes());
            return 1;
        }
    };
    let dpi = plat::arg_value("--dpi").and_then(|v| v.parse::<f64>().ok()).unwrap_or(72.0);
    let pages: Vec<usize> = match plat::arg_value("--page").and_then(|v| v.parse::<usize>().ok()) {
        Some(p) if p >= 1 => alloc::vec![p - 1],
        _ => {
            let last = plat::arg_value("--last").and_then(|v| v.parse::<usize>().ok()).unwrap_or(usize::MAX);
            (0..doc.page_count().min(last)).collect()
        }
    };
    let ropt = RenderOptions { dpi, ..Default::default() };
    let mut status = 0;
    for p in pages {
        match doc.render(p, &ropt) {
            Ok(sf) => {
                let (w, h) = (sf.w as usize, sf.h as usize);
                let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                let head = ppm.len();
                ppm.resize(head + w * h * 3, 0);
                for (row, out) in sf.px.chunks(sf.stride as usize).zip(ppm[head..].chunks_exact_mut(w * 3)) {
                    for (p, o) in row[..w * 4].chunks_exact(4).zip(out.chunks_exact_mut(3)) {
                        // premultiplied over white
                        let ia = 255 - p[3] as u32;
                        o[0] = (p[2] as u32 + ia).min(255) as u8;
                        o[1] = (p[1] as u32 + ia).min(255) as u8;
                        o[2] = (p[0] as u32 + ia).min(255) as u8;
                    }
                }
                let name = format!("{out}-{}.ppm", p + 1);
                if !plat::write_file(&name, &ppm) {
                    plat::write_stdout(format!("pdftest: cannot write {name}\n").as_bytes());
                    status = 1;
                }
            }
            Err(e) => {
                plat::write_stdout(format!("pdftest: {path} page {}: {e}\n", p + 1).as_bytes());
                status = 1;
            }
        }
    }
    status
}
