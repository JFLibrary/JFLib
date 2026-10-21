// sec_list -- the gallery's page scroll view, containers, values, colour wheel and lists
//   [apps/gallery/src/main.s: the page scroll view of gal_frame, gal_containers, gal_values,
//    gal_tracks, gal_hsv_changed, gal_rgb_changed, gal_track_build, gal_wheel, gal_lists,
//    ants_draw, the CMD_ANTS / CMD_DEC_SEP parts of gal_u1_cmd]; the list flyout of test hook
//   9061 (Rust only: apps/gallery/tests/listfly.uis)

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::Cell;

use ui::*;

use crate::{cap, Gal};

// containers (container agent); tabs / list use id + 1 + i for their items
const ID_PAGE: u32 = 7000; // page scroll view
const ID_TABS: u32 = 7010; // tabs 7011..7013
const ID_LIST: u32 = 7100; // rows 7101..7300
const ID_EXP: u32 = 7400;
const ID_EXP_CB: u32 = 7401;
const ID_IB0: u32 = 7410; // info bars 7410, 7412, 7414, 7416 (close buttons + 1)
const ID_DLG_BTN: u32 = 7420;
const ID_HSCROLL: u32 = 7440;
const ID_SBAR: u32 = 7441;
const ID_CHIP0: u32 = 7450; // buttons in the horizontal scroll view 7450..7469
// values and colours (U1)
const ID_VAL_SF: u32 = 7700; // slider_f
const ID_VAL_NBF: u32 = 7701; // number_box_f
const ID_VAL_PROP: u32 = 7702; // property row (i32)
const ID_VAL_PROPF: u32 = 7703; // property row (f64)
const ID_VAL_ANG: u32 = 7704;
const ID_VAL_PAN: u32 = 7705;
const ID_VAL_H: u32 = 7710; // colour tracks H S V R G B A
const ID_VAL_VT: u32 = 7717; // vertical track
const ID_VAL_SW: u32 = 7718; // swatch of the track colour
const ID_VAL_WINBTN: u32 = 7740;
// colour wheel (U1)
const ID_WHEEL: u32 = 7720;
const ID_WHEEL_V: u32 = 7721;
const ID_PICKER: u32 = 7730;
// lists (U1)
const ID_RLIST: u32 = 7800; // rows 7801 .. 7800 + RL_N
const ID_RL_CB: u32 = 7850; // visibility check boxes 7850 + item, swatches 7870 + item
const ID_CANVAS: u32 = 7890; // marching-ants canvas (canvas_damage)
const RL_N: usize = 12;
const CMD_ANTS: u32 = 9053; // test hook: advance the marching ants (damage only)
const CMD_DEC_SEP: u32 = 9060; // test hook: decimal separator ',' <-> '.'
const CMD_LIST_FLY: u32 = 9061; // test hook: the list flyout (arg 1: no row can be chosen)
const ID_LIST_FLY: u32 = 7900; // flyout at the pointer: a list without a selection
const ID_LIST_FLY_L: u32 = 7910; // its list, rows 7911 .. 7922

const TAB_NAMES: [&str; 3] = ["Liste", "Erweitert", "Dialog"];
const IB_TITLES: [&str; 4] = ["Hinweis", "Gespeichert", "Achtung", "Fehler"];
const IB_MSGS: [&str; 4] = [
    "Eine neue Version ist verfügbar.",
    "Das Bild wurde gespeichert.",
    "Der Akku ist fast leer.",
    "Die Datei konnte nicht geöffnet werden.",
];

const RF_VAL: RangeF = RangeF::new(0.0, 100.0, 0.5, 1, 0);
const RF_PROP: RangeF = RangeF::new(0.0, 10.0, 0.0, 2, 0);
const C_PROP_DEF: f64 = 1.5;
const RANGE_PROP: Range = Range::new(0, 100, 1, 0);
const RANGE_HUE: Range = Range::new(0, 359, 1, 0);
const RANGE_PCT_V: Range = Range::new(0, 100, 1, RG_VERT);
const RANGE_PCT2: Range = Range::new(0, 100, 1, 0);
const RANGE_BYTE: Range = Range::new(0, 255, 1, 0);
const RANGE_WV: Range = Range::new(0, 1000, 10, RG_VERT);
const TK_H: [u32; 7] = [0xFFFF0000, 0xFFFFFF00, 0xFF00FF00, 0xFF00FFFF, 0xFF0000FF, 0xFFFF00FF, 0xFFFF0000];
const RL_COLORS: [u32; RL_N] = [
    0xFFE81123, 0xFFFF8C00, 0xFFFFB900, 0xFF107C10, 0xFF00B7C3, 0xFF0078D4, 0xFF8764B8, 0xFFE3008C, 0xFF7A7574,
    0xFF498205, 0xFF038387, 0xFF881798,
];

/// ants_draw: a dashed rectangle whose dashes shift with the phase (read at render time, as
/// the asm reads g_ants_phase)
struct Ants {
    phase: Cell<i32>,
}

impl Draw for Ants {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        // background
        g.dc.set_color(0xFFFF_FFFF);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        // dashes along the top and bottom edges (8 px long, phase-shifted)
        g.dc.set_color(0xFF00_0000);
        let ph = self.phase.get();
        let mut x = 0;
        while x < r[2] {
            if (x.wrapping_add(ph) & 15) < 8 {
                g.dc.fill(g.sf, r[0] + x, r[1] + 8, 1, 1);
                g.dc.fill(g.sf, r[0] + x, r[1] + r[3] - 9, 1, 1);
            }
            x += 1;
        }
    }
}

/// the section's state (the .data / .bss of the containers, values, wheel and lists parts)
pub(crate) struct S {
    // containers
    tab: i32,
    list_dbl: i32,
    ib_closed: u32,
    sbar_pos: i32,
    exp_open: i32,
    exp_cb: i32,
    // values and colours
    val_f: f64,
    prop_f: f64,
    pan_x: f64,
    pan_y: f64,
    prop_i: i32,
    /// g_trk_h, s, v, r, g, b, a
    trk: [i32; 7],
    /// g_tk_h, s, v, r, g, b, a
    tk: [Track; 7],
    trk_argb: u32,
    // colour wheel
    whsv: [u32; 3],
    pick: u32,
    tk_wv: Track,
    // lists
    rl_sel: i32,
    rl_vis: [i32; RL_N],
    rl_order: [u8; RL_N],
    rl_moves: i32,
    ants: Rc<Ants>,
    /// the list flyout (test hook 9061): to open (0 no, else arg + 1), no row can be chosen,
    /// its list's selection
    fly_open: u32,
    fly_none: bool,
    fly_sel: i32,
}

impl Default for S {
    fn default() -> S {
        S {
            tab: 0,
            list_dbl: 0,
            ib_closed: 0,
            sbar_pos: 0,
            exp_open: 0,
            exp_cb: 1,
            val_f: 12.5,
            prop_f: 1.5,
            pan_x: 0.0,
            pan_y: 0.0,
            prop_i: 25,
            trk: [210, 80, 90, 46, 126, 230, 200],
            tk: [
                Track::new(0, &TK_H),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(TK_CHECKER, &[0, 0]),
            ],
            trk_argb: 0,
            whsv: [1200, 700, 850], // green, 70 %, 85 %
            pick: 0xC0E0_4040,
            tk_wv: Track::new(0, &[0xFF00_0000, 0xFF00_FF00]),
            rl_sel: 0,
            rl_vis: [1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            rl_order: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            rl_moves: 0,
            ants: Rc::new(Ants { phase: Cell::new(0) }),
            fly_open: 0,
            fly_none: false,
            fly_sel: -1,
        }
    }
}

/// gal_frame: next_size(0, -1); scroll_begin(ID_PAGE, SC_V) -- the page scrolls, the status
/// bar stays at the bottom
pub(crate) fn page_begin(g: &mut Gal, ui: &mut Ui) {
    let _ = g;
    ui.next_size(0, -1);
    ui.scroll_begin(ID_PAGE, SC_V);
}

/// gal_frame: scroll_end of the page
pub(crate) fn page_end(g: &mut Gal, ui: &mut Ui) {
    let _ = g;
    ui.scroll_end();
}

/// gal_containers: tabs switching between a virtualized list (Gal::list_sel), expander + info
/// bars, and a dialog launcher (Gal::dlg_open / dlg_result) with horizontal scrolling
pub(crate) fn containers(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.list;
    ui.text(TS_SUBTITLE, "Container");
    ui.tabs(ID_TABS, &mut s.tab, &TAB_NAMES);
    match s.tab {
        1 => {
            // ---- tab 1: expander and closable info bars
            if ui.expander_begin(ID_EXP, "Erweiterte Einstellungen", &mut s.exp_open) != 0 {
                ui.checkbox(ID_EXP_CB, "Automatisch speichern", &mut s.exp_cb);
                ui.text(TS_CAPTION | TX_SECONDARY, "Der Inhalt erscheint, wenn der Expander offen ist.");
            }
            ui.expander_end();
            for i in 0..4u32 {
                if s.ib_closed & (1 << i) != 0 {
                    continue;
                }
                let ev = ui.infobar(ID_IB0 + i * 2, i, IB_TITLES[i as usize], IB_MSGS[i as usize]);
                if ev & EV_ACT != 0 {
                    s.ib_closed |= 1 << i;
                }
            }
        }
        2 => {
            // ---- tab 2: dialog launcher, horizontal scroll view, standalone scroll bar
            ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
            if ui.button(ID_DLG_BTN, "Dialog öffnen") & EV_ACT != 0 {
                g.dlg_open.set(1); // (ui-win: a Shared open flag)
            }
            let t = cap(format!("Ergebnis: {}", g.dlg_result), 96);
            ui.text(TS_BODY, &t);
            ui.box_end();
            ui.scroll_begin(ID_HSCROLL, SC_H | BOX_CARD | PAD_8);
            for i in 0..20u32 {
                let t = cap(format!("Knopf {}", i + 1), 32);
                ui.button(ID_CHIP0 + i, &t);
                ui.spacer(8);
            }
            ui.scroll_end();
            ui.next_size(300, 0);
            ui.scrollbar(ID_SBAR, &mut s.sbar_pos, 1000, 250, false);
        }
        _ => {
            // ---- tab 0: 200 entries, only the visible rows are emitted
            ui.next_size(0, 240);
            let (first, end) = ui.list_begin(ID_LIST, &mut g.list_sel, 200, 0);
            for i in first..end {
                let t = cap(format!("Eintrag {}", i + 1), 32);
                if ui.list_item(i, &t) & EV_DOUBLE != 0 {
                    s.list_dbl += 1;
                }
            }
            ui.list_end(&mut g.list_sel);
            let t = cap(format!("Auswahl: {}   Doppelklicks: {}", g.list_sel, s.list_dbl), 96);
            ui.text(TS_BODY | TX_SECONDARY, &t);
        }
    }
}

/// gal_values: "Werte & Farben" -- decimal slider and number box, property rows, angle
/// (Gal::ang) and pan, colour tracks (gal_tracks), the "Werte-Fenster" toggle
/// (Gal::win_val_open)
pub(crate) fn values(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.list;
    ui.text(TS_SUBTITLE, "Werte & Farben");
    // ---- decimal slider + number box on the same value
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(240, 0);
    ui.slider_f(ID_VAL_SF, &mut s.val_f, &RF_VAL);
    ui.number_box_f(ID_VAL_NBF, &mut s.val_f, &RF_VAL);
    let t = ui.fmt_f64(s.val_f, 2);
    ui.text(TS_BODY | TX_SECONDARY, &t);
    ui.box_end();
    // ---- property rows (paint.net effect dialogs)
    ui.next_size(420, 0);
    ui.box_begin(0, BOX_V, 12);
    ui.prop_slider(ID_VAL_PROP, "Radius", &mut s.prop_i, &RANGE_PROP, 25);
    ui.prop_slider_f(ID_VAL_PROPF, "Härte", &mut s.prop_f, &RF_PROP, Some(C_PROP_DEF));
    ui.box_end();
    // ---- angle dial and pan control
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.angle(ID_VAL_ANG, &mut g.ang, 0);
    let t = ui.fmt_f64(g.ang, 2);
    ui.next_size(56, 0);
    ui.text(TS_BODY | TX_RIGHT, &t);
    ui.pan(ID_VAL_PAN, &mut s.pan_x, &mut s.pan_y, None, 0, 0);
    ui.box_end();
    // ---- colour tracks
    tracks(s, ui);
    // ---- tool window with value widgets
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    let mut v = g.win_val_open.get(); // (ui-win: a Shared open flag)
    ui.toggle_button(ID_VAL_WINBTN, "Werte-Fenster", &mut v);
    g.win_val_open.set(v);
    ui.box_end();
}

// gal_tracks: H S V / R G B / A tracks on one colour, a vertical track, a swatch
fn tracks(s: &mut S, ui: &mut Ui) {
    const RANGES: [&Range; 7] = [&RANGE_HUE, &RANGE_PCT2, &RANGE_PCT2, &RANGE_BYTE, &RANGE_BYTE, &RANGE_BYTE, &RANGE_BYTE];
    // tracks that depend on the colour: S, V, R, G, B, A
    track_build(s);
    ui.box_begin(0, BOX_H, 24);
    ui.next_size(300, 0);
    ui.box_begin(0, BOX_V, 0);
    for i in 0..7 {
        let mut v = s.trk[i];
        let tk = s.tk[i].clone();
        let ev = ui.track_slider(ID_VAL_H + i as u32, &mut v, RANGES[i], Some(&tk));
        s.trk[i] = v;
        if ev & EV_ACT != 0 {
            if i < 3 {
                hsv_changed(s);
            } else if i < 6 {
                rgb_changed(s);
            }
        }
    }
    ui.box_end();
    // vertical V track and a swatch of the colour
    ui.next_size(32, 160);
    let tk = s.tk[2].clone();
    if ui.track_slider(ID_VAL_VT, &mut s.trk[2], &RANGE_PCT_V, Some(&tk)) & EV_ACT != 0 {
        hsv_changed(s);
    }
    ui.next_size(48, 48);
    // a right click (or the menu key while it has the focus) opens a menu with menu_popup:
    // at the pointer, for the keyboard at the swatch
    if ui.swatch(ID_VAL_SW, s.trk_argb, false) & EV_RCLICK != 0 {
        ui.menu_popup(&crate::sec_popup::M_CTX);
    }
    ui.box_end();
}

// gal_hsv_changed: H, S or V moved: RGB follows
fn hsv_changed(s: &mut S) {
    let t = &mut s.trk;
    let c = hsv_to_rgb((t[0] * 10) as u32, (t[1] * 10) as u32, (t[2] * 10) as u32);
    t[3] = ((c >> 16) & 255) as i32;
    t[4] = ((c >> 8) & 255) as i32;
    t[5] = (c & 255) as i32;
    track_build(s);
}

// gal_rgb_changed: R, G or B moved: HSV follows (the hue stays for greys)
fn rgb_changed(s: &mut S) {
    let t = &mut s.trk;
    let rgb = (t[3] as u32) << 16 | (t[4] as u32) << 8 | t[5] as u32;
    let (h, sa, v) = rgb_to_hsv(rgb);
    if sa != 0 {
        let mut hh = (h + 5) / 10;
        if hh >= 360 {
            hh = 0;
        }
        t[0] = hh as i32;
    }
    t[1] = ((sa + 5) / 10) as i32;
    t[2] = ((v + 5) / 10) as i32;
    track_build(s);
}

// gal_track_build: colour and the stops of the S, V, R, G, B, A tracks
fn track_build(s: &mut S) {
    let t = s.trk;
    let rgb = (t[3] as u32) << 16 | (t[4] as u32) << 8 | t[5] as u32;
    s.trk_argb = (t[6] as u32) << 24 | rgb;
    let (h, sa, v) = ((t[0] * 10) as u32, (t[1] * 10) as u32, (t[2] * 10) as u32);
    // S: grey .. full saturation (at V); V: black .. full value (at S)
    s.tk[1].stops[0] = hsv_to_rgb(h, 0, v);
    s.tk[1].stops[1] = hsv_to_rgb(h, 1000, v);
    s.tk[2].stops[0] = 0xFF00_0000;
    s.tk[2].stops[1] = hsv_to_rgb(h, sa, 1000);
    // R, G, B: the channel from 0 to 255 with the others fixed
    let c = (rgb & 0x00FFFF) | 0xFF00_0000;
    s.tk[3].stops[0] = c;
    s.tk[3].stops[1] = c | 0x00FF_0000;
    let c = (rgb & 0xFF00FF) | 0xFF00_0000;
    s.tk[4].stops[0] = c;
    s.tk[4].stops[1] = c | 0x0000_FF00;
    let c = (rgb & 0xFFFF00) | 0xFF00_0000;
    s.tk[5].stops[0] = c;
    s.tk[5].stops[1] = c | 0x0000_00FF;
    // A: transparent .. opaque over a checkerboard
    s.tk[6].stops[0] = rgb;
    s.tk[6].stops[1] = rgb | 0xFF00_0000;
}

/// gal_wheel: a colour wheel with a value track, the colour picker with alpha and fields
pub(crate) fn wheel(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.list;
    ui.box_begin(0, BOX_H | BOX_XSTART, 24);
    // wheel on an HSV record + vertical value track (black .. the colour at full value)
    s.tk_wv.stops[1] = hsv_to_rgb(s.whsv[0], s.whsv[1], 1000);
    ui.color_wheel(ID_WHEEL, &mut s.whsv, CW_HSV);
    ui.next_size(32, 160);
    let mut v = s.whsv[2] as i32;
    let tk = s.tk_wv.clone();
    ui.track_slider(ID_WHEEL_V, &mut v, &RANGE_WV, Some(&tk));
    s.whsv[2] = v as u32;
    ui.color_picker_ex(ID_PICKER, &mut s.pick, CP_ALPHA | CP_FIELDS);
    ui.box_end();
}

/// gal_lists: "Listen" -- reorderable rows, the order and info texts (Gal::recent_last), the
/// marching-ants canvas (ants_draw, canvas_damage)
pub(crate) fn lists(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.list;
    ui.text(TS_SUBTITLE, "Listen");
    ui.text(TS_BODY | TX_SECONDARY, "Zeilen lassen sich ziehen; das Häkchen schaltet die Sichtbarkeit.");
    ui.box_begin(0, BOX_H, 24);
    ui.next_size(280, 200);
    let (first, end) = ui.list_begin(ID_RLIST, &mut s.rl_sel, RL_N as i32, 40 | LF_REORDER);
    for row in first..end {
        ui.list_item_begin(row);
        // thumbnail: a small square in the item's colour
        let it = s.rl_order[row as usize] as usize; // item shown in row `row`
        ui.next_size(24, 24);
        ui.next_flags(ST_NOFOCUS); // a picture: no Tab stop
        ui.swatch(ID_RL_CB + 20 + it as u32, RL_COLORS[it], false);
        let t = cap(format!("Ebene {}", it + 1), 32);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &t);
        ui.checkbox(ID_RL_CB + it as u32, "", &mut s.rl_vis[it]);
        ui.list_item_end();
    }
    let (ev, v) = ui.list_end(&mut s.rl_sel);
    if ev & EV_DRAG != 0 {
        // move item `from` to `to` in the order
        let from = (v as u32 & 0xFFFF) as usize;
        let to = (v as u32 >> 16) as usize;
        let o = &mut s.rl_order;
        let item = o[from];
        let mut i = from;
        while i < to {
            o[i] = o[i + 1];
            i += 1;
        }
        while i > to {
            o[i] = o[i - 1];
            i -= 1;
        }
        o[to] = item;
        s.rl_moves += 1;
    }
    // the order as text: "Reihenfolge: 1 2 3 ..."
    let mut t = String::from("Reihenfolge: ");
    for (k, &o) in s.rl_order.iter().enumerate() {
        if k > 0 {
            t.push(' ');
        }
        t.push_str(&format!("{}", o as u32 + 1));
    }
    ui.box_begin(0, BOX_V, 8);
    ui.text(TS_BODY, &t);
    let t = cap(format!("Auswahl: {}   Verschoben: {}   Zuletzt: {}", s.rl_sel, s.rl_moves, g.recent_last), 96);
    ui.text(TS_BODY | TX_SECONDARY, &t);
    // marching ants: redrawn through canvas_damage only (the version stays)
    ui.next_size(200, 64);
    let d: DrawRef = s.ants.clone();
    ui.canvas(ID_CANVAS, Some(d), 0, 1);
    ui.box_end();
    ui.box_end();
}

/// the list flyout of test hook 9061 (after the page, like an app's popover): a list without
/// a selection gets the keyboard focus in the flyout's first frame and keeps -1 (the focus
/// alone selects nothing).  Arg 1: no row can be chosen, the app puts -1 back (JFNetworker's
/// port choice when no port fits).
pub(crate) fn list_flyout(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.list;
    if s.fly_open != 0 {
        s.fly_none = s.fly_open == 2;
        s.fly_open = 0;
        s.fly_sel = -1;
        ui.popup_open(ID_LIST_FLY);
        ui.focus(ID_LIST_FLY_L);
    }
    if ui.flyout_begin(ID_LIST_FLY, FL_POINT) {
        ui.next_size(260, 0);
        ui.box_begin(0, BOX_V | pad(8), 8);
        ui.text(TS_BODY_STRONG, "Liste im Flyout");
        ui.next_size(240, 6 * 28);
        let mut sel = s.fly_sel;
        let (first, end) = ui.list_begin(ID_LIST_FLY_L, &mut sel, 12, 28);
        for i in first..end {
            let t = cap(format!("Zeile {}", i + 1), 32);
            ui.list_item(i, &t);
        }
        ui.list_end(&mut sel);
        if !s.fly_none {
            s.fly_sel = sel;
        }
        ui.box_end();
    }
    ui.flyout_end();
}

/// gal_u1_cmd(id, arg) -> handled, its list / value parts: CMD_ANTS (canvas_damage),
/// CMD_DEC_SEP (the decimal separator); CMD_LIST_FLY opens the list flyout
pub(crate) fn u1_cmd(g: &mut Gal, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let _ = arg;
    match id {
        CMD_ANTS => {
            let p = &g.list.ants.phase;
            p.set(p.get().wrapping_add(3));
            // only the edges changed
            ui.canvas_damage(ID_CANVAS, 0, 0, 400, 16);
            ui.canvas_damage(ID_CANVAS, 0, 40, 400, 400);
            true
        }
        CMD_DEC_SEP => {
            let c = ui.decimal_sep() ^ (44 ^ 46); // ',' <-> '.'
            ui.set_decimal_sep(c);
            true
        }
        CMD_LIST_FLY => {
            g.list.fly_open = arg.min(1) + 1;
            true
        }
        _ => false,
    }
}
