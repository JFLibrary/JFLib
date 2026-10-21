// clip -- PaintCraft: Zwischenablage and file drops (agent K1).  Docs: apps/PaintCraft/docs/CLIPBOARD.md
//                                                         [apps/PaintCraft/src/clip.s]
//
// Module contract (app/lifecycle.rs):
//   clip_init()        nothing to prepare
//   clip_states()      command states 160-171: Ausschneiden / Kopieren / Sichtbares kopieren
//                      with an image; Einfügen / Einfügen als neue Ebene with an image and an
//                      image on the clipboard (clip_has, no data is read), In neues Bild
//                      einfügen with a clipboard image; Auswahl kopieren with a selection;
//                      Auswahl einfügen ▸ * with an image and clipboard text or an image
//   clip_frame()       the questions "Einfügen" (image larger than the canvas) and "Drag & Drop",
//                      the hidden probe record #0x82200
//   clip_cmd(id, arg) -> handled   ids 160-179 + UIC_DROP
// Commands:
//   Kopieren (Strg+C): the current layer's pixels inside the selection bounds (the whole layer
//     without a selection), alpha * selection coverage, as CLIP_IMAGE (PNG + DIB / image/bmp);
//     Sichtbares kopieren (Strg+Umschalt+C) the same from the composite image
//   Ausschneiden (Strg+X): Kopieren, then the selected pixels become transparent (no selection:
//     the whole layer) -- one step "Ausschneiden"
//   Einfügen (Strg+V): the clipboard image floats in Pixel verschieben (tmove_paste)
//     at the top left of the visible canvas area (kept inside the canvas when it fits); larger
//     than the canvas -> question Leinwand vergrößern (Leinwandgröße, anchored top left,
//     secondary colour) / Leinwand nicht ändern / Abbrechen; the commit is the step
//     "Einfügen"
//   Einfügen als neue Ebene (Strg+Umschalt+V): step "Neue Ebene" above the current one, then
//     Einfügen into it; Einfügen als neues Bild (Strg+Alt+V): a new image of the clipboard
//     image's size (white background, "Neues Bild"), then Einfügen at (0, 0)
//   Auswahl kopieren: sel_geom_text as CLIP_TEXT; Auswahl einfügen ▸ Ersetzen / Hinzufügen /
//     Ausschließen / Schneiden / Umkehren: sel_geom_parse of the clipboard text, else the
//     clipboard image's rectangle at (0, 0); one selection step (paint.net's names)
//   UIC_DROP: without an image the files are opened; else the question "Drag & Drop":
//     Öffnen (fio_open_path per file) / Ebene(n) hinzufügen (lay_import_paths) / Abbrechen
// Test hooks (commands 172-174, main window records):
//   cmd 172 X|Y<<16   decode the clipboard image -> #0x82200 text "W,H,AARRGGBB" of pixel
//                     (X, Y) (clamped), value 1; "none" / value 0 without an image
//   cmd 173           #0x82200 text "docs,layers,cur,W,H" of the active image, value = enabled
//                     commands 1 Einfügen 2 In neue Ebene 4 In neues Bild 8 Auswahl kopieren
//                     16 Auswahl einfügen 32 Kopieren (test commands commit a pending edit)
//   cmd 174 A         answer an open question: paste 0 cancel / 1 enlarge / 2 keep; drop
//                     0 cancel / 1 open / 2 add layers
// Record #0x82201: value = open questions (1 Einfügen, 2 Drag & Drop).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::straight::erase_row;
use gfx::Surface;
use ui::*;

use crate::document::composite::comp_surface;
use crate::app::constants::*;
use crate::workspace::dialogs::image::IopPar;
use crate::document::model::{doc_active, doc_count, doc_cur_layer, doc_index, doc_mark_px, doc_new, Doc, DocId};
use crate::integrations::file_io::{fio_error, fio_queue_path};
use crate::document::history::{hist_begin_compound, hist_end_compound, hist_init, hist_push_layers, hist_push_sel, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch};
use crate::document::operations::iop_canvas;
use crate::document::layers::{lay_import_paths, lay_lines, lay_new};
use crate::workspace::palette::pal_secondary;
use crate::document::selection::{sel_active, sel_bbox, sel_clip_row, sel_combine, sel_erase, sel_geom_parse, sel_geom_text, sel_poly_rect, surf_try};
use crate::tools::move_pixels::tmove_paste;
use crate::workspace::view::{view_canvas_rect, view_screen_to_doc_fx};
use crate::app::commands::pn_cmd_flags;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const CMD_K1_PROBE: u32 = 172;
const CMD_K1_STATE: u32 = 173;
const CMD_K1_ANSWER: u32 = 174;
const CMD_K1_ASKED: u32 = 175; // internal: paste question answered (arg)
const CMD_K1_DROPPED: u32 = 176; // internal: drop question answered (arg)
const WINID_K1_PASTE: u32 = WINID_DLG0 + 0x70; // 0x6080 question "Einfügen"
const WINID_K1_DROP: u32 = WINID_DLG0 + 0x71; // 0x6081 question "Drag & Drop"
const ID_K1_PROBE: u32 = ID_CLIP_FIRST; // 0x82200 hidden probe records (K1 block 0x82200..0x822FF)
const ID_K1_QUEST: u32 = ID_CLIP_FIRST + 1;
const ID_K1_ENLARGE: u32 = ID_CLIP_FIRST + 0x10;
const ID_K1_KEEP: u32 = ID_CLIP_FIRST + 0x11;
const ID_K1_OPEN: u32 = ID_CLIP_FIRST + 0x20;
const ID_K1_IMPORT: u32 = ID_CLIP_FIRST + 0x21;
const PM_LAYER: u32 = 0; // paste modes
const PM_NEWLAYER: u32 = 1;
const PM_NEWIMAGE: u32 = 2;

const S_CUT: &str = "Ausschneiden";
/// history names of Auswahl einfügen per SM_*
static PS_NAMES: [&str; 5] = [
    "Auswahl einfügen",
    "Auswahl einfügen (vereinigen)",
    "Auswahl einfügen (abziehen)",
    "Auswahl einfügen (Schnittmenge)",
    "Auswahl einfügen (XOR)",
];
/// commands of the k1_state bits
static KS_CMDS: [u32; 6] = [CMD_PASTE, CMD_PASTE_LAYER, CMD_PASTE_IMAGE, CMD_COPY_SEL, CMD_PASTE_SEL, CMD_COPY];
/// Drag & Drop texts: singular / plural (question, Öffnen, Ebene(n) hinzufügen)
static DW_TEXTS: [[&str; 2]; 4] = [
    ["Wie soll die Datei geöffnet werden?", "Wie sollen die Dateien geöffnet werden?"],
    ["Öffnet das Bild.", "Öffnet alle Bilder."],
    ["Ebene hinzufügen", "Ebenen hinzufügen"],
    ["Fügt das Bild dem geöffneten Bild als zusätzliche Ebene hinzu.", "Fügt die Bilder dem geöffneten Bild als zusätzliche Ebenen hinzu."],
];

/// the state of clip.s (its g_* globals)
pub(crate) struct Clip {
    /// the image waiting for the question, its document and paste mode
    pend_sf: Option<Surface>,
    pend_doc: Option<DocId>,
    pend_mode: u32,
    ask_open: Shared<i32>,
    ask_pending: bool,
    ask_rect: Shared<WinRect>,
    /// copy of the dropped paths
    drop_list: Option<Vec<String>>,
    drop_n: u32,
    drop_open: Shared<i32>,
    drop_pending: bool,
    drop_rect: Shared<WinRect>,
    probe_txt: String,
    probe_val: i32,
}

impl Default for Clip {
    fn default() -> Clip {
        Clip {
            pend_sf: None,
            pend_doc: None,
            pend_mode: 0,
            ask_open: Shared::new(0),
            ask_pending: false,
            ask_rect: Shared::new(WinRect::new(0, 0, 480, 0, 0)),
            drop_list: None,
            drop_n: 0,
            drop_open: Shared::new(0),
            drop_pending: false,
            drop_rect: Shared::new(WinRect::new(0, 0, 440, 0, 0)),
            probe_txt: String::new(),
            probe_val: 0,
        }
    }
}

/// clip_init() -- nothing to prepare
pub(crate) fn clip_init(_p: &mut Pn, _ui: &mut Ui) {}

// ------------------------------------------------------------------ states

/// clip_states() -- command states of 160-171
pub(crate) fn clip_states(p: &mut Pn, ui: &mut Ui) {
    let doc = doc_active(&p.doc);
    let fd = if doc.is_some() { 0 } else { CF_DISABLED }; // no image
    pn_cmd_set(ui, CMD_CUT, fd);
    pn_cmd_set(ui, CMD_COPY, fd);
    pn_cmd_set(ui, CMD_COPY_MERGED, fd);
    let img = ui.clip_has(CLIP_IMAGE);
    let txt = ui.clip_has(CLIP_TEXT);
    // Einfügen / In neue Ebene: an image and a clipboard image
    let f = if img { fd } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_PASTE, f);
    pn_cmd_set(ui, CMD_PASTE_LAYER, f);
    pn_cmd_set(ui, CMD_PASTE_IMAGE, if img { 0 } else { CF_DISABLED });
    // Auswahl kopieren: a selection
    let s = doc.is_some_and(|d| sel_active(p.doc.d(d)));
    pn_cmd_set(ui, CMD_COPY_SEL, if s { 0 } else { CF_DISABLED });
    // Auswahl einfügen: an image and clipboard text or an image
    let f = if img || txt { fd } else { CF_DISABLED };
    for id in CMD_PASTE_SEL..=CMD_PASTE_SEL_XOR {
        pn_cmd_set(ui, id, f);
    }
}

// ------------------------------------------------------------------ commands

/// clip_cmd(id, arg) -> handled
pub(crate) fn clip_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == UIC_DROP {
        clip_drop(p, ui);
        return true;
    }
    if !(CMD_CLIP_FIRST..=CMD_CLIP_LAST).contains(&id) {
        return false;
    }
    match id {
        CMD_CUT => clip_copy(p, ui, false, true),
        CMD_COPY => clip_copy(p, ui, false, false),
        CMD_COPY_MERGED => clip_copy(p, ui, true, false),
        CMD_PASTE => clip_paste(p, ui, PM_LAYER),
        CMD_PASTE_LAYER => clip_paste(p, ui, PM_NEWLAYER),
        CMD_PASTE_IMAGE => clip_paste(p, ui, PM_NEWIMAGE),
        CMD_COPY_SEL => clip_copy_sel(p, ui),
        _ if (CMD_PASTE_SEL..=CMD_PASTE_SEL_XOR).contains(&id) => clip_paste_sel(p, ui, id - CMD_PASTE_SEL), // SM_* in menu order
        CMD_K1_PROBE => k1_probe(p, ui, arg),
        CMD_K1_STATE => k1_state(p, ui),
        CMD_K1_ANSWER => {
            if p.clip.ask_pending {
                p.clip.ask_pending = false;
                p.clip.ask_open.set(0);
                paste_answer(p, ui, arg);
            } else if p.clip.drop_pending {
                p.clip.drop_pending = false;
                p.clip.drop_open.set(0);
                drop_answer(p, ui, arg);
            }
        }
        CMD_K1_ASKED => paste_answer(p, ui, arg),
        CMD_K1_DROPPED => drop_answer(p, ui, arg),
        _ => {} // the rest of the range: nothing
    }
    true
}

// ------------------------------------------------------------------ copy

/// copy_rect(doc) -> the selection bounds, the whole image without a selection; None for an
/// empty selection
fn copy_rect(d: &Doc) -> Option<[i32; 4]> {
    if sel_active(d) {
        return sel_bbox(d);
    }
    Some([0, 0, d.w, d.h])
}

/// clip_copy(merged, cut) -- Kopieren / Sichtbares kopieren / Ausschneiden
fn clip_copy(p: &mut Pn, ui: &mut Ui, merged: bool, cut: bool) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let Some(r) = copy_rect(p.doc.d(doc)) else { return }; // an empty selection: nothing
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    // the source: the composite or the current layer (straight ARGB): the rows first
    let sf = {
        let src: &Surface = if merged {
            comp_surface(p, doc)
        } else {
            match doc_cur_layer(p.doc.d(doc)) {
                Some(l) => &l.sf,
                None => return,
            }
        };
        if src.px.is_empty() {
            return;
        }
        let Some(mut sf) = surf_try(w, h) else {
            fio_error(p, Some("Kopieren"), "Die Zwischenablage hat die Daten nicht angenommen.");
            return;
        };
        let (ds, ss) = (sf.stride as usize, src.stride as usize);
        for y in 0..h as usize {
            let so = (r[1] as usize + y) * ss + r[0] as usize * 4;
            sf.px[y * ds..y * ds + w as usize * 4].copy_from_slice(&src.px[so..so + w as usize * 4]);
        }
        sf
    };
    let mut sf = sf;
    let d = p.doc.d(doc);
    if sel_active(d) {
        // alpha * coverage
        let ds = sf.stride as usize;
        let mut cov = [0u8; 64];
        for y in 0..h {
            let row = &mut sf.px[y as usize * ds..];
            let mut c = 0;
            while c < w {
                let n = (w - c).min(64);
                match sel_clip_row(d, r[0] + c, r[1] + y, n, &mut cov) as i32 {
                    SCR_ALL => {
                        c += n;
                        continue;
                    }
                    SCR_PART => {
                        for v in cov[..n as usize].iter_mut() {
                            *v = !*v;
                        }
                    }
                    _ => cov[..n as usize].fill(255),
                }
                erase_row(&mut row[c as usize * 4..(c + n) as usize * 4], &cov[..n as usize], 255);
                c += n;
            }
        }
    }
    if ui.clip_set_image(&sf) != 0 {
        fio_error(p, Some("Kopieren"), "Die Zwischenablage hat die Daten nicht angenommen.");
        return;
    }
    if cut {
        cut_erase(p, doc);
    }
}

/// cut_erase(doc) -- Ausschneiden: the selected pixels (the whole layer without a selection)
/// become transparent, one step "Ausschneiden"
fn cut_erase(p: &mut Pn, doc: DocId) {
    if sel_active(p.doc.d(doc)) {
        hist_begin_compound(p, doc);
        sel_erase(p, doc);
        hist_end_compound(p, doc, S_CUT, IC_CUT);
        return;
    }
    let d = p.doc.d(doc);
    match doc_cur_layer(d) {
        Some(l) if !l.sf.px.is_empty() => {}
        _ => return,
    }
    let (cur, w, h) = (d.cur, d.w, d.h);
    if hist_px_begin(p, doc, cur) != 0 {
        return;
    }
    if hist_px_touch(p, doc, 0, 0, w, h) != 0 {
        hist_px_cancel(p, doc);
        return;
    }
    {
        let d = p.doc.dm(doc);
        let ci = d.cur.clamp(0, d.layers.len() as i32 - 1) as usize;
        let l = &mut d.layers[ci];
        let (n, s) = (l.sf.w as usize * 4, l.sf.stride as usize);
        for y in 0..l.sf.h.max(0) as usize {
            l.sf.px[y * s..y * s + n].fill(0);
        }
    }
    doc_mark_px(p, doc, cur, 0, 0, w, h);
    hist_px_end(p, doc, S_CUT, IC_CUT);
}

/// clip_copy_sel() -- Auswahl kopieren: the selection geometry as text
fn clip_copy_sel(p: &mut Pn, ui: &mut Ui) {
    let Some(doc) = doc_active(&p.doc) else { return };
    if let Some(t) = sel_geom_text(p.doc.d(doc)) {
        ui.clip_set(CLIP_TEXT, t.as_bytes());
    }
}

/// clip_paste_sel(mode) -- Auswahl einfügen: SM_* combine of the clipboard geometry (text of
/// Auswahl kopieren, else the clipboard image's rectangle at 0, 0)
fn clip_paste_sel(p: &mut Pn, ui: &mut Ui, mode: u32) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let mut poly = ui.clip_get(CLIP_TEXT).and_then(|t| sel_geom_parse(&t));
    if poly.is_none() {
        match ui.clip_get_image() {
            Ok(sf) => {
                poly = sel_poly_rect(0, 0, sf.w, sf.h);
                if poly.is_none() {
                    return;
                }
            }
            Err(_) => {
                fio_error(p, Some("Auswahl einfügen"), "Nichts zum Einfügen: Die Zwischenablage hat weder Auswahl noch Bild.");
                return;
            }
        }
    }
    let poly = poly.unwrap();
    let name = PS_NAMES[if mode < crate::app::constants::SM_COUNT as u32 { mode as usize } else { 0 }];
    if hist_push_sel(p, doc, name, IC_PASTE) {
        sel_combine(p, doc, &poly, mode, SQ_AA as u32);
    }
}

// ------------------------------------------------------------------ paste

/// paste_pos(doc, w, h) -> (x, y) -- the top left of the visible canvas area (paint.net:
/// (0, 0) while it is visible), kept inside the image when the pasted image fits
fn paste_pos(p: &Pn, doc: DocId, w: i32, h: i32) -> (i32, i32) {
    let cr = view_canvas_rect(&p.view);
    let (fx, fy) = view_screen_to_doc_fx(&p.view, cr[0] << 8, cr[1] << 8); // 24.8 in and out
    let mut x = fx.wrapping_add(255) >> 8; // ceil of 24.8
    let mut y = fy.wrapping_add(255) >> 8;
    let d = p.doc.d(doc);
    x = x.min(d.w - w).max(0);
    y = y.min(d.h - h).max(0);
    (x, y)
}

/// pend_free() -- drop the image waiting for the question
fn pend_free(p: &mut Pn) {
    p.clip.pend_sf = None;
    p.clip.pend_doc = None;
}

/// clip_paste(mode) -- Einfügen / Einfügen als neue Ebene / Einfügen als neues Bild
fn clip_paste(p: &mut Pn, ui: &mut Ui, mode: u32) {
    if p.clip.ask_pending {
        return; // a question is open
    }
    let doc = doc_active(&p.doc);
    if mode != PM_NEWIMAGE && doc.is_none() {
        return;
    }
    let sf = match ui.clip_get_image() {
        Ok(sf) => sf,
        Err(_) => {
            let m = if mode == PM_NEWIMAGE { "In der Zwischenablage liegt kein Bild, aus dem ein neues entstehen kann." } else { "In der Zwischenablage liegt kein Bild." };
            fio_error(p, Some("Einfügen"), m);
            return;
        }
    };
    if mode == PM_NEWIMAGE {
        let Some(nd) = doc_new(p, sf.w, sf.h, 0xFFFF_FFFF, 0) else { return }; // white background (paint.net)
        hist_init(p, nd, "Neues Bild", IC_PAGE);
        tmove_paste(p, ui, nd, sf, 0, 0);
        return;
    }
    let doc = doc.unwrap();
    // larger than the canvas: ask first
    let d = p.doc.d(doc);
    if sf.w > d.w || sf.h > d.h {
        pend_free(p);
        p.clip.pend_sf = Some(sf);
        p.clip.pend_doc = Some(doc);
        p.clip.pend_mode = mode;
        p.clip.ask_open.set(1);
        p.clip.ask_pending = true;
        ui.repass();
        return;
    }
    paste_go(p, ui, doc, sf, mode, false);
}

/// paste_go(doc, sf, mode, enlarge) -- (answered) paste sf (taken over) into doc
fn paste_go(p: &mut Pn, ui: &mut Ui, doc: DocId, sf: Surface, mode: u32, enlarge: bool) {
    if enlarge {
        // Leinwand vergrößern: anchored top left, the secondary colour on the background
        let d = p.doc.d(doc);
        let par = IopPar { w: d.w.max(sf.w), h: d.h.max(sf.h), anchor: AN_TL as u32, fill: pal_secondary(&p.pal), ..IopPar::default() };
        iop_canvas(p, doc, &par);
    }
    if mode == PM_NEWLAYER {
        if !hist_push_layers(p, doc, "Neue Ebene", IC_ADD) {
            return;
        }
        let cur = p.doc.d(doc).cur + 1;
        if lay_new(p, doc, cur, None).is_none() {
            return;
        }
        p.doc.dm(doc).cur = cur;
    }
    let (x, y) = paste_pos(p, doc, sf.w, sf.h);
    tmove_paste(p, ui, doc, sf, x, y);
}

/// paste_answer(a) -- the question: 1 enlarge, 2 keep the size, else cancel
fn paste_answer(p: &mut Pn, ui: &mut Ui, a: u32) {
    p.clip.ask_pending = false;
    p.clip.ask_open.set(0);
    let doc = p.clip.pend_doc;
    let alive = doc.is_some_and(|d| doc_index(&p.doc, d) >= 0); // the image was closed meanwhile?
    if !alive || a.wrapping_sub(1) > 1 {
        pend_free(p);
        return;
    }
    let sf = p.clip.pend_sf.take();
    let mode = p.clip.pend_mode;
    p.clip.pend_doc = None;
    if let Some(sf) = sf {
        paste_go(p, ui, doc.unwrap(), sf, mode, a == 1);
    }
}

// ------------------------------------------------------------------ drops

/// clip_drop() -- UIC_DROP: keep a copy of the list, then open / ask
fn clip_drop(p: &mut Pn, ui: &mut Ui) {
    let n = ui.drop_count();
    if n == 0 || p.clip.drop_pending {
        return; // (the last drop is still being asked about)
    }
    let list: Vec<String> = ui.drop_list().iter().take_while(|s| !s.is_empty()).cloned().collect();
    p.clip.drop_list = Some(list);
    p.clip.drop_n = n;
    if doc_active(&p.doc).is_none() {
        drop_answer(p, ui, 1); // no image: open them
        return;
    }
    p.clip.drop_open.set(1);
    p.clip.drop_pending = true;
    ui.repass();
}

/// drop_answer(a) -- 1 open every file, 2 import them as layers, else nothing
fn drop_answer(p: &mut Pn, ui: &mut Ui, a: u32) {
    p.clip.drop_pending = false;
    p.clip.drop_open.set(0);
    let Some(list) = p.clip.drop_list.take() else { return };
    if a == 2 {
        if let Some(doc) = doc_active(&p.doc) {
            lay_import_paths(p, ui, doc, &list);
        }
    } else if a == 1 {
        for path in &list {
            fio_queue_path(p, ui, path);
        }
    }
}

// ------------------------------------------------------------------ frame

/// clip_frame() -- the questions and the probe records
pub(crate) fn clip_frame(p: &mut Pn, ui: &mut Ui) {
    ask_window(p, ui);
    drop_window(p, ui);
    k1_sem(ui, ID_K1_PROBE, &p.clip.probe_txt, p.clip.probe_val);
    let q = p.clip.ask_open.get() | p.clip.drop_open.get() << 1;
    k1_sem(ui, ID_K1_QUEST, "", q);
}

// k1_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn k1_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

/// ask_window() -- "Einfügen": the clipboard image is larger than the canvas
fn ask_window(p: &mut Pn, ui: &mut Ui) {
    if p.clip.ask_pending && p.clip.ask_open.get() == 0 {
        p.clip.ask_pending = false; // Esc / close button: Abbrechen
        ui.cmd_run(CMD_K1_ASKED, 0);
    }
    let (open, rect) = (p.clip.ask_open.clone(), p.clip.ask_rect.clone());
    if ui.window_begin(WINID_K1_PASTE, crate::preferences::localization::tr("Einfügen"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let mut answer = 0;
        lay_lines(ui, TS_BODY, "Das Bild ragt über die Leinwand hinaus.\nWie soll es weitergehen?");
        ui.box_begin(0, BOX_V, 4);
        if ui.accent_button(ID_K1_ENLARGE, "Leinwand vergrößern") & EV_ACT != 0 {
            answer = 1;
        }
        lay_lines(ui, TS_CAPTION | TX_SECONDARY, "Die Leinwand wächst mit, bis das neue Bild\nvollständig darauf passt.");
        ui.box_end();
        ui.box_begin(0, BOX_V, 4);
        if ui.button(ID_K1_KEEP, crate::preferences::localization::tr("Leinwand nicht ändern")) & EV_ACT != 0 {
            answer = 2;
        }
        lay_lines(ui, TS_CAPTION | TX_SECONDARY, "Die Leinwand bleibt, wie sie ist. Was über den Rand ragt,\nwird abgeschnitten; schieben Sie das Bild so, dass der\ngewünschte Teil auf der Leinwand liegt.");
        ui.box_end();
        if ui.dialog_buttons(None, None, Some("Abbrechen")) != DLG_NONE {
            answer = 3; // Abbrechen
        }
        if answer != 0 {
            p.clip.ask_open.set(0);
            p.clip.ask_pending = false;
            ui.cmd_run(CMD_K1_ASKED, if answer == 3 { 0 } else { answer });
        }
    }
    ui.window_end();
}

/// drop_window() -- "Drag & Drop": open the files or add them as layers
fn drop_window(p: &mut Pn, ui: &mut Ui) {
    if p.clip.drop_pending && p.clip.drop_open.get() == 0 {
        p.clip.drop_pending = false; // Esc / close button: Abbrechen
        ui.cmd_run(CMD_K1_DROPPED, 0);
    }
    let (open, rect) = (p.clip.drop_open.clone(), p.clip.drop_rect.clone());
    if ui.window_begin(WINID_K1_DROP, crate::preferences::localization::tr("Drag & Drop"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let mut answer = 0;
        let k = if p.clip.drop_n > 1 { 1 } else { 0 }; // plural
        lay_lines(ui, TS_BODY, DW_TEXTS[0][k]);
        ui.box_begin(0, BOX_V, 4);
        if ui.accent_button(ID_K1_OPEN, "Öffnen") & EV_ACT != 0 {
            answer = 1;
        }
        lay_lines(ui, TS_CAPTION | TX_SECONDARY, DW_TEXTS[1][k]);
        ui.box_end();
        ui.box_begin(0, BOX_V, 4);
        if ui.button(ID_K1_IMPORT, DW_TEXTS[2][k]) & EV_ACT != 0 {
            answer = 2;
        }
        lay_lines(ui, TS_CAPTION | TX_SECONDARY, DW_TEXTS[3][k]);
        ui.box_end();
        if ui.dialog_buttons(None, None, Some("Abbrechen")) != DLG_NONE {
            answer = 3;
        }
        if answer != 0 {
            p.clip.drop_open.set(0);
            p.clip.drop_pending = false;
            ui.cmd_run(CMD_K1_DROPPED, if answer == 3 { 0 } else { answer });
        }
    }
    ui.window_end();
}

// ------------------------------------------------------------------ test hooks

/// k1_probe(x | y << 16) -- #0x82200 = "W,H,AARRGGBB" of the decoded clipboard image
fn k1_probe(p: &mut Pn, ui: &mut Ui, arg: u32) {
    match ui.clip_get_image() {
        Ok(sf) => {
            // x, y clamped into the image
            let x = (arg & 0xFFFF).min((sf.w - 1) as u32);
            let y = (arg >> 16).min((sf.h - 1) as u32);
            let o = y as usize * sf.stride as usize + x as usize * 4;
            let v = u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]]);
            p.clip.probe_txt = format!("{},{},{:08X}", sf.w as u32, sf.h as u32, v);
            p.clip.probe_val = 1;
        }
        Err(_) => {
            p.clip.probe_txt = String::from("none");
            p.clip.probe_val = 0;
        }
    }
    ui.repass();
}

/// k1_state() -- #0x82200 = "docs,layers,cur,W,H" of the active image, value = enabled commands
/// (a test command commits a pending edit first, like every command)
fn k1_state(p: &mut Pn, ui: &mut Ui) {
    let n = doc_count(&p.doc) as u32;
    let (nl, cur, w, h) = match doc_active(&p.doc) {
        Some(doc) => {
            let d = p.doc.d(doc);
            (d.layers.len() as u32, d.cur as u32, d.w as u32, d.h as u32)
        }
        None => (0, 0, 0, 0),
    };
    p.clip.probe_txt = format!("{n},{nl},{cur},{w},{h}");
    clip_states(p, ui); // the states of this moment
    let mut v = 0;
    for (i, &id) in KS_CMDS.iter().enumerate() {
        if pn_cmd_flags(ui, id) & CF_DISABLED == 0 {
            v |= 1 << i;
        }
    }
    p.clip.probe_val = v;
    ui.repass();
}
