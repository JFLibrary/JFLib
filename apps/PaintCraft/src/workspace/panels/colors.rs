// wnd_colors -- PaintCraft: the "Farben" tool window like paint.net 5 (COLOR-001..008, 019).
// Owner: agent A3 (colours).  The colour model and the palette are in palette.rs.
//                                              [apps/PaintCraft/src/wnd_colors.s]
//
//   wcol_init()            window record (compact width, height = content)
//   wcol_frame()           the window while chrome_wnd_open(WND_COLORS) != 0
//                          (WF_TOOL | WF_AUTOHEIGHT; the close button clears the flag)
//   wcol_reset_pos()       default place (bottom left of the main window) on the next frame
//
// Compact ("Weniger"):
//   [Primärfarbe v] [Mehr >>]                  which colour the controls edit
//   [primary/secondary] [colour wheel] [V]     primary square over the secondary square (a
//   [+][palette v]                             click edits that one), swap arrow (X) top
//                                              right, black / white reset bottom left; HSV
//                                              wheel (hue, saturation) + vertical value track
//   [palette grid 16 x 6]                      click = primary, right click = secondary
// Expanded ("Mehr >>"; "<< Weniger" collapses) adds a column on the right:
//   RGB   R: G: B: track + number box, Hex.: RRGGBB (6 digits keep the alpha, AARRGGBB sets it)
//   HSV   H: 0..360, S: 0..100, V: 0..100 track + number box
//   Transparenz - Alpha: track over a checkerboard + number box 0..255
// Every control updates the colour live.  Hue and saturation are kept while a colour is grey
// or black (the HSV record of the window is the source of the wheel).  When the colour
// changes elsewhere (palette, X, tools, other window) the fields follow at the next frame.
// Widget ids (4400-4699): ID_WC_* below; palette parts 4700-4999 (palette.rs).

use alloc::format;
use alloc::string::String;

use ui::*;

use crate::app::constants::*;
use crate::workspace::palette::{pal_buttons, pal_checker, pal_color, pal_grid, pal_primary, pal_reset, pal_secondary, pal_set_color, pal_swap};
use crate::app::Pn;

const WC_PAD: u32 = 8; // padding of the content (DIP)
const WC_GAP: i32 = 8; // between the rows of the left column
const WC_LEFT_W: i32 = 256; // left column (= palette grid: 16 x 16 DIP)
const WC_RIGHT_W: i32 = 208; // right column (Mehr)
const WC_COLGAP: i32 = 16;
const WC_W_LESS: i32 = WC_LEFT_W + 2 * WC_PAD as i32;
const WC_W_MORE: i32 = WC_W_LESS + WC_COLGAP + WC_RIGHT_W;
const WC_ROWA_H: i32 = 32; // combo box + Mehr
const WC_ROWB_H: i32 = 160; // swatches, wheel, value track
const WC_GRID_H: i32 = 96;
const WC_H_LESS: i32 = 2 * WC_PAD as i32 + WC_ROWA_H + WC_ROWB_H + WC_GRID_H + 2 * WC_GAP;
const WC_H_MORE: i32 = 2 * WC_PAD as i32 + 3 * 20 + 8 * 32 + 10 * 4; // right column: 3 headers, 8 rows
const WC_OUTER: i32 = 28 + 2; // tool window title bar + border (DIP)
const WC_MARGIN: i32 = 8; // default place: from the main window's edges
const WC_SB_H: i32 = 32; // status bar of the main window
const TOOLS_RIGHT: i32 = 8 + 112 + 2; // the Werkzeuge window's default place (wnd_tools.rs):
const TOOLS_BOTTOM: i32 = 140 + 346 + WC_OUTER; //   x 8, y 140, 112 x 346 DIP client
const COL1_W: i32 = 64; // swatches + palette buttons
const VAL_W: i32 = 24; // value track
const LBL_W: i32 = 22; // "R:" ...
const NUM_W: i32 = 56; // number boxes
const HEX_W: i32 = 80;
const SW_BOX: i32 = 52; // primary / secondary widget
const SW_SQ: i32 = 34; // one square
const SW_OFF: i32 = 18; // offset of the secondary square
const SW_ICON: i32 = 16; // swap / reset hot spots
const HEX_CAP: u32 = 16;
const NROWS: usize = 7; // R G B H S V A
const ROW_H: usize = 3;
const ROW_A: usize = 6;
// widget ids
const ID_WC_WHICH: u32 = ID_WCOL_FIRST; // 4400 combo box Primärfarbe / Sekundärfarbe
const ID_WC_MORE: u32 = ID_WCOL_FIRST + 1; // 4401 Mehr >> / << Weniger
const ID_WC_PRI: u32 = ID_WCOL_FIRST + 2; // 4402 primary square
const ID_WC_SEC: u32 = ID_WCOL_FIRST + 3; // 4403 secondary square
const ID_WC_SWAP: u32 = ID_WCOL_FIRST + 4; // 4404 swap arrow
const ID_WC_RESET: u32 = ID_WCOL_FIRST + 5; // 4405 black / white
const ID_WC_WHEEL: u32 = ID_WCOL_FIRST + 6; // 4406 colour wheel
const ID_WC_VALUE: u32 = ID_WCOL_FIRST + 7; // 4407 value track (compact)
const ID_WC_ROW0: u32 = ID_WCOL_FIRST + 10; // 4410 + 2k track, 4411 + 2k number box
const ID_WC_HEX: u32 = ID_WCOL_FIRST + 24; // 4424 hex text box

// swap arrow segments (x0, y0, x1, y1 in DIP * 256 inside the 16 DIP hot spot): the arc
// (4, 4) -> (12, 12) around (4, 12) and the heads at both ends
static ARROW_SEGS: [[i32; 4]; 8] = [
    [1024, 1024, 1807, 1180],
    [1807, 1180, 2473, 1623],
    [2473, 1623, 2916, 2289],
    [2916, 2289, 3072, 3072],
    [1024, 1024, 1664, 384],
    [1024, 1024, 1664, 1664],
    [3072, 3072, 2432, 2432],
    [3072, 3072, 3712, 2432],
];
const RG_BYTE: Range = Range::new(0, 255, 1, 0);
const RG_HUE: Range = Range::new(0, 360, 1, 0);
const RG_PCT: Range = Range::new(0, 100, 1, 0);
const RG_VAL: Range = Range::new(0, 1000, 10, RG_VERT);
// rows R G B H S V A: label, range (the track is tracks[k])
static ROW_TAB: [(&str, Range); NROWS] = [
    ("R:", RG_BYTE),
    ("G:", RG_BYTE),
    ("B:", RG_BYTE),
    ("H:", RG_HUE),
    ("S:", RG_PCT),
    ("V:", RG_PCT),
    ("", RG_BYTE),
];
static WHICH_LIST: [&str; 2] = ["Primärfarbe", "Sekundärfarbe"];

/// the state of wnd_colors.s (its g_* globals)
pub(crate) struct WCol {
    /// g_wc_rect: WINRECT (persisted by the settings module)
    rect: Shared<WinRect>,
    /// g_wc_more: expanded (Mehr)
    more: bool,
    /// g_wc_which: 0 = primary, 1 = secondary is edited
    which: i32,
    /// g_wc_valid: the fields show `seen`
    valid: bool,
    /// g_wc_seen: colour the fields were made from; g_wc_seen_which
    seen: u32,
    seen_which: i32,
    /// g_wc_hexfoc: the hex box had the focus in the last pass
    hexfoc: bool,
    /// g_wc_tf_cur / g_wc_tfocus: text box with the focus in this / the last pass (0 = none)
    tf_cur: u32,
    tfocus: u32,
    /// g_wc_src: widget whose event changes the colour now
    src: u32,
    /// g_wc_guard: box whose next commit is stale; g_wc_guard_n: passes the guard lives on
    guard: u32,
    guard_n: i32,
    /// g_wc_blur: take the focus from the guarded box
    blur: bool,
    /// g_wc_main_w / g_wc_main_h: main window client size (DIP)
    main_w: i32,
    main_h: i32,
    /// g_wc_hsv: hue 0..3599, saturation / value 0..1000; g_wc_whsv: the wheel's copy
    hsv: [u32; 3],
    whsv: [u32; 3],
    /// g_wc_val: R G B (0..255) H (0..360) S V (0..100) A (0..255)
    val: [i32; NROWS],
    /// g_wc_hex: the hex text box
    hex: String,
    /// gradient tracks R G B H S V A (stops rewritten every frame by wc_tracks; the hue
    /// track is fixed)
    tk: [Track; NROWS],
}

impl Default for WCol {
    fn default() -> WCol {
        WCol {
            rect: Shared::default(),
            more: false,
            which: 0,
            valid: false,
            seen: 0,
            seen_which: 0,
            hexfoc: false,
            tf_cur: 0,
            tfocus: 0,
            src: 0,
            guard: 0,
            guard_n: 0,
            blur: false,
            main_w: 0,
            main_h: 0,
            hsv: [0; 3],
            whsv: [0; 3],
            val: [0; NROWS],
            hex: String::new(),
            tk: [
                Track::new(0, &[0xFF00_0000, 0xFFFF_0000]),
                Track::new(0, &[0xFF00_0000, 0xFF00_FF00]),
                Track::new(0, &[0xFF00_0000, 0xFF00_00FF]),
                Track::new(0, &[0xFFFF_0000, 0xFFFF_FF00, 0xFF00_FF00, 0xFF00_FFFF, 0xFF00_00FF, 0xFFFF_00FF, 0xFFFF_0000]),
                Track::new(0, &[0xFFFF_FFFF, 0xFFFF_0000]),
                Track::new(0, &[0xFF00_0000, 0xFFFF_FFFF]),
                Track::new(TK_CHECKER, &[0x0000_0000, 0xFF00_0000]),
            ],
        }
    }
}

/// wcol_init() -- window record: compact width, height from the content
pub(crate) fn wcol_init(p: &mut Pn, _ui: &mut Ui) {
    let w = &mut p.wcol;
    let mut r = w.rect.get();
    r.w = WC_W_LESS;
    r.h = 0;
    r.flags = 0;
    w.rect.set(r);
    w.valid = false;
}

/// wcol_reset_pos() -- default place on the next frame
pub(crate) fn wcol_reset_pos(p: &mut Pn) {
    let mut r = p.wcol.rect.get();
    r.flags &= !WRF_POS;
    p.wcol.rect.set(r);
    crate::workspace::panels::dock::dock_reset(&mut p.dock, WND_COLORS);
}

/// wcol_frame() -- the window (main window context: from pn_frame's module loop)
pub(crate) fn wcol_frame(p: &mut Pn, ui: &mut Ui) {
    let (mw, mh) = ui.client_size(); // the main window (for placing ours)
    p.wcol.main_w = mw;
    p.wcol.main_h = mh;
    if p.wcol.rect.get().flags & WRF_POS == 0 {
        wc_place(&mut p.wcol);
    }
    // text box focus of the last pass; a guard only lives for a few passes
    let w = &mut p.wcol;
    w.tfocus = w.tf_cur;
    w.tf_cur = 0;
    w.hexfoc = w.tfocus == ID_WC_HEX; // (a hidden hex box has no focus)
    if w.guard_n != 0 {
        w.guard_n -= 1;
        if w.guard_n == 0 {
            w.guard = 0;
        }
    } else {
        w.guard = 0;
    }
    wc_sync(p);
    wc_tracks(&mut p.wcol);
    if crate::workspace::panels::dock::dock_hidden(&p.dock, WND_COLORS) {
        return; // no room in the main window (wnd_dock.rs)
    }
    let open = crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_COLORS);
    let rect = p.wcol.rect.clone();
    if ui.window_begin(WINID_COLORS, crate::preferences::localization::tr("Farben"), Some(&open), WF_TOOL | WF_AUTOHEIGHT, Some(&rect)) {
        ui.box_begin(0, BOX_H | BOX_XSTART | (WC_PAD << PAD_SHIFT), WC_COLGAP);
        wc_left(p, ui);
        if p.wcol.more {
            wc_right(p, ui);
        }
        ui.box_end();
    }
    ui.window_end();
}

// wc_place() -- default place: bottom left of the main window (right of the Werkzeuge window
// when they would overlap)
fn wc_place(w: &mut WCol) {
    let mut y = w.main_h - (WC_SB_H + WC_MARGIN + WC_H_LESS + WC_OUTER);
    let x = if y < TOOLS_BOTTOM + WC_MARGIN { TOOLS_RIGHT + WC_MARGIN } else { WC_MARGIN };
    if y < 0 {
        y = 0;
    }
    let mut r = w.rect.get();
    r.x = x;
    r.y = y;
    r.flags |= WRF_POS;
    w.rect.set(r);
}

// wc_toggle_more() -- Mehr >> / << Weniger: width of the window; the larger window is kept
// inside the main window
fn wc_toggle_more(w: &mut WCol, ui: &mut Ui) {
    w.more = !w.more;
    let mut r = w.rect.get();
    let mut width = WC_W_LESS;
    if w.more {
        width = WC_W_MORE;
        let x = w.main_w - (WC_W_MORE + 2 + WC_MARGIN);
        if r.x > x {
            r.x = x.max(0);
        }
        let y = w.main_h - (WC_SB_H + WC_MARGIN + WC_H_MORE + WC_OUTER);
        if r.y > y {
            r.y = y.max(0);
        }
    }
    r.w = width;
    w.rect.set(r);
    ui.repass();
}

// ------------------------------------------------------------------ colour state

// wc_sync() -- the fields follow the edited colour when it changed outside the window
fn wc_sync(p: &mut Pn) {
    let c = pal_color(&p.pal, p.wcol.which);
    let w = &mut p.wcol;
    if w.valid && c == w.seen && w.which == w.seen_which {
        return;
    }
    wc_load(w, c);
}

// wc_load(argb) -- every field from a colour; hue (grey) and saturation (black) are kept
fn wc_load(w: &mut WCol, c: u32) {
    wc_guard(w);
    w.seen = c;
    w.seen_which = w.which;
    w.valid = true;
    wc_rgba_fields(w);
    let hsv = rgb_to_hsv(c);
    wc_hsv_keep(w, hsv);
    wc_hsv_ints(w);
    if !w.hexfoc {
        wc_hex_text(w);
    }
}

// wc_rgba_fields() -- R G B A fields from `seen`
fn wc_rgba_fields(w: &mut WCol) {
    let c = w.seen;
    w.val[0] = (c >> 16 & 255) as i32;
    w.val[1] = (c >> 8 & 255) as i32;
    w.val[2] = (c & 255) as i32;
    w.val[ROW_A] = (c >> 24) as i32;
}

// wc_hsv_keep(h, s, v) -- the HSV record from rgb_to_hsv; a black colour keeps hue and
// saturation, a grey one keeps the hue
fn wc_hsv_keep(w: &mut WCol, (h, s, v): (u32, u32, u32)) {
    w.hsv[2] = v;
    if v == 0 {
        return;
    }
    w.hsv[1] = s;
    if s == 0 {
        return;
    }
    w.hsv[0] = h;
}

// wc_hsv_ints() -- H S V fields (0..360, 0..100, 0..100) from the HSV record
fn wc_hsv_ints(w: &mut WCol) {
    for k in 0..3 {
        w.val[ROW_H + k] = (w.hsv[k].wrapping_add(5) / 10) as i32;
    }
}

// wc_hex_text() -- the hex box shows RRGGBB of the colour
fn wc_hex_text(w: &mut WCol) {
    w.hex = format!("{:06X}", w.seen & 0xFF_FFFF);
}

// wc_commit(argb) -- the edited colour becomes argb (the fields already show it)
fn wc_commit(p: &mut Pn, c: u32) {
    let w = &mut p.wcol;
    wc_guard(w);
    w.seen = c;
    w.seen_which = w.which;
    pal_set_color(&mut p.pal, w.which, c);
    if !w.hexfoc {
        wc_hex_text(w);
    }
}

// wc_hsv_rgb() -> the colour of the HSV record with the alpha field
fn wc_hsv_rgb(w: &WCol) -> u32 {
    let c = hsv_to_rgb(w.hsv[0] % 3600, w.hsv[1], w.hsv[2]);
    (c & 0xFF_FFFF) | (w.val[ROW_A] as u32) << 24
}

// wc_from_hsv() -- the wheel or a value track moved (HSV record): everything follows
fn wc_from_hsv(p: &mut Pn) {
    let w = &mut p.wcol;
    w.seen = wc_hsv_rgb(w);
    wc_rgba_fields(w);
    wc_hsv_ints(w);
    let c = w.seen;
    wc_commit(p, c);
}

// wc_from_hsv_ints() -- an H, S or V field changed: the HSV record and the RGB fields follow
fn wc_from_hsv_ints(p: &mut Pn) {
    let w = &mut p.wcol;
    let mut h = (w.val[ROW_H] as u32).wrapping_mul(10);
    if h >= 3600 {
        h = h.wrapping_sub(3600);
    }
    w.hsv[0] = h;
    w.hsv[1] = (w.val[ROW_H + 1] as u32).wrapping_mul(10);
    w.hsv[2] = (w.val[ROW_H + 2] as u32).wrapping_mul(10);
    w.seen = wc_hsv_rgb(w);
    wc_rgba_fields(w);
    let c = w.seen;
    wc_commit(p, c);
}

// wc_from_rgb() -- an R, G or B field changed: HSV follows (keeping hue / saturation)
fn wc_from_rgb(p: &mut Pn) {
    let w = &mut p.wcol;
    let v = &w.val;
    let c = (v[ROW_A] as u32) << 24 | (v[0] as u32) << 16 | (v[1] as u32) << 8 | v[2] as u32;
    let hsv = rgb_to_hsv(c);
    wc_hsv_keep(w, hsv);
    wc_hsv_ints(w);
    wc_commit(p, c);
}

// wc_from_alpha() -- the alpha field changed
fn wc_from_alpha(p: &mut Pn) {
    let w = &p.wcol;
    let c = (w.seen & 0xFF_FFFF) | (w.val[ROW_A] as u32) << 24;
    wc_commit(p, c);
}

// wc_from_hex() -- the hex text was committed: RRGGBB (alpha kept) or AARRGGBB, an optional
// '#', blanks around it; anything else restores the text
fn wc_from_hex(p: &mut Pn) {
    let b = p.wcol.hex.as_bytes();
    let at = |i: usize| -> u32 { if i < HEX_CAP as usize { b.get(i).copied().unwrap_or(0) as u32 } else { 0 } };
    let mut i = 0usize;
    let parsed = 'p: {
        loop {
            if i >= HEX_CAP as usize {
                break 'p None;
            }
            if at(i) != 32 {
                break;
            }
            i += 1;
        }
        if at(i) == 35 {
            i += 1;
        }
        let mut v = 0u32;
        let mut nd = 0;
        let mut c;
        loop {
            if i >= HEX_CAP as usize {
                c = 0;
                break;
            }
            c = at(i);
            let d = if (48..=57).contains(&c) {
                c - 48
            } else {
                let l = c | 32;
                if (97..=102).contains(&l) {
                    l - 97 + 10
                } else {
                    // only blanks may follow
                    while c == 32 {
                        i += 1;
                        c = if i >= HEX_CAP as usize { 0 } else { at(i) };
                        if i >= HEX_CAP as usize {
                            break;
                        }
                    }
                    break;
                }
            };
            v = v << 4 | d;
            nd += 1;
            i += 1;
            if nd > 8 {
                break 'p None;
            }
        }
        if c != 0 {
            break 'p None;
        }
        match nd {
            8 => Some(v),
            6 => Some((v & 0xFF_FFFF) | (p.wcol.val[ROW_A] as u32) << 24),
            _ => None,
        }
    };
    match parsed {
        Some(c) => {
            p.wcol.hexfoc = false;
            wc_load(&mut p.wcol, c);
            wc_commit(p, c);
        }
        None => wc_hex_text(&mut p.wcol),
    }
}

// wc_tracks() -- gradients of the tracks from the current fields
fn wc_tracks(w: &mut WCol) {
    let v = &w.val;
    let rgb = (v[0] as u32) << 16 | (v[1] as u32) << 8 | v[2] as u32; // 0x00RRGGBB
    let set = |t: &mut Track, a: u32, b: u32| {
        t.stops[0] = a;
        t.stops[1] = b;
    };
    // R: 0..255 with G, B
    let a = (rgb & 0xFFFF) | 0xFF00_0000;
    set(&mut w.tk[0], a, a | 0xFF_0000);
    let a = (rgb & 0xFF_00FF) | 0xFF00_0000;
    set(&mut w.tk[1], a, a | 0xFF00);
    let a = (rgb & 0xFF_FF00) | 0xFF00_0000;
    set(&mut w.tk[2], a, a | 0xFF);
    // A: transparent .. opaque colour
    set(&mut w.tk[6], rgb, rgb | 0xFF00_0000);
    // hue of the record (0..3599)
    let h = w.hsv[0] % 3600;
    // S: grey .. full saturation at the value
    let a = hsv_to_rgb(h, 0, w.hsv[2]);
    let b = hsv_to_rgb(h, 1000, w.hsv[2]);
    set(&mut w.tk[4], a, b);
    // V: black .. full value
    let b = hsv_to_rgb(h, w.hsv[1], 1000);
    set(&mut w.tk[5], 0xFF00_0000, b);
}

// ------------------------------------------------------------------ layout

// wc_left() -- selector + Mehr, swatches / wheel / value, palette
fn wc_left(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(WC_LEFT_W, 0);
    ui.box_begin(0, BOX_V, WC_GAP);
    // ---- which colour, Mehr >> / << Weniger
    ui.next_size(-1, WC_ROWA_H);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    if ui.combobox(ID_WC_WHICH, &mut p.wcol.which, &WHICH_LIST) & EV_ACT != 0 {
        wc_sync(p);
        ui.repass();
    }
    ui.next_size(104, 0);
    let l = if p.wcol.more { "<< Weniger" } else { "Mehr >>" };
    if ui.button(ID_WC_MORE, l) & EV_ACT != 0 {
        wc_toggle_more(&mut p.wcol, ui);
    }
    ui.box_end();
    // ---- swatches + palette buttons | wheel | value
    ui.next_size(-1, WC_ROWB_H);
    ui.box_begin(0, BOX_H, 4);
    ui.next_size(COL1_W, -1);
    ui.box_begin(0, BOX_V | BOX_XSTART, 0);
    wc_swatches(p, ui);
    ui.spacer(-1);
    pal_buttons(p, ui);
    ui.box_end();
    // the wheel is always drawn at full value (like paint.net): its own HSV record
    let w = &mut p.wcol;
    w.whsv = [w.hsv[0], w.hsv[1], 1000];
    ui.next_size(-1, -1);
    if ui.color_wheel(ID_WC_WHEEL, &mut w.whsv, CW_HSV) & EV_ACT != 0 {
        w.hsv[0] = w.whsv[0];
        w.hsv[1] = w.whsv[1];
        // picking on the wheel while the colour is black gives the colour at full value
        if w.hsv[2] == 0 {
            w.hsv[2] = 1000;
        }
        wc_from_hsv(p);
    }
    ui.next_size(VAL_W, -1);
    let w = &mut p.wcol;
    let mut v = w.hsv[2] as i32;
    let ev = ui.track_slider(ID_WC_VALUE, &mut v, &RG_VAL, Some(&w.tk[5]));
    w.hsv[2] = v as u32;
    if ev & EV_ACT != 0 {
        wc_from_hsv(p);
    }
    ui.box_end();
    // ---- palette
    pal_grid(p, ui);
    ui.box_end();
}

// wc_right() -- RGB, hex, HSV, alpha (Mehr)
fn wc_right(p: &mut Pn, ui: &mut Ui) {
    wc_blur(&mut p.wcol, ui);
    ui.next_size(WC_RIGHT_W, 0);
    ui.box_begin(0, BOX_V, 4);
    wc_header(ui, "RGB");
    for k in 0..ROW_H {
        wc_row(p, ui, k);
    }
    wc_hex_row(p, ui);
    wc_header(ui, "HSV");
    for k in ROW_H..ROW_A {
        wc_row(p, ui, k);
    }
    wc_header(ui, "Transparenz - Alpha");
    wc_row(p, ui, ROW_A);
    ui.box_end();
}

// wc_header(str) -- "RGB" and a line to the right edge (paint.net group header)
fn wc_header(ui: &mut Ui, s: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY_STRONG, s);
    ui.next_size(-1, 1);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    ui.box_end();
    ui.box_end();
}

// wc_row(k) -- label, track and number box of field k (R G B H S V A)
fn wc_row(p: &mut Pn, ui: &mut Ui, k: usize) {
    if k >= NROWS {
        return;
    }
    wc_blur(&mut p.wcol, ui);
    let (label, rg) = ROW_TAB[k];
    let w = &mut p.wcol;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(LBL_W, 0);
    ui.text(TS_BODY, label);
    ui.next_size(-1, 0);
    let tid = ID_WC_ROW0 + 2 * k as u32;
    let evt = ui.track_slider(tid, &mut w.val[k], &rg, Some(&w.tk[k]));
    ui.next_size(NUM_W, 0);
    let evn = ui.number_box(tid + 1, &mut w.val[k], &rg);
    ui.box_end();
    // the number box: focus bookkeeping, a stale commit is undone
    if wc_text_ev(p, ui, tid + 1, evn) {
        return;
    }
    // the source of a change: the number box or the track
    p.wcol.src = if evn & EV_ACT != 0 { tid + 1 } else { tid };
    if (evt | evn) & EV_ACT != 0 {
        if k < ROW_H {
            wc_from_rgb(p);
        } else if k < ROW_A {
            wc_from_hsv_ints(p);
        } else {
            wc_from_alpha(p);
        }
    }
    p.wcol.src = 0;
}

// wc_hex_row() -- "Hex.:" and the text box (taken on Enter / focus loss)
fn wc_hex_row(p: &mut Pn, ui: &mut Ui) {
    wc_blur(&mut p.wcol, ui);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.text(TS_BODY, crate::preferences::localization::tr("Hex.:"));
    ui.spacer(-1);
    ui.next_size(HEX_W, 0);
    let ev = ui.textbox(ID_WC_HEX, None, &mut p.wcol.hex, HEX_CAP);
    ui.box_end();
    p.wcol.hexfoc = ev & EV_FOCUS != 0;
    // EV_ACT of a text box = typing
    if wc_text_ev(p, ui, ID_WC_HEX, ev & !EV_ACT) {
        return;
    }
    if ev & EV_COMMIT == 0 {
        return;
    }
    p.wcol.src = ID_WC_HEX;
    wc_from_hex(p);
    p.wcol.src = 0;
    ui.repass();
}

// ------------------------------------------------------------------ stale text guard
// ui.number_box (and the hex box) keep the text they had when they got the focus: when the
// colour changes elsewhere while one of them holds the focus (wheel, palette, swap, tools
// ...), that box would commit its old text when it loses the focus.  So such a change arms
// a guard: the box loses the focus (ui.focus(0) while this window is built) and its next
// commit is undone.

// wc_guard() -- the colour changes; a text box other than the source holds the focus
fn wc_guard(w: &mut WCol) {
    let f = w.tfocus;
    if f == 0 || f == w.src {
        return;
    }
    w.guard = f;
    w.guard_n = 4;
    w.blur = true;
}

// wc_blur() -- (inside the window) the guarded box loses the focus
fn wc_blur(w: &mut WCol, ui: &mut Ui) {
    if !w.blur {
        return;
    }
    w.blur = false;
    if ui.focused() == w.guard {
        ui.focus(0);
    }
}

// wc_text_ev(id, ev) -> true = a stale commit of box id was undone (ignore its events)
fn wc_text_ev(p: &mut Pn, ui: &mut Ui, id: u32, ev: u32) -> bool {
    let w = &mut p.wcol;
    if ev & EV_FOCUS != 0 {
        w.tf_cur = id;
    }
    if w.guard != id || ev & (EV_ACT | EV_COMMIT) == 0 {
        return false;
    }
    w.guard = 0;
    if ev & EV_ACT != 0 {
        w.src = id; // the fields show the colour again
        let c = w.seen;
        wc_load(w, c);
        w.src = 0;
        ui.repass();
    }
    wc_hex_text(w);
    true
}

// ------------------------------------------------------------------ primary / secondary

// wc_square(x, y, size, argb, edited) -- one colour square (checkerboard under transparent
// colours) with a frame: accent 2 px when it is the edited colour
fn wc_square(ui: &mut Ui, x: i32, y: i32, size: i32, argb: u32, edited: bool) {
    let hair = ui.hair();
    if argb < 0xFF00_0000 {
        pal_checker(ui, x, y, size, size);
    }
    ui.draw_color(argb);
    ui.draw_rect(x, y, size, size);
    let (tok, t) = if edited { (T_ACCENT, hair + hair) } else { (T_TEXT3, hair) };
    let c = ui.color(tok);
    ui.draw_color(c);
    ui.draw_frame(x, y, size, size, 0, t);
}

// wc_icon_bg(ev, x, y, size) -- subtle hover / pressed background of a small hot spot
fn wc_icon_bg(ui: &mut Ui, ev: u32, x: i32, y: i32, size: i32) {
    let tok = if ev & EV_HELD != 0 {
        T_SUBTLE_PRESSED
    } else if ev & EV_HOVER != 0 {
        T_SUBTLE_HOVER
    } else {
        return;
    };
    let c = ui.color(tok);
    ui.draw_color(c);
    let r = ui.dp(3);
    ui.draw_rrect(x, y, size, size, r);
}

// wc_sw_record(id, argb, edited) -- semantic record of a square: label AARRGGBB, value and
// the selected state = edited
fn wc_sw_record(ui: &mut Ui, id: u32, argb: u32, edited: bool) {
    if edited {
        let r = ui.last_rect();
        let s = ui.last_state() | SS_SELECTED as u32;
        ui.last_set(r, s);
    }
    ui.sem(K_SWATCH, id, Some(&format!("{:08X}", argb)), edited as i32);
}

// wc_select(which) -- the controls edit that colour now
fn wc_select(p: &mut Pn, ui: &mut Ui, which: i32) {
    if p.wcol.which == which {
        return;
    }
    p.wcol.which = which;
    wc_sync(p);
    ui.repass();
}

// wc_swatches() -- primary square over the secondary one, swap arrow, black / white
fn wc_swatches(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(SW_BOX, SW_BOX);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    let br = ui.box_rect();
    let (x0, y0) = (br[0], br[1]);
    let sq = ui.dp(SW_SQ);
    let off = ui.dp(SW_OFF);
    let ic = ui.dp(SW_ICON);
    let bx = ui.dp(SW_BOX);
    let hair = ui.hair();
    // the keyboard: the four are one Tab stop (the edited colour first), the arrows move inside
    ui.focus_group_begin(0);
    // ---- secondary (hit box first: the primary square wins where they overlap)
    let (x, y) = (x0 + off, y0 + off);
    let ed = p.wcol.which != 0;
    let ev = ui.hit_rect(ID_WC_SEC, x, y, sq, sq, HB_FOCUS | if ed { HB_CURRENT } else { 0 });
    let c = pal_secondary(&p.pal);
    wc_square(ui, x, y, sq, c, ed);
    ui.focus_ring_draw(ev);
    wc_sw_record(ui, ID_WC_SEC, c, ed);
    ui.tooltip("Sekundärfarbe");
    if ev & EV_ACT != 0 {
        wc_select(p, ui, 1);
    }
    // ---- primary
    let ed = p.wcol.which == 0;
    let ev = ui.hit_rect(ID_WC_PRI, x0, y0, sq, sq, HB_FOCUS | if ed { HB_CURRENT } else { 0 });
    let c = pal_primary(&p.pal);
    wc_square(ui, x0, y0, sq, c, ed);
    ui.focus_ring_draw(ev);
    wc_sw_record(ui, ID_WC_PRI, c, ed);
    ui.tooltip("Primärfarbe");
    if ev & EV_ACT != 0 {
        wc_select(p, ui, 0);
    }
    // ---- swap arrow (top right)
    let (x, y) = (x0 + bx - ic, y0);
    let ev = ui.hit_rect(ID_WC_SWAP, x, y, ic, ic, HB_FOCUS);
    wc_icon_bg(ui, ev, x, y, ic);
    wc_swap_arrow(ui, x, y);
    ui.focus_ring_draw(ev);
    ui.sem(K_BUTTON, ID_WC_SWAP, Some("Farben vertauschen"), 0);
    ui.tooltip("Farben vertauschen (X)");
    if ev & EV_ACT != 0 {
        pal_swap(&mut p.pal);
    }
    // ---- black / white (bottom left)
    let (x, y) = (x0, y0 + bx - ic);
    let ev = ui.hit_rect(ID_WC_RESET, x, y, ic, ic, HB_FOCUS);
    wc_icon_bg(ui, ev, x, y, ic);
    // white square (6, 6), then black (2, 2), 8 x 8 DIP, hairline frames
    for (o, c) in [(6, 0xFFFF_FFFFu32), (2, 0xFF00_0000)] {
        let o = ui.dp(o);
        ui.draw_color(c);
        let s = ui.dp(8);
        ui.draw_rect(x + o, y + o, s, s);
        let t = ui.color(T_TEXT2);
        ui.draw_color(t);
        ui.draw_frame(x + o, y + o, s, s, 0, hair);
    }
    ui.last_set([x, y, ic, ic], ui.last_state());
    ui.focus_ring_draw(ev);
    ui.sem(K_BUTTON, ID_WC_RESET, Some("Standardfarben"), 0);
    ui.tooltip("Standardfarben");
    if ev & EV_ACT != 0 {
        pal_reset(&mut p.pal);
    }
    ui.focus_group_end();
    ui.box_end();
}

// wc_swap_arrow(x, y) -- the swap icon: a quarter arc with arrow heads at both ends (16 DIP)
fn wc_swap_arrow(ui: &mut Ui, x: i32, y: i32) {
    let (xf, yf) = (x << 8, y << 8);
    let c = ui.color(T_TEXT);
    ui.draw_color(c);
    let wd = ui.dp_fx(320); // 1.25 DIP
    for s in ARROW_SEGS.iter() {
        let x0 = ui.dp_fx(s[0]) + xf;
        let y0 = ui.dp_fx(s[1]) + yf;
        let x1 = ui.dp_fx(s[2]) + xf;
        let y1 = ui.dp_fx(s[3]) + yf;
        ui.draw_line(x0, y0, x1, y1, wd);
    }
}

/// wcol_rect() -> the WINRECT of the window (settings.rs saves / restores it)
pub(crate) fn wcol_rect(w: &WCol) -> Shared<WinRect> {
    w.rect.clone()
}

/// wcol_more() -> true while expanded ("Mehr")
pub(crate) fn wcol_more(w: &WCol) -> bool {
    w.more
}

/// wcol_set_more(on) -- expanded / compact with the matching width (settings.rs at start-up)
pub(crate) fn wcol_set_more(p: &mut Pn, on: bool) {
    p.wcol.more = on;
    let mut r = p.wcol.rect.get();
    r.w = if on { WC_W_MORE } else { WC_W_LESS };
    p.wcol.rect.set(r);
    p.repass();
}
