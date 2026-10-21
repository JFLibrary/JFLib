//! Script diagnostics and crash handling.
pub(crate) mod crash;

// diag -- Test-Hooks für Skripte (cmd 900..999), Diagnose.  Every agent adds its hooks in its
// own marked block (commands and hidden widgets of its range) and chains them into diag_frame
// / diag_cmd below.                                                 [apps/PaintCraft/src/diag.s]
//
// Ported by pn-core ahead of pn-sys (the file is the dispatcher plus the A1 block, the test
// hooks of the document core: doc / layer / compose / view / cursors); pn-sys owns the file.
//
// Module contract (app/lifecycle.rs):
//   diag_init()  diag_states()  diag_frame()  diag_cmd(id, arg) -> handled
// Blocks: A1 (document core) commands 900-929 / widgets 9900-9929 (a1_diag_*, below);
//         A2 (history) CMD_HIST_TEST0..1 (950-979) -> hist_diag_cmd (hist_diag.rs).
//         A3 (colours) commands 985-994 -> pal_diag_cmd, records 9985-9989 (palette.rs).
//         T1 (tools) commands 930-949 -> t1_diag_cmd, records 9930-9949 (tool.rs).
//         T2 (selection) commands 980-984 -> sel_diag_cmd, records 9980-9984 (sel.rs).
//         A4 (layers): the block above is full, so its hooks are Ebenen commands 370-379
//         (a4_test in layer.rs, routed by lay_cmd) and records 4390-4395 (a4_probes).
//         A6 (effects) commands 995-999 -> fx_diag_cmd, records 9990-9995 (fx.rs).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;

use ui::*;

use crate::document::composite::{comp_mark, comp_pixel, comp_set_tool, lay_thumb, CompHook, HookKey};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_count, doc_cur, doc_cur_layer, doc_mark_px, Doc};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const ID_A1_PROBE: u32 = ID_TEST_FIRST;
static A1_COLORS: [u32; 10] =
    [0x00000000, 0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0x80FF0000, 0xFFFFFFFF, 0xFF000000, 0xFF808080, 0x80000000, 0xFFFFFF00];

/// the state of diag.s (the A1 block)
#[derive(Default)]
pub(crate) struct Diag {
    /// g_a1_color: the test colour of 904
    color: u32,
    /// g_a1_x0 / g_a1_y0: the first corner of the next 904 fill
    x0: u32,
    y0: u32,
    /// g_a1_probe: #9900 text
    probe: String,
}

/// diag_init() -- nothing
pub(crate) fn diag_init(_p: &mut Pn, _ui: &mut Ui) {}

/// diag_states() -- (Rust) the crash hook's diagnostics snapshot (crash.rs)
pub(crate) fn diag_states(p: &mut Pn, _ui: &mut Ui) {
    crate::diagnostics::crash::crash_states(p);
}

/// diag_frame() -- hidden probe widgets
pub(crate) fn diag_frame(p: &mut Pn, ui: &mut Ui) {
    a1_diag_frame(p, ui);
    // ---- A3 (colours): records 9985..9989 (palette.rs)
    crate::workspace::palette::pal_diag_frame(p, ui);
    // ---- T1 (tools): records 9930..9949 (tool.rs)
    crate::tools::t1_diag_frame(p, ui);
    // ---- T2 (selection): records 9980..9984 (sel.rs)
    crate::document::selection::sel_diag_frame(p, ui);
    // ---- A6 (effects): records 9990..9995 (fx.rs)
    crate::effects::fx_diag_frame(p, ui);
}

/// diag_cmd(id, arg) -> handled
pub(crate) fn diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id.wrapping_sub(CMD_A6_TEST0) <= 4 {
        return crate::effects::fx_diag_cmd(p, ui, id, arg); // A6 (effects): 995..999
    }
    if id.wrapping_sub(CMD_HIST_TEST0) <= CMD_HIST_TEST1 - CMD_HIST_TEST0 {
        return crate::document::history::diagnostics::hist_diag_cmd(p, ui, id, arg); // A2 (history)
    }
    if id.wrapping_sub(985) <= 9 {
        return crate::workspace::palette::pal_diag_cmd(p, ui, id, arg); // A3 (colours): 985..994
    }
    if id.wrapping_sub(CMD_T2_TEST0) <= 4 {
        return crate::document::selection::sel_diag_cmd(p, ui, id, arg); // T2 (selection): 980..984
    }
    if id.wrapping_sub(CMD_T1_TEST0) <= 19 {
        return crate::tools::t1_diag_cmd(p, ui, id, arg); // T1 (tools): 930..949
    }
    if id.wrapping_sub(900) <= 29 {
        return a1_diag_cmd(p, ui, id, arg); // A1 (document core): 900..929
    }
    false
}

/// the compose hook of test hook 909 (comp_set_tool(doc, Some(HookKey::Diag))): resolved by
/// compose at update time
pub(crate) fn diag_hook(_p: &Pn) -> Option<Box<dyn CompHook + '_>> {
    Some(Box::new(A1ToolHook))
}

// ================================================================== A1 (document core) 900-929
// Commands (cmd ID ARG in scripts):
//   900 x | y << 16    composite pixel of the active image -> #9900 text "AARRGGBB"
//   901 x | y << 16    pixel of the current layer        -> #9900 text
//   902 i              test colour i (0 transparent, 1 red, 2 green, 3 blue, 4 red 50 %,
//                      5 white, 6 black, 7 grey, 8 black 50 %, 9 yellow)
//   903 x0 | y0 << 16  first corner of the next 904 fill
//   904 x1 | y1 << 16  fill [x0, x1) x [y0, y1) of the current layer with the test colour
//                      (doc_mark_px: modified, composite updated; no history entry)
//   905 v              opacity of the current layer (history: layer properties)
//   906 mode           blend mode BM_* of the current layer (history: layer properties)
//   907 flags          view: 1 = Space held (scripts cannot hold Space)
//   908 -              thumbnail of the current layer (lay_thumb 32 x 24, cell 4): its pixel
//                      (0, 0) -> #9900 text
//   909 on             live tool hook (comp_set_tool): 1 = the current layer shows opaque
//                      yellow in [0, 10) x [0, 10), 0 = removed
//   910 -              every PaintCraft cursor at 100 / 150 / 200 % -> build/tests/cursors.png
//                      (cursors.rs pn_cursor_shot); #9900 text "ok" / "err"
//   911 -              crash now (crash.rs crash_test) -- with --crash-dir DIR the protocol goes
//                      there (tools/pn_crash_check.py)
// Hidden records (refreshed every frame; `expect #id value N` / `text "..."`):
//   9900 text last probe            9901 value documents         9902 value active index
//   9903 value layers               9904 value current layer     9905 value modified (0 / 1)
//   9906 text current layer name, value visible (0 / 1)          9907 value its opacity
//   9908 value its blend mode       9909 text view origin "x,y" (canvas px), value zoom
//   9910 text path ("" = none)      9911 text name, value DOC_FMT
//   9912 value Ganzes Bild mode     9913 value width             9914 value height
//   9915 value resolution * 100     9916 value resolution unit   9917 value 1 = tab thumbnail
//                                                                 up to date
//   9918 text message of the error bar (fileio)
//   9919 text window title
//   9920 value the canvas cursor of the tool (tool_cursor: 15 = CUR_CUSTOM, its own cursor),
//        text "tool N" / "rotate" / "" -- which PaintCraft cursor ui.cursor_custom names

// a1_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn a1_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

// a1_diag_frame()
fn a1_diag_frame(p: &mut Pn, ui: &mut Ui) {
    a1_sem(ui, ID_A1_PROBE, &p.diag.probe, 0);
    a1_sem(ui, ID_A1_PROBE + 1, "", doc_count(&p.doc));
    a1_sem(ui, ID_A1_PROBE + 2, "", doc_cur(&p.doc));
    if let Some(d) = p.doc.active() {
        a1_sem(ui, ID_A1_PROBE + 3, "", d.layers.len() as i32);
        a1_sem(ui, ID_A1_PROBE + 4, "", d.cur);
        a1_sem(ui, ID_A1_PROBE + 5, "", (d.flags & DF_MODIFIED as u32) as i32);
        if let Some(l) = doc_cur_layer(d) {
            a1_sem(ui, ID_A1_PROBE + 6, &l.name, (l.flags & LYF_VISIBLE as u32) as i32);
            a1_sem(ui, ID_A1_PROBE + 7, "", l.sf.opac as i32);
            a1_sem(ui, ID_A1_PROBE + 8, "", l.sf.blend as i32);
        }
        a1_sem(ui, ID_A1_PROBE + 10, d.path.as_deref().unwrap_or(""), 0);
        a1_sem(ui, ID_A1_PROBE + 11, &d.name, d.fmt as i32);
        a1_sem(ui, ID_A1_PROBE + 12, "", d.view.fit as i32);
        a1_sem(ui, ID_A1_PROBE + 13, "", d.w);
        a1_sem(ui, ID_A1_PROBE + 14, "", d.h);
        a1_sem(ui, ID_A1_PROBE + 15, "", rt::fx::cvtd(d.dpu_x * 100.0));
        a1_sem(ui, ID_A1_PROBE + 16, "", d.dpu_unit as i32);
        let ok = d.ver_px.wrapping_add(d.ver_lay) == d.thumb_ver && !d.thumb.px.is_empty();
        a1_sem(ui, ID_A1_PROBE + 17, "", ok as i32);
    }
    let t = String::from(crate::integrations::file_io::fio_error_text(&p.fio));
    a1_sem(ui, ID_A1_PROBE + 18, &t, 0);
    let t = String::from(crate::integrations::file_io::fio_title_text(&p.fio));
    a1_sem(ui, ID_A1_PROBE + 19, &t, 0);
    let (ox, oy) = crate::workspace::view::view_origin(&p.view);
    let t = fmt_cap(format!("{},{}", ox, oy), 64);
    a1_sem(ui, ID_A1_PROBE + 9, &t, crate::workspace::view::view_zoom(p) as i32);
    // 9920: the canvas cursor and which PaintCraft cursor it shows
    let cur = crate::tools::tool_cursor(p, ui);
    let mut t = String::new();
    if cur == CUR_CUSTOM {
        let h = ui.cursor_current();
        for k in 0..=TOOL_COUNT as u32 {
            let kind = if k < TOOL_COUNT as u32 { k } else { PNC_ROTATE as u32 };
            if crate::workspace::cursors::pn_cursor(p, ui, kind) == h {
                t = if k == TOOL_COUNT as u32 { String::from("rotate") } else { format!("tool {}", k as i32) };
                break;
            }
        }
    }
    a1_sem(ui, ID_A1_PROBE + 20, &t, cur as i32);
}

// a1_probe(argb) -- #9900 = "AARRGGBB"
fn a1_probe(p: &mut Pn, argb: u32) {
    p.diag.probe = format!("{:08X}", argb);
    p.repass();
}

// a1_diag_cmd(id, arg) -> handled
fn a1_diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let doc = doc_active(&p.doc);
    match id {
        902 => {
            if arg < 10 {
                p.diag.color = A1_COLORS[arg as usize];
            }
            return true;
        }
        903 => {
            p.diag.x0 = arg & 0xFFFF;
            p.diag.y0 = arg >> 16;
            return true;
        }
        907 => {
            crate::workspace::view::view_test_keys(&mut p.view, arg);
            return true;
        }
        _ => {}
    }
    let Some(doc) = doc else { return true };
    match id {
        900 => {
            let v = comp_pixel(p, doc, (arg & 0xFFFF) as i32, (arg >> 16) as i32);
            a1_probe(p, v);
        }
        901 => {
            let Some(l) = doc_cur_layer(p.doc.d(doc)) else { return true };
            let (x, y) = (arg & 0xFFFF, arg >> 16);
            let mut v = 0;
            if x < l.sf.w as u32 && y < l.sf.h as u32 {
                let o = y as usize * l.sf.stride as usize + x as usize * 4;
                v = u32::from_le_bytes([l.sf.px[o], l.sf.px[o + 1], l.sf.px[o + 2], l.sf.px[o + 3]]);
            }
            a1_probe(p, v);
        }
        904 => {
            let d = p.doc.dm(doc);
            let cur = d.cur;
            let Some(l) = crate::document::model::doc_cur_layer_mut(d) else { return true };
            if l.sf.px.is_empty() {
                return true;
            }
            let x1 = (arg & 0xFFFF).min(l.sf.w as u32);
            let y1 = (arg >> 16).min(l.sf.h as u32);
            let (x0, y0) = (p.diag.x0, p.diag.y0);
            let c = p.diag.color.to_le_bytes();
            let stride = l.sf.stride as usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    let o = y as usize * stride + x as usize * 4;
                    l.sf.px[o..o + 4].copy_from_slice(&c);
                }
            }
            doc_mark_px(p, doc, cur, x0 as i32, y0 as i32, x1 as i32, y1 as i32);
        }
        905 | 906 => {
            let cur = p.doc.d(doc).cur;
            if crate::document::history::hist_push_layer_props(p, doc, cur, "Ebeneneigenschaften", IC_EDIT) {
                if id == 905 {
                    crate::document::layers::lay_set_opacity(p, doc, cur, arg as i32);
                } else {
                    crate::document::layers::lay_set_blend(p, doc, cur, arg);
                }
            }
        }
        908 => {
            let cur = p.doc.d(doc).cur;
            let v = match lay_thumb(p, doc, cur, 32, 24, 4) {
                Some(t) if !t.px.is_empty() => u32::from_le_bytes([t.px[0], t.px[1], t.px[2], t.px[3]]),
                _ => 0,
            };
            a1_probe(p, v);
        }
        909 => {
            comp_set_tool(p, doc, if arg != 0 { Some(HookKey::Diag) } else { None });
            comp_mark(p, doc, 0, 0, 10, 10);
        }
        910 => {
            let r = crate::workspace::cursors::pn_cursor_shot(p, ui, "build/tests/cursors.png");
            p.diag.probe = String::from(if r == 0 { "ok" } else { "err" });
            p.repass();
        }
        911 => crate::diagnostics::crash::crash_test(),
        _ => {}
    }
    true
}

/// a1_tool_hook(ctx, dst, x, y, n, layer_row) -> 1 -- test tool layer: the row with opaque
/// yellow in columns 0..9 of rows 0..9 (worker thread)
struct A1ToolHook;

impl CompHook for A1ToolHook {
    fn row(&self, _doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool {
        if y as u32 >= 10 {
            return false;
        }
        for i in 0..n.max(0) as usize {
            let mut px = [layer_row[i * 4], layer_row[i * 4 + 1], layer_row[i * 4 + 2], layer_row[i * 4 + 3]];
            if ((x + i as i32) as u32) < 10 {
                px = 0xFFFF_FF00u32.to_le_bytes();
            }
            dst[i * 4..i * 4 + 4].copy_from_slice(&px);
        }
        true
    }
}
