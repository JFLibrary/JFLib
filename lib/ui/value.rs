// value -- decimal values, gradient tracks, angle dial and pan control (U1)
//                                                                  [src/ui/ui_value.s]
//
//   slider_f(id, &mut v, &rf) -> EV           slider for an f64 (RangeF): EV_ACT while the
//        value changes, EV_COMMIT on release and on key steps.  Keys (focused): arrows = one
//        step, PgUp / PgDn = 10 steps, Home / End = minimum / maximum.
//   track_slider(id, &mut v, &rg, track) -> EV   slider for an i32 (Range) whose track shows
//        the colour gradient of a Track (TK_CHECKER: over a checkerboard, for alpha);
//        horizontal or RG_VERT; same keys as slider_f
//   prop_slider(id, label, &mut v, &rg, default) -> EV   paint.net property row: label over
//        slider + number box + reset button (i32; default NO_DEFAULT = no reset button);
//        EV_ACT while the slider is dragged or a number is typed (live preview), EV_COMMIT
//        on release, Enter / focus loss, key steps and reset
//   prop_slider_f(id, label, &mut v, &rf, default) -> EV   the same for an f64 (None = no
//        reset button).  Parts: slider = id, number box = part_id(id, 1), reset =
//        part_id(id, 2); the label selects the slider in scripts.
//   angle(id, &mut deg, flags) -> EV          angle dial (f64 degrees, -180 .. 180 or ANG_360
//        0 .. 360, counter-clockwise, 0 = right): drag sets the angle, Shift snaps to 15
//        degrees; keys: arrows 1 (Shift 15), PgUp / PgDn 15, Home 0.  Natural 64 DIP.
//        Semantic value = degrees * 100.
//   pan(id, &mut x, &mut y, thumb, ver, flags) -> EV   pan control: a thumbnail (surface or
//        None, ver = its version) with a crosshair; drag sets the point (f64, -1 .. 1,
//        PAN_UNIT 0 .. 1; y down); arrows move 1 % (Shift 10 %), Home centres.  Semantic
//        value = x * 1000, max = y * 1000.
//   part_id(id, k) -> id                      id of part k (0..15) of a composite widget
//   fmt_f64(&v, decimals) -> String           "-12,50" with the decimal separator
//   parse_f64(s) -> Some(v) / None            accepts ',' and '.' as decimal separator
//   decimal_sep() / set_decimal_sep(c)        separator shown (default ',')
// Shared with the styles and ui_color / ui_wheel: grad_at, px_mix, deg_atan2, deg_sincos,
// Ui::cmd_track (the cmd_custom of a gradient track: track_cb / track_hash of the asm),
// Ui::slider_track_draw; internal: rf_fix, rf_scaled, rf_t16, sld_*, vl_state, f_round.
//
// No per-window state: everything lives in the app's variables and in state slots.  The
// decimal separator (ui_decimal_sep) and the cache of gradient track drawings are shared.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::cmd::{Draw, DrawRef, Gfx};
use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};
use gfx::Surface;
use rt::fx::{cvtt, cvttd, cvttd64, maxsd, minsd};
/// cvtsd2si (32-bit, round to nearest even) for the styles' geometry
pub use rt::fx::cvtd;

const TRK_POOL: usize = 64; // cached gradient drawings (the asm's TRK_POOL ring)
const PROP_NB_W: i32 = 88; // number box width in a property row (DIP)
/// prop_slider: no reset button
pub const NO_DEFAULT: i32 = i32::MIN;

static POW10: [f64; 19] = [
    1.0, 10.0, 100.0, 1000.0, 10000.0, 100000.0, 1000000.0, 1.0e7, 1.0e8, 1.0e9, 1.0e10, 1.0e11, 1.0e12, 1.0e13,
    1.0e14, 1.0e15, 1.0e16, 1.0e17, 1.0e18,
];
const RAD2DEG: f64 = 57.29577951308232;
const DEG2RAD: f64 = 0.017453292519943295;

/// RANGE_F min, max, step, decimals, flags -- descriptor of an f64 value
#[derive(Clone, Copy, Default, Debug)]
pub struct RangeF {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub dec: u32,
    pub flags: u32,
}

impl RangeF {
    pub const fn new(min: f64, max: f64, step: f64, dec: u32, flags: u32) -> RangeF {
        RangeF { min, max, step, dec, flags }
    }
}

/// TRACK -- the colour gradient of track_slider: TK_* flags and 1..TK_MAXN straight-ARGB
/// stops; with TK_CUSTOM the track is drawn by `custom` (draw object, user value, version:
/// bump it when the drawing changes) instead
#[derive(Clone, Default)]
pub struct Track {
    pub flags: u32,
    pub n: u32,
    pub stops: [u32; TK_MAXN],
    pub custom: Option<(DrawRef, usize, u32)>,
}

impl Track {
    /// TRACK flags, argb0, argb1, ... (at most TK_MAXN stops)
    pub fn new(flags: u32, stops: &[u32]) -> Track {
        let mut t = Track { flags, n: stops.len().min(TK_MAXN) as u32, ..Track::default() };
        t.stops[..t.n as usize].copy_from_slice(&stops[..t.n as usize]);
        t
    }
}

/// shared state (ui_decimal_sep, the gradient drawings of the TRK_POOL ring)
pub(crate) struct S {
    pub(crate) decimal_sep: u32,
    trk: Vec<(u64, Rc<TrackDraw>)>,
    trk_next: usize,
}

impl Default for S {
    fn default() -> S {
        S { decimal_sep: 44, trk: Vec::new(), trk_next: 0 }
    }
}

// ------------------------------------------------------------------ small helpers

/// ui_part_id(id, k) -> PART_BASE | (id << 4 | k) & 0x0FFFFFFF
pub fn part_id(id: u32, k: u32) -> u32 {
    ((id << 4) | (k & 15)) & 0x0FFF_FFFF | PART_BASE
}

/// vl_state(ev, disabled) -> 0 rest, 1 hover, 2 pressed (held), 3 disabled
pub(crate) fn vl_state(ev: u32, disabled: bool) -> u32 {
    if disabled {
        3
    } else if ev & EV_HELD != 0 {
        2
    } else if ev & EV_HOVER != 0 {
        1
    } else {
        0
    }
}

/// f_round(x) -> nearest integer, halves away from zero (cvttsd2si of x +- 0.5)
pub(crate) fn f_round(x: f64) -> i64 {
    if x >= 0.0 { cvttd64(x + 0.5) } else { cvttd64(x - 0.5) }
}

// f_floor(x) -> floor (|x| < 2^62): truncate, step down when that went up
fn f_floor(x: f64) -> f64 {
    let mut t = cvttd64(x);
    if (t as f64) > x {
        t = t.wrapping_sub(1);
    }
    t as f64
}

// rf_dec(rf) -> decimals clamped to 0..6
fn rf_dec(rf: &RangeF) -> usize {
    rf.dec.min(6) as usize
}

/// rf_fix(rf, v) -> NaN -> min, clamped to min..max, snapped to min + k * step (step > 0),
/// rounded to the range's decimals, -0 -> +0
pub(crate) fn rf_fix(rf: &RangeF, v: f64) -> f64 {
    let mut x = if v.is_nan() { rf.min } else { v };
    x = maxsd(x, rf.min);
    x = minsd(x, rf.max);
    if rf.step > 0.0 {
        x = x - rf.min;
        x = x / rf.step;
        x = f_round(x) as f64;
        x = x * rf.step;
        x = x + rf.min;
        x = minsd(x, rf.max);
    }
    let p = POW10[rf_dec(rf)];
    x = x * p;
    x = f_round(x) as f64;
    x = x / p;
    x = maxsd(x, rf.min);
    x = minsd(x, rf.max);
    x + 0.0
}

/// rf_scaled(rf, v) -> round(v * 10^decimals), saturated to i32
pub(crate) fn rf_scaled(rf: &RangeF, v: f64) -> i32 {
    let r = f_round(v * POW10[rf_dec(rf)]);
    r.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// rf_t16(rf, v) -> (v - min) / (max - min) as 0..65536
pub(crate) fn rf_t16(rf: &RangeF, v: f64) -> i32 {
    let x = v - rf.min;
    let d = rf.max - rf.min;
    if !(d > 0.0) {
        return 0;
    }
    let r = f_round(x / d * 65536.0);
    if r < 0 {
        0
    } else if r > 65536 {
        65536
    } else {
        r as i32
    }
}

// rf_from_t16(rf, t) -> min + t / 65536 * (max - min), fixed with rf_fix
fn rf_from_t16(rf: &RangeF, t: i32) -> f64 {
    let mut x = t as f64 / 65536.0;
    let d = rf.max - rf.min;
    x = x * d;
    x = x + rf.min;
    rf_fix(rf, x)
}

/// deg_atan2(y, x) -> degrees in (-180, 180] (x87 fpatan)
pub fn deg_atan2(y: f64, x: f64) -> f64 {
    rt::x87::atan2(y, x) * RAD2DEG
}

/// deg_sincos(degrees) -> (cos, sin) (x87 fsincos)
pub fn deg_sincos(d: f64) -> (f64, f64) {
    rt::x87::sincos(d * DEG2RAD)
}

// utf8_encode(cp) -> the bytes of one code point (no checks, as the asm)
fn utf8_push(out: &mut Vec<u8>, c: u32) {
    if c < 0x80 {
        out.push(c as u8);
    } else if c < 0x800 {
        out.push(0xC0 | (c >> 6) as u8);
        out.push(0x80 | (c & 63) as u8);
    } else if c < 0x10000 {
        out.push(0xE0 | (c >> 12) as u8);
        out.push(0x80 | ((c >> 6) & 63) as u8);
        out.push(0x80 | (c & 63) as u8);
    } else {
        out.push(0xF0 | ((c >> 18) & 7) as u8);
        out.push(0x80 | ((c >> 12) & 63) as u8);
        out.push(0x80 | ((c >> 6) & 63) as u8);
        out.push(0x80 | (c & 63) as u8);
    }
}

/// fmt_f64 with an explicit separator (ui_fmt_f64)
pub(crate) fn fmt_f64_sep(v: f64, decimals: u32, sep: u32) -> String {
    let dec = decimals.min(6) as usize;
    let x = if v.is_nan() { 0.0 } else { v };
    let mut r = f_round(x * POW10[dec]) as u64;
    let mut out = Vec::with_capacity(32);
    if (r as i64) < 0 {
        out.push(b'-');
        r = (r as i64).wrapping_neg() as u64;
    }
    // digits, reversed: at least decimals + 1, at most 24
    let mut dig = [0u8; 32];
    let mut n = 0;
    loop {
        dig[n] = b'0' + (r % 10) as u8;
        r /= 10;
        n += 1;
        if n >= 24 {
            break;
        }
        if r != 0 || n <= dec {
            continue;
        }
        break;
    }
    let mut i = n;
    for _ in 0..n - dec {
        i -= 1;
        out.push(dig[i]);
    }
    if dec != 0 {
        utf8_push(&mut out, sep);
        while i > 0 {
            i -= 1;
            out.push(dig[i]);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// parse_f64 with an explicit separator (ui_parse_f64)
pub(crate) fn parse_f64_sep(s: &[u8], sep: u32) -> Option<f64> {
    let at = |i: usize| if i < s.len() { s[i] } else { 0 };
    let mut i = 0;
    while at(i) == b' ' {
        i += 1;
    }
    let mut neg = false;
    if at(i) == b'-' {
        neg = true;
        i += 1;
    } else if at(i) == b'+' {
        i += 1;
    }
    let mut m: u64 = 0; // mantissa
    let mut digits = 0; // digits seen
    let mut frac = 0usize; // fraction digits kept
    let mut dropped = 0usize; // integer digits dropped (too long)
    let mut seen_sep = false;
    loop {
        let c = at(i);
        if c.is_ascii_digit() {
            digits += 1;
            i += 1;
            if m >= 100_000_000_000_000_000 {
                if !seen_sep {
                    dropped += 1; // extra fraction digits: ignored
                }
            } else {
                m = m * 10 + (c - b'0') as u64;
                if seen_sep {
                    frac += 1;
                }
            }
        } else if c == b',' || c == b'.' || c as u32 == sep {
            if seen_sep {
                break;
            }
            seen_sep = true;
            i += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut x = m as i64 as f64;
    x = x / POW10[frac.min(18)];
    x = x * POW10[dropped.min(18)];
    if neg {
        x = 0.0 - x;
    }
    Some(x)
}

// ------------------------------------------------------------------ tracks and gradients

/// grad_at(track, t) -> colour (straight ARGB) of the gradient at t (0..65536)
pub fn grad_at(tk: &Track, t: i32) -> u32 {
    let n = tk.n.min(TK_MAXN as u32);
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return tk.stops[0];
    }
    let t = if t < 0 { 0 } else if t as u32 > 65536 { 65536 } else { t };
    let segs = n - 1;
    let pos = (t as u32).wrapping_mul(segs); // 16.16
    let mut seg = pos >> 16;
    let mut f = (pos & 0xFFFF) as i32;
    if seg >= segs {
        seg = segs - 1; // t = 1: the end of the last segment
        f = 0x10000;
    }
    let c0 = tk.stops[seg as usize];
    let c1 = tk.stops[seg as usize + 1];
    let mut r = 0u32;
    for sh in [0, 8, 16, 24] {
        let a = ((c0 >> sh) & 255) as i32;
        let b = ((c1 >> sh) & 255) as i32;
        let v = a + ((b - a).wrapping_mul(f) >> 16);
        r |= (v as u32) << sh;
    }
    r
}

/// px_mix(dst, src, k) -> per channel (dst * (255 - k) + src * k) / 255 of 0x..RRGGBB
/// (alpha byte 0)
pub fn px_mix(dst: u32, src: u32, k: u32) -> u32 {
    let ik = 255u32.wrapping_sub(k);
    let mut r = 0;
    for sh in [0, 8, 16] {
        let mut v = ((dst >> sh) & 255).wrapping_mul(ik).wrapping_add(((src >> sh) & 255).wrapping_mul(k)).wrapping_add(128);
        v = (v.wrapping_add(v >> 8)) >> 8;
        r |= v << sh;
    }
    r
}

// track_hash(track) -> 32-bit hash of the record (flags, n, stops): FNV-1a 64, folded
fn track_hash(tk: &Track) -> u32 {
    let n = tk.n.min(TK_MAXN as u32) as usize;
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut eat = |v: u32| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    eat(tk.flags);
    eat(tk.n);
    for &s in &tk.stops[..n] {
        eat(s);
    }
    (h as u32) ^ (h >> 32) as u32
}

/// the render callback of a gradient track (track_cb): the colours along the rectangle (the
/// long side), rounded antialiased ends unless TK_RECT, over a checkerboard with TK_CHECKER;
/// drawn straight into the target, inside its clip.  `cell` = the checker cell (dp(4) when
/// the command was recorded).
pub(crate) struct TrackDraw {
    tk: Track,
    cell: i32,
}

impl Draw for TrackDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let tk = &self.tk;
        let k = g.dc.clip;
        // clip = clip ∩ rect
        let cx0 = k[0].max(r[0]);
        let cy0 = k[1].max(r[1]);
        let cx1 = k[2].min(r[0].wrapping_add(r[2]));
        let cy1 = k[3].min(r[1].wrapping_add(r[3]));
        let flags = tk.flags;
        // geometry
        let vert = r[3] > r[2];
        let (thick, len) = if vert { (r[2], r[3]) } else { (r[3], r[2]) };
        let rad = thick as f32 * 0.5;
        let inset = if flags & TK_RECT != 0 { 0 } else { ((thick as u32) >> 1) as i32 };
        let travel = len.wrapping_sub(inset + inset).max(1);
        let cell = self.cell as u32;
        let stride = g.sf.stride as usize;
        let mut y = cy0;
        while y < cy1 {
            let row = y as usize * stride;
            let mut x = cx0;
            while x < cx1 {
                // u along the track, v across
                let (mut u, mut v) = (x.wrapping_sub(r[0]), y.wrapping_sub(r[1]));
                if vert {
                    let uu = r[3].wrapping_sub(1).wrapping_sub(v);
                    v = u;
                    u = uu;
                }
                // t of the column
                let t = ((((u.wrapping_sub(inset)) as i64) << 16) / travel as i64) as i32;
                let c = grad_at(tk, t);
                // coverage of the rounded ends (0..255)
                let mut cov: u32 = 255;
                let mut skip = false;
                if flags & TK_RECT == 0 {
                    let fu = u as f32 + 0.5;
                    let mut centre = rad;
                    let mut cap = !(fu >= rad); // jb: below or unordered
                    if !cap {
                        let e = len as f32 - rad;
                        if !(fu <= e) {
                            cap = true;
                            centre = e;
                        }
                    }
                    if cap {
                        let du = fu - centre;
                        let dv = v as f32 + 0.5 - rad;
                        let d = rt::fx::sqrt(du * du + dv * dv);
                        let kk = cvtt((rad + 0.5 - d) * 255.0);
                        if kk <= 0 {
                            skip = true;
                        } else {
                            cov = kk.min(255) as u32;
                        }
                    }
                }
                if !skip {
                    let a = c >> 24;
                    let (src, kk) = if flags & TK_CHECKER != 0 {
                        // checker cell colour, then the gradient over it
                        let odd = ((u as u32) / cell).wrapping_add((v as u32) / cell) & 1;
                        let bg = if odd != 0 { 0xCC_CCCC } else { 0xFF_FFFF };
                        (px_mix(bg, c, a), cov)
                    } else {
                        (c, (a * cov + 127) / 255)
                    };
                    let o = row + x as usize * 4;
                    let px = u32::from_le_bytes([g.sf.px[o], g.sf.px[o + 1], g.sf.px[o + 2], g.sf.px[o + 3]]);
                    let out = px_mix(px, src, kk);
                    g.sf.px[o..o + 4].copy_from_slice(&out.to_le_bytes());
                }
                x += 1;
            }
            y += 1;
        }
    }
}

impl Ui {
    /// cmd_custom of a value slider's gradient track over (x, y, w, h): the track's own
    /// drawing (TK_CUSTOM) or the gradient (track_cb, version = track_hash)  [styles' value.s]
    pub fn cmd_track(&mut self, x: i32, y: i32, w: i32, h: i32, tk: &Track) {
        if tk.flags & TK_CUSTOM != 0 {
            let (f, user, ver) = match &tk.custom {
                Some((f, u, v)) => (Some(f.clone()), *u, *v),
                None => (None, 0, 0),
            };
            self.cmd_custom(x, y, w, h, f, user, ver);
            return;
        }
        let ver = track_hash(tk);
        let f = self.track_drawing(tk);
        self.cmd_custom(x, y, w, h, Some(f), 0, ver);
    }

    // the drawing of a gradient (trk_alloc + track_cb): one shared object per content and
    // checker cell, so an unchanged track keeps its command hash
    fn track_drawing(&mut self, tk: &Track) -> DrawRef {
        let cell = self.dp(4).max(1);
        let mut t = tk.clone();
        t.custom = None;
        let key = ((track_hash(&t) as u64) << 32) | cell as u32 as u64;
        let s = &mut self.m.value;
        let same = |d: &TrackDraw| d.cell == cell && d.tk.flags == t.flags && d.tk.n == t.n && d.tk.stops == t.stops;
        if let Some((_, d)) = s.trk.iter().find(|(k, d)| *k == key && same(d)) {
            return d.clone();
        }
        let d = Rc::new(TrackDraw { tk: t, cell });
        if s.trk.len() < TRK_POOL {
            s.trk.push((key, d.clone()));
        } else {
            let i = s.trk_next % TRK_POOL;
            s.trk[i] = (key, d.clone());
            s.trk_next = i + 1;
        }
        d
    }

    // ------------------------------------------------------------------ decimal separator

    /// ui_decimal_sep: the separator fmt_f64 shows (and parse_f64 accepts besides ',' / '.')
    pub fn decimal_sep(&self) -> u32 {
        self.m.value.decimal_sep
    }

    /// set ui_decimal_sep (a code point)
    pub fn set_decimal_sep(&mut self, c: u32) {
        self.m.value.decimal_sep = c;
    }

    /// ui_fmt_f64(&v, decimals) -> fixed decimals with the decimal separator; -0 is shown as 0
    pub fn fmt_f64(&self, v: f64, decimals: u32) -> String {
        fmt_f64_sep(v, decimals, self.m.value.decimal_sep)
    }

    /// ui_parse_f64(s) -> the value, or None when s has no digits.  Blanks, a sign, digits,
    /// one ',' or '.' (or the decimal separator when ASCII), digits.
    pub fn parse_f64(&self, s: &str) -> Option<f64> {
        parse_f64_sep(s.as_bytes(), self.m.value.decimal_sep)
    }

    // ------------------------------------------------------------------ slider building blocks

    // vl_disabled: the last behave reported a disabled widget
    fn vl_disabled(&self) -> bool {
        self.wp.ctx.last_state == SS_DISABLED as u32
    }

    /// sld_place(rgflags): place a value slider at the style's natural size
    pub(crate) fn sld_place(&mut self, rgflags: u32) {
        let st = self.style;
        let mut sp = Sp { flags: rgflags, ..Sp::default() };
        let (w, h) = st.value_slider_size(self, &mut sp);
        self.place(w, h);
    }

    /// sld_behave(id, stflags) -> EV: behave with focus and hand cursor (ST_DISABLED,
    /// ST_NOFOCUS honoured)
    pub(crate) fn sld_behave(&mut self, id: u32, stflags: u32) -> u32 {
        let mut hb = HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT;
        if stflags & ST_NOFOCUS != 0 {
            hb &= !HB_FOCUS;
        }
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        self.behave(id, hb)
    }

    /// sld_ptr_t(rgflags) -> 0..65536: the pointer's place on the thumb travel
    pub(crate) fn sld_ptr_t(&mut self, rgflags: u32) -> i32 {
        let st = self.style;
        let thumb = st.value_slider_thumb(self);
        let half = ((thumb as u32) >> 1) as i32;
        let r = self.wp.ctx.last_rect;
        let (mx, my) = (self.wp.ctx.mx, self.wp.ctx.my);
        let (mut p, len) = if rgflags & RG_VERT == 0 {
            (mx.wrapping_sub(r[0]).wrapping_sub(half), r[2])
        } else {
            (r[1].wrapping_add(r[3]).wrapping_sub(half).wrapping_sub(my), r[3])
        };
        let len = len.wrapping_sub(thumb).max(1);
        if p < 0 {
            p = 0;
        }
        if p > len {
            p = len;
        }
        (((p as i64) << 16) / len as i64) as i32
    }

    /// sld_keys() -> (signed steps (arrows 1, PgUp / PgDn 10), 0 / 1 Home / 2 End)
    pub(crate) fn sld_keys(&mut self) -> (i32, u32) {
        let mut steps = 0i32;
        steps += self.key(VK_RIGHT) as i32;
        steps += self.key(VK_UP) as i32;
        steps -= self.key(VK_LEFT) as i32;
        steps -= self.key(VK_DOWN) as i32;
        steps += self.key(VK_PRIOR) as i32 * 10;
        steps -= self.key(VK_NEXT) as i32 * 10;
        let mut he = 0;
        if self.key(VK_HOME) {
            he = 1;
        }
        if self.key(VK_END) {
            he = 2;
        }
        if steps != 0 || he != 0 {
            self.wp.ctx.focus_visible = true;
        }
        (steps, he)
    }

    /// sld_draw(t, state, rgflags, track): the style paints track and thumb of a value slider
    /// on last_rect; t = 0..65536 along the thumb travel; None = the standard accent track
    pub(crate) fn sld_draw(&mut self, t: i32, state: u32, rgflags: u32, track: Option<&Track>) {
        let st = self.style;
        let mut sp = Sp { value: t, state, flags: rgflags, track, ..Sp::default() };
        sp.place(self.wp.ctx.last_rect);
        st.value_slider(self, &sp);
    }

    // ------------------------------------------------------------------ slider_f

    /// ui_slider_f(id, &mut v, &rf) -> EV_*
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn slider_f(&mut self, id: u32, v: &mut f64, rf: &RangeF) -> u32 {
        let _src = self.src_scope();
        let old = v.to_bits();
        let stflags = self.take_flags();
        let rgf = rf.flags;
        let mut keych = false;
        self.sld_place(rgf);
        let mut ev = self.sld_behave(id, stflags);
        if ev & EV_HELD != 0 {
            let t = self.sld_ptr_t(rgf);
            *v = rf_from_t16(rf, t);
        } else if ev & EV_FOCUS != 0 {
            let (steps, he) = self.sld_keys();
            if he == 1 {
                *v = rf.min;
                keych = true;
            } else if he == 2 {
                *v = rf.max;
                keych = true;
            } else if steps != 0 {
                keych = true;
                // step (0: 1 % of the range)
                let st = if rf.step > 0.0 { rf.step } else { (rf.max - rf.min) * 0.01 };
                *v = steps as f64 * st + *v;
            }
        }
        // a value a screen reader set (a11y.rs; scaled like the record's value)
        if let Some(x) = self.a11y_value(id) {
            *v = x as f64 / POW10[rf_dec(rf)];
            keych = true;
        }
        *v = rf_fix(rf, *v);
        if v.to_bits() != old {
            ev |= EV_ACT;
            if keych {
                ev |= EV_COMMIT;
            }
            self.repass();
        }
        let t = rf_t16(rf, *v);
        let state = vl_state(ev, self.vl_disabled());
        self.sld_draw(t, state, rgf, None);
        self.focus_ring_draw(ev);
        let val = rf_scaled(rf, *v);
        let r = self.sem(K_SLIDER, id, None, val);
        r.max = rf_scaled(rf, rf.max);
        r.min = rf_scaled(rf, rf.min);
        ev
    }

    // ------------------------------------------------------------------ track_slider

    /// ui_track_slider(id, &mut v, &rg, track) -> EV_*: i32 slider with a gradient track
    /// (None = the standard look)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn track_slider(&mut self, id: u32, v: &mut i32, rg: &Range, track: Option<&Track>) -> u32 {
        let _src = self.src_scope();
        let old = *v;
        let stflags = self.take_flags();
        let rgf = rg.flags;
        let mut keych = false;
        self.sld_place(rgf);
        let mut ev = self.sld_behave(id, stflags);
        if ev & EV_HELD != 0 {
            let t = self.sld_ptr_t(rgf);
            // v = min + round(t * (max - min) / 65536), snapped to min + k * step
            let span = rg.max.wrapping_sub(rg.min) as i64;
            let mut x = ((t as i64).wrapping_mul(span).wrapping_add(32768) >> 16) as i32;
            x = x.wrapping_add(rg.min);
            if rg.step > 1 {
                let half = ((rg.step as u32) >> 1) as i32;
                x = (x.wrapping_sub(rg.min).wrapping_add(half) / rg.step).wrapping_mul(rg.step).wrapping_add(rg.min);
            }
            *v = x;
        } else if ev & EV_FOCUS != 0 {
            let (steps, he) = self.sld_keys();
            if he == 1 {
                *v = rg.min;
                keych = true;
            } else if he == 2 {
                *v = rg.max;
                keych = true;
            } else if steps != 0 {
                keych = true;
                let st = rg.step.max(1);
                *v = v.wrapping_add(steps.wrapping_mul(st));
            }
        }
        // a value a screen reader set (a11y.rs)
        if let Some(x) = self.a11y_value(id) {
            *v = x;
            keych = true;
        }
        let mut x = *v;
        if x < rg.min {
            x = rg.min;
        }
        if x > rg.max {
            x = rg.max;
        }
        *v = x;
        if x != old {
            ev |= EV_ACT;
            if keych {
                ev |= EV_COMMIT;
            }
            self.repass();
        }
        // t = (v - min) * 65536 / (max - min)
        let span = rg.max.wrapping_sub(rg.min);
        let t: i32 = if span > 0 { ((((v.wrapping_sub(rg.min)) as i64) << 16) / span as i64) as i32 } else { 0 };
        let state = vl_state(ev, self.vl_disabled());
        self.sld_draw(t, state, rgf, track);
        self.focus_ring_draw(ev);
        let r = self.sem(K_SLIDER, id, None, *v);
        r.max = rg.max;
        r.min = rg.min;
        ev
    }

    // ------------------------------------------------------------------ property rows

    // prop_begin(label) -> the label of the text record (for the slider): the row's boxes,
    // BOX_V (label, 4 DIP) and a BOX_H with centred children (8 DIP)
    fn prop_begin(&mut self, label: &str) -> Option<String> {
        self.box_begin(0, BOX_V, 4);
        self.text(TS_BODY, label);
        let l = self.sems.last().and_then(|s| s.label.clone());
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_size(-1, 0);
        l
    }

    // set_last_label(label): give the last semantic record this label
    fn set_last_label(&mut self, l: Option<String>) {
        if let Some(s) = self.sems.last_mut() {
            s.label = l;
        }
    }

    // prop_reset(id, same) -> EV of the reset button (disabled while the value is the default)
    fn prop_reset(&mut self, id: u32, same: bool) -> u32 {
        if same {
            self.next_flags(ST_DISABLED);
        }
        self.icon_button(part_id(id, 2), IC_UNDO, crate::i18n::tr("Zurücksetzen"))
    }

    /// ui_prop_slider(id, label, &mut v, &rg, default) -> EV_* (default NO_DEFAULT: no reset
    /// button)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn prop_slider(&mut self, id: u32, label: &str, v: &mut i32, rg: &Range, default: i32) -> u32 {
        let _src = self.src_scope();
        let l = self.prop_begin(label);
        let mut ev = self.slider(id, v, rg);
        self.set_last_label(l);
        self.next_size(PROP_NB_W, 0);
        let nb = part_id(id, 1);
        ev |= self.number_box(nb, v, rg);
        if self.nb_live(nb, v) {
            // typing previews the value
            ev |= EV_ACT;
            self.repass();
        }
        if default != NO_DEFAULT {
            let same = *v == default;
            if self.prop_reset(id, same) & EV_ACT != 0 {
                *v = default;
                ev |= EV_ACT | EV_COMMIT;
                self.repass();
            }
        }
        self.box_end();
        self.box_end();
        ev
    }

    /// ui_prop_slider_f(id, label, &mut v, &rf, default) -> EV_* (default None: no reset
    /// button)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn prop_slider_f(&mut self, id: u32, label: &str, v: &mut f64, rf: &RangeF, default: Option<f64>) -> u32 {
        let _src = self.src_scope();
        let l = self.prop_begin(label);
        let mut ev = self.slider_f(id, v, rf);
        self.set_last_label(l);
        self.next_size(PROP_NB_W, 0);
        let nb = part_id(id, 1);
        ev |= self.number_box_f(nb, v, rf);
        if self.nb_live_f(nb, v) {
            // typing previews the value
            ev |= EV_ACT;
            self.repass();
        }
        if let Some(d) = default {
            let d = rf_fix(rf, d); // the default as the range shows it
            let same = v.to_bits() == d.to_bits();
            if self.prop_reset(id, same) & EV_ACT != 0 {
                *v = d;
                ev |= EV_ACT | EV_COMMIT;
                self.repass();
            }
        }
        self.box_end();
        self.box_end();
        ev
    }

    // ------------------------------------------------------------------ angle dial

    /// ui_angle(id, &mut deg, flags) -> EV_*
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn angle(&mut self, id: u32, deg: &mut f64, flags: u32) -> u32 {
        let _src = self.src_scope();
        let old = deg.to_bits();
        let mut keych = false;
        let stflags = self.take_flags();
        let st = self.style;
        let mut sp = Sp::default();
        let (w, h) = st.angle_size(self, &mut sp);
        self.place(w, h);
        let mut ev = self.sld_behave(id, stflags);
        // geometry: circle in the centre of the placed rectangle (right-to-left: angles stay)
        let r = self.wp.ctx.last_rect;
        self.ltr_begin(r[0], r[2]);
        let cx = (r[0] << 8).wrapping_add(r[2] << 7);
        let cy = (r[1] << 8).wrapping_add(r[3] << 7);
        if ev & EV_HELD != 0 {
            // pointer: atan2(cy - y, x - cx)
            let dx = (self.ltr_x(self.wp.ctx.mx) << 8).wrapping_sub(cx);
            let dy = cy.wrapping_sub(self.wp.ctx.my << 8);
            if dx | dy != 0 {
                let mut a = deg_atan2(dy as f64, dx as f64);
                if self.mods & MOD_SHIFT != 0 {
                    a = f_round(a / 15.0) as f64 * 15.0;
                }
                *deg = a;
            }
        } else if ev & EV_FOCUS != 0 {
            let (steps, he) = self.sld_keys();
            if he == 1 {
                *deg = 0.0; // Home: 0
                keych = true;
            } else if steps != 0 {
                keych = true;
                // arrows: 1 degree (Shift 15); PgUp / PgDn: 15 degrees (sld_keys reports 10)
                let x = if steps >= 10 || steps <= -10 {
                    ((steps / 10).wrapping_mul(15)) as f64
                } else if self.mods & MOD_SHIFT != 0 {
                    steps.wrapping_mul(15) as f64
                } else {
                    steps as f64
                };
                *deg = x + *deg;
            }
        }
        *deg = ang_norm(*deg, flags);
        if deg.to_bits() != old {
            ev |= EV_ACT;
            if keych {
                ev |= EV_COMMIT;
            }
            self.repass();
        }
        let state = vl_state(ev, self.vl_disabled());
        // paint (style): rectangle, state, angle, focus visual
        sp.place(self.wp.ctx.last_rect);
        sp.state = state;
        sp.ev = ev;
        sp.f = *deg;
        sp.s[0] = self.focus_ring(ev) as i32;
        st.angle(self, &sp);
        self.ltr_end();
        let v = f_round(*deg * 100.0) as i32;
        self.sem(K_SLIDER, id, None, v).max = if flags & ANG_360 != 0 { 36000 } else { 18000 };
        ev
    }

    // ------------------------------------------------------------------ pan control

    /// ui_pan(id, &mut x, &mut y, thumb, ver, flags) -> EV_* (thumb: a surface or None; ver
    /// stands for its version, as in draw_image)
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn pan(&mut self, id: u32, x: &mut f64, y: &mut f64, thumb: Option<&Rc<Surface>>, ver: u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let (ox, oy) = (x.to_bits(), y.to_bits());
        let mut keych = false;
        let stflags = self.take_flags();
        // range
        let (lo, span) = if flags & PAN_UNIT != 0 { (0.0, 1.0) } else { (-1.0, 2.0) };
        let st = self.style;
        let mut sp = Sp { img: thumb, img_ver: ver, ..Sp::default() };
        let (w, h) = st.pan_size(self, &mut sp);
        self.place(w, h);
        let mut ev = self.sld_behave(id, stflags);
        let frac = |p: i32, len: i32| {
            let len = len.max(1);
            let mut f = p as f64 / len as f64;
            f = maxsd(f, 0.0);
            f = minsd(f, 1.0);
            f = f * span;
            f + lo
        };
        let r = self.wp.ctx.last_rect;
        self.ltr_begin(r[0], r[2]); // right-to-left: x keeps its direction
        if ev & EV_HELD != 0 {
            // pointer -> fraction of the rectangle -> lo + f * span
            let r = self.wp.ctx.last_rect;
            *x = frac(self.ltr_x(self.wp.ctx.mx).wrapping_sub(r[0]), r[2]);
            *y = frac(self.wp.ctx.my.wrapping_sub(r[1]), r[3]);
        } else if ev & EV_FOCUS != 0 {
            // arrows: 1 % of the range (Shift 10 %), Home: centre
            let mut dx = 0i32;
            let mut dy = 0i32;
            // right-to-left: the queue has the arrows swapped, the pad is not mirrored
            let (right, left) = if self.rtl { (VK_LEFT, VK_RIGHT) } else { (VK_RIGHT, VK_LEFT) };
            dx += self.key(right) as i32;
            dx -= self.key(left) as i32;
            dy += self.key(VK_DOWN) as i32;
            dy -= self.key(VK_UP) as i32;
            if self.key(VK_HOME) {
                let c = span * 0.5 + lo;
                *x = c;
                *y = c;
                keych = true;
                self.wp.ctx.focus_visible = true;
            } else if dx | dy != 0 {
                keych = true;
                self.wp.ctx.focus_visible = true;
                let mut st = span * 0.01;
                if self.mods & MOD_SHIFT != 0 {
                    st = st * 10.0;
                }
                *x = dx as f64 * st + *x;
                *y = dy as f64 * st + *y;
            }
        }
        // clamp to lo .. lo + span, 4 decimals
        let clamp = |v: f64| {
            let mut v = if v.is_nan() { lo } else { v };
            v = v * 10000.0;
            v = f_round(v) as f64;
            v = v / 10000.0;
            v = maxsd(v, lo);
            v = minsd(v, lo + span);
            v + 0.0
        };
        *x = clamp(*x);
        *y = clamp(*y);
        if x.to_bits() != ox || y.to_bits() != oy {
            ev |= EV_ACT;
            if keych {
                ev |= EV_COMMIT;
            }
            self.repass();
        }
        let state = vl_state(ev, self.vl_disabled());
        // the point in px
        let r = self.wp.ctx.last_rect;
        let px = cvttd((*x - lo) / span * r[2] as f64).wrapping_add(r[0]);
        let py = cvttd((*y - lo) / span * r[3] as f64).wrapping_add(r[1]);
        // paint (style)
        sp.place(r);
        sp.state = state;
        sp.value = px;
        sp.value2 = py;
        st.pan(self, &sp);
        self.focus_ring_draw(ev);
        self.ltr_end();
        // semantic record: value = x * 1000, max = y * 1000
        let my = f_round(*y * 1000.0) as i32;
        let mx = f_round(*x * 1000.0) as i32;
        self.sem(K_SLIDER, id, None, mx).max = my;
        ev
    }
}

// ang_norm(v, flags) -> the angle in [-180, 180] (outside values wrap into (-180, 180];
// ANG_360: [0, 360))
fn ang_norm(v: f64, flags: u32) -> f64 {
    let x = if v.is_nan() { 0.0 } else { v };
    let a360 = flags & ANG_360 != 0;
    // -180 .. 180 (both ends) stay as they are (a slider on the same value may set -180);
    // only values outside wrap into (-180, 180]
    if !a360 && x >= -180.0 && x <= 180.0 {
        return x + 0.0;
    }
    // k = floor((v - lo) / 360), v -= 360 k   (lo = -180 or 0)
    let lo = if a360 { 0.0 } else { -180.0 };
    let mut k = x - lo;
    k = k / 360.0;
    k = f_floor(k);
    k = k * 360.0;
    let mut y = x - k; // in [lo, lo + 360)
    if !a360 && !(y > -180.0) {
        y = 180.0; // (-180, 180]: -180 becomes 180
    }
    y + 0.0
}
