// t_picker -- PaintCraft: Farbauswahl (colour picker, agent T1).  Sub-module of tool.rs.
// Left click / drag = primary colour, right = secondary (COLOR-009..015).  Probennahme:
// Einzelne Pixel, 3 × 3, 5 × 5, 11 × 11, 31 × 31, 51 × 51 Pixel (averaged over the pixels
// inside the image, weighted by alpha = premultiplied average, then straight again), Ebene /
// Bild (Strg held = Bild, like paint.net), "Nach dem Klick:" Werkzeug beibehalten / Zurück
// zum vorigen Werkzeug / Zum Zeichenstift wechseln (on release).  Clicks outside the
// image pick nothing.
//                                              [apps/PaintCraft/src/t_picker.s]
//
//   t_picker_init()       register TOOL_PICKER
//   pk_test(which, v)     test hooks: 0 = sample size index, 1 = "Nach dem Klick" index
//   pk_state() -> text    test record 9942: "size,after,sampling"

use alloc::format;
use alloc::string::String;

use gfx::Surface;
use ui::*;

use crate::document::composite::comp_surface;
use crate::app::constants::*;
use crate::document::model::DocId;
use crate::workspace::palette::{pal_set_primary, pal_set_secondary, pal_version};
use crate::tools::*;
use crate::app::Pn;

const PK_ID: u32 = (TOOLOPT_BASE + TOOL_PICKER * TOOLOPT_STRIDE) as u32;
const PK_ID_SIZE: u32 = PK_ID; // sample size (rows + 1 .. + 6)
const PK_ID_SMP: u32 = PK_ID + 10; // Ebene / Bild (rows + 1 .. + 2)
const PK_ID_AFTER: u32 = PK_ID + 20; // Nach dem Klick (rows + 1 .. + 3)
const PK_SIZES: u32 = 6;
static PK_HALF: [u8; 6] = [0, 1, 2, 5, 15, 25];
static SIZE_LABELS: [&str; 6] = ["Einzelne Pixel", "3 × 3 Pixel", "5 × 5 Pixel", "11 × 11 Pixel", "31 × 31 Pixel", "51 × 51 Pixel"];
static SMP_LABELS: [&str; 2] = ["Ebene", "Bild"];
static SMP_ICONS: [u32; 2] = [IC_LAYERS, IC_IMAGE];
static AFTER_LABELS: [&str; 3] = ["Werkzeug beibehalten", "Zurück zum vorigen Werkzeug", "Zum Zeichenstift wechseln"];

/// the state of t_picker.s (its g_* globals)
#[derive(Default)]
pub(crate) struct TPicker {
    /// sample size index (0 = one pixel)
    size: u32,
    /// 0 stay, 1 previous tool, 2 pencil
    after: u32,
    /// 0 Ebene, 1 Bild
    smp: u32,
    /// a button is held
    on: bool,
    btn: u32,
}

static DESC_PICKER: ToolDesc = ToolDesc { deactivate: Some(pk_deactivate), input: Some(pk_input), options: Some(pk_options), ..ToolDesc::NONE };

/// t_picker_init()
pub(crate) fn t_picker_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_PICKER, Some(&DESC_PICKER));
}

/// pk_test(which, v) -- 0: sample size index, 1: after-click index
pub(crate) fn pk_test(p: &mut Pn, which: u32, v: i32) {
    let v = v as u32;
    if which == 0 {
        if v < PK_SIZES {
            p.tpicker.size = v;
        }
    } else if v < 3 {
        p.tpicker.after = v;
    }
}

/// pk_state() -> "size,after,sampling"
pub(crate) fn pk_state(t: &TPicker) -> String {
    format!("{},{},{}", t.size as i32, t.after as i32, t.smp as i32)
}

// the pixel (x, y) of a straight surface
fn sf_px(s: &Surface, x: i32, y: i32) -> u32 {
    let o = y as usize * s.stride as usize + x as usize * 4;
    u32::from_le_bytes([s.px[o], s.px[o + 1], s.px[o + 2], s.px[o + 3]])
}

// pk_average(sf, x, y, half): one pixel = its exact value, larger sizes average premultiplied
// (alpha-weighted, the pixels inside the image)
fn pk_average(s: &Surface, x: i32, y: i32, half: i32) -> u32 {
    if half == 0 {
        return sf_px(s, x, y);
    }
    let (mut sa, mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let x0 = (x - half).max(0);
    let x1 = (x + half + 1).min(s.w);
    let y1 = (y + half + 1).min(s.h);
    for yy in (y - half).max(0)..y1 {
        for xx in x0..x1 {
            let c = sf_px(s, xx, yy);
            let a = (c >> 24) as u64;
            sa += a;
            sb += (c & 255) as u64 * a;
            sg += ((c >> 8) & 255) as u64 * a;
            sr += ((c >> 16) & 255) as u64 * a;
            n += 1;
        }
    }
    if sa == 0 {
        return 0; // every pixel transparent
    }
    // alpha = round(sum a / count), colour = round(sum c*a / sum a)
    let a = ((sa + (n >> 1)) / n) as u32;
    let h = sa >> 1;
    (a << 24) | ((((sr + h) / sa) as u32) << 16) | ((((sg + h) / sa) as u32) << 8) | (((sb + h) / sa) as u32)
}

// pk_sample(doc, x_fx, y_fx, mods): pick the colour under (x, y) into the colour of the
// button of the drag
fn pk_sample(p: &mut Pn, doc: DocId, x_fx: i32, y_fx: i32, mods: u32) {
    let x = x_fx >> 8;
    let y = y_fx >> 8;
    let Some(d) = p.doc.get(doc) else { return };
    if x as u32 >= d.w as u32 || y as u32 >= d.h as u32 {
        return;
    }
    let half = PK_HALF[if p.tpicker.size <= PK_SIZES - 1 { p.tpicker.size as usize } else { 0 }] as i32;
    let image = p.tpicker.smp != 0 || mods & MOD_CTRL != 0;
    let c = {
        let s: &Surface = if image {
            comp_surface(p, doc)
        } else {
            let d = p.doc.d(doc);
            match crate::document::model::doc_cur_layer(d) {
                Some(l) => &l.sf,
                None => return,
            }
        };
        if s.px.is_empty() || x as u32 >= s.w as u32 || y as u32 >= s.h as u32 {
            return;
        }
        pk_average(s, x, y, half)
    };
    if p.tpicker.btn == BTN_RIGHT {
        pal_set_secondary(&mut p.pal, c);
    } else {
        pal_set_primary(&mut p.pal, c);
    }
}

// pk_input(doc, ui, x_fx, y_fx) -> changed
fn pk_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    if !p.tpicker.on {
        let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
            BTN_LEFT
        } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
            BTN_RIGHT
        } else {
            return false;
        };
        p.tpicker.btn = btn;
        p.tpicker.on = true;
    } else {
        let m = 1 << p.tpicker.btn;
        if inp.btn & m == 0 || inp.released & m != 0 {
            // released: the tool switch of "Nach dem Klick"
            p.tpicker.on = false;
            match p.tpicker.after {
                1 => {
                    let t = tool_prev(&p.tool);
                    tool_set(p, ui, t);
                }
                2 => tool_set(p, ui, TOOL_PENCIL),
                _ => {}
            }
            return true;
        }
    }
    let v = pal_version(&p.pal);
    pk_sample(p, doc, x, y, inp.mods);
    if inp.pressed & ((1 << BTN_LEFT) | (1 << BTN_RIGHT)) != 0 {
        return true; // the press itself
    }
    pal_version(&p.pal) != v
}

// pk_deactivate(doc): a drag in progress ends
fn pk_deactivate(p: &mut Pn, _ui: &mut Ui, _doc: Option<DocId>) {
    p.tpicker.on = false;
}

// pk_options(): "Probennahme:" size, Ebene / Bild | "Nach dem Klick:"
fn pk_options(p: &mut Pn, ui: &mut Ui) {
    opt_label(ui, "Probennahme:");
    let mut v = p.tpicker.size;
    opt_menu(p, ui, PK_ID_SIZE, &mut v, &SIZE_LABELS, &[], None, OMF_NOICON as u32);
    p.tpicker.size = v;
    let mut v = p.tpicker.smp;
    opt_menu(p, ui, PK_ID_SMP, &mut v, &SMP_LABELS, &SMP_ICONS, None, 0);
    p.tpicker.smp = v;
    opt_sep(ui);
    opt_label(ui, "Nach dem Klick:");
    let mut v = p.tpicker.after;
    opt_menu(p, ui, PK_ID_AFTER, &mut v, &AFTER_LABELS, &[], None, OMF_NOICON as u32);
    p.tpicker.after = v;
}
