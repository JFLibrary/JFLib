// timeline -- a scrubber for recorded events: a time axis, one lane of event ticks per track
// and a playhead (the event timeline of a simulation)
//
//   timeline(id, &mut st, span, tracks) -> EV_*
//        span: the recorded time range in seconds (span.1 is the live end); a track is a
//        label and its ticks (time, 0xAARRGGBB; colour 0 = the chart palette colour of the
//        track), sorted by time.  Natural size 480 DIP x (the axis + 22 DIP per track);
//        next_size sets another (the lanes share the height).  The axis has 1 / 2 / 5 steps
//        with labels in seconds (the UI's decimal separator), down to nanoseconds (nine
//        decimals; the narrowest view is 10 ns: simulated networks).  Each pixel column of a lane
//        draws the ticks that fall into it as one bar (taller for more ticks, their colours
//        stacked by share; a column alone is two wide), so many thousands of ticks stay
//        cheap.  The playhead (st.t) is an accent line with its time on the axis.
//        Mouse: a press in the lanes seeks and drags the playhead, so does its time on the
//        axis; dragging the axis elsewhere pans the view, a click there seeks, a double
//        click shows the whole span; Shift+drag in the lanes selects a range (st.sel,
//        Shift+click clears it); Ctrl+wheel zooms around the pointer, Shift+wheel and the
//        horizontal wheel pan (the plain wheel is left to scroll views around it).  Keys
//        (focused): Left / Right the previous / next tick of any track, Home / End the
//        start / the live end (End follows it again), PgUp / PgDn a view width, + / - zoom
//        at the playhead, Esc clears the range.
//        EV_ACT: the user moved st.t; EV_COMMIT: a drag of the playhead ended (a key or a
//        click gives both).  st.follow keeps the playhead at the live end and the view
//        with it; moving the playhead by hand ends that.  st.view is the visible range
//        (empty: the whole span; zoomed out fully it grows with the span).  Seeks by the
//        pointer are rounded to the decimal resolution of a pixel.
//
// Ids: id (the lanes; focus), part(id, 1) the axis, part(id, 2) the playhead's time.
// Records: K_SLIDER id (value, min, max: t and the span in ms), K_TEXT per track label
// (value: its number of ticks), K_TEXT part(id, 2) with the playhead's time ("1,250 s").
// Everything keeps its left-to-right order in mirrored windows (ltr_begin).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::keys::*;
use crate::stock::text_at;
use crate::Ui;

const LANE_H: i32 = 22; // DIP per track
const MIN_VIEW: f64 = 1e-8; // the narrowest view (s): its axis labels still differ in 9 decimals (ns)
const ZOOM: f64 = 1.25; // per wheel notch / key
const SCAN: usize = 64; // the ticks of a column whose colours are counted

// what a press started
const M_NONE: u8 = 0;
const M_SCRUB: u8 = 1;
const M_PAN: u8 = 2;
const M_SELECT: u8 = 3;

/// one lane of a timeline: a label and its event ticks
#[derive(Clone, Copy, Debug)]
pub struct TlTrack<'a> {
    pub label: &'a str,
    /// (time in s, 0xAARRGGBB; 0 = the chart palette colour of the track), sorted by time
    pub ticks: &'a [(f64, u32)],
}

/// the app-owned state of a timeline: playhead, visible range, selected range, following
#[derive(Clone, Default)]
pub struct TimelineState {
    /// the playhead (s)
    pub t: f64,
    /// the visible time range (s); empty (the default) = the whole span
    pub view: (f64, f64),
    /// a selected time range (Shift+drag), start <= end
    pub sel: Option<(f64, f64)>,
    /// keep the playhead at the live end of the span (moving it by hand ends this)
    pub follow: bool,
    /// modifier keys (MOD_*) treated as held: scripts cannot hold Ctrl or Shift during a
    /// `wheel` (an app maps a test command to it)
    pub test_mods: u32,
    mode: u8,
    part: u8,
    dragged: bool,
    click: Option<(u32, i32)>,
    fitted: bool,
    seen: (f64, f64),
    grab_t: f64,
    grab_dx: i32,
    grab_view: (f64, f64),
    cache: Option<(u64, DrawRef)>,
}

impl TimelineState {
    /// show the whole span again
    pub fn fit(&mut self) {
        self.view = (0.0, 0.0);
    }
}

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

fn floor_f(x: f64) -> f64 {
    let f = x as i64 as f64;
    if f > x { f - 1.0 } else { f }
}

// x in ms, rounded (records)
fn ms(x: f64) -> i32 {
    let v = x * 1000.0;
    (if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32
}

// the axis step: the smallest 1 / 2 / 5 x 10^k that is at least min -> (step, minor ticks
// per step)
fn nice_step(min: f64) -> (f64, u32) {
    if !min.is_finite() || min > 1e12 {
        return (1e12, 5);
    }
    let min = if min > 1e-9 { min } else { 1e-9 };
    let mut p = 1.0f64;
    while p * 10.0 <= min {
        p *= 10.0;
    }
    while p > min {
        p /= 10.0;
    }
    if p >= min {
        (p, 5)
    } else if p * 2.0 >= min {
        (p * 2.0, 4)
    } else if p * 5.0 >= min {
        (p * 5.0, 5)
    } else {
        (p * 10.0, 5)
    }
}

// the decimals that show multiples of step (0..9: down to nanoseconds, simulated networks)
fn decimals(step: f64) -> u32 {
    let mut d = 0;
    let mut s = step;
    while d < 9 && s < 0.999_999 {
        s *= 10.0;
        d += 1;
    }
    d
}

// t rounded to the power of ten not above the time of a pixel
fn snap(t: f64, per_px: f64) -> f64 {
    if !(per_px > 1e-9) {
        return t;
    }
    let mut q = 1.0f64;
    while q > per_px {
        q /= 10.0;
    }
    while q * 10.0 <= per_px {
        q *= 10.0;
    }
    floor_f(t / q + 0.5) * q
}

// log2 of n, linear between the powers of two (0 for n <= 1)
fn log2f(n: u32) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    let e = 31 - n.leading_zeros();
    e as f64 + (n as f64 / (1u64 << e) as f64 - 1.0)
}

// the zoom factor of a number of wheel notches (> 1: in), ZOOM per notch
fn zoom_factor(notches: f64) -> f64 {
    let mut k = if notches < 0.0 { -notches } else { notches };
    let mut f = 1.0;
    while k >= 1.0 {
        f *= ZOOM;
        k -= 1.0;
    }
    f *= 1.0 + (ZOOM - 1.0) * k;
    if notches < 0.0 { 1.0 / f } else { f }
}

// the view moved inside full (as wide as it was, at most as wide as full)
fn clamp_view(v: (f64, f64), full: (f64, f64)) -> (f64, f64) {
    let fw = full.1 - full.0;
    let w = (v.1 - v.0).min(fw).max(MIN_VIEW.min(fw));
    let mut a = v.0;
    if a + w > full.1 {
        a = full.1 - w;
    }
    if a < full.0 {
        a = full.0;
    }
    (a, a + w)
}

// the view zoomed by f (> 1: in) around time at, which keeps its place
fn zoom_view(v: (f64, f64), at: f64, f: f64, full: (f64, f64)) -> (f64, f64) {
    let w = v.1 - v.0;
    let fw = full.1 - full.0;
    let nw = (w / f).min(fw).max(MIN_VIEW.min(fw));
    let k = if w > 0.0 { (at - v.0) / w } else { 0.5 };
    let a = at - k * nw;
    clamp_view((a, a + nw), full)
}

// the view moved so that t is inside it (a tenth of its width from the edge it came in at)
fn reveal(v: (f64, f64), t: f64, full: (f64, f64)) -> (f64, f64) {
    let w = v.1 - v.0;
    if t < v.0 {
        clamp_view((t - w * 0.1, t + w * 0.9), full)
    } else if t > v.1 {
        clamp_view((t - w * 0.9, t + w * 0.1), full)
    } else {
        v
    }
}

// the latest tick of any track before t / the earliest after it
fn tick_before(tracks: &[TlTrack], t: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for tr in tracks {
        let i = tr.ticks.partition_point(|k| k.0 < t);
        if i > 0 {
            let x = tr.ticks[i - 1].0;
            if best.is_none_or(|b| x > b) {
                best = Some(x);
            }
        }
    }
    best
}

fn tick_after(tracks: &[TlTrack], t: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for tr in tracks {
        let i = tr.ticks.partition_point(|k| k.0 <= t);
        if let Some(k) = tr.ticks.get(i) {
            if best.is_none_or(|b| k.0 < b) {
                best = Some(k.0);
            }
        }
    }
    best
}

// a column of a lane: its x (px from the plot), how many ticks, up to three colours with the
// number of ticks of each among the first SCAN
#[derive(Clone, Copy)]
struct Col {
    lane: u32,
    x: i32,
    n: u32,
    c: [(u32, u32); 3],
}

// the columns (bw px wide) of a lane's visible ticks: one partition_point per column
#[allow(clippy::too_many_arguments)]
fn bin_lane(ticks: &[(f64, u32)], lane: u32, v0: f64, pps: f64, bw: i32, pw: i32, pal: u32, out: &mut Vec<Col>) {
    let ncol = ((pw + bw - 1) / bw).max(1);
    let v1 = v0 + pw as f64 / pps;
    let a = ticks.partition_point(|k| k.0 < v0);
    let b = a + ticks[a..].partition_point(|k| k.0 <= v1);
    let vis = &ticks[a..b];
    let cps = pps / bw as f64; // columns per second
    let mut i = 0;
    while i < vis.len() {
        let f = (vis[i].0 - v0) * cps;
        let c = if f > 0.0 { (f as i64).min(ncol as i64 - 1) as i32 } else { 0 };
        let j = if c + 1 >= ncol {
            vis.len()
        } else {
            let end = v0 + (c + 1) as f64 / cps;
            i + 1 + vis[i + 1..].partition_point(|k| k.0 < end)
        };
        let mut col = Col { lane, x: c * bw, n: (j - i) as u32, c: [(0, 0); 3] };
        for k in &vis[i..j.min(i + SCAN)] {
            let cc = if k.1 == 0 { pal } else { k.1 };
            if let Some(s) = col.c.iter_mut().find(|s| s.1 != 0 && s.0 == cc) {
                s.1 += 1;
            } else if let Some(s) = col.c.iter_mut().find(|s| s.1 == 0) {
                *s = (cc, 1);
            } else {
                col.c[2].1 += 1;
            }
        }
        out.push(col);
        i = j;
    }
}

// the bars of the columns (x, y, w, h, colour from the top left of the lanes): the height
// grows with log2 of the ticks (relative to the fullest column of all lanes), the colours
// are stacked by share, the most frequent at the bottom, each at least a pixel
fn bars(cols: &[Col], lane_h: i32, pad: i32, bw: i32) -> Vec<(i32, i32, i32, i32, u32)> {
    let maxn = cols.iter().map(|c| c.n).max().unwrap_or(1);
    let lmax = log2f(maxn);
    let avail = (lane_h - pad * 2).max(2);
    let hmin = (avail * 2 / 5).max(2).min(avail);
    let mut out = Vec::with_capacity(cols.len() + cols.len() / 4);
    for (k, c) in cols.iter().enumerate() {
        let h = if lmax > 0.0 { hmin + ((avail - hmin) as f64 * log2f(c.n) / lmax) as i32 } else { hmin };
        let y = c.lane as i32 * lane_h + (lane_h - h) / 2;
        // a column alone in its lane is drawn two columns wide
        let before = k > 0 && cols[k - 1].lane == c.lane && cols[k - 1].x + bw >= c.x;
        let after = k + 1 < cols.len() && cols[k + 1].lane == c.lane && cols[k + 1].x <= c.x + bw;
        let (x, w) = if before || after { (c.x, bw) } else { (c.x - bw / 2, bw * 2) };
        let mut s = c.c;
        s.sort_by(|a, b| b.1.cmp(&a.1));
        let total: u32 = s.iter().map(|x| x.1).sum();
        let used = s.iter().filter(|x| x.1 > 0).count() as i32;
        let (mut top, mut rest) = (y, h);
        for (i, &(cc, cn)) in s.iter().filter(|x| x.1 > 0).rev().enumerate() {
            let left = used - 1 - i as i32; // segments below this one
            let hs = if left == 0 { rest } else { ((h as i64 * cn as i64 / total.max(1) as i64) as i32).max(1).min(rest - left).max(0) };
            if hs > 0 {
                out.push((x, top, w, hs, cc));
            }
            top += hs;
            rest -= hs;
        }
    }
    out
}

// the bars of the lanes (drawn at render time; kept while they do not change)
struct Bars(Vec<(i32, i32, i32, i32, u32)>);

impl Draw for Bars {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        for &(x, y, w, h, c) in &self.0 {
            g.dc.set_color(c);
            g.dc.fill(g.sf, r[0] + x, r[1] + y, w, h);
        }
    }
}

struct Fnv(u64);

impl Fnv {
    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

impl Ui {
    // a time as the timeline shows it: "1,25 s"; below a microsecond with up to nine decimals
    // ("0,000000722 s"; fmt_f64 stops at six)
    fn tl_text(&self, t: f64, d: u32) -> String {
        if d <= 6 {
            return format!("{} s", self.fmt_f64(t, d));
        }
        let neg = t < 0.0;
        let ns = ((if neg { -t } else { t }) * 1e9 + 0.5) as u64;
        let frac = format!("{:09}", ns % 1_000_000_000);
        let sep = char::from_u32(self.m.value.decimal_sep).unwrap_or(',');
        format!("{}{}{}{} s", if neg { "-" } else { "" }, ns / 1_000_000_000, sep, &frac[..(d as usize).min(9)])
    }

    // the axis of a view over pw px -> (step, minor ticks per step, decimals): labels at
    // least 80 DIP apart and never closer than their width
    fn tl_axis(&mut self, v: (f64, f64), pw: i32) -> (f64, u32, u32) {
        let pps = pw as f64 / (v.1 - v.0);
        let big = if -v.0 > v.1 { v.0 } else { v.1 };
        self.ts_font(TS_CAPTION);
        let mut r = nice_step(self.dp(80) as f64 / pps);
        for _ in 0..3 {
            let s = self.tl_text(big, decimals(r.0));
            let need = (self.text.width(&s) + self.dp(16)) as f64 / pps;
            if r.0 >= need {
                break;
            }
            r = nice_step(need);
        }
        (r.0, r.1, decimals(r.0))
    }

    // the playhead's time on the axis: its rectangle (x centred on cx, inside lo .. hi)
    fn tl_head(&mut self, s: &str, cx: i32, y: i32, lo: i32, hi: i32) -> [i32; 4] {
        self.ts_font(TS_CAPTION);
        let w = self.text.width(s) + self.dp(12);
        let h = self.ts_line(TS_CAPTION) + self.dp(2);
        let x = (cx - w / 2).min(hi - w).max(lo);
        [x, y, w, h]
    }

    /// ui_timeline(id, &mut st, span, tracks) -> EV_*: a time axis, a lane of event ticks per
    /// track and a playhead (st.t) to seek with (see the module header of timeline.rs)
    pub fn timeline(&mut self, id: u32, st: &mut TimelineState, span: (f64, f64), tracks: &[TlTrack]) -> u32 {
        let n = tracks.len() as i32;
        let line = self.ts_line(TS_CAPTION);
        let axis_h = line + self.dp(10);
        let r = self.place(self.dp(480), axis_h + self.dp(LANE_H) * n.max(1));
        let lane_h = if n > 0 { ((r[3] - axis_h) / n).clamp(self.dp(12), self.dp(48)) } else { 0 };
        let hair = self.hair();
        // the label column, the plot (the pixels of the time axis), the lanes
        self.ts_font(TS_CAPTION);
        let mut lw = 0;
        for tr in tracks {
            lw = lw.max(self.text.width(crate::basic::cstr(tr.label)));
        }
        let lw = if lw > 0 { (lw + self.dp(16)).min(r[2] * 3 / 10) } else { 0 };
        let m = self.dp(12);
        let px = r[0] + lw + m;
        let pw = (r[2] - lw - m * 2).max(1);
        let ly = r[1] + axis_h;
        let lh = (r[3] - axis_h).max(1);
        let (head_lo, head_hi) = (r[0] + lw + hair, r[0] + r[2] - hair);
        let head_y = r[1] + self.dp(3);
        // the span, the playhead and the view
        let s0 = if span.0.is_finite() { span.0 } else { 0.0 };
        let s1 = if span.1.is_finite() && span.1 > s0 { span.1 } else { s0 };
        let full = (s0, if s1 > s0 { s1 } else { s0 + 1.0 });
        let sel_in = st.sel;
        let mut t = if st.t.is_finite() { st.t.clamp(s0, s1) } else { s0 };
        // zoomed out fully (and not changed by the app since), the view grows with the span
        let mut v = st.view;
        if (st.fitted && v == st.seen) || !(v.0.is_finite() && v.1.is_finite() && v.1 > v.0) {
            v = full;
        }
        v = clamp_view(v, full);
        if st.follow {
            t = s1;
            v = reveal(v, t, full);
        }
        // input maps the pointer with the view as it shows now
        let v_in = v;
        let pps = pw as f64 / (v.1 - v.0);
        let per_px = 1.0 / pps;
        let time_at = |x: i32| v_in.0 + ((x - px) as f64 + 0.5) / pps;
        let x_at = |v: (f64, f64), t: f64| px + floor_f(((t - v.0) * (pw as f64 / (v.1 - v.0))).clamp(-1e6, 1e6)) as i32;
        // hit boxes: the axis, the lanes (focus), the playhead's time on top
        let pid_axis = part(id, 1);
        let pid_head = part(id, 2);
        self.ltr_begin(r[0], r[2]);
        let rtl = self.rtl;
        let lx = |x: i32, w: i32| if rtl { r[0] + r[0] + r[2] - x - w } else { x }; // logical x of a span drawn LTR
        let mx = self.ltr_x(self.wp.ctx.mx);
        let pmx = self.ltr_x(self.wp.ctx.press_x);
        let (_, _, dec) = self.tl_axis(v, pw);
        let ht = self.tl_text(t, dec.max(3));
        let hr = self.tl_head(&ht, x_at(v, t), head_y, head_lo, head_hi);
        let ev_axis = self.hit_rect(pid_axis, r[0], r[1], r[2], axis_h, HB_MOVES);
        let ev_lane = self.hit_rect(id, r[0], ly, r[2], lh, HB_FOCUS | HB_MOVES);
        let ev_head = self.hit_rect(pid_head, lx(hr[0], hr[2]), hr[1], hr[2], hr[3], HB_MOVES | CUR_SIZE_WE << HB_CURSOR_SHIFT);
        let evs = ev_axis | ev_lane | ev_head;
        let mut out = 0u32;
        let mut user_t: Option<f64> = None;
        let mut commit = false;
        let mut keyed = false;
        let mut live = false;
        // a press: the keyboard focus and what the drag does
        let (act, pressed) = (self.wp.ctx.active, self.wp.ctx.pressed & (1 << BTN_LEFT) != 0);
        let ours = act == id || act == pid_axis || act == pid_head;
        if pressed && ours {
            let c = &mut self.wp.ctx;
            c.focus = id;
            c.focus_visible = false;
            st.dragged = false;
            st.part = if act == pid_axis { 1 } else if act == pid_head { 2 } else { 0 };
            // a second click on the axis (the first one put the playhead's time under the
            // pointer, so it may land there): the whole span
            let now = self.now_ms;
            let again = st.click.is_some_and(|(ms, x)| now.wrapping_sub(ms) <= self.dbl_ms && (mx - x).abs() <= self.dp(4));
            st.click = None;
            st.mode = if st.part != 0 && again {
                v = full;
                M_NONE
            } else if st.part == 1 {
                st.grab_view = v;
                M_PAN
            } else if st.part == 0 && self.mods & MOD_SHIFT != 0 {
                let a = snap(time_at(mx).clamp(v.0, v.1), per_px);
                st.grab_t = a;
                st.sel = Some((a, a));
                M_SELECT
            } else {
                // the playhead's time: grabbed where it is (no jump)
                st.grab_dx = if st.part == 2 { mx - x_at(v, t).clamp(px, px + pw) } else { 0 };
                M_SCRUB
            };
        }
        if evs & EV_HELD != 0 {
            if evs & EV_DRAG != 0 {
                st.dragged = true;
            }
            match st.mode {
                M_SCRUB if st.part != 2 || st.dragged => {
                    user_t = Some(snap(time_at(mx - st.grab_dx).clamp(v.0, v.1), per_px));
                }
                M_PAN if st.dragged => {
                    let g = st.grab_view;
                    let dt = (mx - pmx) as f64 * (g.1 - g.0) / pw as f64;
                    v = clamp_view((g.0 - dt, g.1 - dt), full);
                }
                M_SELECT => {
                    let (a, b) = (st.grab_t, snap(time_at(mx).clamp(v.0, v.1), per_px));
                    st.sel = Some(if a <= b { (a, b) } else { (b, a) });
                }
                _ => {}
            }
        }
        if evs & EV_COMMIT != 0 {
            match st.mode {
                M_SCRUB => commit = true,
                M_PAN if !st.dragged => {
                    user_t = Some(snap(time_at(pmx).clamp(v_in.0, v_in.1), per_px));
                    commit = true;
                }
                M_SELECT if st.sel.is_some_and(|s| s.1 <= s.0) => st.sel = None,
                _ => {}
            }
            // a click on the axis or the playhead's time (a double click's first half)
            if st.mode != M_NONE && st.part != 0 && !st.dragged {
                st.click = Some((self.now_ms, pmx));
            }
            st.mode = M_NONE;
        } else if !ours {
            st.mode = M_NONE;
        }
        // the wheel: Ctrl zooms around the pointer, Shift / the horizontal wheel pan
        if evs & EV_HOVER != 0 {
            let mods = self.mods | st.test_mods;
            let c = &mut self.wp.ctx;
            let (wx, wy) = (c.wheel_x, c.wheel_y);
            if mods & MOD_CTRL != 0 && wy != 0 {
                v = zoom_view(v, time_at(mx).clamp(v.0, v.1), zoom_factor(wy as f64 / 120.0), full);
                c.wheel_x = 0;
                c.wheel_y = 0;
            } else if wx != 0 || (mods & MOD_SHIFT != 0 && wy != 0) {
                let d = (wx - if mods & MOD_SHIFT != 0 { wy } else { 0 }) as f64 / 120.0;
                let dt = (v.1 - v.0) * 0.1 * d;
                let nv = clamp_view((v.0 + dt, v.1 + dt), full);
                if nv != v {
                    v = nv;
                    c.wheel_x = 0;
                    c.wheel_y = 0;
                }
            }
        }
        // keys: ticks, ends, pages, zoom, Esc
        if ev_lane & EV_FOCUS != 0 {
            let swap = rtl && self.mods & (MOD_CTRL | MOD_ALT) == 0; // the arrows as drawn
            let (kp, kn) = if swap { (VK_RIGHT, VK_LEFT) } else { (VK_LEFT, VK_RIGHT) };
            let w = v.1 - v.0;
            let mut k: Option<Option<f64>> = None;
            if self.key(kp) {
                k = Some(tick_before(tracks, t));
            }
            if self.key(kn) {
                k = Some(tick_after(tracks, t));
            }
            if self.key(VK_HOME) {
                k = Some(Some(s0));
            }
            if self.key(VK_END) {
                k = Some(Some(s1));
                live = true;
            }
            if self.key(VK_PRIOR) {
                k = Some(Some(t - w));
            }
            if self.key(VK_NEXT) {
                k = Some(Some(t + w));
            }
            if let Some(k) = k {
                self.wp.ctx.focus_visible = true;
                if let Some(x) = k {
                    user_t = Some(x);
                    keyed = true;
                    commit = true;
                }
            }
            let zin = self.key(VK_OEM_PLUS) || self.key(VK_ADD);
            let zout = self.key(VK_OEM_MINUS) || self.key(VK_SUBTRACT);
            if zin || zout {
                let at = if t >= v.0 && t <= v.1 { t } else { (v.0 + v.1) / 2.0 };
                v = zoom_view(v, at, if zin { ZOOM } else { 1.0 / ZOOM }, full);
                self.wp.ctx.focus_visible = true;
            }
            if st.sel.is_some() && self.key(VK_ESCAPE) {
                st.sel = None;
            }
        }
        // a screen reader set the value (ms)
        if let Some(x) = self.a11y_value(id) {
            user_t = Some(x as f64 / 1000.0);
            keyed = true;
            commit = true;
        }
        if let Some(x) = user_t {
            let x = x.clamp(s0, s1);
            st.follow = false;
            if x != t {
                t = x;
                out |= EV_ACT;
            }
            if keyed {
                v = reveal(v, t, full);
            }
        }
        if commit {
            out |= EV_COMMIT;
        }
        if live {
            st.follow = true;
        }
        if st.follow {
            t = s1;
            v = reveal(v, t, full);
        }
        if t != st.t || v != st.view || st.sel != sel_in {
            self.repass();
        }
        st.t = t;
        st.view = v;
        st.seen = v;
        st.fitted = v.0 <= full.0 && v.1 >= full.1;
        // the record of the scrubber (first: a screen reader names it from the text before it)
        let mut ss = 0u32;
        if evs & EV_HOVER != 0 {
            ss |= SS_HOVER as u32;
        }
        if evs & EV_HELD != 0 {
            ss |= SS_PRESSED as u32;
        }
        if ev_lane & EV_FOCUS != 0 {
            ss |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = ss;
        let rec = self.sem(K_SLIDER, id, None, ms(t));
        rec.min = ms(s0);
        rec.max = ms(s1);

        // ---- drawing (the view and the playhead after the input)
        let pps = pw as f64 / (v.1 - v.0);
        let (step, minor, dec) = self.tl_axis(v, pw);
        let (ctrl, stroke, div) = (self.color(T_CTRL), self.color(T_CTRL_STROKE), self.color(T_DIVIDER));
        let (t2, t3, acc) = (self.color(T_TEXT2), self.color(T_TEXT3), self.color(T_ACCENT));
        self.cmd_rrect(r[0], r[1], r[2], r[3], self.dp(4), hair, ctrl, stroke, stroke);
        self.clip_push(r[0] + hair, r[1] + hair, r[2] - hair * 2, r[3] - hair * 2);
        // the axis line, the label column, the lanes' separators and labels
        self.draw_color(div);
        self.draw_rect(r[0], ly - hair, r[2], hair);
        if lw > 0 {
            self.draw_rect(r[0] + lw, ly, hair, lh);
        }
        for (k, tr) in tracks.iter().enumerate() {
            let y = ly + lane_h * k as i32;
            if k > 0 {
                self.draw_color(div);
                self.draw_rect(r[0], y, r[2], hair);
            }
            if lw > 0 {
                let label = crate::basic::cstr(tr.label);
                let maxw = lw - self.dp(12);
                self.ts_font(TS_CAPTION);
                let s = if self.text.width(label) > maxw { self.ellipsize(label.as_bytes(), maxw).0 } else { String::from(label) };
                let x = r[0] + self.dp(8);
                text_at(self, TS_CAPTION, x, [x, y, maxw, lane_h], &s, t2, 0);
                self.wp.ctx.last_rect = [lx(r[0], lw), y, lw, lane_h];
                self.wp.ctx.last_state = 0;
                self.sem(K_TEXT, 0, Some(label), tr.ticks.len().min(i32::MAX as usize) as i32);
            }
        }
        // ticks of the axis with their labels, grid lines through the lanes
        let (sub, nsub) = if step / minor as f64 * pps >= self.dp(6) as f64 { (step / minor as f64, minor as i64) } else { (step, 1) };
        let mut j = floor_f(v.0 / sub) as i64;
        let (big, small) = (self.dp(5), self.dp(3));
        for _ in 0..4096 {
            let tt = j as f64 * sub;
            let major = j % nsub == 0;
            j += 1;
            if tt < v.0 - sub * 1e-6 {
                continue;
            }
            if tt > v.1 + sub * 1e-6 {
                break;
            }
            let x = x_at(v, tt);
            if !major {
                self.draw_color(div);
                self.draw_rect(x, ly - small, hair, small);
                continue;
            }
            self.draw_color(div);
            self.draw_rect(x, ly, hair, lh);
            self.draw_color(t3);
            self.draw_rect(x, ly - big, hair, big);
            let s = self.tl_text(tt, dec);
            self.ts_font(TS_CAPTION);
            let w = self.text.width(&s);
            let tx = x + self.dp(4);
            if tx + w <= r[0] + r[2] - self.dp(2) {
                text_at(self, TS_CAPTION, tx, [tx, head_y, w, line + self.dp(2)], &s, t2, 0);
            }
        }
        // the selected range under the ticks
        let sel_x = st.sel.map(|(a, b)| (x_at(v, a).max(px - hair), x_at(v, b).min(px + pw)));
        if let Some((xa, xb)) = sel_x {
            if xb >= xa {
                self.draw_color((acc & 0x00FF_FFFF) | 0x3000_0000);
                self.draw_rect(xa, ly, (xb - xa).max(hair), lh);
            }
        }
        // the ticks: a column per pixel (or per hair at higher dpi), one cached draw object
        let pal = self.style.data().chart_palette(self);
        let mut cols = Vec::new();
        if lane_h > 0 {
            for (k, tr) in tracks.iter().enumerate() {
                bin_lane(tr.ticks, k as u32, v.0, pps, hair, pw, pal[k % 8], &mut cols);
            }
        }
        if !cols.is_empty() {
            let bars = bars(&cols, lane_h, self.dp(3), hair);
            let mut h = Fnv(0xCBF2_9CE4_8422_2325);
            for &(x, y, w, bh, c) in &bars {
                for q in [x as u32, y as u32, w as u32, bh as u32, c] {
                    h.u32(q);
                }
            }
            h.u32(bars.len() as u32);
            let hash = h.0;
            let d = match &st.cache {
                Some((k, d)) if *k == hash => d.clone(),
                _ => {
                    let d: DrawRef = Rc::new(Bars(bars));
                    st.cache = Some((hash, d.clone()));
                    d
                }
            };
            self.cmd_custom(px, ly, pw, lane_h * n, Some(d), 0, hash as u32);
        } else {
            st.cache = None;
        }
        if let Some((xa, xb)) = sel_x {
            if xb >= xa {
                self.draw_color(acc);
                self.draw_rect(xa, ly, hair, lh);
                self.draw_rect(xb, ly, hair, lh);
            }
        }
        // where a click would seek
        if evs & EV_HOVER != 0 && st.mode == M_NONE && ev_head & EV_HOVER == 0 && mx >= px && mx < px + pw {
            self.draw_color(t3);
            self.draw_rect(mx, r[1] + hair, hair, r[3] - hair * 2);
        }
        // the playhead: a line through the lanes, its time on the axis
        let ht = self.tl_text(t, dec.max(3));
        let x = x_at(v, t);
        let hr = self.tl_head(&ht, x, head_y, head_lo, head_hi);
        let pwid = (hair * 2).max(2);
        if x >= px - hair && x <= px + pw {
            self.draw_color(acc);
            self.draw_rect(x - pwid / 2, hr[1] + hr[3], pwid, r[1] + r[3] - hr[1] - hr[3]);
        }
        self.cmd_rrect(hr[0], hr[1], hr[2], hr[3], hr[3] / 2, 0, acc, 0, 0);
        let on = self.color(T_ON_ACCENT);
        text_at(self, TS_CAPTION, hr[0] + self.dp(6), hr, &ht, on, 0);
        self.wp.ctx.last_rect = [lx(hr[0], hr[2]), hr[1], hr[2], hr[3]];
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, pid_head, Some(&ht), ms(t));
        self.clip_pop();
        self.ltr_end();
        if self.focus_ring(ev_lane) {
            let s = self.style;
            s.focus_ring(self, r, 4);
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = ss;
        out | (evs & (EV_HOVER | EV_HELD)) | (ev_lane & EV_FOCUS)
    }
}
