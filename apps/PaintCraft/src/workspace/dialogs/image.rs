// dlg_image -- PaintCraft: the Bild dialogs (agent A5, sub-module of imgops.rs) as modal
// dialog windows with the layout of paint.net 5.1:
//   "Größe ändern" (Strg+R):   Aktuelle Bildgröße / Neue Größe (memory of all layers),
//       Prozentual: n % | Absolut: (Seitenverhältnis beibehalten, Pixelgröße Breite / Höhe /
//       Pixeldichte in Pixel/in | Pixel/cm, Druckgröße Breite / Höhe in Zoll | Zentimeter),
//       Optionen: Interpolation (Adaptiv (scharf), Lanczos, Bikubisch, Bikubisch (glatt),
//       Bilineare, Bilinear (schnell), Fant, Nächster Nachbar) and "Verwenden Sie
//       die Gammakorrektur".
//   "Leinwandgröße" (Strg+Umschalt+R): the same size part, Anker (combo box + 3 x 3 grid
//       with arrows pointing away from the anchor), Fülleffekt (Sekundärfarbe, Grundfarbe,
//       Transparent, Schwarz, Weiß: the new area of the background layer; other layers get
//       transparent pixels, paint.net's rule).
// The resolution unit and the print unit are linked (one variable: inch / centimetre);
// changing the resolution keeps the pixel size (the print size follows), a print size gives
// the pixel size.  Settings are remembered per dialog for the session (on OK).
//   dimg_resize_open(doc) / dimg_canvas_open(doc)   show a dialog for the image
//   dimg_frame()                                    the windows (iop_frame calls it)
//   dimg_fill_argb(choice)                          FILLC_* -> straight ARGB
//   DImg::par                                       IOPPAR written on OK, then the command
//                                                   CMD_IOP_RESIZE_DO / CMD_IOP_CANVAS_DO
// Windows WINID_RESIZE / WINID_CANVAS (WINID_DLG0 + 24 / 25), widgets ID_DLG_RESIZE +
// DI_* / ID_DLG_CANVAS + DI_* (pn_iop.inc).
//                                              [apps/PaintCraft/src/dlg_image.s]

use alloc::format;
use alloc::string::String;

use img_core::resample::{RS_ADAPTIVE, RS_BICUBIC, RS_BICUBIC_SMOOTH, RS_BILINEAR, RS_BILINEAR_LQ, RS_FANT, RS_GAMMA, RS_LANCZOS3, RS_NEAREST};
use rt::fx::{cvtd, maxsd, minsd};
use ui::*;

use crate::app::constants::*;
use crate::document::model::DocId;
use crate::workspace::palette::{pal_primary, pal_secondary};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const DK_RESIZE: i32 = 0;
const DK_CANVAS: i32 = 1;
const LABEL_W: i32 = 110; // DIP, like dlg_new.rs
const FIELD_W: i32 = 120;
const UNIT_W: i32 = 130;
const WIDE_W: i32 = FIELD_W + 8 + UNIT_W;
const CELL: i32 = 32; // anchor grid cell (DIP)
const ANCH_W: i32 = 180; // anchor / fill combo boxes
const GAP: i32 = 8; // rows of the dialogs (the content box has 12)
const MAXDIM: i32 = 65535;
const TXT_CAP: usize = 96;
const RSQRT2: f64 = 0.70710678118654752;
const RANGE_DIM: Range = Range::new(1, MAXDIM, 1, 0);
const RANGE_PCT: Range = Range::new(1, 10000, 1, 0);
const RANGEF_RES: RangeF = RangeF::new(0.01, 100000.0, 0.01, 2, 0);
const RANGEF_PRINT: RangeF = RangeF::new(0.01, 100000.0, 0.01, 2, 0);
static ANCHOR_NAMES: [&str; 9] =
    ["Links oben", "Oben", "Rechts oben", "Links", "Mitte", "Rechts", "Links unten", "Unten", "Rechts unten"];
// combo box index -> RS_* (the order of FILTER_LIST)
static FILTER_MAP: [u32; 8] = [RS_ADAPTIVE, RS_LANCZOS3, RS_BICUBIC, RS_BICUBIC_SMOOTH, RS_BILINEAR, RS_BILINEAR_LQ, RS_FANT, RS_NEAREST];
static FILTER_LIST: [&str; 8] = [
    "Adaptiv (scharf)",
    "Lanczos",
    "Bikubisch",
    "Bikubisch (glatt)",
    "Bilineare",
    "Bilinear (schnell)",
    "Fant",
    "Nächster Nachbar",
];
static RESUNIT_LIST: [&str; 2] = ["Pixel/in", "Pixel/cm"];
static PUNIT_LIST: [&str; 2] = ["Zoll", "Zentimeter"];
static FILL_LIST: [&str; 5] = ["Sekundärfarbe", "Grundfarbe", "Transparent", "Schwarz", "Weiß"];

/// the parameter record of the Bild dialogs (IOPPAR, g_iop_par; imgops.rs reads it)
#[derive(Clone, Copy, Default)]
pub(crate) struct IopPar {
    /// IP_W / IP_H: new size (1 .. 65535)
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// IP_FILTER: RS_* (Größe ändern)
    pub(crate) filter: u32,
    /// IP_FLAGS: RS_GAMMA
    pub(crate) flags: u32,
    /// IP_ANCHOR: AN_* (Leinwandgröße)
    pub(crate) anchor: u32,
    /// IP_FILL: straight ARGB for the new area of background layers
    pub(crate) fill: u32,
    /// IP_RESSET: 1 = set the resolution below
    pub(crate) resset: u32,
    /// IP_UNIT: DOC_DPU_UNIT value (1 inch, 2 centimetre)
    pub(crate) unit: u32,
    /// IP_DPU: dots per unit
    pub(crate) dpu: f64,
}

/// the Bild dialogs (dlg_image.s)
pub(crate) struct DImg {
    /// g_iop_par: the parameters of the last OK
    pub(crate) par: IopPar,
    /// resolution in the chosen unit; at the start (unchanged = no resolution change)
    res: f64,
    res0: f64,
    /// print width / height in the unit
    pw: f64,
    ph: f64,
    init: bool,
    kind: i32,
    /// Enter was pressed in a number box
    enter: bool,
    rs_open: Shared<i32>,
    cs_open: Shared<i32>,
    /// the image
    w0: i32,
    h0: i32,
    nlay: i32,
    /// new size
    w: i32,
    h: i32,
    /// radio: 0 Prozentual, 1 Absolut
    bypct: i32,
    pct: i32,
    keep: i32,
    /// 0 inch (Pixel/in, Zoll), 1 centimetre
    unit: i32,
    unit0: i32,
    filter: i32,
    gamma: i32,
    anchor: i32,
    fillc: i32,
    /// remembered settings (Größe ändern)
    rsp: [i32; 5],
    /// remembered settings (Leinwandgröße)
    csp: [i32; 5],
    rs_rect: Shared<WinRect>,
    cs_rect: Shared<WinRect>,
}

impl Default for DImg {
    fn default() -> DImg {
        DImg {
            par: IopPar::default(),
            res: 0.0,
            res0: 0.0,
            pw: 0.0,
            ph: 0.0,
            init: false,
            kind: 0,
            enter: false,
            rs_open: Shared::new(0),
            cs_open: Shared::new(0),
            w0: 0,
            h0: 0,
            nlay: 0,
            w: 0,
            h: 0,
            bypct: 0,
            pct: 0,
            keep: 0,
            unit: 0,
            unit0: 0,
            filter: 0,
            gamma: 0,
            anchor: 0,
            fillc: 0,
            rsp: [0; 5],
            csp: [0; 5],
            rs_rect: Shared::new(WinRect::new(0, 0, 460, 0, 0)),
            cs_rect: Shared::new(WinRect::new(0, 0, 460, 0, 0)),
        }
    }
}

/// dimg_fill_argb(choice) -> straight ARGB of a FILLC_* choice
pub(crate) fn dimg_fill_argb(p: &Pn, choice: u32) -> u32 {
    match choice as i32 {
        FILLC_PRIMARY => pal_primary(&p.pal),
        FILLC_TRANSPARENT => 0,
        FILLC_BLACK => 0xFF00_0000,
        FILLC_WHITE => 0xFFFF_FFFF,
        _ => pal_secondary(&p.pal),
    }
}

// di_defaults() -- the settings a dialog shows the first time
fn di_defaults(g: &mut DImg) {
    if g.init {
        return;
    }
    g.init = true;
    g.rsp = [0, 100, 1, 0, 1]; // Größe ändern: Prozentual 100 %, keep, Adaptiv (scharf), gamma
    g.csp = [1, 100, 0, AN_M, FILLC_SECONDARY]; // Leinwandgröße: Absolut
}

/// dimg_resize_open(doc) -- "Größe ändern"
pub(crate) fn dimg_resize_open(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    let g = &mut p.dimg;
    di_defaults(g);
    g.kind = DK_RESIZE;
    g.enter = false;
    [g.bypct, g.pct, g.keep, g.filter, g.gamma] = g.rsp;
    di_begin(p, ui, doc);
    p.dimg.rs_open.set(1);
    ui.repass();
}

/// dimg_canvas_open(doc) -- "Leinwandgröße"
pub(crate) fn dimg_canvas_open(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    let g = &mut p.dimg;
    di_defaults(g);
    g.kind = DK_CANVAS;
    g.enter = false;
    [g.bypct, g.pct, g.keep, g.anchor, g.fillc] = g.csp;
    di_begin(p, ui, doc);
    p.dimg.cs_open.set(1);
    ui.repass();
}

// di_begin(doc) -- current size, layers and resolution of the image; the start values
fn di_begin(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    let d = p.doc.d(doc);
    let g = &mut p.dimg;
    g.w0 = d.w;
    g.h0 = d.h;
    g.nlay = d.layers.len() as i32;
    // resolution: Pixel/cm for centimetre images, else Pixel/in (no unit = 96 dpi)
    g.unit = (d.dpu_unit == 2) as i32;
    let mut r = d.dpu_x;
    // ucomisd + jae: not below 0.01 and not NaN
    if d.dpu_unit == 0 || !(r >= 0.01) {
        g.unit = 0;
        r = 96.0;
    }
    g.res = r;
    g.res0 = r;
    g.unit0 = g.unit;
    if g.bypct == 0 {
        di_from_pct(g);
    } else {
        g.w = g.w0;
        g.h = g.h0;
    }
    di_print(g, ui);
}

// di_round(v) -> the pixel count: clamped to 1 .. 65535, rounded to nearest (ties to even,
// like .NET Math.Round)
fn di_round(v: f64) -> i32 {
    cvtd(minsd(maxsd(v, 1.0), 65535.0)) // NaN -> 1
}

// di_from_pct() -- width / height from the percentage
fn di_from_pct(g: &mut DImg) {
    g.w = di_round(g.w0 as f64 * g.pct as f64 / 100.0);
    g.h = di_round(g.h0 as f64 * g.pct as f64 / 100.0);
}

// di_keep(changed_w) -- with "Seitenverhältnis beibehalten" the other side from the ratio of
// the current image
fn di_keep(g: &mut DImg, changed_w: bool) {
    if g.keep == 0 {
        return;
    }
    if changed_w {
        g.h = di_round(g.w as f64 * g.h0 as f64 / g.w0 as f64);
    } else {
        g.w = di_round(g.h as f64 * g.w0 as f64 / g.h0 as f64);
    }
}

// di_print() -- print size = pixels / resolution (same unit system)
fn di_print(g: &mut DImg, ui: &mut Ui) {
    g.pw = g.w as f64 / g.res;
    g.ph = g.h as f64 / g.res;
    ui.repass(); // widgets drawn earlier show the new values
}

// di_px_from_print(is_width) -- a print size was entered: pixels = print * resolution
fn di_px_from_print(g: &mut DImg, ui: &mut Ui, is_width: bool) {
    if is_width {
        g.w = di_round(g.pw * g.res);
    } else {
        g.h = di_round(g.ph * g.res);
    }
    di_keep(g, is_width);
    di_print(g, ui);
}

// di_unit_changed() -- inch <-> centimetre (both combo boxes share unit): the resolution is
// converted, the pixel size stays
fn di_unit_changed(g: &mut DImg, ui: &mut Ui) {
    if g.unit != 0 {
        g.res /= 2.54;
    } else {
        g.res *= 2.54;
    }
    di_print(g, ui);
}

// di_size_str(bytes) -- paint.net's Utility.SizeStringFromBytes: "n,n GB" / "n,n MB" /
// "n,n KB" (one decimal) or "n Bytes"
fn di_size_str(b: u64) -> String {
    let (sh, unit) = if b > 1 << 30 {
        (30, "GB")
    } else if b > 1 << 20 {
        (20, "MB")
    } else if b > 1 << 10 {
        (10, "KB")
    } else {
        return format!("{} Bytes", b as u32);
    };
    // tenths = round(bytes * 10 / unit)
    let t = (b.wrapping_mul(20).wrapping_add(1 << sh)) >> (sh + 1);
    fmt_cap(format!("{},{} {}", (t / 10) as u32, (t % 10) as u32, unit), 32)
}

// di_row(label) -- open a row: label (LABEL_W), then the next widget gets FIELD_W; the caller
// adds field and unit, then box_end
fn di_row(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_BODY, label);
    ui.next_size(FIELD_W, 0);
}

// di_unit(str) -- the unit column of a row
fn di_unit(ui: &mut Ui, s: &str) {
    ui.next_size(UNIT_W, 0);
    ui.text(TS_BODY, s);
}

// di_size_block(base) -- the part both dialogs share (widget ids base + DI_*)
fn di_size_block(g: &mut DImg, ui: &mut Ui, base: u32) {
    let id = |k: i32| base + k as u32;
    // memory of the current and of the new image (all layers, 4 bytes per pixel)
    let cur = (g.w0 as u32 as u64 * g.h0 as u32 as u64).wrapping_mul(g.nlay as u32 as u64) << 2;
    ui.text(TS_BODY, &fmt_cap(format!("Aktuelle Bildgröße: {}", di_size_str(cur)), TXT_CAP));
    let new = (g.w as u32 as u64 * g.h as u32 as u64).wrapping_mul(g.nlay as u32 as u64) << 2;
    ui.text(TS_BODY_STRONG, &fmt_cap(format!("Neue Größe: {}", di_size_str(new)), TXT_CAP));
    // Prozentual: [n] %
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    if ui.radio(id(DI_PCT_RB), "Prozentual:", &mut g.bypct, 0) & EV_ACT != 0 {
        di_from_pct(g);
        di_print(g, ui);
    }
    let off = g.bypct != 0;
    if off {
        ui.disabled_push();
    }
    ui.next_size(FIELD_W, 0);
    let ev = ui.number_box(id(DI_PCT), &mut g.pct, &RANGE_PCT);
    di_nb_enter(g, ui, ev, id(DI_PCT));
    if ev & EV_ACT != 0 {
        di_from_pct(g);
        di_print(g, ui);
    }
    ui.text(TS_BODY, crate::preferences::localization::tr("%"));
    if off {
        ui.disabled_pop();
    }
    ui.box_end();
    // Absolut:
    ui.radio(id(DI_ABS_RB), "Absolut:", &mut g.bypct, 1);
    // the absolute part (disabled while Prozentual is chosen)
    let mut f = BOX_V;
    if g.bypct == 0 {
        f |= BOX_DISABLED;
    }
    ui.box_begin(0, f, GAP);
    if ui.checkbox(id(DI_KEEP), crate::preferences::localization::tr("Seitenverhältnis beibehalten"), &mut g.keep) & EV_ACT != 0 {
        di_keep(g, true);
        di_print(g, ui);
    }
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Pixelgröße"));
    // Breite
    di_row(ui, "Breite:");
    let ev = ui.number_box(id(DI_W), &mut g.w, &RANGE_DIM);
    di_nb_enter(g, ui, ev, id(DI_W));
    if ev & EV_ACT != 0 {
        di_keep(g, true);
        di_print(g, ui);
    }
    di_unit(ui, "Pixel");
    ui.box_end();
    // Höhe
    di_row(ui, "Höhe:");
    let ev = ui.number_box(id(DI_H), &mut g.h, &RANGE_DIM);
    di_nb_enter(g, ui, ev, id(DI_H));
    if ev & EV_ACT != 0 {
        di_keep(g, false);
        di_print(g, ui);
    }
    di_unit(ui, "Pixel");
    ui.box_end();
    // Pixeldichte
    di_row(ui, "Pixeldichte:");
    let ev = ui.number_box_f(id(DI_RES), &mut g.res, &RANGEF_RES);
    di_nb_enter(g, ui, ev, id(DI_RES));
    if ev & EV_ACT != 0 {
        di_print(g, ui);
    }
    ui.next_size(UNIT_W, 0);
    if ui.combobox(id(DI_RUNIT), &mut g.unit, &RESUNIT_LIST) & EV_ACT != 0 {
        di_unit_changed(g, ui);
    }
    ui.box_end();
    // Druckgröße
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Druckgröße"));
    di_row(ui, "Breite:");
    let ev = ui.number_box_f(id(DI_PW), &mut g.pw, &RANGEF_PRINT);
    di_nb_enter(g, ui, ev, id(DI_PW));
    if ev & EV_ACT != 0 {
        di_px_from_print(g, ui, true);
    }
    ui.next_size(UNIT_W, 0);
    if ui.combobox(id(DI_PUNIT), &mut g.unit, &PUNIT_LIST) & EV_ACT != 0 {
        di_unit_changed(g, ui);
    }
    ui.box_end();
    di_row(ui, "Höhe:");
    let ev = ui.number_box_f(id(DI_PH), &mut g.ph, &RANGEF_PRINT);
    di_nb_enter(g, ui, ev, id(DI_PH));
    if ev & EV_ACT != 0 {
        di_px_from_print(g, ui, false);
    }
    di_unit(ui, if g.unit == 0 { "Zoll" } else { "Zentimeter" });
    ui.box_end();
    ui.box_end(); // the absolute part
}

// di_nb_enter(ev, id) -- Enter in a number box (committed and still focused; Esc and Tab drop
// the focus) accepts the dialog like paint.net's AcceptButton: enter = true
fn di_nb_enter(g: &mut DImg, ui: &Ui, ev: u32, id: u32) {
    if ev & EV_COMMIT != 0 && ui.focused() == id {
        g.enter = true;
    }
}

// di_ok(kind) -- OK: IOPPAR from the dialog, remember the settings, queue the operation
fn di_ok(p: &mut Pn, ui: &mut Ui, kind: i32) {
    let g = &p.dimg;
    let mut par = p.dimg.par;
    par.w = g.w;
    par.h = g.h;
    let cmd;
    if kind == DK_CANVAS {
        par.anchor = g.anchor as u32;
        par.fill = dimg_fill_argb(p, g.fillc as u32);
        let g = &mut p.dimg;
        g.csp = [g.bypct, g.pct, g.keep, g.anchor, g.fillc];
        cmd = CMD_IOP_CANVAS_DO;
    } else {
        let f = if (g.filter as u32) <= 7 { g.filter as usize } else { 0 };
        par.filter = FILTER_MAP[f];
        par.flags = if g.gamma != 0 { RS_GAMMA } else { 0 };
        let g = &mut p.dimg;
        g.rsp = [g.bypct, g.pct, g.keep, g.filter, g.gamma];
        cmd = CMD_IOP_RESIZE_DO;
    }
    // the resolution only when it was changed in the dialog (ucomisd: jp / jne -> changed)
    let g = &mut p.dimg;
    par.resset = (g.unit != g.unit0 || !(g.res == g.res0)) as u32;
    par.unit = (g.unit + 1) as u32;
    par.dpu = g.res;
    g.par = par;
    ui.cmd_run(cmd, 0);
}

/// dimg_frame() -- both dialog windows (only the open one shows)
pub(crate) fn dimg_frame(p: &mut Pn, ui: &mut Ui) {
    di_resize_frame(p, ui);
    di_canvas_frame(p, ui);
}

// di_resize_frame() -- the "Größe ändern" window
fn di_resize_frame(p: &mut Pn, ui: &mut Ui) {
    let open = p.dimg.rs_open.clone();
    let rect = p.dimg.rs_rect.clone();
    if ui.window_begin(WINID_RESIZE, crate::preferences::localization::tr("Größe ändern"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let g = &mut p.dimg;
        ui.box_begin(0, BOX_V, GAP);
        di_size_block(g, ui, ID_DLG_RESIZE);
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Optionen"));
        di_row(ui, "Interpolation:");
        ui.next_size(WIDE_W, 0);
        ui.combobox(ID_RS_FILTER, &mut g.filter, &FILTER_LIST);
        ui.box_end();
        ui.checkbox(ID_RS_GAMMA, crate::preferences::localization::tr("Gammakorrektur anwenden"), &mut g.gamma);
        ui.box_end();
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        let mut ok = r == DLG_PRIMARY;
        if !ok && p.dimg.enter {
            p.dimg.rs_open.set(0); // Enter in a number box
            ui.repass();
            ok = true;
        }
        if ok {
            p.dimg.enter = false;
            di_ok(p, ui, DK_RESIZE);
        }
    }
    ui.window_end();
}

// di_canvas_frame() -- the "Leinwandgröße" window
fn di_canvas_frame(p: &mut Pn, ui: &mut Ui) {
    let open = p.dimg.cs_open.clone();
    let rect = p.dimg.cs_rect.clone();
    if ui.window_begin(WINID_CANVAS, crate::preferences::localization::tr("Leinwandgröße"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let g = &mut p.dimg;
        ui.box_begin(0, BOX_V, GAP);
        di_size_block(g, ui, ID_DLG_CANVAS);
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Anker"));
        // Anker / Fülleffekt on the left, the 3 x 3 grid on the right
        ui.box_begin(0, BOX_H, 16);
        ui.box_begin(0, BOX_V, GAP);
        di_row(ui, "Anker:");
        ui.next_size(ANCH_W, 0);
        ui.combobox(ID_CS_ANCHOR, &mut g.anchor, &ANCHOR_NAMES);
        ui.box_end();
        di_row(ui, "Fülleffekt:");
        ui.next_size(ANCH_W, 0);
        ui.combobox(ID_CS_FILL, &mut g.fillc, &FILL_LIST);
        ui.box_end();
        ui.box_end();
        di_anchor_grid(g, ui);
        ui.box_end();
        ui.box_end();
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        let mut ok = r == DLG_PRIMARY;
        if !ok && p.dimg.enter {
            p.dimg.cs_open.set(0); // Enter in a number box
            ui.repass();
            ok = true;
        }
        if ok {
            p.dimg.enter = false;
            di_ok(p, ui, DK_CANVAS);
        }
    }
    ui.window_end();
}

// di_anchor_grid() -- 3 x 3 buttons (ids ID_CS_CELL0 + AN_*): the anchor shows the picture
// icon, its neighbours an arrow pointing away from it (the directions the canvas grows)
fn di_anchor_grid(g: &mut DImg, ui: &mut Ui) {
    ui.box_begin(0, BOX_V, 2);
    for row in 0..3 {
        ui.box_begin(0, BOX_H, 2);
        for col in 0..3 {
            let cell = row * 3 + col; // AN_*
            ui.next_size(CELL, CELL);
            let icon = if cell == g.anchor { IC_IMAGE } else { 0 };
            ui.tool_button(ID_CS_CELL0 + cell as u32, icon, ANCHOR_NAMES[cell as usize], &mut g.anchor, cell);
            if cell != g.anchor {
                di_cell_frame(ui); // the checked cell has its own border
            }
            // an arrow when the cell is a neighbour of the anchor
            let a = g.anchor;
            if a as u32 > AN_BR as u32 {
                continue;
            }
            let (ar, ac) = (a / 3, a % 3);
            let (dx, dy) = (col - ac, row - ar);
            if dx | dy == 0 || (dx + 1) as u32 > 2 || (dy + 1) as u32 > 2 {
                continue;
            }
            di_arrow(ui, dx, dy);
        }
        ui.box_end();
    }
    ui.box_end();
}

// di_cell_frame() -- a thin rounded border around the last placed widget (grid cells)
fn di_cell_frame(ui: &mut Ui) {
    let r = ui.last_rect();
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let rr = ui.dp(4);
    let hair = ui.hair();
    ui.draw_frame(r[0], r[1], r[2], r[3], rr, hair);
}

// di_arrow(dx, dy) -- an arrow in the centre of the last placed widget pointing in direction
// (dx, dy) (each -1, 0, 1), drawn with draw_line in the text colour
fn di_arrow(ui: &mut Ui, dx: i32, dy: i32) {
    let (mut ux, mut uy) = (dx as f64, dy as f64);
    if dx != 0 && dy != 0 {
        ux *= RSQRT2;
        uy *= RSQRT2;
    }
    let r = ui.last_rect();
    let cx = (r[0] as f64 + r[2] as f64 * 0.5) * 256.0;
    let cy = (r[1] as f64 + r[3] as f64 * 0.5) * 256.0;
    let half = ui.dp(7) as f64 * 256.0;
    let head = ui.dp(5) as f64 * 256.0;
    let lw = ui.dp(3) << 7; // 1.5 DIP in 24.8
    let c = ui.color(T_TEXT);
    ui.draw_color(c);
    // shaft: centre - u * half .. centre + u * half
    let (hx, hy) = (ux * half, uy * half);
    let tx = cx + hx;
    let ty = cy + hy;
    ui.draw_line(cvtd(cx - hx), cvtd(cy - hy), cvtd(tx), cvtd(ty), lw);
    // head: tip + head * R(+-135 degrees) u, R(135) u = s (-ux - uy, ux - uy),
    // R(-135) u = s (-ux + uy, -ux - uy), s = 1 / sqrt 2
    let a = (ux + uy) * RSQRT2 * head;
    let b = (ux - uy) * RSQRT2 * head;
    ui.draw_line(cvtd(tx), cvtd(ty), cvtd(tx - a), cvtd(ty + b), lw);
    ui.draw_line(cvtd(tx), cvtd(ty), cvtd(tx - b), cvtd(ty - a), lw);
}
