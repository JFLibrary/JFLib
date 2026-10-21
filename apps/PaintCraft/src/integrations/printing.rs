// print -- PaintCraft: Datei > Drucken… (Strg+P, CMD_PRINT; agent PR1).  paint.net 5.1 prints
// the flattened image through the Windows photo printing wizard; here an own modal dialog
// (window WINID_PRINT) with a page preview and the options, then plat_print (library: system
// print dialog on Windows, xdg-desktop-portal Print on Linux; docs/modules/PRINT.md):
//   Papierformat    A4 / Letter / A5 / Legal / A3 (suggested to the system dialog, preview)
//   Ausrichtung     Hochformat / Querformat
//   Druckgröße      An Seite anpassen / Originalgröße (Pixeldichte of the image) / Prozentual
//   Zentrieren, Rand (mm)
// "Drucken…" closes the dialog and prints the composite image (alpha over white); errors go
// to the error bar.  Options live for the session.  Hooks in fileio.s (marked PR1):
//   prt_states()            Drucken… enabled while an image is open (fio_states)
//   prt_frame()             the dialog window (fio_frame)
//   prt_cmd(id, arg) -> eax CMD_PRINT: open the dialog; CMD_PRT_DONE: arg 1 print, 0 cancel
// Widgets ID_DLG_PRINT + 1.. (see below).
//                                              [apps/PaintCraft/src/print.s]

use alloc::format;
use alloc::rc::Rc;

use gfx::{Surface, FMT_PARGB};
use plat::{Place, PrintJob, PrintSrc, PPage};
use rt::fx::{cvtd, maxsd, minsd};
use ui::*;

use crate::document::composite::{comp_sf, comp_update, thumb_render};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_count, doc_index, DocId};
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const WINID_PRINT: u32 = WINID_DLG0 + 48; // 0x6040
const ID_PR_PAPER: u32 = ID_DLG_PRINT + 1;
const ID_PR_PORTRAIT: u32 = ID_DLG_PRINT + 2;
const ID_PR_LANDSCAPE: u32 = ID_DLG_PRINT + 3;
const ID_PR_FIT: u32 = ID_DLG_PRINT + 4;
const ID_PR_DPI: u32 = ID_DLG_PRINT + 5;
const ID_PR_PCT_RB: u32 = ID_DLG_PRINT + 6;
const ID_PR_PCT: u32 = ID_DLG_PRINT + 7;
const ID_PR_CENTER: u32 = ID_DLG_PRINT + 8;
const ID_PR_MARGIN: u32 = ID_DLG_PRINT + 9;
const ID_PR_PREVIEW: u32 = ID_DLG_PRINT + 10;
const PREV_DIP: i32 = 240; // preview square
const THUMB_DIP: i32 = 200; // thumbnail's long side
const PAD_DIP: i32 = 12;
const FIELD_W: i32 = 72;
const INFO_CAP: usize = 160;
const PAPER_AREA_LINE: u32 = 0xFFD4D4D4; // preview: printable area outline on the paper
const PAPER_IMG_LINE: u32 = 0xFFA0A0A0; // preview: image outline
const C_PR_CM100: f64 = 3.5277777777777777; // pt -> 1/100 cm (254 / 72)
const RANGE_PCT: Range = Range::new(1, 1000, 1, 0);
const RANGE_MARGIN: Range = Range::new(0, 50, 1, 0);
static PAPER_LIST: [&str; 5] =
    ["A4 (210 × 297 mm)", "Letter (8,5 × 11 in)", "A5 (148 × 210 mm)", "Legal (8,5 × 14 in)", "A3 (297 × 420 mm)"];

/// the state of print.s (its g_* globals)
pub(crate) struct Prt {
    rect: Shared<WinRect>,
    pct: i32,
    center: i32,
    margin: i32,
    /// g_pr_k: preview px per pt
    k: f64,
    doc: Option<DocId>,
    page: PPage,
    place: Place,
    /// g_pr_job's derived values (pr_job): dpi_x of the document, the composite's size
    dpi_x: f64,
    src_w: i32,
    src_h: i32,
    thumb: Option<Rc<Surface>>,
    /// SF_AUX of g_pr_thumb (the renderer's image version)
    thumb_ver: u32,
    open: Shared<i32>,
    pending: bool,
    changed: bool,
    /// g_pr_result: last plat_print result
    result: i32,
    paper: i32,
    orient: i32,
    scale: i32,
}

impl Default for Prt {
    fn default() -> Prt {
        Prt {
            rect: Shared::new(WinRect::new(0, 0, 560, 0, 0)),
            pct: 100,
            center: 1,
            margin: 5,
            k: 0.0,
            doc: None,
            page: PPage::default(),
            place: Place::default(),
            dpi_x: 0.0,
            src_w: 0,
            src_h: 0,
            thumb: None,
            thumb_ver: 0,
            open: Shared::new(0),
            pending: false,
            changed: false,
            result: 0,
            paper: 0,
            orient: 0,
            scale: 0,
        }
    }
}

/// prt_states() -- Drucken… only with an image
pub(crate) fn prt_states(p: &mut Pn, ui: &mut Ui) {
    let f = if doc_count(&p.doc) != 0 { 0 } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_PRINT, f);
}

/// prt_cmd(id, arg) -> eax 1
pub(crate) fn prt_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == CMD_PRINT {
        pr_open(p, ui);
    } else {
        pr_done(p, ui, arg != 0);
    }
    true
}

const S_NOMEM: &str = "Der Arbeitsspeicher reicht für diesen Schritt nicht aus.";

// pr_open() -- show the dialog for the active image (ignored while it is open)
fn pr_open(p: &mut Pn, ui: &mut Ui) {
    if p.prt.pending {
        return;
    }
    let Some(doc) = doc_active(&p.doc) else { return };
    p.prt.doc = Some(doc);
    comp_update(p, doc);
    let sf = comp_sf(p.doc.d(doc));
    if sf.px.is_empty() {
        crate::integrations::file_io::fio_error(p, Some("Drucken"), S_NOMEM);
        return;
    }
    // thumbnail over white (a checkerboard cell larger than any image), long side THUMB_DIP
    let mut w = ui.dp(THUMB_DIP);
    let mut h = w;
    if sf.w >= sf.h {
        h = h.wrapping_mul(sf.h) / sf.w;
    } else {
        w = w.wrapping_mul(sf.w) / sf.h;
    }
    let w = w.max(1);
    let h = h.max(1);
    let g = &mut p.prt;
    g.thumb = None;
    let mut t = Surface::new(w, h, FMT_PARGB);
    if !t.px.is_empty() {
        thumb_render(&mut t, sf, 0x10000);
        g.thumb_ver = g.thumb_ver.wrapping_add(1);
        g.thumb = Some(Rc::new(t));
    }
    g.open.set(1);
    g.pending = true;
    ui.repass();
}

// pr_done(ok) -- close the dialog; ok = 1: print the image
fn pr_done(p: &mut Pn, ui: &mut Ui, ok: bool) {
    let g = &mut p.prt;
    g.pending = false;
    g.open.set(0);
    g.thumb = None;
    ui.repass();
    if !ok {
        return;
    }
    let Some(doc) = p.prt.doc else { return };
    if doc_index(&p.doc, doc) < 0 {
        return;
    }
    comp_update(p, doc);
    let d = p.doc.d(doc);
    let sf = comp_sf(d);
    let msg = if sf.px.is_empty() {
        S_NOMEM
    } else {
        let mut job = pr_job(&p.prt, d.name.as_str(), d.dpu_unit, d.dpu_x, d.dpu_y, Some(sf));
        let r = plat::print(&mut job);
        p.prt.result = r;
        if r == 0 || r == plat::ERR_CANCEL {
            return;
        }
        if r == plat::ERR_NOMEM {
            S_NOMEM
        } else if r == plat::ERR_UNSUPPORTED {
            "Es ist kein Drucker bzw. Druckdienst verfügbar."
        } else {
            "Das Drucken ist fehlgeschlagen."
        }
    };
    crate::integrations::file_io::fio_error(p, Some("Drucken"), msg);
}

// pr_job(doc) -- g_pr_job from the dialog's options and the document (composite, name, dpi)
fn pr_job<'a>(g: &Prt, name: &'a str, unit: u8, dx: f64, dy: f64, sf: Option<&'a Surface>) -> PrintJob<'a> {
    let src = sf.map(|s| PrintSrc { px: &s.px, w: s.w, h: s.h, stride: s.stride, fmt: s.fmt });
    let mut job = PrintJob::new(src, name);
    job.dpi_x = plat::print_dpi(unit as u32, dx);
    job.dpi_y = plat::print_dpi(unit as u32, dy);
    let mut f = 0;
    if g.orient != 0 {
        f |= plat::PJF_LANDSCAPE;
    }
    if g.center != 0 {
        f |= plat::PJF_CENTER;
    }
    job.flags = f;
    job.scale = g.scale as u32;
    job.percent = g.pct as u32;
    job.margin = (g.margin as u32).wrapping_mul(100);
    job.paper = g.paper as u32;
    job
}

// pr_map(v, origin) -> origin + round(v * g_pr_k)
fn pr_map(k: f64, v: f64, origin: i32) -> i32 {
    cvtd(v * k).wrapping_add(origin)
}

// pr_rect(x, y, x1, y1) -- ui_draw_rect of the corners (w / h at least 1)
fn pr_rect(ui: &mut Ui, x: i32, y: i32, x1: i32, y1: i32) {
    ui.draw_rect(x, y, (x1.wrapping_sub(x)).max(1), (y1.wrapping_sub(y)).max(1));
}

// pr_outline(x0, y0, x1, y1) -- the four 1 px edges of a rectangle
fn pr_outline(ui: &mut Ui, x0: i32, y0: i32, x1: i32, y1: i32) {
    pr_rect(ui, x0, y0, x1, y0 + 1);
    pr_rect(ui, x0, y1 - 1, x1, y1);
    pr_rect(ui, x0, y0, x0 + 1, y1);
    pr_rect(ui, x1 - 1, y0, x1, y1);
}

// pr_preview(rect) -- the page (white, fitted into the canvas), the printable area (outline)
// and the image clipped to it, from g_pr_page / g_pr_place
fn pr_preview(g: &mut Prt, ui: &mut Ui, r: [i32; 4]) {
    let (rx, ry, rw, rh) = (r[0], r[1], r[2], r[3]);
    let c = ui.color(T_BG_ALT);
    ui.draw_color(c);
    ui.draw_rect(rx, ry, rw, rh);
    // scale: the page into the canvas minus a padding
    let pad = ui.dp(PAD_DIP * 2);
    let a = (rw.wrapping_sub(pad)) as f64 / g.page.w;
    let b = (rh.wrapping_sub(pad)) as f64 / g.page.h;
    g.k = maxsd(minsd(a, b), 0.0);
    let k = g.k;
    // page origin: centred
    let pw = pr_map(k, g.page.w, 0);
    let ph = pr_map(k, g.page.h, 0);
    let px = (rw.wrapping_sub(pw) >> 1).wrapping_add(rx);
    let py = (rh.wrapping_sub(ph) >> 1).wrapping_add(ry);
    // frame + white paper
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    ui.draw_rect(px - 1, py - 1, pw + 2, ph + 2);
    ui.draw_color(0xFFFFFFFF);
    ui.draw_rect(px, py, pw, ph);
    // printable area
    let pl = g.place;
    let ax0 = pr_map(k, pl.ax, px);
    let ay0 = pr_map(k, pl.ay, py);
    let ax1 = pr_map(k, pl.ax + pl.aw, px);
    let ay1 = pr_map(k, pl.ay + pl.ah, py);
    ui.draw_clip(ax0, ay0, ax1 - ax0, ay1 - ay0);
    // the image
    if let Some(t) = &g.thumb {
        let ix0 = pr_map(k, pl.x, px);
        let iy0 = pr_map(k, pl.y, py);
        let ix1 = pr_map(k, pl.x + pl.w, px);
        let iy1 = pr_map(k, pl.y + pl.h, py);
        ui.draw_image(t, g.thumb_ver, ix0, iy0, (ix1 - ix0).max(1), (iy1 - iy0).max(1));
        ui.draw_color(PAPER_IMG_LINE); // (the paper is white in both themes)
        pr_outline(ui, ix0, iy0, ix1, iy1);
    }
    ui.draw_unclip();
    // outline of the printable area
    ui.draw_color(PAPER_AREA_LINE);
    pr_outline(ui, ax0, ay0, ax1, ay1);
}

// pr_cm100(pt) -> hundredths of a centimetre
fn pr_cm100(pt: f64) -> u32 {
    cvtd(pt * C_PR_CM100) as u32
}

// pr_info() -- the two lines under the options: image size / dpi, print size
fn pr_info(g: &Prt, ui: &mut Ui) {
    let s = fmt_cap(format!("Bild: {} × {} Pixel, {} dpi", g.src_w as u32, g.src_h as u32, cvtd(g.dpi_x) as u32), INFO_CAP);
    ui.text(TS_BODY | TX_SECONDARY, &s);
    let w = pr_cm100(g.place.w);
    let h = pr_cm100(g.place.h);
    let s = fmt_cap(format!("Druckgröße: {},{:02} × {},{:02} cm", w / 100, w % 100, h / 100, h % 100), INFO_CAP);
    ui.text(TS_BODY | TX_SECONDARY, &s);
}

// pr_row_begin() -- a horizontal row (centred on the cross axis, gap 8)
fn pr_row_begin(ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
}

// pr_changed(ev) -- remember EV_ACT of a widget (another pass shows its effect)
fn pr_changed(g: &mut Prt, ev: u32) {
    if ev & EV_ACT != 0 {
        g.changed = true;
    }
}

/// prt_frame() -- the dialog window
pub(crate) fn prt_frame(p: &mut Pn, ui: &mut Ui) {
    if p.prt.pending {
        let gone = p.prt.doc.is_none_or(|d| doc_index(&p.doc, d) < 0);
        if gone || p.prt.open.get() == 0 {
            // Esc / close button / the image was closed: Abbrechen
            p.prt.pending = false;
            p.prt.open.set(0);
            ui.cmd_run(CMD_PRT_DONE, 0);
        }
    }
    let open = p.prt.open.clone();
    let rect = p.prt.rect.clone();
    if ui.window_begin(WINID_PRINT, crate::preferences::localization::tr("Drucken"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        pr_body(p, ui);
    }
    ui.window_end();
}

// the dialog's content (prt_frame between ui_window_begin and ui_window_end)
fn pr_body(p: &mut Pn, ui: &mut Ui) {
    // the layout of the current options (preview, info)
    if let Some(doc) = p.prt.doc.filter(|&d| doc_index(&p.doc, d) >= 0) {
        comp_update(p, doc);
        let d = p.doc.d(doc);
        let sf = comp_sf(d);
        let g = &mut p.prt;
        let job = pr_job(g, d.name.as_str(), d.dpu_unit, d.dpu_x, d.dpu_y, Some(sf));
        g.page = plat::page_init(g.paper as u32, job.flags);
        g.place = plat::layout(&job, &g.page);
        g.dpi_x = job.dpi_x;
        g.src_w = sf.w;
        g.src_h = sf.h;
    }
    let g = &mut p.prt;
    // preview | options
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(PREV_DIP, PREV_DIP);
    let r = ui.canvas(ID_PR_PREVIEW, None, 0, 0).rect;
    pr_preview(g, ui, r);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Papierformat"));
    ui.next_size(-1, 0);
    let ev = ui.combobox(ID_PR_PAPER, &mut g.paper, &PAPER_LIST);
    pr_changed(g, ev);
    ui.spacer(8);
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Ausrichtung"));
    let ev = ui.radio(ID_PR_PORTRAIT, "Hochformat", &mut g.orient, 0);
    pr_changed(g, ev);
    let ev = ui.radio(ID_PR_LANDSCAPE, "Querformat", &mut g.orient, 1);
    pr_changed(g, ev);
    ui.spacer(8);
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Druckgröße"));
    let ev = ui.radio(ID_PR_FIT, "An Seite anpassen", &mut g.scale, plat::PJS_FIT as i32);
    pr_changed(g, ev);
    let ev = ui.radio(ID_PR_DPI, "Originalgröße (Pixeldichte)", &mut g.scale, plat::PJS_DPI as i32);
    pr_changed(g, ev);
    pr_row_begin(ui);
    let ev = ui.radio(ID_PR_PCT_RB, "Prozentual:", &mut g.scale, plat::PJS_PERCENT as i32);
    pr_changed(g, ev);
    let pct_on = g.scale == plat::PJS_PERCENT as i32;
    if !pct_on {
        ui.disabled_push();
    }
    ui.next_size(FIELD_W, 0);
    let ev = ui.number_box(ID_PR_PCT, &mut g.pct, &RANGE_PCT);
    pr_changed(g, ev);
    ui.text(TS_BODY, crate::preferences::localization::tr("%"));
    if !pct_on {
        ui.disabled_pop();
    }
    ui.box_end();
    ui.spacer(8);
    let ev = ui.checkbox(ID_PR_CENTER, crate::preferences::localization::tr("Zentrieren"), &mut g.center);
    pr_changed(g, ev);
    pr_row_begin(ui);
    ui.text(TS_BODY, crate::preferences::localization::tr("Rand:"));
    ui.next_size(FIELD_W, 0);
    let ev = ui.number_box(ID_PR_MARGIN, &mut g.margin, &RANGE_MARGIN);
    pr_changed(g, ev);
    ui.text(TS_BODY, crate::preferences::localization::tr("mm"));
    ui.box_end();
    ui.box_end(); // options
    ui.box_end(); // preview | options
    ui.spacer(8);
    pr_info(g, ui);
    if g.changed {
        g.changed = false;
        ui.repass();
    }
    let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("Drucken…")), None, Some(crate::preferences::localization::tr("Abbrechen")));
    if r != DLG_NONE {
        p.prt.pending = false;
        ui.cmd_run(CMD_PRT_DONE, (r == DLG_PRIMARY) as u32);
    }
}
