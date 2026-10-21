// t_text -- PaintCraft: Text (T), agent T5.  Sub-module of the tool framework (tool.s).
//                                              [apps/PaintCraft/src/t_text.s]
//
// paint.net 5.1's text tool: a left click places the text cursor (the anchor is the top of
// the first line: its left end, centre or right end by the alignment), typing inserts UTF-8
// text (several lines with Enter; Rücktaste / Entf / Pfeile / Pos1 / Ende (Strg: whole text),
// Strg+V / Umschalt+Einfg paste the text clipboard), a blinking caret and a handle
// (Ziehpunkt, lower right corner of the text) that moves the text block; Strg held hides
// caret and handle.  The text stays LIVE: font, size, style, alignment, rendering mode,
// antialiasing, fill style, blend mode and the primary colour apply to it until
// Fertigstellen / a click outside the text / another tool or image / a document command
// (one history step "Text"; an empty text makes none); Esc / Strg+Z drop it.  Enter is a
// line break here (TF_TEXTKEYS: tool.rs leaves Enter to TD_KEY).  A click inside the text
// moves the caret.
// Rendering: the laid-out lines are drawn with text draw_n into 64-row bands (A8; colour
// fonts: PARGB) and stored as the coverage TMASK of the tool layer: TLK_SOLID with the fill
// style / primary colour / blend mode, colour fonts TLK_SOURCE with the glyph colours.  The
// preview is the result (tl_commit).  Antialiasing off = coverage >= 128 -> 255.
// Fonts: the system font list (fontdb) is scanned once on the worker pool when the tool is
// first chosen (cache <config dir>/PaintCraft/fontcache.fdb, not written in headless runs);
// the UI thread touches fontdb only after the job has finished.  Until then, and when a
// family cannot be loaded, the embedded JFLib Sans renders (Selawik, identical outlines).
// Default family: Segoe UI, else DejaVu Sans / Noto Sans / Liberation Sans / Ubuntu /
// Cantarell / Arial, else the first family with "Sans" in its name.
//
//   ttext_init()                      defaults, register TOOL_TEXT
//   ttext_cmd(id, arg) -> true        test hooks CMD_TTEXT_FIRST .. CMD_TTEXT_LAST:
//       840 test font: arg = an embedded / loaded font id (FONT_REG ..), -1 = the family
//       841 size in points (1 .. 2000)
//       842 options: bits 0-3 Fett / Kursiv / Unterstrichen / Durchgestrichen, bits 4-5
//           alignment (0 links, 1 zentriert, 2 rechts), bits 6-7 rendering mode (0 Glatt,
//           1 Scharf (Modern), 2 Scharf (Klassisch)), bit 8 metric (1 = Fest (96 DPI))
//       843 move the live text: anchor = x | y << 16 (signed 16-bit each)
//       844 misc: 1 = wait for the font scan, 2 | i << 8 = caret to byte i,
//           3 | i << 8 = family i of the font list
// Hidden records (emitted with the options while the tool is current), ids ID_TTEXT_FIRST+k:
//   +0 the text (value 1 = live)        +1 "caret,line,ax,ay" (value caret)
//   +2 family (value size in points)    +3 "style,align,render,metric" (value font id)
//   +4 box "x0,y0,x1,y1" of the tool layer (value font scan 0 / 1 running / 2 done)
//   +5 "families,filtered" (value 1 = the filtered list has entries)
//   +6 "" (value 1 = the font comes from a system font file)
//   +7 text rectangle "x0,y0,x1,y1" (image px; the handle is at its lower right corner)
//
// Rust model: the coverage mask lives in the tool layer record (TlMask::Own) while a text is
// live, the colour fonts' source surface there too (TlSrc::Own); the font database is shared
// with the scan job (Arc + SpinLock, the job holds the lock while it scans).  The asm patches
// TD_HINT at run time: here the tool is registered with a second descriptor while live.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use gfx::tmask::{Block, TMask, TM_REPLACE};
use gfx::{Dc, Surface, FMT_A8, FMT_ARGB, FMT_PARGB};
use rt::sync::SpinLock;
use ::text::fontdb::FontDb;
use ::text::{FONT_BOLD, FONT_REG, FS_SHARP, FS_SYN_BOLD, FS_SYN_ITALIC, TD_STRIKEOUT, TD_UNDERLINE};
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, DocId};
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::workspace::palette::pal_version;
use crate::tools::*;
use crate::workspace::view::{view_cursor, view_doc_to_screen_fx, view_dpi100};
use crate::app::Pn;

const TX_CAP: usize = 16384; // text bytes (incl. the NUL)
const TX_LINES: usize = 1024; // lines at most
const FAM_CAP: usize = 256;
const SRCH_CAP: u32 = 128;
const PATH_CAP: usize = 512;
const FILT_MAX: usize = 8192;
const NCP: usize = 256; // typed codepoints taken per frame
const BLINK_MS: u32 = 530;
const SIZE_MIN: i32 = 1;
const SIZE_MAX: i32 = 2000;
const SZFX_MAX: u64 = 8192 << 8; // largest pixel size (24.8)
const TXS_BOLD: u32 = 1;
const TXS_ITALIC: u32 = 2;
const ROW_H: u32 = 28; // font list row (DIP)
const NSIZES: usize = 23;
const BAND: i32 = 64; // rows per band (= TMASK tile)
const ID_TX0: u32 = (TOOLOPT_BASE + TOOL_TEXT * TOOLOPT_STRIDE) as u32; // 3000
const ID_TX_SZDEC: u32 = ID_TX0 + 1;
const ID_TX_SIZE: u32 = ID_TX0 + 2;
const ID_TX_SZLIST: u32 = ID_TX0 + 3;
const ID_TX_SZINC: u32 = ID_TX0 + 4;
const ID_TX_METRIC: u32 = ID_TX0 + 5; // metric rows +6, +7 (in the size drop-down)
const ID_TX_BOLD: u32 = ID_TX0 + 8; // +8 .. +11 Fett, Kursiv, Unterstrichen, Durchgestrichen
const ID_TX_RENDER: u32 = ID_TX0 + 12; // rows +13 .. +15
const ID_TX_ALIGN: u32 = ID_TX0 + 16; // rows +17 .. +19
const ID_FONT_SEARCH: u32 = ID_FONT_LIST + 1;
const ID_FONT_LV: u32 = ID_FONT_LIST + 2; // rows ID_FONT_LV + 1 + i
const ID_TX_REC: u32 = ID_TTEXT_FIRST; // hidden records +0 .. +7
const ID_TX_SZLV: u32 = ID_TTEXT_FIRST + 0x0F; // size list (rows ID_TX_SZLV + 1 + i)
const CMD_TX_FONT: u32 = CMD_TTEXT_FIRST;
const CMD_TX_SIZE: u32 = CMD_TTEXT_FIRST + 1;
const CMD_TX_OPTS: u32 = CMD_TTEXT_FIRST + 2;
const CMD_TX_MOVE: u32 = CMD_TTEXT_FIRST + 3;
const CMD_TX_MISC: u32 = CMD_TTEXT_FIRST + 4;
const RANGE_SIZE: RangeF = RangeF::new(SIZE_MIN as f64, SIZE_MAX as f64, 0.0, 2, 0);

static DEF_FAMILIES: [&str; 7] = ["Segoe UI", "DejaVu Sans", "Noto Sans", "Liberation Sans", "Ubuntu", "Cantarell", "Arial"];
// labels and tooltips of the text tool row
static RENDER_LABELS: [&str; 3] = ["Glatt", "Scharf (Modern)", "Scharf (Klassisch)"];
static ALIGN_LABELS: [&str; 3] = ["Linksbündig", "Zentrieren", "Rechtsbündig"];
static METRIC_LABELS: [&str; 2] = ["Punkte (Bild DPI)", "Fest (96 DPI)"];
static STYLE_TIPS: [&str; 4] = ["Fett", "Kursiv", "Unterstrichen", "Durchgestrichen"];
static ALIGN_ICONS: [u32; 3] = [IC_ALIGN_LEFT, IC_ALIGN_CENTER, IC_ALIGN_RIGHT];
static STYLE_ICONS: [u32; 4] = [IC_BOLD, IC_ITALIC, IC_UNDERLINE, IC_STRIKETHROUGH];
/// paint.net's size list (points)
static SIZE_TAB: [u16; NSIZES] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72, 84, 96, 108, 144, 192, 216, 288];
const S_SZDEC_TIP: &str = "Kleinere Schrift\nMit Strg in Fünferschritten";
const S_SZINC_TIP: &str = "Größere Schrift\nMit Strg in Fünferschritten";
const S_HINT_LIVE: &str = "Am Anfasser lässt sich der Text verschieben. Mit gedrückter Strg-Taste verschwinden Anfasser und Schreibmarke.";

/// a layout line (LN_*)
#[derive(Clone, Copy, Default)]
struct Line {
    /// byte offset, bytes
    start: u32,
    len: u32,
    /// x of the line (image px), width (24.8)
    x: i32,
    w: i32,
}

/// the state of t_text.s (its g_* globals)
pub(crate) struct TText {
    preedit: plat::Preedit,
    fallback_fonts: Vec<u32>,
    fallback_loaded: bool,
    /// the font scan (background job) and the database it fills
    job: plat::Job,
    db: Arc<SpinLock<FontDb>>,
    nub: [Nub; 1],
    /// band (A8 / colour fonts PARGB); colour fonts: coverage band (A8), straight glyph colours
    scr: Surface,
    band: Surface,
    src: Surface,
    /// text rectangle (image px); tiles of the last rendering (x0, y0, x1, y1); caret as drawn
    /// (screen px)
    rect: [i32; 4],
    mbox: [i32; 4],
    crect: [i32; 4],
    lines: Vec<Line>,
    /// the text (UTF-8, without the asm's NUL)
    buf: Vec<u8>,
    family: String,
    search: String,
    filt: Vec<u32>,
    doc: Option<DocId>,
    live: bool,
    caret: u32,
    anchor: u32,
    selecting: bool,
    highlights: Vec<[i32;4]>,
    /// anchor (image px)
    ax: i32,
    ay: i32,
    /// points; 0 Punkte (Bild DPI), 1 Fest (96 DPI); TXS_*; 0 links, 1 zentriert, 2 rechts;
    /// 0 Glatt, 1 Scharf (Modern), 2 Scharf (Klassisch)
    size: f64,
    metric: u32,
    style: u32,
    align: u32,
    render: u32,
    /// -1 or a font id (hook 840); the resolved font id
    testfont: i32,
    font: u32,
    fromfile: bool,
    /// size in px (24.8), ascent, line height, lines
    szfx: i32,
    asc: i32,
    lh: i32,
    nlines: u32,
    /// 0 / 1 running / 2 done
    scan: u32,
    nosave: bool,
    /// the family was chosen
    famset: bool,
    lsel: i32,
    fopen: bool,
    szopen: bool,
    pver: u32,
    palv: u32,
    vsel: u32,
    vcur: i32,
    blink0: u32,
    drag: bool,
    ddx: i32,
    ddy: i32,
}

impl Default for TText {
    fn default() -> TText {
        TText {
            job: plat::Job::new(),
            db: Arc::new(SpinLock::new(FontDb::new())),
            nub: [Nub::default()],
            scr: Surface::default(),
            band: Surface::default(),
            src: Surface::default(),
            rect: [0; 4],
            mbox: [0; 4],
            crect: [0; 4],
            preedit: Default::default(),
            fallback_fonts: Vec::new(), fallback_loaded: false,
            lines: vec![Line::default(); TX_LINES],
            buf: Vec::new(),
            family: String::new(),
            search: String::new(),
            filt: Vec::new(),
            doc: None,
            live: false,
            caret: 0,
            anchor: 0,
            selecting: false,
            highlights: Vec::new(),
            ax: 0,
            ay: 0,
            size: 0.0,
            metric: 0,
            style: 0,
            align: 0,
            render: 0,
            testfont: 0,
            font: 0,
            fromfile: false,
            szfx: 0,
            asc: 0,
            lh: 0,
            nlines: 0,
            scan: 0,
            nosave: false,
            famset: false,
            lsel: 0,
            fopen: false,
            szopen: false,
            pver: 0,
            palv: 0,
            vsel: 0,
            vcur: 0,
            blink0: 0,
            drag: false,
            ddx: 0,
            ddy: 0,
        }
    }
}

const TEXT_FLAGS: u32 = (TF_LIVE | TF_CLIP_SEL | TF_AA | TF_BLEND | TF_FILL | TF_TEXTKEYS) as u32;

static DESC_TEXT: ToolDesc = ToolDesc {
    flags: TEXT_FLAGS,
    activate: Some(tx_activate),
    input: Some(tx_input),
    options: Some(tx_options),
    key: Some(tx_key),
    commit: Some(tx_commit),
    cancel: Some(tx_cancel),
    cursor: Some(tx_cursor),
    overlay: Some(tx_overlay),
    tick: Some(tx_tick),
    ..ToolDesc::NONE
};

/// DESC_TEXT while a text is live: the "Ziehpunkt" hint (the asm patches TD_HINT)
static DESC_TEXT_LIVE: ToolDesc = ToolDesc {
    flags: TEXT_FLAGS,
    hint: S_HINT_LIVE,
    activate: Some(tx_activate),
    input: Some(tx_input),
    options: Some(tx_options),
    key: Some(tx_key),
    commit: Some(tx_commit),
    cancel: Some(tx_cancel),
    cursor: Some(tx_cursor),
    overlay: Some(tx_overlay),
    tick: Some(tx_tick),
    ..ToolDesc::NONE
};

/// ttext_init() -- defaults (Segoe UI, 12 pt) and TOOL_TEXT
pub(crate) fn ttext_init(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.ttext;
    t.size = 12.0;
    t.testfont = plat::arg_value("--text-font").map_or(-1, |path| ui.text.load_file(&path,0));
    t.lsel = -1;
    t.family = String::from("Segoe UI");
    tool_register(&mut p.tool, TOOL_TEXT, Some(&DESC_TEXT));
}

/// ttext_cmd(id, arg) -> 1 handled -- the test hooks 840 .. 844 (see the file header)
pub(crate) fn ttext_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let changed = match id {
        CMD_TX_FONT => {
            if arg as i32 != -1 && !ui.text.valid(arg) {
                false
            } else {
                p.ttext.testfont = arg as i32;
                true
            }
        }
        CMD_TX_SIZE => {
            tx_size_set(&mut p.ttext, arg as f64);
            true
        }
        CMD_TX_OPTS => {
            let t = &mut p.ttext;
            t.style = arg & 15;
            t.align = ((arg >> 4) & 3).min(2);
            t.render = ((arg >> 6) & 3).min(2);
            t.metric = (arg >> 8) & 1;
            true
        }
        CMD_TX_MOVE => {
            if p.ttext.live {
                p.ttext.ax = arg as u16 as i16 as i32;
                p.ttext.ay = (arg as i32) >> 16;
                tx_update(p, ui);
            }
            false
        }
        CMD_TX_MISC => {
            let k = arg & 255;
            let i = arg >> 8;
            if k == 1 {
                tx_scan_start(p, ui);
                if p.ttext.scan == 1 {
                    let _ = p.ttext.job.wait();
                    tx_scan_poll(p, ui);
                }
            } else if k == 4 { plat::ime_preedit_set(0,"にほん",6);
            } else if k == 5 { plat::ime_preedit_set(0,"",0);
            } else if k == 6 { plat::ime_commit_set(0,&"日".repeat(200));
            } else if k == 2 {
                if p.ttext.live {
                    p.ttext.caret = i.min(p.ttext.buf.len() as u32);
                    while p.ttext.caret>0 && p.ttext.buf.get(p.ttext.caret as usize).is_some_and(|v|v&0xC0==0x80) {p.ttext.caret-=1;}
                    p.ttext.anchor=p.ttext.caret;
                    tx_caret_moved(p, ui);
                }
            } else if k == 3 && p.ttext.scan == 2 && (i as usize) < p.ttext.db.lock().family_count() {
                tx_choose_family(p, ui, i as usize);
            }
            false
        }
        _ => return false,
    };
    if changed {
        tool_cfg_changed(p);
        tx_update(p, ui);
    }
    ui.repass();
    true
}

// ================================================================== options

// tx_size_set(pt) -- font size clamped to 1 .. 2000 points
fn tx_size_set(t: &mut TText, pt: f64) {
    t.size = if pt.is_finite() {pt.clamp(SIZE_MIN as f64, SIZE_MAX as f64)} else {12.0};
}

// tx_size_step(dir) -- size +- 1 (Strg: 5)
fn tx_size_step(p: &mut Pn, ui: &Ui, dir: i32) {
    let d = if ui.mods() & MOD_CTRL != 0 { dir * 5 } else { dir };
    let s = p.ttext.size + d as f64;
    tx_size_set(&mut p.ttext, s);
}

// tx_refocus() -- while a text is live the keyboard goes back to the canvas (after a click on
// an option button the typed text and Space / Enter must reach the text again)
fn tx_refocus(p: &Pn, ui: &mut Ui) {
    if p.ttext.live {
        ui.focus(ID_CANVAS);
    }
}

// tx_choose_family(i) -- family i of fontdb becomes the text's font
fn tx_choose_family(p: &mut Pn, ui: &mut Ui, i: usize) {
    let name = String::from(p.ttext.db.lock().family_name(i));
    p.ttext.family = String::from(cut(&name, FAM_CAP));
    p.ttext.famset = true;
    tool_cfg_changed(p);
    tx_update(p, ui);
}

// str_copy: at most cap - 1 bytes, never splitting a UTF-8 sequence
fn cut(s: &str, cap: usize) -> &str {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

// tx_options() -- the tool row: Schriftart, size (list + metric), B I U S, rendering mode,
// alignment, Fülleffekt, Füllmethode, Antialiasing (the framework adds Fertigstellen)
fn tx_options(p: &mut Pn, ui: &mut Ui) {
    tx_records(p, ui);
    opt_label(ui, "Schriftart:");
    tx_font_widget(p, ui); // (refocuses itself: its search box types)
    let mut ch = tx_size_widgets(p, ui);
    ui.spacer(4);
    ch |= tx_style_buttons(p, ui);
    ui.spacer(4);
    let mut v = p.ttext.render;
    ch |= opt_menu(p, ui, ID_TX_RENDER, &mut v, &RENDER_LABELS, &[], Some("Textdarstellung"), OMF_NOICON as u32);
    p.ttext.render = v;
    let mut v = p.ttext.align;
    ch |= opt_menu(p, ui, ID_TX_ALIGN, &mut v, &ALIGN_LABELS, &ALIGN_ICONS, None, OMF_ICONONLY as u32);
    p.ttext.align = v;
    ui.spacer(4);
    ch |= opt_fill(p, ui);
    ch |= opt_blend(p, ui);
    ch |= opt_aa(p, ui);
    if ch {
        tx_update(p, ui);
        tx_refocus(p, ui);
        ui.repass();
    }
}

// tx_font_widget() -> 1 changed -- the family drop-down: a search box (filters the list, the
// first match is taken at once) and the virtualized family list
fn tx_font_widget(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut changed = false;
    ui.next_size(130, 0);
    let fam = p.ttext.family.clone();
    let ev = ui.dropdown_button(ID_FONT_LIST, &fam);
    ui.tooltip("Schriftart");
    // opened just now: focus in the search box, the current family scrolled into view
    let open = ev & EV_OPEN != 0;
    if open != p.ttext.fopen {
        p.ttext.fopen = open;
        p.ttext.search.clear(); // opened or closed: the whole list again
        tx_filter(&mut p.ttext);
        if p.ttext.fopen {
            ui.focus(ID_FONT_SEARCH);
            let r = (p.ttext.lsel - 4).max(0);
            ui.scroll_to(ID_FONT_LV, -1, r.wrapping_mul(ROW_H as i32));
        }
    }
    if ui.flyout_begin(ID_FONT_LIST, FL_BELOW | FL_MINW) {
        ui.next_size(260, 0);
        let mut s = core::mem::take(&mut p.ttext.search);
        let ev = ui.textbox(ID_FONT_SEARCH, Some("Suchen"), &mut s, SRCH_CAP);
        p.ttext.search = s;
        if ev & EV_ACT != 0 {
            tx_filter(&mut p.ttext);
            ui.scroll_to(ID_FONT_LV, -1, 0);
            if !p.ttext.filt.is_empty() {
                let f = p.ttext.filt[0] as usize;
                tx_choose_family(p, ui, f);
                p.ttext.lsel = 0;
                changed = true;
            }
        }
        if p.ttext.scan != 2 {
            ui.text(TS_BODY, crate::preferences::localization::tr("Schriften werden eingelesen…"));
        } else {
            ui.next_size(260, 320);
            let mut clicked = -1;
            let nf = p.ttext.filt.len() as i32;
            let mut sel = p.ttext.lsel;
            let (first, end) = ui.list_begin(ID_FONT_LV, &mut sel, nf, ROW_H);
            p.ttext.lsel = sel;
            let mut row = first;
            while row < end && row < nf {
                let name = String::from(p.ttext.db.lock().family_name(p.ttext.filt[row as usize] as usize));
                if ui.list_item(row, &name) & EV_ACT != 0 {
                    clicked = row;
                }
                row += 1;
            }
            let mut sel = p.ttext.lsel;
            let (lev, _) = ui.list_end(&mut sel);
            p.ttext.lsel = sel;
            if clicked >= 0 {
                p.ttext.lsel = clicked;
                let f = p.ttext.filt[clicked as usize] as usize;
                tx_choose_family(p, ui, f);
                ui.popup_close();
                tx_refocus(p, ui);
                changed = true;
            } else if lev & EV_ACT != 0 {
                // keyboard: the selection follows at once
                let s = p.ttext.lsel;
                if s >= 0 && s < p.ttext.filt.len() as i32 {
                    let f = p.ttext.filt[s as usize] as usize;
                    tx_choose_family(p, ui, f);
                    changed = true;
                }
            }
        }
    }
    ui.flyout_end();
    changed
}

// tx_size_widgets() -> 1 changed -- the size box and its drop-down: [A-] [A+] (Strg: 5),
// paint.net's size list, the size metric (Punkte (Bild DPI) / Fest (96 DPI))
fn tx_size_widgets(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut changed = false;
    ui.next_size(48, 0);
    let ev = ui.number_box_f(ID_TX_SIZE, &mut p.ttext.size, &RANGE_SIZE);
    if ev & EV_ACT != 0 {
        changed = true;
    }
    if ev & EV_COMMIT != 0 {
        tx_refocus(p, ui);
    }
    // the row of the current size in paint.net's list (-1)
    let mut sel = -1;
    for (i, &s) in SIZE_TAB.iter().enumerate() {
        if s as f64 == p.ttext.size {
            sel = i as i32;
        }
    }
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
    let ev = ui.dropdown_button(ID_TX_SZLIST, crate::preferences::localization::tr("Schriftgröße"));
    // opened just now: the current size scrolled into view
    let open = ev & EV_OPEN != 0;
    if open != p.ttext.szopen {
        p.ttext.szopen = open;
        if open {
            let r = (sel - 3).max(0);
            ui.scroll_to(ID_TX_SZLV, -1, r * 30);
        }
    }
    let mut chosen = -1;
    if ui.flyout_begin(ID_TX_SZLIST, FL_BELOW) {
        // [A-] [A+]
        ui.box_begin(0, BOX_H, 4);
        if ui.icon_button(ID_TX_SZDEC, IC_FONT_DECREASE, S_SZDEC_TIP) & EV_ACT != 0 {
            tx_size_step(p, ui, -1);
            changed = true;
        }
        if ui.icon_button(ID_TX_SZINC, IC_FONT_INCREASE, S_SZINC_TIP) & EV_ACT != 0 {
            tx_size_step(p, ui, 1);
            changed = true;
        }
        ui.box_end();
        // the size list
        ui.next_size(120, 300);
        let (first, end) = ui.list_begin(ID_TX_SZLV, &mut sel, NSIZES as i32, 30);
        let mut row = first;
        while row < end && (row as usize) < NSIZES {
            let s = format!("{}", SIZE_TAB[row as usize]);
            if ui.list_item(row, &s) & EV_ACT != 0 {
                chosen = row;
            }
            row += 1;
        }
        ui.list_end(&mut sel);
        // the metric
        ui.menu_sep();
        for b in 0..2u32 {
            if b == p.ttext.metric {
                ui.next_flags(ST_CHECKED);
            }
            if ui.menu_item(ID_TX_METRIC + 1 + b, METRIC_LABELS[b as usize]) & EV_ACT != 0 && b != p.ttext.metric {
                p.ttext.metric = b;
                changed = true;
            }
        }
    }
    ui.flyout_end();
    if chosen >= 0 && (chosen as usize) < NSIZES {
        tx_size_set(&mut p.ttext, SIZE_TAB[chosen as usize] as f64);
        ui.popup_close();
        changed = true;
    }
    if changed {
        tool_cfg_changed(p);
    }
    changed
}

// tx_style_buttons() -> 1 changed -- Fett, Kursiv, Unterstrichen, Durchgestrichen
fn tx_style_buttons(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut changed = false;
    for b in 0..4u32 {
        let mut checked = ((p.ttext.style >> b) & 1) as i32;
        if ui.tool_button(ID_TX_BOLD + b, STYLE_ICONS[b as usize], STYLE_TIPS[b as usize], &mut checked, 1) & EV_ACT != 0 {
            p.ttext.style ^= 1 << b;
            changed = true;
        }
    }
    if changed {
        tool_cfg_changed(p);
    }
    changed
}

// tx_sem(id, label, value) -- a hidden K_TEXT record (no rectangle)
fn tx_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

// tx_records() -- the hidden test records ID_TX_REC + 0 .. 7
fn tx_records(p: &mut Pn, ui: &mut Ui) {
    let t = &p.ttext;
    let txt = String::from_utf8_lossy(&t.buf).into_owned();
    tx_sem(ui, ID_TX_REC, &txt, t.live as i32);
    // +1 caret, line, anchor
    let line = if t.live { tx_caret_line(t) } else { 0 };
    let s = crate::app::formatting::fmt_cap(format!("{},{},{},{}", t.caret as i32, line, t.ax, t.ay), 128);
    tx_sem(ui, ID_TX_REC + 1, &s, t.caret as i32);
    // +2 family, size
    tx_sem(ui, ID_TX_REC + 2, &t.family.clone(), t.size as i32);
    // +3 options, font id
    let s = format!("{},{},{},{}", t.style, t.align, t.render, t.metric);
    tx_sem(ui, ID_TX_REC + 3, &s, t.font as i32);
    // +4 tool layer box, scan state
    let b = p.tool.tls[TL_TEXT].bx;
    let s = format!("{},{},{},{}", b[0], b[1], b[2], b[3]);
    tx_sem(ui, ID_TX_REC + 4, &s, t.scan as i32);
    // +5 families, filtered
    let nfam = if t.scan == 2 { t.db.lock().family_count() } else { 0 };
    let s = format!("{},{}", nfam, t.filt.len());
    tx_sem(ui, ID_TX_REC + 5, &s, !t.filt.is_empty() as i32);
    tx_sem(ui, ID_TX_REC + 6, "", t.fromfile as i32);
    tx_sem(ui, ID_TX_REC + 8, &t.preedit.text, t.preedit.cursor as i32);
    tx_sem(ui, ID_TX_REC + 9, &format!("{}",t.highlights.iter().map(|r|(r[2]-r[0]).max(0)).sum::<i32>()), t.highlights.len() as i32);
    // +7 text rectangle (the handle sits at its lower right corner)
    let r = t.rect;
    let s = format!("{},{},{},{}", r[0], r[1], r[2], r[3]);
    tx_sem(ui, ID_TX_REC + 7, &s, 0);
}

// ================================================================== system fonts

// tx_scan_start() -- the font list is scanned once, on the worker pool
fn tx_scan_start(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.ttext;
    if t.scan != 0 {
        return;
    }
    let mut path = String::new();
    let mut path2 = String::new();
    if let Some(c) = plat::config_dir() {
        if c.len() < PATH_CAP - 64 {
            path = c;
            path.push_str("/PaintCraft");
            path2 = path.clone();
            path2.push_str("/fontcache.fdb");
        }
    }
    t.nosave = ui.headless();
    t.scan = 1;
    let db = t.db.clone();
    let nosave = t.nosave;
    t.job.set(1, move |_, _| tx_scan_job(&db, &path, &path2, nosave));
    if plat::thr_count() == 0 {
        // no worker: scan here
        t.job.start();
        t.scan = 2;
        tx_fonts_ready(p, ui);
        return;
    }
    t.job.start();
}

// tx_scan_job() -- worker: fontdb cache + scan (+ save).  Touches only the database (the UI
// thread does not use it before the job finished)
fn tx_scan_job(db: &SpinLock<FontDb>, path: &str, path2: &str, nosave: bool) {
    let mut db = db.lock();
    if !path2.is_empty() {
        db.load_cache(path2);
    }
    db.scan(0);
    if !path2.is_empty() && !nosave {
        plat::mkdir(path);
        db.save_cache(path2);
    }
}

// tx_scan_poll() -- the background scan finished?
fn tx_scan_poll(p: &mut Pn, ui: &mut Ui) {
    if p.ttext.scan != 1 || !p.ttext.job.done() {
        return;
    }
    p.ttext.scan = 2;
    tx_fonts_ready(p, ui);
    ui.repass();
}

// tx_fonts_ready() -- scan done: the default family (unless one was chosen), the list
fn tx_fonts_ready(p: &mut Pn, ui: &mut Ui) {
    if !p.ttext.famset {
        let fam = {
            let db = p.ttext.db.lock();
            let mut f = None;
            for name in DEF_FAMILIES.iter() {
                if let Some((i, _)) = db.find(name, 400, false) {
                    f = db.face(i).map(|x| String::from(x.family.as_str()));
                    break;
                }
            }
            if f.is_none() {
                let n = db.family_count();
                let k = (0..n).find(|&i| tx_ci_find(db.family_name(i).as_bytes(), b"sans"));
                match k {
                    Some(i) => f = Some(String::from(db.family_name(i))),
                    None if n > 0 => f = Some(String::from(db.family_name(0))),
                    None => {} // no system fonts: the embedded JFLib Sans
                }
            }
            f
        };
        if let Some(f) = fam {
            p.ttext.family = String::from(cut(&f, FAM_CAP));
        }
    }
    tx_filter(&mut p.ttext);
    tx_update(p, ui);
}

// tx_ci_find(hay, needle) -> 1 when needle occurs in hay (ASCII case-insensitive)
fn tx_ci_find(hay: &[u8], needle: &[u8]) -> bool {
    let hay = &hay[..hay.iter().position(|&c| c == 0).unwrap_or(hay.len())];
    if needle.is_empty() {
        return true;
    }
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

// tx_filter() -- filt = the families containing the search text, lsel = the row of the current
// family (-1)
fn tx_filter(t: &mut TText) {
    t.filt.clear();
    t.lsel = -1;
    if t.scan != 2 {
        return;
    }
    let db = t.db.lock();
    let n = db.family_count();
    for i in 0..n {
        if t.filt.len() >= FILT_MAX {
            break;
        }
        let name = db.family_name(i);
        if !t.search.is_empty() && !tx_ci_find(name.as_bytes(), t.search.as_bytes()) {
            continue;
        }
        if name == t.family {
            t.lsel = t.filt.len() as i32;
        }
        t.filt.push(i as u32);
    }
}

// tx_resolve_font() -- font from the options (test font, the family through fontdb, else the
// embedded JFLib Sans; synthetic bold / italic where needed; Scharf = FS_SHARP) and szfx =
// points x dpi / 72 (Fest: 96 dpi) in 24.8 px
fn tx_resolve_font(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.ttext;
    t.fromfile = false;
    let mut add = 0u32; // FS_* flags to add
    let st = t.style;
    let mut f;
    if t.testfont >= 0 {
        f = t.testfont as u32;
        if st & TXS_BOLD != 0 {
            if f == FONT_REG {
                f = FONT_BOLD;
            } else {
                add |= FS_SYN_BOLD;
            }
        }
        if st & TXS_ITALIC != 0 {
            add |= FS_SYN_ITALIC;
        }
    } else {
        let mut id = -1;
        if t.scan == 2 {
            let w = if st & TXS_BOLD != 0 { 700 } else { 400 };
            id = t.db.lock().load(&mut ui.text, &t.family, w, st & TXS_ITALIC != 0);
        }
        if id >= 0 {
            f = id as u32;
            t.fromfile = true;
        } else {
            f = if st & TXS_BOLD != 0 { FONT_BOLD } else { FONT_REG };
            if st & TXS_ITALIC != 0 {
                add |= FS_SYN_ITALIC;
            }
        }
    }
    if t.render != 0 {
        add |= FS_SHARP;
    }
    if t.render == 2 {
        add |= ::text::font::FS_CLASSIC;
    }
    if add != 0 {
        let r = ui.text.style(f, add);
        if r >= 0 {
            f = r as u32;
        }
    }
    if t.scan==2 && !t.fallback_loaded && t.buf.iter().any(|&b| b>=128) {
        t.fallback_loaded=true;
        for name in ["Noto Sans","Segoe UI","Arial","Noto Sans Arabic","Noto Sans Hebrew","Noto Sans Devanagari","Nirmala UI","Yu Gothic","Noto Sans CJK JP","Microsoft YaHei","DejaVu Sans","Segoe UI Symbol"] {
            let id=t.db.lock().load(&mut ui.text,name,400,false);
            if id>=0 && !t.fallback_fonts.contains(&(id as u32)) { t.fallback_fonts.push(id as u32); }
        }
    }
    ui.text.set_fallback_fonts(&t.fallback_fonts);
    t.font = f;
    let dpi = if t.metric != 0 {
        9600
    } else {
        let d = view_dpi100(p);
        if (d as i32) <= 0 { 9600 } else { d }
    };
    let v = (dpi as f64 * p.ttext.size * 256.0 / 7200.0) as u64;
    p.ttext.szfx = v.clamp(256, SZFX_MAX) as i32;
}

// ================================================================== layout and raster

// tx_layout() -- font, ascent, line height and the lines (start, length, x, width) of the
// text; rect = the text rectangle (image px, x1 / y1 exclusive)
fn tx_layout(p: &mut Pn, ui: &mut Ui) {
    tx_resolve_font(p, ui);
    let t = &mut p.ttext;
    let saved_shaping = ui.text.set_shaping(true);
    let saved = ui.text.get_font();
    ui.text.set_font(t.font, t.szfx);
    t.asc = ui.text.ascent();
    t.lh = ui.text.line_h().max(1);
    let mut x0 = i32::MAX;
    let mut x1 = i32::MIN;
    let len = t.buf.len();
    let mut line = 0usize;
    let mut s = 0usize; // its start
    loop {
        let mut e = s;
        while e < len && e < TX_CAP - 1 && t.buf[e] != b'\n' {
            e += 1;
        }
        let n = e - s;
        let w = if n != 0 { ui.text.measure_n(&t.buf[s..], n) } else { 0 };
        let mut x = t.ax.wrapping_shl(8);
        match t.align {
            0 => {}
            1 => x = x.wrapping_sub(w >> 1),
            _ => x = x.wrapping_sub(w),
        }
        let x = x.wrapping_add(128) >> 8;
        t.lines[line] = Line { start: s as u32, len: n as u32, x, w };
        x0 = x0.min(x);
        x1 = x1.max(((w.wrapping_add(255)) >> 8).wrapping_add(x));
        line += 1;
        if e >= len || line >= TX_LINES {
            break;
        }
        s = e + 1;
    }
    t.nlines = line as u32;
    t.rect = [x0, t.ay, x1, t.ay.wrapping_add(t.lh.wrapping_mul(line as i32))];
    ui.text.set_font(saved.0, saved.1); ui.text.set_shaping(saved_shaping);
}

// tx_surf(sf, w, h, fmt) -> true when sf has that size and format (reallocated when not; the
// contents are undefined)
fn tx_surf(sf: &mut Surface, w: i32, h: i32, fmt: u8) -> bool {
    if !sf.px.is_empty() && sf.w == w && sf.h == h && sf.fmt == fmt {
        return true;
    }
    if w as u32 > 65536 || h as u32 > 65536 {
        *sf = Surface::default();
        return false;
    }
    *sf = Surface::new(w, h, fmt);
    true
}

// the tool layer's mask (the text's coverage while live)
fn tx_mask(p: &mut Pn) -> Option<&mut TMask> {
    match &mut p.tool.tls[TL_TEXT].mask {
        TlMask::Own(m) => Some(m),
        _ => None,
    }
}

// tx_mask_clear() -- the tiles of the last rendering become empty
fn tx_mask_clear(p: &mut Pn) {
    let b = p.ttext.mbox;
    let Some(m) = tx_mask(p) else { return };
    for ty in b[1]..b[3] {
        for tx in b[0]..b[2] {
            m.tile_set(tx as u32, ty as u32, false);
        }
    }
    p.ttext.mbox = [0; 4];
}

// tx_band_colour(row0, aa, alpha) -- colour fonts: the PARGB band (scr) -> coverage (band:
// alpha; aliased >= 128 -> 255) and straight colours (src rows row0 .. row0 + 63, alpha = the
// primary colour's alpha)
fn tx_band_colour(t: &mut TText, row0: i32, aa: bool, alpha: u32) {
    let a24 = alpha << 24;
    let (ss, bs, cs) = (t.scr.stride as usize, t.band.stride as usize, t.src.stride as usize);
    for r in 0..BAND as usize {
        for x in 0..t.scr.w as usize {
            let o = r * ss + x * 4;
            let v = u32::from_le_bytes([t.scr.px[o], t.scr.px[o + 1], t.scr.px[o + 2], t.scr.px[o + 3]]);
            let a = v >> 24;
            let so = (row0 as usize + r) * cs + x * 4;
            let bo = r * bs + x;
            if a == 0 {
                t.src.px[so..so + 4].fill(0);
                t.band.px[bo] = 0;
                continue;
            }
            let mut c = 0u32; // straight rgb
            for sh in [16u32, 8, 0] {
                let ch = (v >> sh) & 255;
                let s = ((ch * 255 + (a >> 1)) / a).min(255);
                c |= s << sh;
            }
            c |= a24;
            t.src.px[so..so + 4].copy_from_slice(&c.to_le_bytes());
            t.band.px[bo] = if aa { a as u8 } else if a >= 128 { 255 } else { 0 };
        }
    }
}

// tx_raster() -- draw the laid-out text into the mask (bands of 64 rows) and set up the tool
// layer; box = the text rectangle + margin (half the size + 4), clipped to the image
fn tx_raster(p: &mut Pn, ui: &mut Ui) {
    let Some(doc) = p.ttext.doc else { return };
    tx_mask_clear(p);
    let (dw, dh) = (p.doc.d(doc).w, p.doc.d(doc).h);
    let t = &p.ttext;
    let margin = ((t.szfx as u32 >> 9) + 4) as i32;
    let x0 = t.rect[0].wrapping_sub(margin).max(0);
    let y0 = t.rect[1].wrapping_sub(margin).max(0);
    let x1 = t.rect[2].wrapping_add(margin).min(dw);
    let y1 = t.rect[3].wrapping_add(margin).min(dh);
    let mut ok = !t.buf.is_empty() && x0 < x1 && y0 < y1;
    let (tx0, ty0) = ((x0 as u32 >> 6) as i32, (y0 as u32 >> 6) as i32);
    let (tx1, ty1) = (((x1 + 63) as u32 >> 6) as i32, ((y1 + 63) as u32 >> 6) as i32);
    let bw = (tx1 - tx0) << 6; // band width
    let colour = ok && ui.text.is_color(t.font);
    if ok {
        // the colour fonts' source surface comes back from the tool layer
        if colour {
            if let TlSrc::Own(s) = core::mem::take(&mut p.tool.tls[TL_TEXT].src) {
                p.ttext.src = s;
            }
        }
        let t = &mut p.ttext;
        ok = if colour {
            tx_surf(&mut t.scr, bw, BAND, FMT_PARGB) && tx_surf(&mut t.band, bw, BAND, FMT_A8) && tx_surf(&mut t.src, bw, (ty1 - ty0) << 6, FMT_ARGB)
        } else {
            tx_surf(&mut t.scr, bw, BAND, FMT_A8)
        };
    }
    if ok {
        let aa = p.tool.cfg.aa != 0;
        let primary = tool_color(p, BTN_LEFT);
        let col = if colour { primary | 0xFF00_0000 } else { 0xFFFF_FFFF };
        let saved_shaping = ui.text.set_shaping(true);
    let saved = ui.text.get_font();
        let Pn { ttext: t, tool, .. } = p;
        ui.text.set_font(t.font, t.szfx);
        let mut ty = ty0;
        while ty < ty1 {
            let by = ty << 6;
            let ss = t.scr.stride as usize;
            t.scr.px[..ss * BAND as usize].fill(0);
            let mut dc = Dc::new(t.scr.w, t.scr.h);
            dc.set_opacity(255);
            dc.origin(-(tx0 << 6), -by);
            dc.set_color(col);
            let mut ln = 0;
            while ln < t.nlines as usize && ln < TX_LINES {
                let top = (ln as i32).wrapping_mul(t.lh).wrapping_add(t.ay); // line top
                let l = t.lines[ln];
                ln += 1;
                if top.wrapping_add(t.lh).wrapping_add(margin) <= by {
                    continue; // above the band
                }
                if top.wrapping_sub(margin) >= by + BAND {
                    continue; // below it
                }
                let base = top.wrapping_add(t.asc); // baseline
                if l.len == 0 {
                    continue;
                }
                ui.text.draw_n(&mut dc, &mut t.scr, l.x, base, tail(&t.buf, l.start), l.len as usize);
                let fl = (t.style >> 2) & (TD_UNDERLINE | TD_STRIKEOUT);
                if fl != 0 {
                    let w = l.w.wrapping_add(255) >> 8;
                    ui.text.decor(&dc, &mut t.scr, l.x, base, w, fl);
                }
            }
            let (data, stride) = if !colour {
                if !aa {
                    for v in t.scr.px[..ss * BAND as usize].iter_mut() {
                        *v = if *v >= 128 { 255 } else { 0 };
                    }
                }
                (&t.scr.px, ss)
            } else {
                tx_band_colour(t, (ty - ty0) << 6, aa, primary >> 24);
                (&t.band.px, t.band.stride as usize)
            };
            if let TlMask::Own(m) = &mut tool.tls[TL_TEXT].mask {
                for tx in tx0..tx1 {
                    m.combine_block(tx as u32, ty as u32, Block::Data(&data[((tx - tx0) << 6) as usize..], stride), TM_REPLACE);
                }
            }
            ty += 1;
        }
        ui.text.set_font(saved.0, saved.1); ui.text.set_shaping(saved_shaping);
        t.mbox = [tx0, ty0, tx1, ty1];
        // the tool layer: colour / pattern / blend mode; colour fonts: their pixels
        tl_set_paint(p, TL_TEXT, BTN_LEFT);
        if colour {
            let tl = &mut p.tool.tls[TL_TEXT];
            tl.kind = TLK_SOURCE as u32;
            tl.src = TlSrc::Own(core::mem::take(&mut p.ttext.src));
            tl.sx = tx0 << 6;
            tl.sy = ty0 << 6;
        }
        tl_set_box(p, TL_TEXT, x0, y0, x1, y1);
    } else {
        tl_set_box(p, TL_TEXT, 0, 0, 0, 0);
    }
    tl_show(p, TL_TEXT);
}

// tx_update() -- layout, raster, handle and caret of the live text; versions noted
fn tx_update(p: &mut Pn, ui: &mut Ui) {
    if !p.ttext.live {
        return;
    }
    tx_layout(p, ui);
    tx_raster(p, ui);
    p.ttext.pver = p.tool.cfg.ver;
    p.ttext.palv = pal_version(&p.pal);
    let r = p.ttext.rect;
    nub_set(&p.view, ui, &mut p.ttext.nub[0], r[2].wrapping_shl(8), r[3].wrapping_shl(8));
    tx_caret_update(p, ui);
}

// ================================================================== caret

// the text from byte `start` on (empty past the end: a layout line may be older than the text,
// the asm reads its 16 KB buffer there)
fn tail(buf: &[u8], start: u32) -> &[u8] {
    buf.get(start as usize..).unwrap_or(&[])
}

// tx_caret_line() -> the line of the caret (layout of the last tx_layout)
fn tx_caret_line(t: &TText) -> i32 {
    let n = (t.nlines as usize).min(TX_LINES);
    let mut line = 0;
    for i in 1..n {
        if t.caret < t.lines[i].start {
            break;
        }
        line = i;
    }
    line as i32
}

// tx_caret_pos() -> (x, top (image px), line) of the caret
fn tx_caret_pos(t: &TText, ui: &mut Ui) -> (i32, i32, i32) {
    let saved_shaping = ui.text.set_shaping(true);
    let saved = ui.text.get_font();
    ui.text.set_font(t.font, t.szfx);
    let line = tx_caret_line(t);
    let l = t.lines[line as usize];
    let x = ui.text.caret_x(tail(&t.buf, l.start), t.caret.wrapping_sub(l.start) as usize).wrapping_add(l.x);
    ui.text.set_font(saved.0, saved.1); ui.text.set_shaping(saved_shaping);
    (x, line.wrapping_mul(t.lh).wrapping_add(t.ay), line)
}

// tx_caret_moved() -- the caret shows at once (blink restarts)
fn tx_caret_moved(p: &mut Pn, ui: &mut Ui) {
    p.ttext.blink0 = ui.now();
    tx_caret_update(p, ui);
}

// tx_caret_update() -- caret rectangle (screen px) and handle visibility: the caret blinks
// (530 ms; always on in headless runs), Strg held hides caret and handle; old and new places
// are damaged
fn tx_caret_update(p: &mut Pn, ui: &mut Ui) {
    let mut r = [0i32; 4];
    let mut hidden = false;
    if p.ttext.live {
        if doc_active(&p.doc) != p.ttext.doc || ui.key_down(VK_CONTROL) {
            hidden = true;
        } else {
            let mut show = true;
            if !ui.headless() {
                let d = ui.now().wrapping_sub(p.ttext.blink0);
                let phase = d / BLINK_MS;
                ui.timer(TMR_TTEXT, BLINK_MS - d % BLINK_MS);
                show = phase & 1 == 0;
            }
            if show {
                let (x, top, _) = tx_caret_pos(&p.ttext, ui);
                let (sx, sy) = view_doc_to_screen_fx(&p.view, x.wrapping_shl(8), top.wrapping_shl(8));
                let (sx, sy) = (sx >> 8, sy >> 8);
                let (_, sy2) = view_doc_to_screen_fx(&p.view, x.wrapping_shl(8), top.wrapping_add(p.ttext.lh).wrapping_shl(8));
                r = [sx, sy, sx + 2, (sy2 >> 8).max(sy + 1)];
            }
        }
    } else {
        hidden = true;
    }
    let n = &mut p.ttext.nub[0];
    if (n.flags & NBF_HIDDEN as u32 != 0) != hidden {
        n.flags ^= NBF_HIDDEN as u32;
        nubs_damage(&p.view, ui, &mut p.ttext.nub);
    }
    if r[3]>r[1] { ui.ime_pos(r[0],r[1],r[3]-r[1]); }
    tx_selection_update(p,ui);
    let c = p.ttext.crect;
    if r == c {
        return;
    }
    if c[0] < c[2] {
        tool_damage(&p.view, ui, c[0], c[1], c[2], c[3]);
    }
    p.ttext.crect = r;
    if r[0] < r[2] {
        tool_damage(&p.view, ui, r[0], r[1], r[2], r[3]);
    }
}

// ================================================================== text editing

fn tx_delete_selection(t: &mut TText) -> bool {
    if t.anchor==t.caret {return false;}
    tx_erase(t,t.anchor.min(t.caret),t.anchor.max(t.caret));true
}
fn tx_selection_update(p:&mut Pn,ui:&mut Ui) {
    let t=&p.ttext;
    let mut rects=Vec::new();
    if t.live && doc_active(&p.doc)==t.doc && t.anchor!=t.caret {
        let (a,b)=(t.anchor.min(t.caret),t.anchor.max(t.caret));
        let saved_shaping = ui.text.set_shaping(true);let saved=ui.text.get_font();ui.text.set_font(t.font,t.szfx);
        for (i,line) in t.lines.iter().take(t.nlines as usize).enumerate() {
            let start=a.max(line.start);let end=b.min(line.start+line.len);
            if start>end || start==end && !(b>line.start+line.len && a<=line.start+line.len) {continue;}
            let text=tail(&t.buf,line.start);
            let mut spans=ui.text.selection_spans(text,(start-line.start) as usize,(end-line.start) as usize);
            if b>line.start+line.len {
                let x=ui.text.caret_x(text,line.len as usize); spans.push((x,x+3));
            }
            let top=t.ay+i as i32*t.lh;
            for (x0,x1) in spans {
                let (sx,sy)=view_doc_to_screen_fx(&p.view,(line.x+x0)<<8,top<<8);
                let (ex,ey)=view_doc_to_screen_fx(&p.view,(line.x+x1)<<8,(top+t.lh)<<8);
                rects.push([sx>>8,sy>>8,(ex>>8).max((sx>>8)+1),ey>>8]);
            }
        }
        ui.text.set_font(saved.0,saved.1); ui.text.set_shaping(saved_shaping);
    }
    if rects!=p.ttext.highlights {
        for r in p.ttext.highlights.iter().chain(rects.iter()) {tool_damage(&p.view,ui,r[0],r[1],r[2],r[3]);}
        p.ttext.highlights=rects;
    }
}

// tx_insert(src) -> 1 inserted -- the bytes at the caret (refused when the buffer or the line
// limit would overflow)
fn tx_insert(t: &mut TText, src: &[u8]) -> bool {
    let n = src.len();
    if n == 0 {
        return false;
    }
    let len = t.buf.len();
    if len + n - t.caret.abs_diff(t.anchor) as usize >= TX_CAP - 1 {
        return false;
    }
    let range=t.caret.min(t.anchor) as usize..t.caret.max(t.anchor) as usize;
    let nl = src.iter().filter(|&&c| c == b'\n').count() + t.buf.iter().filter(|&&c| c == b'\n').count() - t.buf[range].iter().filter(|&&c|c==b'\n').count();
    if nl > TX_LINES - 1 {
        return false;
    }
    let at = t.caret.min(t.anchor) as usize;
    let end = t.caret.max(t.anchor) as usize;
    t.buf.splice(at..end, src.iter().copied());
    t.caret = (at + n) as u32;
    t.anchor = t.caret;
    true
}

// tx_erase(from, to) -- remove the bytes [from, to); caret = from
fn tx_erase(t: &mut TText, from: u32, to: u32) {
    let to = to.min(t.buf.len() as u32);
    if from >= to {
        return;
    }
    t.buf.drain(from as usize..to as usize);
    t.caret = from;
    t.anchor = from;
}

// tx_prev(i) -> start of the character before byte i (0 stays 0)
fn tx_word(t: &TText, mut i: u32, forward: bool) -> u32 {
    let class = |at:u32| {
        let cp=base::utf8_decode(&t.buf,at as usize).0;
        match char::from_u32(cp) {Some(c) if c.is_whitespace()=>0,Some(c) if c.is_alphanumeric() || c=='_'=>1,_=>2}
    };
    let end=t.buf.len() as u32;
    if forward {
        if i<end {let kind=class(i);while i<end && class(i)==kind {i=tx_next(t,i);}}
        while i<end && class(i)==0 {i=tx_next(t,i);}
    } else {
        while i>0 && class(tx_prev(t,i))==0 {i=tx_prev(t,i);}
        if i>0 {let kind=class(tx_prev(t,i));while i>0 && class(tx_prev(t,i))==kind {i=tx_prev(t,i);}}
    }
    i
}

fn tx_prev(t: &TText, i: u32) -> u32 {
    let mut k = i as usize;
    loop {
        if k == 0 {
            return 0;
        }
        k -= 1;
        if k == 0 {
            return 0;
        }
        let c = t.buf.get(k).copied().unwrap_or(0);
        if c & 0xC0 != 0x80 {
            return k as u32;
        }
    }
}

// tx_next(i) -> start of the character after byte i (at most the length)
fn tx_next(t: &TText, i: u32) -> u32 {
    let len = t.buf.len() as u32;
    if i >= len {
        return len;
    }
    (utf8_next(&t.buf, i as usize) as u32).min(len)
}

// utf8_next(s, i): the position after the character at i (utf8_decode: invalid sequences are
// one byte; bytes past the end read as NUL)
fn utf8_next(s: &[u8], i: usize) -> usize {
    let at = |k: usize| s.get(k).copied().unwrap_or(0) as u32;
    let b0 = at(i);
    if b0 == 0 {
        return i;
    }
    if b0 < 0x80 {
        return i + 1;
    }
    let (n, mut cp, min) = match b0 {
        0xC2..=0xDF => (1, b0 & 0x1F, 0x80),
        0xE0..=0xEF => (2, b0 & 0x0F, 0x800),
        0xF0..=0xF4 => (3, b0 & 7, 0x10000),
        _ => return i + 1,
    };
    let mut j = i + 1;
    for _ in 0..n {
        let c = at(j);
        if c & 0xC0 != 0x80 {
            return i + 1;
        }
        cp = cp << 6 | (c & 0x3F);
        j += 1;
    }
    if cp < min || cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 {
        return i + 1;
    }
    j
}

// tx_vmove(dir) -> caret on the line above (-1) / below (1) at the same x (unchanged at the
// first / last line)
fn tx_vmove(t: &TText, ui: &mut Ui, dir: i32) -> u32 {
    let (x, _, line) = tx_caret_pos(t, ui);
    let tl = line + dir;
    if tl < 0 || tl as u32 >= t.nlines || tl as usize >= TX_LINES {
        return t.caret;
    }
    let saved_shaping = ui.text.set_shaping(true);
    let saved = ui.text.get_font();
    ui.text.set_font(t.font, t.szfx);
    let l = t.lines[tl as usize];
    let r = (ui.text.hit(tail(&t.buf, l.start), Some(l.len as usize), x.wrapping_sub(l.x)) as u32).wrapping_add(l.start);
    ui.text.set_font(saved.0, saved.1); ui.text.set_shaping(saved_shaping);
    r
}

// tx_hit(x_fx, y_fx) -> byte index of the caret position nearest to the image point
fn tx_hit(t: &TText, ui: &mut Ui, x: i32, y: i32) -> u32 {
    let saved_shaping = ui.text.set_shaping(true);
    let saved = ui.text.get_font();
    ui.text.set_font(t.font, t.szfx);
    let mut line = ((y >> 8).wrapping_sub(t.ay)).wrapping_div(t.lh);
    line = line.max(0);
    line = line.min((t.nlines as i32).wrapping_sub(1));
    line = line.min(TX_LINES as i32 - 1);
    line = line.max(0);
    let l = t.lines[line as usize];
    let r = (ui.text.hit(tail(&t.buf, l.start), Some(l.len as usize), (x >> 8).wrapping_sub(l.x)) as u32).wrapping_add(l.start);
    ui.text.set_font(saved.0, saved.1); ui.text.set_shaping(saved_shaping);
    r
}

// tx_paste() -- the text clipboard at the caret (CR LF / CR -> LF, tabs -> spaces, other
// control characters dropped; cut to the free space on a character boundary)
fn tx_paste(t: &mut TText, ui: &mut Ui) {
    let Some(d) = ui.clip_get(CLIP_TEXT) else { return };
    let mut out: Vec<u8> = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() && d[i] != 0 {
        let c = d[i];
        i += 1;
        match c {
            13 => {
                if d.get(i) == Some(&10) {
                    i += 1;
                }
                out.push(10);
            }
            9 => out.push(32),
            10 => out.push(10),
            c if c < 32 => {}
            c => out.push(c),
        }
    }
    let room = TX_CAP as i32 - 2 - t.buf.len() as i32 + t.caret.abs_diff(t.anchor) as i32;
    if room <= 0 {
        return;
    }
    let mut n = out.len();
    if n > room as usize {
        // utf8_clip
        n = room as usize;
        while n > 0 && out[n] & 0xC0 == 0x80 {
            n -= 1;
        }
    }
    tx_insert(t, &out[..n]);
}

// tx_is_charkey(vk) -> 1 for keys that type a character (their text arrives as typed text)
fn tx_is_charkey(vk: u32) -> bool {
    vk == VK_SPACE
        || vk.wrapping_sub(0x30) <= 9
        || vk.wrapping_sub(0x41) <= 25
        || vk.wrapping_sub(0x60) <= 15
        || vk.wrapping_sub(0xBA) <= 6
        || vk.wrapping_sub(0xDB) <= 4
        || vk == 0xE2
}

// tx_focus_ok() -> 1 when keys / typed text belong to the text: the canvas, the tool row's
// buttons or nothing has the keyboard focus (not the size box or a search box)
fn tx_focus_ok(ui: &Ui) -> bool {
    let f = ui.focused();
    if f == 0 || f == ID_CANVAS || f == ID_FONT_LIST || f == CMD_TOOL_COMMIT {
        return true;
    }
    if f == ID_TX_SIZE {
        return false;
    }
    f.wrapping_sub(ID_TOOLOPT_FIRST) <= ID_TOOLOPT_LAST - ID_TOOLOPT_FIRST || f.wrapping_sub(ID_TX0) <= TOOLOPT_STRIDE as u32 - 1
}

// tx_key1(vk, mods) -> 0 not handled / 1 consumed / 2 the text or the caret changed
fn tx_key1(p: &mut Pn, ui: &mut Ui, vk: u32, mods: u32) -> u32 {
    let m = mods & (MOD_CTRL | MOD_ALT | MOD_SHIFT);
    let ca = m & (MOD_CTRL | MOD_ALT);
    if ca == MOD_ALT {
        return 0; // Alt chords belong to the application
    }
    let t = &mut p.ttext;
    if ca != MOD_CTRL | MOD_ALT {
        // (AltGr: characters)
        let set = match vk {
            VK_RETURN => {
                if m & MOD_CTRL != 0 {
                    return 0;
                }
                tx_insert(t, b"\n");
                return 2;
            }
            VK_BACK => {
                if tx_delete_selection(t) {return 2;}
                let c = t.caret;
                if c == 0 {
                    return 1;
                }
                let a = tx_prev(t, c);
                tx_erase(t, a, c);
                return 2;
            }
            VK_DELETE => {
                if tx_delete_selection(t) {return 2;}
                let c = t.caret;
                if c >= t.buf.len() as u32 {
                    return 1;
                }
                let b = tx_next(t, c);
                tx_erase(t, c, b);
                return 2;
            }
            VK_LEFT => if m & MOD_SHIFT==0 && t.anchor!=t.caret {t.anchor.min(t.caret)} else if m & MOD_CTRL != 0 {tx_word(t,t.caret,false)} else {tx_prev(t, t.caret)},
            VK_RIGHT => if m & MOD_SHIFT==0 && t.anchor!=t.caret {t.anchor.max(t.caret)} else if m & MOD_CTRL != 0 {tx_word(t,t.caret,true)} else {tx_next(t, t.caret)},
            VK_HOME => {
                if m & MOD_CTRL != 0 {
                    0
                } else {
                    t.lines[tx_caret_line(t) as usize].start
                }
            }
            VK_END => {
                if m & MOD_CTRL != 0 {
                    t.buf.len() as u32
                } else {
                    let l = t.lines[tx_caret_line(t) as usize];
                    l.start + l.len
                }
            }
            VK_UP => tx_vmove(t, ui, -1),
            VK_DOWN => tx_vmove(t, ui, 1),
            VK_INSERT => {
                if m != MOD_SHIFT {
                    return 0;
                }
                tx_paste(t, ui);
                return 2;
            }
            _ => {
                if m & MOD_CTRL != 0 {
                    if vk==b'A' as u32 {t.anchor=0;t.caret=t.buf.len() as u32;return 2;}
                    if vk==b'C' as u32 || vk==b'X' as u32 {
                        if t.anchor!=t.caret && ui.clip_set(CLIP_TEXT,&t.buf[t.anchor.min(t.caret) as usize..t.anchor.max(t.caret) as usize])!=0 {return 1;}
                        if vk==b'X' as u32 && tx_delete_selection(t) {return 2;}
                        return 1;
                    }
                    if vk == b'V' as u32 {
                        tx_paste(t, ui);
                        return 2;
                    }
                    return 0;
                }
                return if tx_is_charkey(vk) { 1 } else { 0 };
            }
        };
        t.caret = set;
        if m & MOD_SHIFT == 0 {t.anchor=set;}
        return 2;
    }
    if tx_is_charkey(vk) { 1 } else { 0 }
}

// tx_take_text() -> 1 when typed text was inserted (only while tx_focus_ok: a focused text box
// takes its own text)
fn tx_take_text(p: &mut Pn, ui: &mut Ui) -> bool {
    if !tx_focus_ok(ui) {
        return false;
    }
    let mut any = false;
    let mut u8b = Vec::with_capacity(4);
    for cp in ui.text_take(NCP) {
        if cp < 0x20 || (0x7F..0xA0).contains(&cp) || (0xD800..0xE000).contains(&cp) || cp > 0x10FFFF {
            continue; // controls, DEL and C1, surrogates
        }
        u8b.clear();
        let mut b = [0u8; 4];
        u8b.extend_from_slice(char::from_u32(cp).map_or(&b"\xEF\xBF\xBD"[..], |c| c.encode_utf8(&mut b).as_bytes()));
        any |= tx_insert(&mut p.ttext, &u8b);
    }
    if any {
        p.ttext.blink0 = ui.now();
    }
    any
}

// ================================================================== live edit

// tx_start(doc, x, y) -- a new live text with its anchor at (x, y) (image px)
fn tx_start(p: &mut Pn, ui: &mut Ui, doc: DocId, x: i32, y: i32) {
    let (w, h, ver, cur) = {
        let d = p.doc.d(doc);
        (d.w, d.h, d.ver_sel, d.cur)
    };
    let Some(m) = TMask::new(w as u32, h as u32) else { return };
    let t = &mut p.ttext;
    t.mbox = [0; 4];
    t.doc = Some(doc);
    t.ax = x;
    t.ay = y;
    t.buf.clear();
    t.caret = 0;
    t.anchor = 0;
    t.selecting = false;
    t.drag = false;
    tl_init(p, TL_TEXT, doc);
    p.tool.tls[TL_TEXT].mask = TlMask::Own(m);
    let t = &mut p.ttext;
    t.vsel = ver;
    t.vcur = cur;
    let n = &mut t.nub[0];
    n.kind = NK_SQUARE as u32;
    n.flags &= NBF_DRAWN as u32;
    n.flags |= NBF_HIDDEN as u32; // tx_caret_update shows it (damaged)
    t.live = true;
    t.blink0 = ui.now();
    tool_register(&mut p.tool, TOOL_TEXT, Some(&DESC_TEXT_LIVE));
    tool_live_begin(p, doc);
    tx_update(p, ui);
}

// tx_end() -- the live text is over: handle and caret away, masks and surfaces freed
fn tx_end(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.ttext;
    t.live = false;
    t.preedit = Default::default(); ui.ime_pos(0,0,0); plat::ime_preedit_set(0,"",0);
    t.drag = false;
    t.nub[0].flags |= NBF_HIDDEN as u32;
    t.nub[0].flags &= !((NBF_HOVER | NBF_DRAG) as u32);
    nubs_damage(&p.view, ui, &mut t.nub);
    tx_caret_update(p, ui);
    let tl = &mut p.tool.tls[TL_TEXT];
    tl.mask = TlMask::None;
    tl.src = TlSrc::None;
    let t = &mut p.ttext;
    t.mbox = [0; 4];
    t.src = Surface::default();
    t.scr = Surface::default();
    t.band = Surface::default();
    t.buf.clear();
    t.caret = 0;
    t.anchor = 0;
    t.selecting = false;
    tool_register(&mut p.tool, TOOL_TEXT, Some(&DESC_TEXT));
}

// ================================================================== TOOLDESC callbacks

// tx_activate(doc) -- the font list is scanned (once) in the background
fn tx_activate(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    tx_scan_start(p, ui);
}

// tx_input(doc, ui, x_fx, y_fx) -> 1 changed -- click: caret into the text / handle drag /
// (commit and) a new text; drag of the handle moves the text
fn tx_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    if p.ttext.selecting {
        if inp.btn & (1 << BTN_LEFT)==0 {p.ttext.selecting=false;return true;}
        p.ttext.caret=tx_hit(&p.ttext,ui,x,y);
        tx_caret_moved(p,ui);return true;
    }
    if p.ttext.drag {
        if inp.btn & (1 << BTN_LEFT) == 0 {
            p.ttext.drag = false;
            p.ttext.nub[0].flags &= !(NBF_DRAG as u32);
            nubs_damage(&p.view, ui, &mut p.ttext.nub);
            return true;
        }
        if Some(doc) != p.ttext.doc {
            return false;
        }
        let ax = (x >> 8).wrapping_sub(p.ttext.ddx);
        let ay = (y >> 8).wrapping_sub(p.ttext.ddy);
        if ax == p.ttext.ax && ay == p.ttext.ay {
            return false;
        }
        p.ttext.ax = ax;
        p.ttext.ay = ay;
        tx_update(p, ui);
        return true;
    }
    if inp.pressed & (1 << BTN_LEFT) == 0 {
        // hover
        if !p.ttext.live || Some(doc) != p.ttext.doc {
            return false;
        }
        return nubs_hover(&p.view, ui, &mut p.ttext.nub, sx, sy);
    }
    if p.ttext.live && Some(doc) == p.ttext.doc {
        if p.ttext.nub[0].flags & NBF_HIDDEN as u32 == 0 && nubs_hit(&p.view, ui, &p.ttext.nub, sx, sy) >= 0 {
            let t = &mut p.ttext;
            t.drag = true;
            t.ddx = (x >> 8).wrapping_sub(t.ax);
            t.ddy = (y >> 8).wrapping_sub(t.ay);
            t.nub[0].flags |= NBF_DRAG as u32;
            nubs_damage(&p.view, ui, &mut t.nub);
            return true;
        }
        // inside the text: the caret
        let (px, py) = (x >> 8, y >> 8);
        let r = p.ttext.rect;
        if px >= r[0].wrapping_sub(2) && px <= r[2].wrapping_add(2) && py >= r[1] && py < r[3] {
            p.ttext.caret = tx_hit(&p.ttext, ui, x, y);
            if ui.mods() & MOD_SHIFT == 0 {p.ttext.anchor=p.ttext.caret;}
            p.ttext.selecting=true;
            tx_caret_moved(p, ui);
            return true;
        }
        tool_commit_live(p, ui);
    }
    tx_start(p, ui, doc, x >> 8, y >> 8);
    true
}

// tx_key(vk, mods) -> 0 -- while a text is live and has the keyboard: editing keys, Enter,
// Strg+V; plain character keys are consumed (their text arrives as typed text).  The keys of
// the frame are taken here one after another in queue order (consumed here, so false tells
// tool.rs not to consume another one)
fn tx_key(p: &mut Pn, ui: &mut Ui, vk: u32, mods: u32) -> bool {
    if !p.ttext.live || !tx_focus_ok(ui) {
        return false;
    }
    let (mut vk, mut mods) = (vk, mods);
    let mut res = 0u32; // 1 consumed, 2 changed
    loop {
        let r = tx_key1(p, ui, vk, mods);
        if r == 0 {
            break;
        }
        res = res.max(r);
        if r == 2 {
            tx_layout(p, ui);
        }
        ui.key(vk);
        let k = ui.key_first();
        if k == 0 {
            break;
        }
        vk = k & 0xFFFF;
        mods = k >> 16;
        if !ui.key_peek(vk) {
            break;
        }
    }
    if res == 2 {
        p.ttext.blink0 = ui.now();
        tx_update(p, ui);
    }
    if res != 0 {
        ui.repass();
    }
    false
}

// tx_commit(doc) -> 1 step -- the text into the layer ("Text"); empty text: no step
fn tx_commit(p: &mut Pn, ui: &mut Ui, _doc: DocId) -> bool {
    if !p.ttext.live {
        return false;
    }
    tx_update(p, ui);
    let b = p.tool.tls[TL_TEXT].bx;
    let r = if p.ttext.buf.is_empty() || b[0] >= b[2] {
        tl_hide(p, TL_TEXT);
        false
    } else {
        if let Some(m) = tx_mask(p) {
            m.optimize();
        }
        tl_commit(p, TL_TEXT, "Text", IC_FONT)
    };
    tx_end(p, ui);
    r
}

// tx_cancel(doc) -- drop the live text (no history; doc None = the image is gone)
fn tx_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.ttext.live {
        return;
    }
    if doc.is_some() {
        tl_hide(p, TL_TEXT);
    }
    p.tool.tls[TL_TEXT].shown = false;
    tx_end(p, ui);
}

// tx_cursor(x_fx, y_fx) -> CUR_MOVE over / while dragging the handle, else the I-beam
fn tx_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    if p.ttext.live && p.ttext.nub[0].flags & (NBF_HOVER | NBF_DRAG) as u32 != 0 {
        return CUR_MOVE;
    }
    CUR_IBEAM
}

// tx_overlay(doc, &rect) -- caret (a dark and a light line) and the handle
fn tx_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    let t = &mut p.ttext;
    if !t.live || t.doc != Some(doc) {
        return;
    }
    let saved=g.dc.get_color();
    g.dc.set_color(0x603378CC);
    for r in &t.highlights {g.dc.fill(g.sf,r[0],r[1],r[2]-r[0],r[3]-r[1]);}
    g.dc.set_color(saved);
    let c = t.crect;
    let h = c[3] - c[1];
    if h > 0 && c[0] < c[2] {
        let saved = g.dc.get_color();
        g.dc.set_color(0xFF00_0000);
        g.dc.fill(g.sf, c[0], c[1], 1, h);
        g.dc.set_color(0xFFFF_FFFF);
        g.dc.fill(g.sf, c[0] + 1, c[1], 1, h);
        g.dc.set_color(saved);
    }
    if !t.preedit.text.is_empty() {
        let saved_font=g.text.get_font(); let shaping=g.text.set_shaping(true); let saved_color=g.dc.get_color();
        g.text.set_font(t.font,16<<8);
        let x=c[0]+3; let y=c[1]; let w=g.text.width(&t.preedit.text).max(12); let h=g.text.line_h().max(18);
        g.dc.set_color(0xFFFFFFFF); g.dc.fill(g.sf,x,y,w+4,h+3);
        g.dc.set_color(0xFF202020); let baseline=y+g.text.ascent();
        g.text.draw(g.dc,g.sf,x+2,baseline,t.preedit.text.as_bytes());
        g.dc.fill(g.sf,x,y+h,w+4,1);
        let caret=g.text.caret_x(t.preedit.text.as_bytes(),t.preedit.cursor);
        g.dc.fill(g.sf,x+2+caret,y,1,h);
        g.dc.set_color(saved_color); g.text.set_font(saved_font.0,saved_font.1); g.text.set_shaping(shaping);
    }
    nubs_draw(&p.view, g, &mut t.nub);
}

// tx_tick(doc) -- font scan, typed text, option / colour / selection changes, caret blink
fn tx_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    tx_scan_poll(p, ui);
    if !p.ttext.live || doc.is_none() || doc != p.ttext.doc {
        return;
    }
    let doc = doc.unwrap();
    let preedit=plat::ime_preedit(0);
    if preedit!=p.ttext.preedit { p.ttext.preedit=preedit; crate::workspace::view::view_invalidate(p); ui.invalidate(); }
    // the pointer left the canvas: the handle is no longer hovered
    if p.ttext.nub[0].flags & NBF_HOVER as u32 != 0 && !view_cursor(&p.view).2 {
        p.ttext.nub[0].flags &= !(NBF_HOVER as u32);
        nubs_damage(&p.view, ui, &mut p.ttext.nub);
    }
    if tx_take_text(p, ui) || p.tool.cfg.ver != p.ttext.pver || pal_version(&p.pal) != p.ttext.palv {
        tx_update(p, ui);
        return;
    }
    let (ver, cur) = (p.doc.d(doc).ver_sel, p.doc.d(doc).cur);
    if ver != p.ttext.vsel || cur != p.ttext.vcur {
        p.ttext.vsel = ver;
        p.ttext.vcur = cur;
        let b = p.tool.tls[TL_TEXT].bx;
        tl_set_box(p, TL_TEXT, b[0], b[1], b[2], b[3]);
    }
    tx_caret_update(p, ui);
}
