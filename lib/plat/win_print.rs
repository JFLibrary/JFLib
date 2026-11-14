// win_print -- printing on Windows: PrintDlgExW (comdlg32, loaded at run time) and GDI
//                                                      [src/plat/win/win_print.s; docs/modules/PRINT.md]
//
//   print(job) -> 0 printed / ERR_CANCEL / ERR_UNSUPPORTED / ERR_IO / ERR_NOMEM / ERR_DIMS.
//        The system print dialog (printer, paper, orientation, copies; the job's paper and
//        PJF_LANDSCAPE preselected through a DEVMODE) returns a printer DC; the page geometry
//        comes from GetDeviceCaps (PHYSICALWIDTH / -OFFSET, HORZRES, LOGPIXELS), layout()
//        places the image, which goes out with StretchDIBits (HALFTONE) in bands of at most
//        BAND_BYTES of 24-bit rows composited over white (row_white), clipped to the
//        printable area (IntersectClipRect).
//   Test variant (never a dialog, never paper):
//        jfgui_PRINT_TO=FILE          output goes to FILE (DOCINFO.lpszOutput) on the printer
//        jfgui_PRINT_PRINTER=NAME     (default "Microsoft Print to PDF"), opened with
//                                      CreateDCW and a DEVMODE for paper / orientation
//        Without a main window (headless) and without jfgui_PRINT_TO: ERR_UNSUPPORTED.
//   doc_begin / doc_image / doc_end / doc_abort   (print.rs PrintDoc, E9) the same dialog (a
//        page range when the job names its pages: PD_PAGENUMS -> DocSys::ranges) and DC,
//        StartDocW once, then per page StartPage, the bands of layout()'s rectangle in a
//        SaveDC / RestoreDC around the clip, EndPage; EndDocW at the end, AbortDoc when dropped.
// The gdi32 functions are imported directly (the assembly took ten of them with
// GetProcAddress only to keep its import library small); PrintDlgExW (comdlg32),
// GetDefaultPrinterW (winspool.drv) and CoInitializeEx (ole32) are loaded at run time.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

use rt::fx::cvtd;

use crate::{
    ERR_CANCEL, ERR_DIMS, ERR_IO, ERR_NOMEM, ERR_UNSUPPORTED, PAPER_N, PJF_LANDSCAPE, PPage, PRINT_ARGB, PRINT_PARGB,
    PRINT_XRGB, PRW_BGR, PRW_OPAQUE, PRW_PREMUL, Place, PrintDoc, PrintJob, PrintSrc, layout, row_white,
};

const BAND_BYTES: u32 = 4 << 20;
const WPATH_CAP: usize = 1040; // wide chars
const WTITLE_CAP: usize = 260;
const WPRN_CAP: usize = 256;
const U8_CAP: usize = 2048;
// PRINTDLGEXW (x64)
const PDX_SIZE: usize = 136;
const PDX_OWNER: usize = 8;
const PDX_DEVMODE: usize = 16;
const PDX_DEVNAMES: usize = 24;
const PDX_DC: usize = 32;
const PDX_FLAGS: usize = 40;
const PDX_NRANGES: usize = 52;
const PDX_MAXRANGES: usize = 56;
const PDX_RANGES: usize = 64;
const PDX_MINPAGE: usize = 72;
const PDX_MAXPAGE: usize = 76;
const PD_PAGENUMS: u32 = 0x2;
const MAX_RANGES: usize = 16;
const PDX_COPIES: usize = 80;
const PDX_START: usize = 128;
const PDX_RESULT: usize = 132;
const PD_NOSELECTION: u32 = 0x4;
const PD_NOPAGENUMS: u32 = 0x8;
const PD_RETURNDC: u32 = 0x100;
const PD_USEDEVMODECOPIESANDCOLLATE: u32 = 0x40000;
const PD_NOCURRENTPAGE: u32 = 0x80_0000;
const PD_RESULT_PRINT: u32 = 1;
const START_PAGE_GENERAL: u32 = 0xFFFF_FFFF;
// DEVMODEW
const DM_SPEC: usize = 64;
const DM_SIZEF: usize = 68;
const DM_FIELDS: usize = 72;
const DM_ORIENT: usize = 76;
const DM_PAPER: usize = 78;
const DM_BYTES: usize = 220;
const DM_SPECVERSION: u16 = 0x401;
const DMF_ORIENT: u32 = 1;
const DMF_PAPER: u32 = 2;
const GMEM_MOVEABLE_ZERO: u32 = 0x42;
// GetDeviceCaps
const DC_HORZRES: i32 = 8;
const DC_VERTRES: i32 = 10;
const DC_LOGPIXELSX: i32 = 88;
const DC_LOGPIXELSY: i32 = 90;
const DC_PHYSW: i32 = 110;
const DC_PHYSH: i32 = 111;
const DC_OFFX: i32 = 112;
const DC_OFFY: i32 = 113;
const HALFTONE_MODE: i32 = 4;
const SRCCOPY_ROP: u32 = 0x00CC_0020;
// DMPAPER_* for PAPER_A4, _LETTER, _A5, _LEGAL, _A3
static WP_DMPAPER: [u16; 5] = [9, 1, 11, 5, 8];

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateDCW(driver: *const u16, device: *const u16, port: *const u16, dm: *const u8) -> isize;
    fn DeleteDC(hdc: isize) -> i32;
    fn StartDocW(hdc: isize, di: *const u8) -> i32;
    fn StartPage(hdc: isize) -> i32;
    fn EndPage(hdc: isize) -> i32;
    fn EndDoc(hdc: isize) -> i32;
    fn AbortDoc(hdc: isize) -> i32;
    fn SaveDC(hdc: isize) -> i32;
    fn RestoreDC(hdc: isize, n: i32) -> i32;
    fn SetStretchBltMode(hdc: isize, mode: i32) -> i32;
    fn SetBrushOrgEx(hdc: isize, x: i32, y: i32, old: *mut i32) -> i32;
    fn IntersectClipRect(hdc: isize, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn GetDeviceCaps(hdc: isize, index: i32) -> i32;
    fn StretchDIBits(hdc: isize, xd: i32, yd: i32, wd: i32, hd: i32, xs: i32, ys: i32, ws: i32, hs: i32,
                     bits: *const u8, bmi: *const u8, usage: u32, rop: u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExA(name: *const u8, file: isize, flags: u32) -> isize;
    fn GetProcAddress(m: isize, name: *const u8) -> usize;
    fn GlobalAlloc(flags: u32, n: usize) -> isize;
    fn GlobalLock(h: isize) -> *mut u8;
    fn GlobalUnlock(h: isize) -> i32;
    fn GlobalFree(h: isize) -> isize;
}

struct Wp {
    state: Cell<u8>, // 0 not loaded, 1 loaded
    dlg: Cell<usize>,
    defprn: Cell<usize>,
}

// print runs on the UI thread only (UiOnly refuses other threads)
static WP: crate::ui_only::UiOnly<Wp> = crate::ui_only::UiOnly::new(Wp { state: Cell::new(0), dlg: Cell::new(0), defprn: Cell::new(0) });

/// a function of a system DLL (0 = missing)
fn dll_proc(dll: &[u8], name: &[u8]) -> usize {
    // SAFETY: NUL-terminated names; both calls return 0 on failure.
    unsafe {
        let m = LoadLibraryExA(dll.as_ptr(), 0, 0x800) /* LOAD_LIBRARY_SEARCH_SYSTEM32 */;
        if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
    }
}

/// COM for the dialog, PrintDlgExW and GetDefaultPrinterW (both optional: the test variant
/// works without)  [wp_load]
fn wp_load() {
    let w = WP.get();
    if w.state.get() != 0 {
        return;
    }
    w.state.set(1);
    let co = dll_proc(b"ole32.dll\0", b"CoInitializeEx\0");
    if co != 0 {
        // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).
        let ci: unsafe extern "system" fn(usize, u32) -> i32 = unsafe { core::mem::transmute(co) };
        unsafe { ci(0, 6) };
    }
    w.dlg.set(dll_proc(b"comdlg32.dll\0", b"PrintDlgExW\0"));
    w.defprn.set(dll_proc(b"winspool.drv\0", b"GetDefaultPrinterW\0"));
}

/// UTF-8 -> NUL-terminated UTF-16, at most cap wide chars with the NUL  [utf8_to_w]
fn wide_cap(s: &str, cap: usize) -> Vec<u16> {
    let mut w: Vec<u16> = Vec::with_capacity(s.len().min(cap) + 1);
    let mut buf = [0u16; 2];
    for c in s.chars() {
        if c == '\0' {
            break;
        }
        let u = c.encode_utf16(&mut buf);
        if w.len() + u.len() > cap.saturating_sub(1) {
            break;
        }
        w.extend_from_slice(u);
    }
    w.push(0);
    w
}

/// dmDeviceName: the first 31 wide chars of name (the rest of the field stays 0)
fn set_device_name(dm: &mut [u8], name: &[u16]) {
    for (i, &c) in name.iter().take(31).enumerate() {
        dm[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
    }
}

/// round(v * dpi / 72) - offset  [wp_dev]
fn wp_dev(v: f64, dpi: f64, off: i32) -> i32 {
    cvtd(v * dpi / 72.0).wrapping_sub(off)
}

/// the absolute wide path of the jfgui_PRINT_TO value ('\' separators)  [wp_outpath]
fn wp_outpath(p: &str) -> Vec<u16> {
    let b = p.as_bytes();
    let mut s = String::new();
    let abs = b.first().is_some_and(|&c| c == b'/' || c == b'\\') || b.get(1) == Some(&b':');
    if !abs {
        s.push_str(&crate::cwd());
        s.push('/');
    }
    s.push_str(p);
    let cut = clip_cap(&s, U8_CAP);
    let mut w = wide_cap(&s[..cut], WPATH_CAP);
    for c in w.iter_mut() {
        if *c == b'/' as u16 {
            *c = b'\\' as u16;
        }
    }
    w
}

// str_cat's capacity: at most cap - 1 bytes, on a character boundary
fn clip_cap(s: &str, cap: usize) -> usize {
    let mut n = s.len().min(cap - 1);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    n
}

/// dmDeviceName = the default printer (GetDefaultPrinterW) -> true when found  [wp_default_name]
fn wp_default_name(dm: &mut [u8]) -> bool {
    let f = WP.get().defprn.get();
    if f == 0 {
        return false;
    }
    let mut name = vec![0u16; WPRN_CAP];
    let mut n: u32 = WPRN_CAP as u32;
    // SAFETY: GetDefaultPrinterW(buffer, &size) with the buffer's size in wide chars.
    let ok = unsafe {
        let g: unsafe extern "system" fn(*mut u16, *mut u32) -> i32 = core::mem::transmute(f);
        g(name.as_mut_ptr(), &mut n)
    };
    if ok == 0 {
        return false;
    }
    // 31 chars + NUL (the assembly copies 62 bytes, then writes the NUL at 62)
    for (i, &c) in name.iter().take(31).enumerate() {
        dm[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
    }
    dm[62] = 0;
    dm[63] = 0;
    true
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn rd64(b: &[u8], o: usize) -> isize {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[o..o + 8]);
    i64::from_le_bytes(v) as isize
}

fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr64(b: &mut [u8], o: usize, v: isize) {
    b[o..o + 8].copy_from_slice(&(v as i64).to_le_bytes());
}

/// GlobalFree hDevMode / hDevNames of a PRINTDLGEXW (hDC stays)  [wp_dlg_free]
fn wp_dlg_free(pdx: &mut [u8]) {
    for o in [PDX_DEVMODE, PDX_DEVNAMES] {
        let h = rd64(pdx, o);
        if h != 0 {
            // SAFETY: a handle the dialog (or we) allocated with GlobalAlloc.
            unsafe { GlobalFree(h) };
            wr64(pdx, o, 0);
        }
    }
}

/// print one image on one page (see the file header)  [plat_print]
pub fn print(job: &mut PrintJob) -> i32 {
    job.copies = 0;
    if !crate::policy::print_allowed() {
        return ERR_UNSUPPORTED;
    }
    let src = match job.src {
        Some(s) if !s.px.is_empty() && s.w > 0 && s.h > 0 => s,
        _ => return ERR_DIMS,
    };
    let mode = match src_mode(&src) {
        Some(m) => m,
        None => return ERR_UNSUPPORTED,
    };
    wp_load();
    let wtitle = wide_cap(if job.title.is_empty() || job.title.starts_with('\0') { "Bild" } else { job.title }, WTITLE_CAP);
    let mut docinfo = [0u8; 40];
    let dc = match open_dc(job.paper, job.flags, 0) {
        Ok(d) => d,
        Err(e) => return e,
    };
    job.copies = dc.copies;
    if !dc.wpath.is_empty() {
        wr64(&mut docinfo, 16, dc.wpath.as_ptr() as isize); // lpszOutput
    }
    let r = print_dc(job, &src, mode, dc.hdc, &wtitle, &mut docinfo); // (lpszOutput points into dc.wpath)
    close_dc(dc.hdc);
    r
}

// the row_white mode of a source format (B, G, R rows for GDI)
fn src_mode(src: &PrintSrc) -> Option<u32> {
    match src.fmt {
        PRINT_ARGB => Some(PRW_BGR),
        PRINT_PARGB => Some(PRW_BGR | PRW_PREMUL),
        PRINT_XRGB => Some(PRW_BGR | PRW_OPAQUE),
        _ => None,
    }
}

// a printer DC: the copies, the test variant's output path (lpszOutput; empty: the printer),
// the page ranges of the dialog
struct Dc {
    hdc: isize,
    copies: u32,
    wpath: Vec<u16>,
    ranges: Vec<(u32, u32)>,
}

// the printer DC of a job: the test variant (jfgui_PRINT_TO, no dialog) or the system dialog
// (needs the main window); pages > 0 offers a range of 1..=pages
fn open_dc(paper: u32, flags: u32, pages: u32) -> Result<Dc, i32> {
    // DEVMODE with the job's paper / orientation
    let mut dm = [0u8; 224];
    dm[DM_SPEC..DM_SPEC + 2].copy_from_slice(&DM_SPECVERSION.to_le_bytes());
    dm[DM_SIZEF..DM_SIZEF + 2].copy_from_slice(&(DM_BYTES as u16).to_le_bytes());
    wr32(&mut dm, DM_FIELDS, DMF_ORIENT | DMF_PAPER);
    let orient: u16 = if flags & PJF_LANDSCAPE != 0 { 2 } else { 1 };
    dm[DM_ORIENT..DM_ORIENT + 2].copy_from_slice(&orient.to_le_bytes());
    let paper = WP_DMPAPER[if paper >= PAPER_N { 0 } else { paper as usize }];
    dm[DM_PAPER..DM_PAPER + 2].copy_from_slice(&paper.to_le_bytes());
    match crate::env("jfgui_PRINT_TO").filter(|v| !v.is_empty()) {
        Some(to) => {
            // ---- test variant: a named printer into a file, no dialog
            let wpath = wp_outpath(&to);
            let pname = crate::env("jfgui_PRINT_PRINTER").filter(|v| !v.is_empty());
            let wprn = wide_cap(pname.as_deref().unwrap_or("Microsoft Print to PDF"), WPRN_CAP);
            set_device_name(&mut dm, &wprn);
            let drv = wide_cap("WINSPOOL", 16);
            // SAFETY: NUL-terminated driver / device names, a DEVMODEW of DM_BYTES bytes.
            let hdc = unsafe { CreateDCW(drv.as_ptr(), wprn.as_ptr(), core::ptr::null(), dm.as_ptr()) };
            if hdc == 0 {
                return Err(ERR_IO);
            }
            Ok(Dc { hdc, copies: 1, wpath, ranges: Vec::new() })
        }
        None => {
            // ---- the system dialog (needs the main window: never headless)
            let owner = super::hwnd_main();
            let dlg = WP.get().dlg.get();
            if owner == 0 || dlg == 0 {
                return Err(ERR_UNSUPPORTED);
            }
            // SAFETY: PrintDlgExW(PRINTDLGEXW*).
            let dlg: unsafe extern "system" fn(*mut u8) -> i32 = unsafe { core::mem::transmute(dlg) };
            let mut pdx = [0u8; PDX_SIZE];
            // PRINTPAGERANGE { nFromPage, nToPage } entries the dialog fills
            let mut rng = [0u32; MAX_RANGES * 2];
            wr32(&mut pdx, 0, PDX_SIZE as u32);
            wr64(&mut pdx, PDX_OWNER, owner);
            let nums = if pages > 0 { 0 } else { PD_NOPAGENUMS };
            wr32(&mut pdx, PDX_FLAGS, PD_RETURNDC | nums | PD_NOSELECTION | PD_NOCURRENTPAGE | PD_USEDEVMODECOPIESANDCOLLATE);
            if pages > 0 {
                rng[0] = 1;
                rng[1] = pages;
                wr32(&mut pdx, PDX_NRANGES, 1);
                wr32(&mut pdx, PDX_MAXRANGES, MAX_RANGES as u32);
                wr64(&mut pdx, PDX_RANGES, rng.as_mut_ptr() as isize);
                wr32(&mut pdx, PDX_MINPAGE, 1);
                wr32(&mut pdx, PDX_MAXPAGE, pages);
            }
            wr32(&mut pdx, PDX_START, START_PAGE_GENERAL);
            wr32(&mut pdx, PDX_COPIES, 1);
            if wp_default_name(&mut dm) {
                // SAFETY: a movable block of DM_BYTES, filled while locked.
                unsafe {
                    let h = GlobalAlloc(GMEM_MOVEABLE_ZERO, DM_BYTES);
                    if h != 0 {
                        wr64(&mut pdx, PDX_DEVMODE, h);
                        let p = GlobalLock(h);
                        if !p.is_null() {
                            core::ptr::copy_nonoverlapping(dm.as_ptr(), p, DM_BYTES);
                            GlobalUnlock(h);
                        }
                    }
                }
            }
            // SAFETY: a valid PRINTDLGEXW (its page ranges point into rng, alive until the end
            // of this block); the dialog is modal over our window.
            if unsafe { dlg(pdx.as_mut_ptr()) } != 0 {
                // rejected (e.g. the DEVMODE): once more with the defaults
                wp_dlg_free(&mut pdx);
                wr64(&mut pdx, PDX_DEVMODE, 0);
                // SAFETY: as above.
                if unsafe { dlg(pdx.as_mut_ptr()) } != 0 {
                    wp_dlg_free(&mut pdx);
                    return Err(ERR_IO);
                }
            }
            let hdc = rd64(&pdx, PDX_DC);
            if rd32(&pdx, PDX_RESULT) != PD_RESULT_PRINT {
                wp_dlg_free(&mut pdx);
                close_dc(hdc);
                return Err(ERR_CANCEL);
            }
            let copies = rd32(&pdx, PDX_COPIES);
            let mut ranges = Vec::new();
            if pages > 0 && rd32(&pdx, PDX_FLAGS) & PD_PAGENUMS != 0 {
                let n = (rd32(&pdx, PDX_NRANGES) as usize).min(MAX_RANGES);
                for k in 0..n {
                    let (a, b) = (rng[k * 2], rng[k * 2 + 1]);
                    ranges.push((a.min(b), a.max(b)));
                }
            }
            wp_dlg_free(&mut pdx);
            if hdc == 0 {
                return Err(ERR_IO);
            }
            Ok(Dc { hdc, copies, wpath: Vec::new(), ranges })
        }
    }
}

fn close_dc(hdc: isize) {
    if hdc != 0 {
        // SAFETY: a DC from CreateDCW / PrintDlgExW.
        unsafe { DeleteDC(hdc) };
    }
}

// the page geometry of a printer DC: the page in pt, dpi x / y, the printable area's offset
fn dc_page(hdc: isize) -> Result<(PPage, f64, f64, i32, i32), i32> {
    // SAFETY: hdc is a valid printer DC.
    let caps = |i: i32| unsafe { GetDeviceCaps(hdc, i) };
    let lx = caps(DC_LOGPIXELSX);
    if lx <= 0 {
        return Err(ERR_IO);
    }
    let ly = caps(DC_LOGPIXELSY);
    if ly <= 0 {
        return Err(ERR_IO);
    }
    let (dpix, dpiy) = (lx as f64, ly as f64);
    let hres = caps(DC_HORZRES);
    let vres = caps(DC_VERTRES);
    if hres <= 0 || vres <= 0 {
        return Err(ERR_IO);
    }
    let mut physw = caps(DC_PHYSW);
    let mut physh = caps(DC_PHYSH);
    let mut offx = caps(DC_OFFX);
    let mut offy = caps(DC_OFFY);
    if physw < hres {
        // no physical size (not a printer): the area
        physw = hres;
        offx = 0;
    }
    if physh < vres {
        physh = vres;
        offy = 0;
    }
    // PPAGE in pt: device px * 72 / dpi
    let px = 72.0 / dpix;
    let py = 72.0 / dpiy;
    let page = PPage {
        w: physw as f64 * px,
        h: physh as f64 * py,
        l: offx as f64 * px,
        t: offy as f64 * py,
        r: physw.wrapping_sub(offx).wrapping_sub(hres) as f64 * px,
        b: physh.wrapping_sub(offy).wrapping_sub(vres) as f64 * py,
        token: 0,
        flags: if physw > physh { PJF_LANDSCAPE } else { 0 },
    };
    Ok((page, dpix, dpiy, offx, offy))
}

// the page geometry, the document and the bands on the printer DC  [plat_print .wp_have_dc]
fn print_dc(job: &mut PrintJob, src: &crate::PrintSrc, mode: u32, hdc: isize, wtitle: &[u16], docinfo: &mut [u8; 40]) -> i32 {
    let (mut page, dpix, dpiy, offx, offy) = match dc_page(hdc) {
        Ok(g) => g,
        Err(e) => return e,
    };
    page.flags = 0;
    job.page_w = page.w;
    job.page_h = page.h;
    let pl = layout(job, &page);
    // ---- the document
    wr32(docinfo, 0, 40);
    wr64(docinfo, 8, wtitle.as_ptr() as isize);
    // SAFETY (the GDI calls below): hdc is a valid printer DC; every pointer handed to GDI points
    // into live buffers of this frame.
    if unsafe { StartDocW(hdc, docinfo.as_ptr()) } <= 0 {
        return ERR_IO;
    }
    let r = dc_draw_page(hdc, src, mode, &pl, dpix, dpiy, offx, offy);
    if r != 0 {
        return r; // (aborted)
    }
    if unsafe { EndDoc(hdc) } <= 0 {
        return ERR_IO;
    }
    0
}

// one page of a started document: StartPage, the image in bands into layout()'s rectangle
// clipped to the printable area, EndPage -> 0 / ERR_* (the document aborted)
#[allow(clippy::too_many_arguments)]
fn dc_draw_page(hdc: isize, src: &crate::PrintSrc, mode: u32, pl: &Place, dpix: f64, dpiy: f64, offx: i32, offy: i32) -> i32 {
    // device rectangles (printable-area origin): v * dpi / 72 - offset; sizes = the far edge
    // minus the near one (bands abut exactly)
    let dx = wp_dev(pl.x, dpix, offx);
    let dy = wp_dev(pl.y, dpiy, offy);
    let cx0 = wp_dev(pl.ax, dpix, offx);
    let cy0 = wp_dev(pl.ay, dpiy, offy);
    let dw = wp_dev(pl.x + pl.w, dpix, offx).wrapping_sub(dx).max(1);
    let dh = wp_dev(pl.y + pl.h, dpiy, offy).wrapping_sub(dy).max(1);
    let cx1 = wp_dev(pl.ax + pl.aw, dpix, offx);
    let cy1 = wp_dev(pl.ay + pl.ah, dpiy, offy);
    // band buffer: 24-bit rows padded to 4 bytes
    let (w, h) = (src.w, src.h);
    let rowb = ((w as u32) * 3 + 3) & !3;
    let band = (BAND_BYTES / rowb).max(1).min(h as u32) as usize;
    let rowb = rowb as usize;
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(rowb * band).is_err() {
        // SAFETY: a started document of hdc.
        unsafe { AbortDoc(hdc) };
        return ERR_NOMEM;
    }
    buf.resize(rowb * band, 0);
    // SAFETY (whole function): hdc is a valid printer DC with a started document; every pointer
    // handed to GDI points into live buffers of this frame.
    if unsafe { StartPage(hdc) } <= 0 {
        unsafe { AbortDoc(hdc) };
        return ERR_IO;
    }
    let saved = unsafe { SaveDC(hdc) };
    unsafe {
        SetStretchBltMode(hdc, HALFTONE_MODE);
        SetBrushOrgEx(hdc, 0, 0, core::ptr::null_mut());
        IntersectClipRect(hdc, cx0, cy0, cx1, cy1);
    }
    // BITMAPINFOHEADER: bottom-up 24-bit
    let mut bmi = [0u8; 48];
    wr32(&mut bmi, 0, 40);
    wr32(&mut bmi, 4, w as u32);
    wr32(&mut bmi, 12, 0x0018_0001); // planes 1, 24 bits
    let stride = src.stride as isize;
    let mut r = 0i32;
    while r < h {
        let r0 = r;
        let r1 = (r0 + band as i32).min(h);
        r = r1;
        // y0 = dy + r0 * dh / h, y1 = dy + r1 * dh / h
        let y0 = ((dh as i64 * r0 as i64) / h as i64) as i32;
        let y0 = y0.wrapping_add(dy);
        let y1 = ((dh as i64 * r1 as i64) / h as i64) as i32;
        let y1 = y1.wrapping_add(dy);
        if y1 <= y0 || y1 <= cy0 || y0 >= cy1 {
            continue; // empty or outside the clip
        }
        // rows r0 .. r1-1, bottom-up
        let n = (r1 - r0) as usize;
        wr32(&mut bmi, 8, n as u32);
        for i in 0..n {
            let d = (n - i - 1) * rowb;
            let o = ((r0 as isize + i as isize) * stride) as usize;
            let s = src.px.get(o..o + w as usize * 4).unwrap_or(&[]);
            row_white(&mut buf[d..d + rowb], s, w as usize, mode);
        }
        let ok = unsafe {
            StretchDIBits(hdc, dx, y0, dw, y1 - y0, 0, 0, w, n as i32, buf.as_ptr(), bmi.as_ptr(), 0, SRCCOPY_ROP)
        };
        if ok == 0 {
            unsafe { AbortDoc(hdc) };
            return ERR_IO;
        }
    }
    if saved != 0 {
        unsafe { RestoreDC(hdc, saved) };
    }
    if unsafe { EndPage(hdc) } <= 0 {
        unsafe { AbortDoc(hdc) };
        return ERR_IO;
    }
    0
}

// ---------------------------------------------------------------- PrintDoc (E9)

/// (print.rs) PrintDoc::begin: the dialog or the test variant's printer, the page geometry,
/// StartDocW  -> 0 / ERR_*
pub(crate) fn doc_begin(d: &mut PrintDoc) -> i32 {
    wp_load();
    let dc = match open_dc(d.paper, d.flags, d.pages) {
        Ok(dc) => dc,
        Err(e) => return e,
    };
    let (page, dpix, dpiy, offx, offy) = match dc_page(dc.hdc) {
        Ok(g) => g,
        Err(e) => {
            close_dc(dc.hdc);
            return e;
        }
    };
    d.page = page;
    d.copies = dc.copies;
    d.sys.hdc = dc.hdc;
    d.sys.dpi = [dpix, dpiy];
    d.sys.off = [offx, offy];
    d.sys.ranges = dc.ranges;
    let wtitle = wide_cap(&d.title, WTITLE_CAP);
    let mut docinfo = [0u8; 40];
    wr32(&mut docinfo, 0, 40);
    wr64(&mut docinfo, 8, wtitle.as_ptr() as isize);
    if !dc.wpath.is_empty() {
        wr64(&mut docinfo, 16, dc.wpath.as_ptr() as isize); // lpszOutput
    }
    // SAFETY: a printer DC; the DOCINFOW's strings live until the call returns.
    if unsafe { StartDocW(dc.hdc, docinfo.as_ptr()) } <= 0 {
        close_dc(dc.hdc);
        d.sys.hdc = 0;
        return ERR_IO;
    }
    d.sys.started = true;
    0
}

/// (print.rs) PrintDoc::add_image: one page of the document -> 0 / ERR_*
pub(crate) fn doc_image(d: &mut PrintDoc, src: &PrintSrc, pl: &Place) -> i32 {
    if !d.sys.started {
        return ERR_IO;
    }
    let Some(mode) = src_mode(src) else { return ERR_UNSUPPORTED };
    let r = dc_draw_page(d.sys.hdc, src, mode, pl, d.sys.dpi[0], d.sys.dpi[1], d.sys.off[0], d.sys.off[1]);
    if r != 0 {
        d.sys.started = false; // (aborted)
    }
    r
}

/// (print.rs) PrintDoc::end: EndDoc -> 0 / ERR_IO
pub(crate) fn doc_end(d: &mut PrintDoc) -> i32 {
    let mut r = ERR_IO;
    if d.sys.started {
        // SAFETY: the started document of our DC.
        r = if unsafe { EndDoc(d.sys.hdc) } > 0 { 0 } else { ERR_IO };
        d.sys.started = false;
    }
    close_dc(d.sys.hdc);
    d.sys.hdc = 0;
    r
}

/// (print.rs) PrintDoc::abort and a job without pages: AbortDoc
pub(crate) fn doc_abort(d: &mut PrintDoc) {
    if d.sys.started {
        // SAFETY: the started document of our DC.
        unsafe { AbortDoc(d.sys.hdc) };
        d.sys.started = false;
    }
    close_dc(d.sys.hdc);
    d.sys.hdc = 0;
}
