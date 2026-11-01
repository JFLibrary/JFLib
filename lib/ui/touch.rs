// touch -- the finger: one gesture recogniser for every host
//
//   touch_takes(ev) -> a raw finger event (pointer data PTR_TOUCH, process_events)
//   touch_input(ev)                 the raw finger: the recogniser's state, events for the widgets
//   touch_tick()                    the clock: the long press, the hover of a resting finger
//   touch_apply()                   run_passes: what the finger and the fling moved, into the views
//   pan_view(...) / pan_take(id)    a view with its own position rolls by finger (vlist, md_view,
//                                   text views, the console)
//   touch_mode() -> the pointer is a finger (no scroll bars)   touch_drag()
//   touch_take(TF_*) a widget takes the finger or declines it   touch_fingers() -> what is down
//   touch_two() -> two fingers on the last widget (TF_TWO): their midpoint and scale (pinch, pan)
//   touch_hold_part(id) -> how far the press and hold on a title bar is (TF_HOLD_LONG)
//
// A host only forwards raw fingers: plat::finger(win, phase, id, x, y, age_ms), or jfgui_finger where
// the windows live in one surface (the Android host: GalleryActivity.java, bridge.c; the browser's
// pointer events: web/loader.js); scripts: swipe, flick, tap, finger, `pointer touch` (auto.rs).  Only
// this file sees them raw.  It knows no platform: all of them behave the same and scripts test it on
// the virtual clock.
//
// Every finger is in a table (touch_fingers).  The first one down drives the gesture.  A second one
// takes nothing from it: while nothing is decided it is no tap any more (no click, no long press);
// when the first one lifts, a pan goes on with the other one, a press ends, the rest waits for the
// last finger to lift.  A widget with TF_TWO gets the second finger for itself: no press (one younger
// than 200 ms is taken back: touch_two says so), and a pass whenever a finger moves, for its own pan and
// zoom.
//
// A pen comes first: while it is down, hovers or left less than 500 ms ago a finger is a palm and does
// not exist (no finger mode either); a pen that comes down ends the fingers already on the screen.
//
// A finger is wider than what it means: on the background (a title bar, an empty bar, no hit box at
// all) the nearest small widget wins whose rectangle, grown to 44 DIP each way around its middle, holds
// the point (touch_aim).  An exact hit stays what it is; a mouse sees none of this and nothing is drawn
// differently.  A tap is a click wherever the finger lifts within the slop.
//
// The first decision waits until the finger left the slop (ViewConfiguration: 8 DIP) or lifted:
//   - lifted: a tap, press and release at once (no flash when a scroll starts instead);
//   - the widget under the down point takes this direction (grabs: a slider its axis, a colour
//     wheel, the handle of a row, a window's title bar every direction -- those get the press at
//     the touch itself, no slop, unless TF_SLOP or TF_HOLD): press and moves go to the widget like
//     a mouse's;
//   - a scroll view around the down point can move along the dominant axis: the pan.  The chain
//     of views is fixed there (from the hit boxes of the last pass), innermost first: each moves
//     as far as it can, the rest goes to the next.  The axis is locked (a view that scrolls both
//     ways stays free unless the finger starts along an axis: rails);
//   - nothing to roll: press and moves go to the widget after all (TF_DECLINE, TF_HOLD: nothing).
// The pan moves the positions with no smoothing, by the finger's pixels; the slop's travel is
// caught up on the next 16 DIP (half a pixel per finger pixel) or when it lifts: no jump, nothing
// lost.  Lifted
// with speed (least squares over the last 100 ms, as Android's VelocityTracker; 50..8000 DIP/s)
// the fling runs along OverScroller's curve; it hands over along the chain, a new touch stops it
// and is no click.  A finger that rests: after 100 ms it hovers (the widget shows it is meant, a
// tooltip follows), after 400 ms the right button (a context menu; plat::touch_long_press lets the
// host give feedback; TF_HOLD: the left button, the drag starts, TF_HOLD_LONG after 800 ms; TF_REST:
// the left button, no context menu: a canvas); if it moves after that, the same decision.  After the
// finger left, the hover goes.
//
// With a finger (and at the start on Android, and in a phone's browser) there are no scroll bars:
// no pixels and no hit box in sbar_core, the room they would have stays, so nothing jumps; a mouse
// brings them back.  ui.touch_drag() marks a rectangle whose hit box is someone else's (the handle
// inside a list row); widgets that need the finger say so themselves (HB_GRAB_*, the shape of their
// cursor, touch_grab).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use super::consts::*;
use super::ctx::{Hit, W};
use super::layout::px2dip_at;
use super::Ui;
use plat::Event;

/// the axes of a grab or a view
pub(crate) const AX_V: u8 = TF_V as u8;
pub(crate) const AX_H: u8 = TF_H as u8;
pub(crate) const AX_ALL: u8 = TF_BOTH as u8;
const G_HOLD: u8 = TF_HOLD as u8;
const G_TWO: u8 = TF_TWO as u8;
const G_SLOP: u8 = TF_SLOP as u8;
const G_DECLINE: u8 = TF_DECLINE as u8;
const G_REST: u8 = TF_REST as u8;
const G_HOLD_LONG: u8 = TF_HOLD_LONG as u8;

// ViewConfiguration's defaults (GalleryActivity overrides them with the device's: jfgui_TOUCH)
const SLOP_DIP: i32 = 8;
const LONG_PRESS_MS: u32 = 400;
const FLING_MIN_DIP: i32 = 50;
const FLING_MAX_DIP: i32 = 8000;
/// the resting finger shows on the widget (ViewConfiguration's tap timeout)
const HOVER_MS: u32 = 100;
/// after the finger left the hover stays this long: a quick tap shows on the widget too
const LEAVE_MS: u32 = 120;
const TIP_LEAVE_MS: u32 = 1500;
/// the velocity: samples of the last 100 ms; a finger that rested this long before it lifted has
/// none (VelocityTracker's horizon and ASSUME_POINTER_STOPPED_TIME)
const VEL_HORIZON_MS: u32 = 100;
const VEL_STOPPED_MS: u32 = 40;
const SAMPLES: usize = 20;
/// OverScroller's fling: ViewConfiguration.getScrollFriction, the deceleration ln 0.78 / ln 0.9, the
/// inflexion, g in inches, its physical coefficient's 0.84 (ppi is scale * 160)
const FRICTION: f64 = 0.015;
const DECEL: f64 = 2.358_201_815_154_572;
const INFLEXION: f64 = 0.35;
const G_INCH: f64 = 9.80665 * 39.37 * 0.84;
/// with a finger a window's title bar is at least this high, its close button this wide (DIP); not in
/// a window narrower than CAPTION_MIN_W: the bar has to stay something to hold on to
pub(crate) const TITLE_DIP: i32 = 40;
pub(crate) const CAPTION_DIP: i32 = 44;
pub(crate) const CAPTION_MIN_W: i32 = 120;
/// where a cancelled press is released: outside everything, so it cannot click
const OUTSIDE: i32 = -10000;
/// TF_HOLD_LONG: a panel's title bar moves after resting this long; nothing shows before HOLD_SHOW_MS
/// (a tap shows nothing)
const HOLD_LONG_MS: u32 = 800;
const HOLD_SHOW_MS: u32 = 120;
/// a finger within this time after the pen's last sign of life is a palm
const PALM_MS: u32 = 500;
/// a second finger takes a press of a TF_TWO widget back that is younger than this
const TAKE_BACK_MS: u32 = 200;
/// a small widget's rectangle for a finger (touch_aim)
const TARGET_DIP: i32 = 44;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Idle,
    /// down, nothing decided
    Wait,
    Pan,
    /// the press and the moves go to the widget
    Widget,
    /// ignored until it lifts (a long press opened something under it)
    Gone,
}

/// a view the finger can roll, in the pass that built it: its hit boxes [hit0, hit1) of its layer
#[derive(Clone, Copy)]
pub(crate) struct PanView {
    id: u32,
    hit0: u32,
    hit1: u32,
    layer: u8,
    /// the widget keeps the position itself (pan_take), else a scroll view's slot
    own: bool,
    axes: u8,
    /// x, y (px)
    pos: [i32; 2],
    max: [i32; 2],
}

#[derive(Clone, Copy, Default)]
struct Fling {
    t0: u32,
    /// duration in ms, the way (px, in the finger's direction), what of it was delivered
    dur: f64,
    dist: [f64; 2],
    done: [f64; 2],
}

/// a finger that is down (ui.touch_fingers): px of the window, logical in a mirrored one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finger {
    /// the host's number for it, while it is down
    pub id: u8,
    pub x: i32,
    pub y: i32,
    /// where it came down
    pub down_x: i32,
    pub down_y: i32,
    /// its last sample (ms, as Ui::now)
    pub time_ms: u32,
    win: u8,
}

/// two fingers on a widget that said TF_TWO (ui.touch_two): px of the window, logical in a mirrored one
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoFingers {
    /// the point between them
    pub x: i32,
    pub y: i32,
    /// where it was when this pair came down
    pub start_x: i32,
    pub start_y: i32,
    /// their distance against the one at the start (1.0 = the same)
    pub scale: f64,
    /// the first read of this pair: take the anchor now
    pub began: bool,
    /// the first finger's press went out less than 200 ms ago and was taken back for this pair: its
    /// release comes through as usual, undo what the press began
    pub taken_back: bool,
}

// the pair of fingers ui.touch_two reports
#[derive(Clone, Copy)]
struct Two {
    ids: [u8; 2],
    start: [i32; 2],
    dist: f64,
}

/// shared state (Ui::m.touch)
pub(crate) struct S {
    /// the pointer is a finger
    pub(crate) on: bool,
    mode: Mode,
    /// the fingers down in the order they came, the one that drives the gesture
    fingers: Vec<Finger>,
    primary: u8,
    /// what the widget under the down point takes (TF_*)
    grab: u8,
    win: u32,
    /// raw px of the window (before the mirror of a right-to-left window)
    down: [i32; 2],
    cur: [i32; 2],
    /// what the finger means minus where it is (touch_aim): the widget sees that much further
    off: [i32; 2],
    down_t: u32,
    /// when the press went out
    press_t: u32,
    /// the widget under the down point: its id (TF_HOLD_LONG shows on it, and the press finds it again)
    down_id: u32,
    last_t: u32,
    /// the down only stopped a fling: lifted without a move it does nothing
    no_click: bool,
    hovered: bool,
    long_done: bool,
    /// the widget under the finger when the long press fired
    long_id: u32,
    data: u32,
    mods: u8,
    leave_at: Option<u32>,
    axes: u8,
    /// inner to outer
    chain: Vec<u32>,
    /// the finger's way since the last frame (px)
    pend: [i32; 2],
    /// the scroll bars come or go
    repaint: bool,
    /// the slop's way to catch up (px, signed)
    catchup: [i32; 2],
    samples: [(u32, i32, i32); SAMPLES],
    n: usize,
    fling: Option<Fling>,
    /// what the widgets get next, before the platform's next event
    out: VecDeque<Event>,
    /// this frame's way for the views with their own position
    take: Vec<(u32, i32, i32)>,
    spline: Vec<f64>,
    /// slop, min and max velocity (DIP, DIP/s), long press (ms): the device's or the defaults
    conf: [i32; 4],
    conf_read: bool,
    /// (a script) the place of the window being moved by a finger when the press came
    pub(crate) win_base: [i32; 2],
    /// the pair ui.touch_two reports, a press taken back for it
    two: Option<Two>,
    took_back: bool,
    /// the pen: down, its last sign of life (ms)
    pen_down: bool,
    pen_t: Option<u32>,
    /// a finger's press is on its way (a text field may get the focus from it); the on-screen
    /// keyboard is up (asked for), and in which UI window
    kb_ask: bool,
    pub(crate) kb_on: bool,
    kb_win: u32,
}

impl Default for S {
    fn default() -> Self {
        S {
            on: plat::touch_primary(),
            mode: Mode::Idle,
            fingers: Vec::new(),
            primary: 0,
            grab: 0,
            win: 0,
            down: [0; 2],
            cur: [0; 2],
            off: [0; 2],
            down_t: 0,
            press_t: 0,
            down_id: 0,
            last_t: 0,
            no_click: false,
            hovered: false,
            long_done: false,
            long_id: 0,
            data: 0,
            mods: 0,
            leave_at: None,
            axes: 0,
            chain: Vec::new(),
            pend: [0; 2],
            repaint: false,
            catchup: [0; 2],
            samples: [(0, 0, 0); SAMPLES],
            n: 0,
            fling: None,
            out: VecDeque::new(),
            take: Vec::new(),
            spline: Vec::new(),
            conf: [SLOP_DIP, FLING_MIN_DIP, FLING_MAX_DIP, LONG_PRESS_MS as i32],
            conf_read: false,
            win_base: [0; 2],
            two: None,
            took_back: false,
            pen_down: false,
            pen_t: None,
            kb_ask: false,
            kb_on: false,
            kb_win: 0,
        }
    }
}

/// the grabs a hit box says by itself: its HB_GRAB_* bits and the shape of its cursor
pub(crate) fn hit_grab(hbflags: u32) -> u8 {
    let mut g = 0;
    if hbflags & HB_GRAB_H != 0 {
        g |= AX_H;
    }
    if hbflags & HB_GRAB_V != 0 {
        g |= AX_V;
    }
    if hbflags & HB_DRAG != 0 {
        g |= AX_ALL;
    }
    g | match (hbflags >> HB_CURSOR_SHIFT) & 15 {
        CUR_SIZE_WE | CUR_IBEAM => AX_H,
        CUR_SIZE_NS => AX_V,
        CUR_SIZE_NWSE | CUR_SIZE_NESW | CUR_MOVE | CUR_CROSS => AX_ALL,
        _ => 0,
    }
}

// the topmost hit box at a point of the last pass (hit_find_list's order): its index
fn hit_index(list: &[Hit], modal: u32, x: i32, y: i32) -> Option<usize> {
    let mut layer = super::consts::L_COUNT as i32 - 1;
    while layer >= modal as i32 {
        for (i, h) in list.iter().enumerate().rev() {
            if h.layer as i32 == layer && x >= h.x0 && x < h.x1 && y >= h.y0 && y < h.y1 {
                return Some(i);
            }
        }
        layer -= 1;
    }
    None
}

// OverScroller's curve: the part of the way done after each hundredth of the time
fn spline_table() -> Vec<f64> {
    const P1: f64 = 0.5 * INFLEXION;
    const P2: f64 = 1.0 - (1.0 - INFLEXION);
    let mut table = Vec::with_capacity(101);
    let mut x_min = 0.0f64;
    for i in 0..100 {
        let alpha = i as f64 / 100.0;
        let mut x_max = 1.0f64;
        let (mut x, mut coef);
        loop {
            x = x_min + (x_max - x_min) / 2.0;
            coef = 3.0 * x * (1.0 - x);
            let tx = coef * ((1.0 - x) * P1 + x * P2) + x * x * x;
            let d = tx - alpha;
            if (if d < 0.0 { -d } else { d }) < 1e-5 {
                break;
            }
            if tx > alpha {
                x_max = x;
            } else {
                x_min = x;
            }
        }
        table.push(coef * ((1.0 - x) * 0.5 + x) + x * x * x);
    }
    table.push(1.0);
    table
}

fn sqrt(v: f64) -> f64 {
    rt::x87::pow(v, 0.5)
}

impl Ui {
    /// ui_touch_mode: the pointer is a finger (the last pointer event came from one; on Android and
    /// in a phone's browser from the start): the library shows no scroll bars.  A mouse ends it
    pub fn touch_mode(&self) -> bool {
        self.m.touch.on
    }

    /// ui_touch_drag: a finger that starts on the last widget's rectangle drags in every direction
    /// instead of rolling what is around it: for a part of a widget that has no hit box of its own
    /// (the handle of a list's row).  Sliders, colour wheels, splitters and the like take the finger
    /// by themselves.  Call it after the widget, every frame
    pub fn touch_drag(&mut self) {
        let c = &mut self.wp.ctx;
        let [x, y, w, h] = c.last_rect;
        let k = c.uiclip;
        let r = [x.max(k[0]), y.max(k[1]), (x + w).min(k[2]), (y + h).min(k[3])];
        let cur = c.hit_cur;
        if r[2] > r[0] && r[3] > r[1] && c.grabs[cur].len() < 256 {
            c.grabs[cur].push(r);
        }
    }

    /// ui_touch_take(flags): what the last widget (all hit boxes in its rectangle) wants of a finger,
    /// TF_*, instead of what its hit boxes say by themselves.  Call it after the widget, every frame
    pub fn touch_take(&mut self, flags: u32) {
        let c = &mut self.wp.ctx;
        let [x, y, w, h] = c.last_rect;
        let k = c.uiclip;
        let r = [x.max(k[0]), y.max(k[1]), (x + w).min(k[2]), (y + h).min(k[3])];
        let cur = c.hit_cur;
        for (i, hit) in c.hits[cur].iter_mut().rev().enumerate() {
            if i != 0 && !(hit.x0 >= r[0] && hit.y0 >= r[1] && hit.x1 <= r[2] && hit.y1 <= r[3]) {
                break;
            }
            hit.grab = flags as u8;
        }
    }

    /// ui_touch_fingers: the fingers down on the live window, the first one first (a widget with
    /// TF_TWO gets a pass whenever one moves)
    pub fn touch_fingers(&self) -> Vec<Finger> {
        let win = self.uiw_cur as u8;
        let fb = self.wp.rt.fb.w.wrapping_sub(1);
        let rtl = self.rtl;
        let m = |x: i32| if rtl { fb.wrapping_sub(x) } else { x };
        self.m.touch.fingers.iter().filter(|f| f.win == win).map(|f| Finger { x: m(f.x), down_x: m(f.down_x), ..*f }).collect()
    }

    /// ui_touch_two: two fingers on the last widget (it said TF_TWO; call it after the widget, every
    /// frame): where they are and how far apart.  None while the first finger drags the widget: a
    /// stroke older than 200 ms goes on, the second finger waits
    pub fn touch_two(&mut self) -> Option<TwoFingers> {
        let win = self.uiw_cur;
        let [x0, y0, w, h] = self.wp.ctx.last_rect;
        let s = &self.m.touch;
        if s.grab & G_TWO == 0 || s.win != win || s.mode == Mode::Widget {
            return None;
        }
        let mut it = s.fingers.iter().filter(|f| f.win as u32 == win);
        let (a, b) = (*it.next()?, *it.next()?);
        // (the first one came down on this widget)
        let d = self.touch_logical([a.down_x, a.down_y]);
        if d[0] < x0 || d[1] < y0 || d[0] >= x0.wrapping_add(w) || d[1] >= y0.wrapping_add(h) {
            return None;
        }
        let m = self.touch_logical([a.x.wrapping_add(b.x) >> 1, a.y.wrapping_add(b.y) >> 1]);
        let (dx, dy) = (a.x.wrapping_sub(b.x) as f64, a.y.wrapping_sub(b.y) as f64);
        let dist = sqrt(dx * dx + dy * dy).max(1.0);
        let s = &mut self.m.touch;
        let began = s.two.is_none_or(|t| t.ids != [a.id, b.id]);
        if began {
            s.two = Some(Two { ids: [a.id, b.id], start: m, dist });
        }
        let t = s.two?;
        let taken_back = began && core::mem::take(&mut s.took_back);
        Some(TwoFingers { x: m[0], y: m[1], start_x: t.start[0], start_y: t.start[1], scale: dist / t.dist, began, taken_back })
    }

    /// ui_touch_hold_part(id) -> how far a finger's press and hold on widget id of this window is, in
    /// 1/1000 (TF_HOLD_LONG): None without one, 1000 once the drag runs.  Show it: a second is long
    /// without a sign
    pub fn touch_hold_part(&self, id: u32) -> Option<u32> {
        let s = &self.m.touch;
        if s.grab & G_HOLD_LONG == 0 || s.win != self.uiw_cur || s.down_id != id || id == 0 {
            return None;
        }
        match s.mode {
            Mode::Wait if !s.no_click => {
                let held = self.now_ms.wrapping_sub(s.down_t);
                (held >= HOLD_SHOW_MS).then(|| ((held - HOLD_SHOW_MS) * 1000 / (HOLD_LONG_MS - HOLD_SHOW_MS)).min(1000))
            }
            Mode::Widget => Some(1000),
            _ => None,
        }
    }

    /// the widget that just called behave takes the finger along these axes (AX_*)
    pub(crate) fn touch_grab(&mut self, axes: u8) {
        let c = &mut self.wp.ctx;
        let cur = c.hit_cur;
        if let Some(h) = c.hits[cur].last_mut() {
            h.grab |= axes;
        }
    }

    /// a view with its own position, at the end of its hit boxes (from hit0): the finger may roll it
    /// (AX_* along axes; x, y px).  Read what it moved with pan_take before drawing
    pub(crate) fn pan_view(&mut self, id: u32, hit0: usize, axes: u8, pos: [i32; 2], max: [i32; 2]) {
        let layer = self.wp.ctx.cmd_layer_ui as u8;
        self.pan_push(id, hit0, layer, true, axes, pos, max);
    }

    // one more view of this pass
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pan_push(&mut self, id: u32, hit0: usize, layer: u8, own: bool, axes: u8, pos: [i32; 2], max: [i32; 2]) {
        let c = &mut self.wp.ctx;
        let cur = c.hit_cur;
        if c.pans[cur].len() < 64 {
            let hit1 = c.hits[cur].len() as u32;
            c.pans[cur].push(PanView { id, hit0: hit0 as u32, hit1, layer, own, axes, pos, max });
        }
    }

    /// pan_take(id) -> (dx, dy): how far the view's position moves in this frame (px)
    pub(crate) fn pan_take(&mut self, id: u32) -> (i32, i32) {
        let take = &mut self.m.touch.take;
        match take.iter().position(|t| t.0 == id) {
            Some(i) => {
                let t = take.swap_remove(i);
                (t.1, t.2)
            }
            None => (0, 0),
        }
    }

    // the device's numbers (GalleryActivity: jfgui_TOUCH = slop, min velocity, max velocity in DIP,
    // long press in ms)
    fn touch_conf(&mut self) {
        if core::mem::replace(&mut self.m.touch.conf_read, true) {
            return;
        }
        let Some(v) = plat::env("jfgui_TOUCH") else { return };
        let mut r = v.as_bytes();
        for k in 0..4 {
            let (n, rest) = super::runtime::parse_int(r);
            if n > 0 {
                self.m.touch.conf[k] = n;
            }
            r = rest;
        }
    }

    /// a raw finger event of the platform or a script
    pub(crate) fn touch_takes(&self, ev: &Event) -> bool {
        matches!(ev.typ, plat::E_MOVE | plat::E_DOWN | plat::E_UP) && (ev.data >> plat::PD_TYPE_SHIFT) & 3 == PTR_TOUCH
    }

    /// process_events: what the pointer is, from every raw pointer event
    pub(crate) fn touch_kind(&mut self, ev: &Event) {
        let kind = (ev.data >> plat::PD_TYPE_SHIFT) & 3;
        if matches!(ev.typ, plat::E_MOVE | plat::E_DOWN | plat::E_UP) && kind != PTR_TOUCH {
            self.touch_pen(ev, kind);
        }
        if !matches!(ev.typ, plat::E_MOVE | plat::E_DOWN) {
            return;
        }
        let finger = kind == PTR_TOUCH;
        if finger && self.touch_palm() {
            return; // (no finger mode for a palm)
        }
        if self.m.touch.on != finger {
            self.touch_mode_set(finger);
        }
        if !finger {
            self.m.touch.fling = None; // a mouse took over
        }
    }

    // the pen shows itself (a press ends the fingers: they are the hand that holds it)
    fn touch_pen(&mut self, ev: &Event, kind: u32) {
        let now = self.now_ms;
        let s = &mut self.m.touch;
        if kind == PTR_MOUSE {
            if ev.typ == plat::E_UP {
                s.pen_down = false; // (the pen's release after a lost capture has no pen data)
            }
            return;
        }
        s.pen_t = Some(now);
        match ev.typ {
            plat::E_DOWN => {
                s.pen_down = true;
                self.touch_end_all();
            }
            plat::E_UP => s.pen_down = false,
            _ => {}
        }
    }

    // a finger now would be a palm
    fn touch_palm(&self) -> bool {
        let s = &self.m.touch;
        s.pen_down || s.pen_t.is_some_and(|t| self.now_ms.wrapping_sub(t) < PALM_MS)
    }

    // the fingers on the screen do not count any more: a press is released outside everything, no
    // click, no fling; what they do from now on is nobody's (their ids are not in the table)
    fn touch_end_all(&mut self) {
        let s = &mut self.m.touch;
        if s.fingers.is_empty() {
            return;
        }
        s.fingers.clear();
        s.two = None;
        s.took_back = false;
        s.repaint = true;
        let mode = core::mem::replace(&mut s.mode, Mode::Idle);
        if mode == Mode::Widget {
            self.touch_out(plat::E_UP, BTN_LEFT as u8, OUTSIDE, OUTSIDE);
        }
        if self.m.touch.hovered {
            self.m.touch.leave_at = Some(self.now_ms.wrapping_add(LEAVE_MS));
        }
    }

    /// a finger or a mouse from now on: the scroll bars come or go in the next frame
    pub(crate) fn touch_mode_set(&mut self, finger: bool) {
        self.m.touch.on = finger;
        self.m.touch.repaint = true;
    }

    // the next event for the widgets from the recogniser
    pub(crate) fn touch_next(&mut self) -> Option<Event> {
        self.m.touch.out.pop_front()
    }

    // an event for the widgets
    fn touch_out(&mut self, typ: u8, btn: u8, x: i32, y: i32) {
        let t = &mut self.m.touch;
        t.kb_ask |= typ == plat::E_DOWN;
        let data = (t.data & plat::PD_PRESSURE) | PTR_TOUCH << plat::PD_TYPE_SHIFT;
        let (win, mods) = (t.win as u8, t.mods);
        t.out.push_back(Event { typ, btn, mods, win, x, y, data });
    }

    // the sample's time on the UI's clock: as old as the host said when it pushed it
    fn touch_time(&self, ev: &Event) -> u32 {
        self.now_ms.wrapping_sub((ev.data >> plat::PD_AGE_SHIFT) & plat::PD_AGE_MASK)
    }

    fn touch_sample(&mut self, t: u32, x: i32, y: i32) {
        let s = &mut self.m.touch;
        if s.n == SAMPLES {
            s.samples.copy_within(1.., 0);
            s.n -= 1;
        }
        s.samples[s.n] = (t, x, y);
        s.n += 1;
    }

    // the point in the window's own coordinates (a mirrored window: logical)
    fn touch_logical(&self, p: [i32; 2]) -> [i32; 2] {
        if self.rtl { [self.wp.rt.fb.w.wrapping_sub(1).wrapping_sub(p[0]), p[1]] } else { p }
    }

    /// the raw finger: never reaches a widget as it is
    pub(crate) fn touch_input(&mut self, ev: &Event) {
        self.touch_conf();
        let t = self.touch_time(ev);
        let id = ev.btn;
        let s = &mut self.m.touch;
        match ev.typ {
            plat::E_DOWN => {
                s.fingers.retain(|f| f.id != id); // (it never lifted)
                s.two = None;
                if self.touch_palm() {
                    return; // never in the table: its moves and its lift are nobody's
                }
                let s = &mut self.m.touch;
                let first = s.fingers.is_empty();
                s.fingers.push(Finger { id, x: ev.x, y: ev.y, down_x: ev.x, down_y: ev.y, time_ms: t, win: ev.win });
                if first {
                    s.primary = id;
                    self.touch_down(ev, t);
                } else {
                    self.touch_second(ev, t);
                }
            }
            plat::E_MOVE => {
                let Some(f) = s.fingers.iter_mut().find(|f| f.id == id) else { return };
                f.x = ev.x;
                f.y = ev.y;
                f.time_ms = t;
                if s.grab & G_TWO != 0 {
                    s.repaint = true;
                }
                if id == s.primary && s.mode != Mode::Idle && ev.win as u32 == s.win {
                    self.touch_move(ev, t);
                }
            }
            _ => {
                let Some(k) = s.fingers.iter().position(|f| f.id == id) else { return };
                s.fingers.remove(k);
                s.two = None;
                if s.grab & G_TWO != 0 {
                    s.repaint = true;
                }
                if id != s.primary {
                    return;
                }
                match s.fingers.first().copied() {
                    Some(next) => self.touch_handover(ev, next),
                    None if s.mode != Mode::Idle => self.touch_up(ev, t),
                    None => {}
                }
            }
        }
    }

    // a second finger came down
    fn touch_second(&mut self, ev: &Event, t: u32) {
        let s = &mut self.m.touch;
        if s.grab & G_TWO != 0 && ev.win as u32 == s.win {
            s.repaint = true;
            match s.mode {
                Mode::Wait => s.mode = Mode::Gone, // the two are the widget's: no press, no click
                Mode::Widget if t.wrapping_sub(s.press_t) < TAKE_BACK_MS => {
                    // the first finger had only just left the slop: that was the start of a
                    // two-finger gesture
                    s.mode = Mode::Gone;
                    s.took_back = true;
                    let [x, y] = [s.cur[0].wrapping_add(s.off[0]), s.cur[1].wrapping_add(s.off[1])];
                    self.touch_out(plat::E_UP, BTN_LEFT as u8, x, y);
                }
                _ => {}
            }
        } else if s.mode == Mode::Wait {
            s.no_click = true; // two fingers are no tap and no long press
        }
    }

    // the first finger lifted, another one is still down
    fn touch_handover(&mut self, ev: &Event, next: Finger) {
        let cancel = ev.data & plat::PD_CANCEL != 0;
        let s = &mut self.m.touch;
        s.primary = next.id;
        match s.mode {
            Mode::Pan => {
                // the pan goes on with the other one, from where it is
                s.cur = [next.x, next.y];
                s.n = 0;
                self.touch_sample(next.time_ms, next.x, next.y);
            }
            Mode::Widget => {
                s.mode = Mode::Gone;
                let [ax, ay] = self.touch_aimed([ev.x, ev.y]);
                let (x, y) = if cancel { (OUTSIDE, OUTSIDE) } else { (ax, ay) };
                self.touch_out(plat::E_UP, BTN_LEFT as u8, x, y);
            }
            Mode::Wait => s.mode = Mode::Gone,
            _ => {}
        }
    }

    fn touch_down(&mut self, ev: &Event, t: u32) {
        let win = ev.win as u32;
        if win as usize >= WIN_MAX || (win != 0 && !self.uiw_drawable(win)) {
            return;
        }
        // (the last finger never lifted: its press ends here)
        if self.m.touch.mode == Mode::Widget {
            self.touch_out(plat::E_UP, BTN_LEFT as u8, OUTSIDE, OUTSIDE);
        }
        let stopped = self.m.touch.fling.take().is_some();
        let s = &mut self.m.touch;
        s.mode = Mode::Wait;
        s.grab = 0;
        s.win = win;
        s.down = [ev.x, ev.y];
        s.cur = s.down;
        s.off = [0; 2];
        s.took_back = false;
        s.down_t = t;
        s.last_t = t;
        s.no_click = stopped;
        s.hovered = false;
        s.long_done = false;
        s.data = ev.data;
        s.mods = ev.mods;
        s.leave_at = None;
        s.pend = [0; 2];
        s.catchup = [0; 2];
        s.n = 0;
        self.touch_sample(t, ev.x, ev.y);
        plat::touch_reached(false);
        self.uiw_use(win);
        self.touch_aim();
        let (hit, grab) = self.touch_under();
        let c = &self.wp.ctx;
        self.m.touch.down_id = hit.map_or(0, |i| c.hits[c.hit_cur ^ 1][i].id);
        self.m.touch.grab = grab;
        if stopped {
            return;
        }
        // a widget that takes every direction has the finger from the start
        if grab & AX_ALL == AX_ALL && grab & (G_HOLD | G_SLOP | G_DECLINE) == 0 {
            self.touch_press();
        }
    }

    // what the finger means, in the live window: on the background the nearest small widget whose
    // rectangle, grown to the target size around its middle, holds the point.  Only one that is on top
    // where the finger would touch it, and an exact hit is left alone
    fn touch_aim(&mut self) {
        if self.uiw_inert_now() {
            return;
        }
        let t = self.dp(TARGET_DIP);
        let c = &self.wp.ctx;
        let p = self.touch_logical(self.m.touch.down);
        let list = &c.hits[c.hit_cur ^ 1];
        // (a disabled button is what it hit, not its neighbour)
        let real = |h: &Hit| h.id != 0 && h.flags as u32 & (HB_BLOCK | HB_DRAG) == 0;
        // (grips and splitters have their own width for a finger, and a title bar next to them is one)
        let grip = |h: &Hit| matches!((h.flags as u32 >> HB_CURSOR_SHIFT) & 15, CUR_SIZE_WE | CUR_SIZE_NS | CUR_SIZE_NWSE | CUR_SIZE_NESW);
        let usable = |h: &Hit| real(h) && h.flags as u32 & HB_DISABLED == 0 && !grip(h);
        let exact = hit_index(list, c.modal_prev, p[0], p[1]);
        if exact.is_some_and(|i| real(&list[i])) {
            return;
        }
        let floor = exact.map_or(c.modal_prev as i32, |i| list[i].layer as i32).max(c.modal_prev as i32);
        let mut best: Option<(i32, [i32; 2])> = None;
        for (i, h) in list.iter().enumerate() {
            if !usable(h) || (h.layer as i32) < floor {
                continue;
            }
            let (ex, ey) = (((t - (h.x1 - h.x0)) / 2).max(0), ((t - (h.y1 - h.y0)) / 2).max(0));
            if ex == 0 && ey == 0 || p[0] < h.x0 - ex || p[0] >= h.x1 + ex || p[1] < h.y0 - ey || p[1] >= h.y1 + ey {
                continue;
            }
            // the nearest point of the widget
            let q = [p[0].clamp(h.x0, h.x1 - 1), p[1].clamp(h.y0, h.y1 - 1)];
            let d = (q[0] - p[0]).abs().max((q[1] - p[1]).abs());
            if best.is_none_or(|b| d < b.0) && hit_index(list, c.modal_prev, q[0], q[1]) == Some(i) {
                best = Some((d, q));
            }
        }
        if let Some((_, q)) = best {
            let raw = self.touch_logical(q); // (its own inverse)
            let s = &mut self.m.touch;
            s.off = [raw[0] - s.down[0], raw[1] - s.down[1]];
        }
    }

    // the press is about to go out: the layout may have moved since the finger came down (the first
    // finger of a session: a tool window's title bar grows, what is below it moves), and what the finger
    // came down on is what it means
    fn touch_find_again(&mut self) {
        let id = self.m.touch.down_id;
        if id == 0 || self.uiw_inert_now() {
            return;
        }
        let c = &self.wp.ctx;
        let list = &c.hits[c.hit_cur ^ 1];
        let p = self.touch_logical(self.touch_aimed(self.m.touch.down));
        if hit_index(list, c.modal_prev, p[0], p[1]).is_some_and(|i| list[i].id == id) {
            return;
        }
        let Some(h) = list.iter().rev().find(|h| h.id == id && h.x1 > h.x0 && h.y1 > h.y0) else { return };
        let q = [p[0].clamp(h.x0, h.x1 - 1), p[1].clamp(h.y0, h.y1 - 1)];
        if hit_index(list, c.modal_prev, q[0], q[1]).is_some_and(|i| list[i].id == id) {
            let raw = self.touch_logical(q);
            let s = &mut self.m.touch;
            s.off = [raw[0] - s.down[0], raw[1] - s.down[1]];
        }
    }

    // where the widget sees the finger now
    fn touch_aimed(&self, p: [i32; 2]) -> [i32; 2] {
        let o = self.m.touch.off;
        [p[0].wrapping_add(o[0]), p[1].wrapping_add(o[1])]
    }

    // the widget under the down point: (its hit box, the axes it takes)
    fn touch_under(&self) -> (Option<usize>, u8) {
        if self.uiw_inert_now() {
            return (None, 0);
        }
        let c = &self.wp.ctx;
        let p = self.touch_logical(self.touch_aimed(self.m.touch.down));
        let list = &c.hits[c.hit_cur ^ 1];
        let i = hit_index(list, c.modal_prev, p[0], p[1]);
        let mut grab = i.map_or(0, |i| if list[i].flags as u32 & HB_DISABLED != 0 { 0 } else { list[i].grab });
        if c.grabs[c.hit_cur ^ 1].iter().any(|r| p[0] >= r[0] && p[1] >= r[1] && p[0] < r[2] && p[1] < r[3]) {
            grab = AX_ALL;
        }
        (i, grab)
    }

    // the press at the down point: from here on the widget has the finger
    fn touch_press(&mut self) {
        self.uiw_use(self.m.touch.win);
        self.touch_find_again();
        let [x, y] = self.touch_aimed(self.m.touch.down);
        let now = self.now_ms;
        let s = &mut self.m.touch;
        s.mode = Mode::Widget;
        s.press_t = now;
        self.touch_out(plat::E_MOVE, 0, x, y);
        self.touch_out(plat::E_DOWN, BTN_LEFT as u8, x, y);
        plat::touch_reached(true);
    }

    fn touch_move(&mut self, ev: &Event, t: u32) {
        let (ox, oy) = (self.m.touch.cur[0], self.m.touch.cur[1]);
        self.m.touch.cur = [ev.x, ev.y];
        self.m.touch.last_t = t;
        self.m.touch.data = ev.data;
        self.touch_sample(t, ev.x, ev.y);
        match self.m.touch.mode {
            Mode::Wait => {
                let s = &self.m.touch;
                let (dx, dy) = (ev.x.wrapping_sub(s.down[0]), ev.y.wrapping_sub(s.down[1]));
                let slop = self.dp(s.conf[0]);
                if (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64) > (slop as i64) * (slop as i64) {
                    self.uiw_use(self.m.touch.win);
                    self.touch_decide(dx, dy);
                    self.touch_after_decide();
                }
            }
            Mode::Pan => self.touch_pan(ev.x.wrapping_sub(ox), ev.y.wrapping_sub(oy)),
            Mode::Widget => {
                let [x, y] = self.touch_aimed([ev.x, ev.y]);
                self.touch_out(plat::E_MOVE, 0, x, y);
            }
            _ => {}
        }
    }

    // after the decision: the widget sees where the finger is by now
    fn touch_after_decide(&mut self) {
        if self.m.touch.mode == Mode::Widget {
            let [x, y] = self.touch_aimed(self.m.touch.cur);
            self.touch_out(plat::E_MOVE, 0, x, y);
        }
    }

    // the finger left the slop by (dx, dy)
    fn touch_decide(&mut self, dx: i32, dy: i32) {
        let (hit, grab) = self.touch_under();
        let s = &self.m.touch;
        if s.long_done {
            // the long press changed what is under the finger (a menu opened): the rest is not
            // for that
            let c = &self.wp.ctx;
            let id = hit.map_or(0, |i| c.hits[c.hit_cur ^ 1][i].id);
            if id != s.long_id {
                self.m.touch.mode = Mode::Gone;
                return;
            }
        }
        let (adx, ady) = (dx.wrapping_abs(), dy.wrapping_abs());
        let axis = if adx > ady { AX_H } else { AX_V };
        let waits = grab & (G_HOLD | G_DECLINE) != 0; // no drag now: what is around it, or nothing
        if grab & axis != 0 && !waits {
            self.touch_press();
            return;
        }
        // the views around the widget, inner to outer
        let mut chain = core::mem::take(&mut self.m.touch.chain);
        chain.clear();
        let mut can = 0u8; // the axes something can roll along
        let mut both = false;
        {
            let c = &self.wp.ctx;
            if let Some(i) = hit {
                let layer = c.hits[c.hit_cur ^ 1][i].layer;
                for v in c.pans[c.hit_cur ^ 1].iter() {
                    if v.layer == layer && i as u32 >= v.hit0 && (i as u32) < v.hit1 {
                        chain.push(v.id);
                        let mut a = 0;
                        if v.axes & AX_H != 0 && v.max[0] > 0 {
                            a |= AX_H;
                        }
                        if v.axes & AX_V != 0 && v.max[1] > 0 {
                            a |= AX_V;
                        }
                        // the first view that takes this axis scrolls both ways: the other is free too
                        if can & axis == 0 && a & axis != 0 && a == AX_ALL {
                            both = true;
                        }
                        can |= a;
                    }
                }
            }
        }
        self.m.touch.chain = chain;
        if can & axis == 0 {
            if waits {
                self.m.touch.mode = Mode::Gone;
            } else {
                self.touch_press();
            }
            return;
        }
        // rails: a finger along an axis stays on it
        let free = both && adx.min(ady).wrapping_mul(2) >= adx.max(ady);
        let hovered = self.m.touch.hovered;
        let slop = self.dp(self.m.touch.conf[0]);
        let s = &mut self.m.touch;
        s.mode = Mode::Pan;
        s.axes = if free { AX_ALL } else { axis };
        // what lies beyond the slop moves now (a slow frame's long first step), the slop is caught up
        for (k, d, ax) in [(0, dx, AX_H), (1, dy, AX_V)] {
            if s.axes & ax != 0 {
                let c = d.clamp(-slop, slop);
                s.catchup[k] = c;
                s.pend[k] = s.pend[k].wrapping_add(d.wrapping_sub(c));
            }
        }
        if hovered {
            let (x, y) = (-1, -1);
            self.touch_out(plat::E_LEAVE, 0, x, y);
        }
    }

    // the finger moved on (px): along the locked axes, the slop's way caught up on the way
    fn touch_pan(&mut self, dx: i32, dy: i32) {
        let s = &mut self.m.touch;
        for (k, d, ax) in [(0, dx, AX_H), (1, dy, AX_V)] {
            if s.axes & ax == 0 {
                continue;
            }
            let c = s.catchup[k];
            let extra = (d.wrapping_abs() / 2).min(c.wrapping_abs());
            let extra = if d != 0 && extra == 0 && c != 0 { 1 } else { extra };
            let extra = if c < 0 { -extra } else { extra };
            s.catchup[k] = c - extra;
            s.pend[k] = s.pend[k].wrapping_add(d).wrapping_add(extra);
        }
    }

    fn touch_up(&mut self, ev: &Event, t: u32) {
        let cancel = ev.data & plat::PD_CANCEL != 0;
        let mode = self.m.touch.mode;
        self.m.touch.mode = Mode::Idle;
        match mode {
            Mode::Wait => {
                if !cancel && !self.m.touch.no_click && !self.m.touch.long_done {
                    // a click where it came down: a finger that rolled a little within the slop
                    // still means that widget
                    self.uiw_use(self.m.touch.win);
                    self.touch_find_again();
                    let [x, y] = self.touch_aimed(self.m.touch.down);
                    self.touch_out(plat::E_MOVE, 0, x, y);
                    self.touch_out(plat::E_DOWN, BTN_LEFT as u8, x, y);
                    self.touch_out(plat::E_UP, BTN_LEFT as u8, x, y);
                    plat::touch_reached(true);
                }
                if self.m.touch.hovered || !self.m.touch.no_click {
                    // (a tooltip the resting finger brought stays long enough to read it)
                    self.uiw_use(self.m.touch.win);
                    let ms = if self.tip_showing() { TIP_LEAVE_MS } else { LEAVE_MS };
                    self.m.touch.leave_at = Some(t.wrapping_add(ms));
                }
            }
            Mode::Widget => {
                let [ax, ay] = self.touch_aimed([ev.x, ev.y]);
                let (x, y) = if cancel { (OUTSIDE, OUTSIDE) } else { (ax, ay) };
                self.touch_out(plat::E_UP, BTN_LEFT as u8, x, y);
                self.m.touch.leave_at = Some(t.wrapping_add(LEAVE_MS));
            }
            Mode::Pan if !cancel => {
                self.uiw_use(self.m.touch.win);
                self.touch_fling(t);
                // no fling: the rest of the slop's way now, nothing is lost
                let s = &mut self.m.touch;
                if s.fling.is_none() {
                    s.pend[0] = s.pend[0].wrapping_add(core::mem::take(&mut s.catchup[0]));
                    s.pend[1] = s.pend[1].wrapping_add(core::mem::take(&mut s.catchup[1]));
                }
            }
            Mode::Gone if self.m.touch.hovered => self.m.touch.leave_at = Some(t.wrapping_add(LEAVE_MS)),
            _ => {}
        }
    }

    // the velocity along one axis at the end of the samples (px/s): a least squares parabola over
    // the last 100 ms, as VelocityTracker's LSQ2; 0 when the finger had stopped
    fn touch_velocity(&self, k: usize, up: u32) -> f64 {
        let s = &self.m.touch;
        if s.n < 2 || up.wrapping_sub(s.samples[s.n - 1].0) > VEL_STOPPED_MS {
            return 0.0;
        }
        let last = s.samples[s.n - 1];
        let at = |e: &(u32, i32, i32)| if k == 0 { e.1 } else { e.2 };
        let (mut n, mut st, mut st2, mut st3, mut st4, mut sx, mut stx, mut st2x) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut first = 0.0f64;
        let mut stamps = 0;
        let mut prev = u32::MAX;
        for e in s.samples[..s.n].iter().rev() {
            let age = last.0.wrapping_sub(e.0);
            if age > VEL_HORIZON_MS {
                break;
            }
            let t = -(age as f64) / 1000.0;
            let x = (at(e) - at(&last)) as f64;
            first = t;
            if age != prev {
                stamps += 1;
                prev = age;
            }
            n += 1.0;
            st += t;
            st2 += t * t;
            st3 += t * t * t;
            st4 += t * t * t * t;
            sx += x;
            stx += t * x;
            st2x += t * t * x;
        }
        if stamps < 2 || first == 0.0 {
            return 0.0;
        }
        if stamps >= 3 {
            // the normal equations of x = a + b t + c t^2 (Cramer): b
            let det = n * (st2 * st4 - st3 * st3) - st * (st * st4 - st3 * st2) + st2 * (st * st3 - st2 * st2);
            let d = if det < 0.0 { -det } else { det };
            if d > 1e-18 {
                let db = n * (stx * st4 - st3 * st2x) - sx * (st * st4 - st3 * st2) + st2 * (st * st2x - st2 * stx);
                return db / det;
            }
        }
        // two instants: the line
        let det = n * st2 - st * st;
        if det == 0.0 { 0.0 } else { (n * stx - st * sx) / det }
    }

    // the finger lifted from a pan
    fn touch_fling(&mut self, up: u32) {
        let (vx, vy) = (
            if self.m.touch.axes & AX_H != 0 { self.touch_velocity(0, up) } else { 0.0 },
            if self.m.touch.axes & AX_V != 0 { self.touch_velocity(1, up) } else { 0.0 },
        );
        let mut speed = sqrt(vx * vx + vy * vy);
        let (lo, hi) = (self.dp(self.m.touch.conf[1]) as f64, self.dp(self.m.touch.conf[2]) as f64);
        if !(speed >= lo) {
            return;
        }
        let (ux, uy) = (vx / speed, vy / speed);
        if speed > hi {
            speed = hi;
        }
        // OverScroller: the distance and the time from the speed
        let ppi = self.wp.rt.dpi as f64 * 160.0 / 96.0;
        let phys = G_INCH * ppi;
        let r = INFLEXION * speed / (FRICTION * phys);
        let dur = 1000.0 * rt::x87::pow(r, 1.0 / (DECEL - 1.0));
        let way = FRICTION * phys * rt::x87::pow(r, DECEL / (DECEL - 1.0));
        if self.m.touch.spline.is_empty() {
            self.m.touch.spline = spline_table();
        }
        let s = &mut self.m.touch;
        s.fling = Some(Fling {
            t0: up,
            dur,
            // (what the slop still owes goes along)
            dist: [way * ux + s.catchup[0] as f64, way * uy + s.catchup[1] as f64],
            done: [0.0; 2],
        });
        self.animating = true;
    }

    /// the clock: the resting finger, the hover after it lifted
    pub(crate) fn touch_tick(&mut self) {
        let now = self.now_ms;
        let s = &self.m.touch;
        if s.mode == Mode::Wait && !s.no_click && !s.long_done {
            let held = now.wrapping_sub(s.down_t);
            let [x, y] = self.touch_aimed(s.down);
            let s = &mut self.m.touch;
            if s.grab & G_HOLD_LONG != 0 {
                s.repaint = true; // (it shows how far the hold is: touch_hold_part)
            }
            if held >= HOVER_MS && !s.hovered {
                self.m.touch.hovered = true;
                self.touch_out(plat::E_MOVE, 0, x, y);
            }
            let long = if self.m.touch.grab & G_HOLD_LONG != 0 { HOLD_LONG_MS } else { self.m.touch.conf[3] as u32 };
            if held >= long {
                self.uiw_use(self.m.touch.win);
                let (hit, grab) = self.touch_under();
                if grab & (G_HOLD | G_REST) != 0 {
                    // it wanted a press and hold: its drag starts here (a canvas: the press)
                    if grab & G_HOLD != 0 {
                        plat::touch_long_press();
                    }
                    self.touch_press();
                    return;
                }
                let c = &self.wp.ctx;
                let id = hit.map_or(0, |i| c.hits[c.hit_cur ^ 1][i].id);
                self.m.touch.long_id = id;
                self.m.touch.long_done = true;
                self.touch_out(plat::E_MOVE, 0, x, y);
                self.touch_out(plat::E_DOWN, BTN_RIGHT as u8, x, y);
                self.touch_out(plat::E_UP, BTN_RIGHT as u8, x, y);
                plat::touch_long_press();
            }
        } else if self.m.touch.mode == Mode::Idle {
            if let Some(at) = self.m.touch.leave_at {
                if (now.wrapping_sub(at) as i32) >= 0 {
                    self.m.touch.leave_at = None;
                    self.touch_out(plat::E_LEAVE, 0, -1, -1);
                }
            }
        }
    }

    /// the host keeps drawing (and a script's clock looks here): a finger is down, a fling runs,
    /// the hover waits to go
    pub(crate) fn touch_busy(&self) -> bool {
        let s = &self.m.touch;
        s.mode != Mode::Idle || s.fling.is_some() || s.leave_at.is_some() || !s.out.is_empty() || s.pend != [0; 2] || s.repaint
    }

    /// the finger moved some way since the last frame (or the scroll bars changed): a frame now
    pub(crate) fn touch_moved(&self) -> bool {
        self.m.touch.pend != [0; 2] || self.m.touch.repaint
    }

    /// (run_passes, before the first pass) the way of the finger and of the fling since the last
    /// frame into the views: this frame shows it
    pub(crate) fn touch_apply(&mut self) {
        let s = &mut self.m.touch;
        s.take.clear();
        s.repaint = false;
        let mut d = core::mem::take(&mut s.pend);
        let mut flinging = false;
        if let Some(f) = s.fling.as_mut() {
            flinging = true;
            let el = self.now_ms.wrapping_sub(f.t0) as f64;
            let part = if el >= f.dur {
                1.0
            } else {
                let p = el / f.dur * 100.0;
                let k = p as usize;
                s.spline[k] + (p - k as f64) * (s.spline[k + 1] - s.spline[k])
            };
            for k in 0..2 {
                let step = (f.dist[k] * part - f.done[k]) as i32; // whole pixels
                f.done[k] += step as f64;
                d[k] = d[k].wrapping_add(step);
            }
            if part >= 1.0 {
                s.fling = None;
            }
        }
        if d == [0; 2] {
            return;
        }
        let win = s.win;
        if win as usize >= WIN_MAX || (win != 0 && !self.uiw_drawable(win)) {
            self.m.touch.fling = None;
            return;
        }
        self.uiw_use(win);
        if self.rtl {
            d[0] = d[0].wrapping_neg(); // the content moves in logical coordinates
        }
        let mut moved = false;
        let Ui { wp, m, slots, .. } = self;
        let W { pans, hit_cur, .. } = &wp.ctx;
        let views = &pans[*hit_cur ^ 1];
        let t = &mut m.touch;
        for &id in t.chain.iter() {
            let Some(v) = views.iter().find(|v| v.id == id) else { continue };
            let mut np = v.pos;
            for (k, ax) in [(0, AX_H), (1, AX_V)] {
                if v.axes & ax != 0 && d[k] != 0 {
                    np[k] = v.pos[k].wrapping_sub(d[k]).clamp(0, v.max[k].max(0));
                    d[k] = d[k].wrapping_sub(v.pos[k].wrapping_sub(np[k]));
                }
            }
            if np == v.pos {
                continue;
            }
            moved = true;
            if v.own {
                if t.take.len() < 16 {
                    t.take.push((id, np[0].wrapping_sub(v.pos[0]), np[1].wrapping_sub(v.pos[1])));
                }
            } else if let Some(sl) = slots.get_mut(&id) {
                if v.axes & AX_H != 0 {
                    sl.scrollx = np[0];
                    sl.at0 = np[0] as u32;
                }
                if v.axes & AX_V != 0 {
                    sl.scrolly = np[1];
                    sl.adur = np[1] as u32;
                }
            }
        }
        // everything at its limit: the fling is over
        if flinging && !moved {
            t.fling = None;
        }
        self.uiw_use(0);
    }

    /// (run_passes, after the passes) a fling goes on; the on-screen keyboard comes when a finger's
    /// press gave a text field the focus (not for a scroll, not for a mouse) and goes with that focus
    pub(crate) fn touch_after_passes(&mut self) {
        if self.m.touch.fling.is_some() {
            self.animating = true;
        }
        let ask = core::mem::take(&mut self.m.touch.kb_ask);
        if !ask && !self.m.touch.kb_on {
            return;
        }
        let win = self.m.win.active;
        self.uiw_use(win);
        let c = &self.wp.ctx;
        let text = c.focus != 0 && c.hits[c.hit_cur ^ 1].iter().any(|h| h.id == c.focus && h.flags as u32 & HB_TEXT != 0);
        let s = &mut self.m.touch;
        let want = text && (s.kb_on || (ask && s.on));
        if want != s.kb_on {
            s.kb_on = want;
            s.kb_win = win;
            if !self.headless {
                plat::touch_keyboard(self.uiw_pw(self.m.touch.kb_win), want);
            }
        }
        self.uiw_use(0);
    }

    /// (chrome) how far a press and hold on this title bar is: a line along its lower edge that grows
    /// until the window comes loose, and stays while it moves
    pub(crate) fn touch_hold_show(&mut self, id: u32) {
        let Some(part) = self.touch_hold_part(id) else { return };
        let [x, y, w, h] = self.wp.ctx.last_rect;
        let t = self.dp(3);
        let c = self.color(T_ACCENT);
        self.draw_color(c);
        self.draw_rect(x, y + h - t, (w as i64 * part as i64 / 1000) as i32, t);
    }

    /// (a script: the platform's job elsewhere) a finger on a window's title bar moves the window,
    /// and not out of reach
    pub(crate) fn touch_window_drag(&mut self, i: u32, pressed: bool) {
        let c = &self.wp.ctx;
        let (dx, dy) = (c.mx.wrapping_sub(c.press_x), c.my.wrapping_sub(c.press_y));
        let dpi = self.wp.rt.dpi;
        let (dx, dy) = (px2dip_at(dx, dpi), px2dip_at(dy, dpi));
        let iu = (i as usize) & (WIN_MAX - 1);
        let rc = self.m.win.recs[iu].rect.get();
        if pressed {
            self.m.touch.win_base = [rc.x, rc.y];
        }
        let [bx, by] = self.m.touch.win_base;
        let fb = self.win_fb(0);
        let mdpi = self.win_dpi(0);
        let (sw, sh) = (px2dip_at(fb.w, mdpi), px2dip_at(fb.h, mdpi));
        let wfb = self.win_fb(i);
        let w = px2dip_at(wfb.w, self.win_dpi(i));
        let (sx, sy) = self.uiw_snap_dip(i, bx.wrapping_add(dx), by.wrapping_add(dy));
        let [x, y] = plat::reach_clamp([sx, sy], w, [sw, sh], 96);
        if x != rc.x || y != rc.y {
            self.uiw_user_rect(i, x, y, 0, 0);
        }
    }
}
