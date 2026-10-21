// dlg_new -- PaintCraft: "Neu" (Datei > Neu…, Strg+N) as a modal dialog window like
// paint.net 5: "Neue Größe: n MB", Seitenverhältnis beibehalten, Pixelgröße (Breite, Höhe,
// Pixeldichte in Pixel/in or Pixel/cm), Druckgröße (Breite, Höhe in Zoll or Zentimeter).
// Starts with the size of an image on the clipboard, else the size used last (800 x 600,
// 96 dpi).  Sub-module of fileio.rs:
//   dlgnew_open()               show the dialog (fresh values)
//   dlgnew_frame()              the window (fio_frame calls it every frame)
//   dlgnew_create() -> 0 / -1   OK: the new white image "Unbenannt n" (fio_cmd CMD 121)
// Widget ids ID_DLG_NEW + 1..9, window WINID_DLG0 + 0.
//                                              [apps/PaintCraft/src/dlg_new.s]

use alloc::format;
use alloc::string::String;

use img_core::ImgMeta;
use ui::*;

use crate::app::constants::*;
use crate::integrations::file_io::CMD_FIO_NEWDO;
use crate::app::Pn;

const WINID_NEW: u32 = WINID_DLG0;
const ID_ND_KEEP: u32 = ID_DLG_NEW + 1;
const ID_ND_W: u32 = ID_DLG_NEW + 2;
const ID_ND_H: u32 = ID_DLG_NEW + 3;
const ID_ND_RES: u32 = ID_DLG_NEW + 4;
const ID_ND_RESUNIT: u32 = ID_DLG_NEW + 5;
const ID_ND_PW: u32 = ID_DLG_NEW + 6;
const ID_ND_PUNIT: u32 = ID_DLG_NEW + 7;
const ID_ND_PH: u32 = ID_DLG_NEW + 8;
const NUM_CAP: u32 = 24;
const LABEL_W: i32 = 110; // DIP
const FIELD_W: i32 = 120;
const UNIT_W: i32 = 130;
const MAXDIM: i32 = 65535;
const RANGE_DIM: Range = Range::new(1, MAXDIM, 1, 0);
static RESUNIT_LIST: [&str; 2] = ["Pixel/in", "Pixel/cm"];
static PUNIT_LIST: [&str; 2] = ["Zoll", "Zentimeter"];

/// the state of dlg_new.s (its g_* globals)
pub(crate) struct DlgNew {
    open: Shared<i32>,
    rect: Shared<WinRect>,
    w: i32,
    h: i32,
    /// aspect ratio basis
    rw: i32,
    rh: i32,
    keep: i32,
    background: i32,
    /// resolution * 100 in `resunit`
    res: i32,
    /// 0 Pixel/in, 1 Pixel/cm
    resunit: i32,
    /// 0 Zoll, 1 Zentimeter
    punit: i32,
    last_w: i32,
    last_h: i32,
    /// Pixel/in * 100
    last_res: i32,
    res_txt: String,
    pw_txt: String,
    ph_txt: String,
}

impl Default for DlgNew {
    fn default() -> DlgNew {
        DlgNew {
            open: Shared::new(0),
            rect: Shared::new(WinRect::new(0, 0, 460, 0, 0)),
            w: 0,
            h: 0,
            rw: 0,
            rh: 0,
            keep: 0,
            background: 0,
            res: 0,
            resunit: 0,
            punit: 0,
            last_w: 0,
            last_h: 0,
            last_res: 0,
            res_txt: String::new(),
            pw_txt: String::new(),
            ph_txt: String::new(),
        }
    }
}

/// dlgnew_open() -- clipboard image size or the last size, then the window
pub(crate) fn dlgnew_open(p: &mut Pn, ui: &mut Ui) {
    let g = &mut p.dlgnew;
    if g.last_w == 0 {
        g.last_w = DEF_W;
        g.last_h = DEF_H;
        g.last_res = 9600;
        g.keep = 1;
    }
    g.w = g.last_w;
    g.h = g.last_h;
    g.res = g.last_res;
    // an image on the clipboard gives the size
    if let Some(data) = ui.clip_get(CLIP_IMAGE) {
        let mut meta = ImgMeta::default();
        if img_core::io::probe(&data, Some(&mut meta)) != 0 {
            let (w, h) = (meta.w as i32, meta.h as i32);
            if (w.wrapping_sub(1) as u32) < MAXDIM as u32 && (h.wrapping_sub(1) as u32) < MAXDIM as u32 {
                g.w = w;
                g.h = h;
            }
        }
    }
    g.rw = g.w;
    g.rh = g.h;
    nd_res_text(g);
    nd_print_texts(g, ui);
    g.open.set(1);
    ui.repass();
}

// ppu_x100() -> print-unit pixels * 100 (resolution converted to the print unit)
fn ppu_x100(g: &DlgNew) -> u32 {
    let r = g.res as u32;
    if g.resunit == g.punit {
        return r;
    }
    if g.resunit == 0 {
        // Pixel/in, print in cm: / 2.54
        r.wrapping_mul(100).wrapping_add(127) / 254
    } else {
        // Pixel/cm, print in inches: * 2.54
        r.wrapping_mul(254).wrapping_add(50) / 100
    }
}

// fmt_hundredths(v) -- "8,33"
fn fmt_hundredths(v: u32) -> String {
    format!("{},{:02}", v / 100, v % 100)
}

// nd_res_text() -- the resolution text from res
fn nd_res_text(g: &mut DlgNew) {
    g.res_txt = fmt_hundredths(g.res as u32);
}

// nd_print_texts() -- print width / height from the pixel size and the resolution
fn nd_print_texts(g: &mut DlgNew, ui: &mut Ui) {
    let ppu = ppu_x100(g) as u64;
    if ppu != 0 {
        // print = px * 100 * 100 / ppu100 (hundredths)
        let pw = ((g.w as u32 as u64 * 10000 + (ppu >> 1)) / ppu) as u32;
        g.pw_txt = fmt_hundredths(pw);
        let ph = ((g.h as u32 as u64 * 10000 + (ppu >> 1)) / ppu) as u32;
        g.ph_txt = fmt_hundredths(ph);
    }
    ui.repass(); // widgets drawn earlier show the new values
}

// parse_hundredths(str) -> value * 100 ("8,33", "8.3", "12"), -1 when invalid / <= 0
fn parse_hundredths(s: &str) -> i32 {
    let b = s.as_bytes();
    let at = |i: usize| -> u8 { b.get(i).copied().unwrap_or(0) };
    let mut i = 0;
    let mut v: u32 = 0;
    let mut nd = 0;
    while at(i) == 32 {
        i += 1;
    }
    loop {
        let d = at(i).wrapping_sub(48);
        if d > 9 {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add(d as u32);
        if v > 10_000_000 {
            return -1;
        }
        nd += 1;
        i += 1;
    }
    v = v.wrapping_mul(100);
    let c = at(i);
    if c == b',' || c == b'.' {
        i += 1;
        let d = at(i).wrapping_sub(48);
        if d <= 9 {
            v = v.wrapping_add(d as u32 * 10);
            nd += 1;
            i += 1;
            let d = at(i).wrapping_sub(48);
            if d <= 9 {
                v = v.wrapping_add(d as u32);
                i += 1;
            }
        }
    }
    loop {
        let c = at(i);
        if c == 32 {
            i += 1;
            continue;
        }
        if c != 0 || nd == 0 || v == 0 {
            return -1;
        }
        return v as i32;
    }
}

// keep_ratio(changed_w) -- with "Seitenverhältnis beibehalten": the other side from the ratio
// rw : rh (changed_w = width changed)
fn keep_ratio(g: &mut DlgNew, changed_w: bool) {
    if g.keep == 0 || g.rw == 0 || g.rh == 0 {
        return;
    }
    let (rw, rh) = (g.rw as u32 as u64, g.rh as u32 as u64);
    let v = if changed_w {
        (g.w as u32 as u64 * rh + (rw >> 1)) / rw
    } else {
        (g.h as u32 as u64 * rw + (rh >> 1)) / rh
    };
    let v = (v as i64).clamp(1, MAXDIM as i64) as i32;
    if changed_w {
        g.h = v;
    } else {
        g.w = v;
    }
}

// nd_row(label) -- open a row: label of LABEL_W DIP (the caller adds field and unit, then
// box_end)
fn nd_row(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_BODY, label);
    ui.next_size(FIELD_W, 0);
}

// unit_text(str) -- the unit column
fn unit_text(ui: &mut Ui, s: &str) {
    ui.next_size(UNIT_W, 0);
    ui.text(TS_BODY, s);
}

/// dlgnew_frame() -- the "Neu" window
pub(crate) fn dlgnew_frame(p: &mut Pn, ui: &mut Ui) {
    let g = &mut p.dlgnew;
    let open = g.open.clone();
    let rect = g.rect.clone();
    if ui.window_begin(WINID_NEW, crate::preferences::localization::tr("Neu"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        // "Neue Größe: 1,8 MB" (w * h * 4 bytes)
        let t = ((g.w as u32 as u64 * g.h as u32 as u64) << 2) * 10;
        let tenths = (t + 524_288) >> 20;
        ui.text(TS_BODY_STRONG, &format!("Neue Größe: {},{} MB", tenths / 10, tenths % 10));
        if ui.checkbox(ID_ND_KEEP, crate::preferences::localization::tr("Seitenverhältnis beibehalten"), &mut g.keep) & EV_ACT != 0 {
            g.rw = g.w;
            g.rh = g.h;
        }
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Pixelgröße"));
        // Breite
        nd_row(ui, "Breite:");
        if ui.number_box(ID_ND_W, &mut g.w, &RANGE_DIM) & EV_ACT != 0 {
            keep_ratio(g, true);
            nd_print_texts(g, ui);
        }
        unit_text(ui, "Pixel");
        ui.box_end();
        // Höhe
        nd_row(ui, "Höhe:");
        if ui.number_box(ID_ND_H, &mut g.h, &RANGE_DIM) & EV_ACT != 0 {
            keep_ratio(g, false);
            nd_print_texts(g, ui);
        }
        unit_text(ui, "Pixel");
        ui.box_end();
        // Pixeldichte
        nd_row(ui, "Pixeldichte:");
        if ui.textbox(ID_ND_RES, None, &mut g.res_txt, NUM_CAP) & EV_COMMIT != 0 {
            let v = parse_hundredths(&g.res_txt);
            if v > 0 {
                g.res = v;
            }
            nd_res_text(g);
            nd_print_texts(g, ui);
        }
        ui.next_size(UNIT_W, 0);
        if ui.combobox(ID_ND_RESUNIT, &mut g.resunit, &RESUNIT_LIST) & EV_ACT != 0 {
            // convert the value: Pixel/in <-> Pixel/cm (the pixel size stays)
            let r = g.res as u32;
            g.res = if g.resunit != 0 {
                r.wrapping_mul(100).wrapping_add(127) / 254
            } else {
                r.wrapping_mul(254).wrapping_add(50) / 100
            } as i32;
            nd_res_text(g);
            nd_print_texts(g, ui);
        }
        ui.box_end();
        // Druckgröße
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Druckgröße"));
        nd_row(ui, "Breite:");
        if ui.textbox(ID_ND_PW, None, &mut g.pw_txt, NUM_CAP) & EV_COMMIT != 0 {
            let t = g.pw_txt.clone();
            nd_print_edit(g, ui, &t, true);
        }
        ui.next_size(UNIT_W, 0);
        if ui.combobox(ID_ND_PUNIT, &mut g.punit, &PUNIT_LIST) & EV_ACT != 0 {
            nd_print_texts(g, ui);
        }
        ui.box_end();
        nd_row(ui, "Höhe:");
        if ui.textbox(ID_ND_PH, None, &mut g.ph_txt, NUM_CAP) & EV_COMMIT != 0 {
            let t = g.ph_txt.clone();
            nd_print_edit(g, ui, &t, false);
        }
        unit_text(ui, if g.punit == 0 { "Zoll" } else { "Zentimeter" });
        ui.box_end();
        nd_row(ui, "Hintergrund:");
        ui.next_size(250, 0);
        ui.combobox(ID_DLG_NEW + 9, &mut g.background, &["Weiß", "Transparent", "Primärfarbe", "Sekundärfarbe"]);
        ui.box_end();
        if ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_FIO_NEWDO, 0);
        }
    }
    ui.window_end();
}

// nd_print_edit(text, is_width) -- a print size was typed: the pixel size from it
fn nd_print_edit(g: &mut DlgNew, ui: &mut Ui, text: &str, is_width: bool) {
    let v = parse_hundredths(text);
    if v > 0 {
        // px = print100 * ppu100 / 10000
        let px = (ppu_x100(g) as u64 * v as u64 + 5000) / 10000;
        let px = (px as i64).clamp(1, MAXDIM as i64) as i32;
        if is_width {
            g.w = px;
            keep_ratio(g, true);
        } else {
            g.h = px;
            keep_ratio(g, false);
        }
    }
    nd_print_texts(g, ui);
}

/// dlgnew_create() -> 0 / -1 -- the new image from the dialog values
pub(crate) fn dlgnew_create(p: &mut Pn, _ui: &mut Ui) -> i32 {
    let (w, h) = (p.dlgnew.w, p.dlgnew.h);
    let background = match p.dlgnew.background {
        1 => 0,
        2 => crate::workspace::palette::pal_primary(&p.pal),
        3 => crate::workspace::palette::pal_secondary(&p.pal),
        _ => 0xFFFF_FFFF,
    };
    let Some(doc) = crate::document::model::doc_new(p, w, h, background, 0) else {
        crate::integrations::file_io::fio_error(p, Some("Neues Bild"), "Zu wenig Arbeitsspeicher für ein Bild dieser Größe.");
        return -1;
    };
    // resolution: Pixel/in -> unit inch, Pixel/cm -> unit cm
    let g = &p.dlgnew;
    let r = g.res as f64 / 100.0;
    let unit = (g.resunit + 1) as u8;
    {
        let d = p.doc.dm(doc);
        d.dpu_x = r;
        d.dpu_y = r;
        d.dpu_unit = unit;
    }
    crate::document::history::hist_init(p, doc, "Neues Bild", IC_PAGE);
    let g = &mut p.dlgnew;
    g.last_w = w;
    g.last_h = h;
    let mut res = g.res;
    if g.resunit != 0 {
        res = ((res as u32).wrapping_mul(254).wrapping_add(50) / 100) as i32; // remember as Pixel/in
    }
    g.last_res = res;
    0
}
