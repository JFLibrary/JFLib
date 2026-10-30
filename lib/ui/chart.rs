// chart -- small charts for dashboards: sparkline, bar chart (grouped or stacked), donut,
// legend, and line charts of time series (line_chart over SeriesBuf rings).  Colours: 0 =
// the style's chart palette (StyleData::chart_palette) by index.
//
//   sparkline(id, values, color)          area + line, natural 120 x 32 DIP (next_size)
//   bar_chart(id, labels, series, stacked) -> hovered column or -1
//                                         value grid with labels, one column per label,
//                                         natural 360 x 180 DIP; hovering a column shows a
//                                         tooltip with its values.  Labels never overlap:
//                                         where they do not fit side by side every k-th one
//                                         is shown (the first and the last kept; a text
//                                         record each), a low plot gets fewer value steps
//   donut(id, slices, center, sub)        ring of slices with a centre text, natural 160 DIP
//                                         (texts kept inside the hole)
//   legend(items)                         colour dot + label per item, wrapping row (an
//                                         item wider than the row: its label cut with "…")
//   line_chart(id, &LineChart) -> Option<(series, x)>
//        series of (x, value) points (LcSeries over a SeriesBuf: label, colour or palette
//        index, line or area, LcUnit) over time -- LX_SECS seconds or LX_NANOS nanoseconds,
//        ticks 0:05 / 1:00 / 1:30 (h:mm:ss from an hour on, decimals of the second below
//        one) -- or plain numbers (LX_NUM).  Natural 480 x 200 DIP for the plot with its
//        axes, the legend (legend(), the series' labels) below it; next_size sizes both.
//        The y axis: "nice" ticks (1 / 2 / 2.5 / 5 x 10^k) labelled in the first series'
//        unit; y_range fixed or automatic per end (NaN: from the visible values and the
//        thresholds, out to the next tick).  The x range: x_range, else the data's --
//        follow live with x_span (the newest x_span; LineChart::view() is what it shows).
//        Grid lines at the ticks; markers (LcMarker: a line or a shaded range with a short
//        label in a tag at the top: fault injections) and thresholds (LcThreshold: a dashed
//        line with its label in a tag at its right end).  Tags never overlap: a marker's tag
//        takes the first of up to three rows with room, else it is left out (the hover card
//        still names the markers under the pointer).  Hovering the plot: a crosshair at the
//        nearest point, a dot on every series and a card beside it at once (the style's
//        tooltip surface) with the x and every series' value there; the return value is the
//        series nearest to the pointer and that x.  Keys (the plot takes the focus: Tab, a
//        click puts the cursor on the point under the pointer): Left / Right the previous /
//        next point of the cursor's series, Up / Down another series, Home / End, Esc ends
//        it; crosshair and card show the cursor, the record's label starts with its point.
//        Up to two points per pixel column the points themselves are drawn, more as one
//        band per series (per column the range of its values, SeriesBuf::columns), so
//        100 000 points draw like 500.  No points at all: LineChart::empty in the middle
//        ("Keine Daten" in the UI language).  High
//        contrast: the scheme's highlight, text and link colours with a dash pattern per
//        series (the legend shows line samples), no translucent fills.  The plot keeps its
//        left-to-right order in mirrored windows (ltr_begin); the legend is mirrored.
//        Records: K_CANVAS id, its label a summary (the name, then per series its last
//        value, min. and max. in the view; value: the number of series); K_TEXT per axis
//        label, tag and hover card row ("p95: 48 ms").  Flags LC_NO_LEGEND, LC_NO_GRID,
//        LC_NO_HOVER.
//   SeriesBuf::new(capacity)              the ring a series draws from: push(t, v) (times
//                                         never decrease; a value that is not finite is a
//                                         gap), clear, len / get / first / last / iter,
//                                         lower_bound(t), nearest(t), min_max /
//                                         min_max_in(t0, t1), columns(t0, t1, n) (per pixel
//                                         column min / max / first / last), ver()
//   LcUnit::Num(sfx) / Si(unit) / Secs    how values read: text(v, sep) ("48,2 ms", "1,25k
//                                         req/s", "1,5 MB"), ticks(first, step, n, sep)
//   lc_nice(lo, hi, steps), lc_time_step(min), lc_time_text(t, dec, hours, sep)
//                                         the axis maths: tick steps, clock labels
//
// Area, line and ring are vector paths filled at render time (cmd_custom); their drawing
// object is kept per id and replaced only when the data or the size change, so an unchanged
// chart does not redraw (dirty.rs hashes the object's identity and version).  A line chart
// keeps its statistics, summary and plot per id, keyed by the series' versions and the
// view; its crosshair, dots and card are ordinary commands over the plot.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::i18n::tr;
use crate::keys::*;
use crate::stock::{card_opaque, sincos, text_at};
use crate::style::Sp;
use crate::theme::color_mix;
use crate::value::fmt_f64_sep;
use crate::Ui;
use gfx::dash::{dash_pattern, dash_stroke, DStyle, DASH_DASH, DASH_DASHDOT, DASH_DOT, DASH_SOLID, LCAP_FLAT};
use gfx::path::Path;

/// one series of a bar chart
#[derive(Clone, Copy, Debug)]
pub struct BarSeries<'a> {
    pub label: &'a str,
    pub values: &'a [f32],
    /// 0xAARRGGBB, 0 = palette colour of the series index
    pub color: u32,
}

/// one slice of a donut
#[derive(Clone, Copy, Debug)]
pub struct DonutSlice<'a> {
    pub label: &'a str,
    pub value: f32,
    /// 0xAARRGGBB, 0 = palette colour of the slice index
    pub color: u32,
}

/// shared state: the drawing objects of the charts by id (hash, object), the line charts'
/// states by id
#[derive(Default)]
pub(crate) struct S {
    cache: BTreeMap<u32, (u64, DrawRef)>,
    lines: BTreeMap<u32, LcState>,
}

struct Fnv(u64);
impl Fnv {
    fn new() -> Fnv {
        Fnv(0xCBF2_9CE4_8422_2325)
    }
    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    fn u64(&mut self, v: u64) {
        self.u32(v as u32);
        self.u32((v >> 32) as u32);
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn str(&mut self, s: &str) {
        for b in s.bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
        self.u32(s.len() as u32);
    }
}

// the line + area of a sparkline (points in 0..1 of the rectangle)
struct Spark {
    pts: Vec<(f32, f32)>,
    line: u32,
    area: u32,
    width: i32,
}

impl Draw for Spark {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        if self.pts.len() < 2 {
            return;
        }
        let (x0, y0, w, h) = (r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32);
        let m = self.width as f32 / 256.0;
        let px = |p: (f32, f32)| (x0 + p.0 * w, y0 + m + (1.0 - p.1) * (h - m * 2.0));
        // area down to the bottom
        let path = &mut g.text.path;
        path.reset();
        path.set_origin(0, 0);
        let (sx, _) = px(self.pts[0]);
        path.move_f(sx, y0 + h);
        for &p in &self.pts {
            let (x, y) = px(p);
            path.line_f(x, y);
        }
        let (ex, _) = px(self.pts[self.pts.len() - 1]);
        path.line_f(ex, y0 + h);
        path.close();
        g.dc.set_color(self.area);
        path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
        // the line
        let path = &mut g.text.path;
        path.reset();
        path.set_origin(0, 0);
        for (i, &p) in self.pts.iter().enumerate() {
            let (x, y) = px(p);
            if i == 0 {
                path.move_f(x, y);
            } else {
                path.line_f(x, y);
            }
        }
        g.dc.set_color(self.line);
        path.stroke(&mut g.text.ras, g.dc, g.sf, self.width, gfx::JOIN_ROUND | gfx::CAP_ROUND << 4);
    }
}

// the slices of a donut: (start fraction, end fraction, colour), ring thickness 0..1 of the
// radius, the track colour
struct Ring {
    slices: Vec<(f32, f32, u32)>,
    thick: f32,
    track: u32,
    gap: f32,
}

fn ring_path(g: &mut Gfx, cx: f32, cy: f32, ro: f32, ri: f32, a0: f32, a1: f32) {
    let path = &mut g.text.path;
    path.reset();
    path.set_origin(0, 0);
    let span = a1 - a0;
    let n = ((span * 48.0) as i32).clamp(2, 96);
    let tau = core::f32::consts::TAU;
    for k in 0..=n {
        let a = (a0 + span * k as f32 / n as f32) * tau - tau / 4.0;
        let (s, c) = sincos(a);
        if k == 0 {
            path.move_f(cx + c * ro, cy + s * ro);
        } else {
            path.line_f(cx + c * ro, cy + s * ro);
        }
    }
    for k in (0..=n).rev() {
        let a = (a0 + span * k as f32 / n as f32) * tau - tau / 4.0;
        let (s, c) = sincos(a);
        path.line_f(cx + c * ri, cy + s * ri);
    }
    path.close();
}

impl Draw for Ring {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let d = r[2].min(r[3]) as f32;
        let cx = r[0] as f32 + r[2] as f32 / 2.0;
        let cy = r[1] as f32 + r[3] as f32 / 2.0;
        let ro = d / 2.0 - 1.0;
        let ri = ro * (1.0 - self.thick);
        ring_path(g, cx, cy, ro, ri, 0.0, 1.0);
        g.dc.set_color(self.track);
        g.text.path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
        for &(a0, a1, c) in &self.slices {
            let (a0, a1) = if self.slices.len() > 1 { (a0 + self.gap / 2.0, a1 - self.gap / 2.0) } else { (a0, a1) };
            if a1 <= a0 {
                continue;
            }
            ring_path(g, cx, cy, ro, ri, a0, a1);
            g.dc.set_color(c);
            g.text.path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
        }
    }
}

/// a short number for axis labels: 0, 12, 1.5k, 2.3M
fn short_num(v: f32) -> String {
    let a = if v < 0.0 { -v } else { v };
    if a >= 1.0e9 {
        format!("{:.1}G", v / 1.0e9)
    } else if a >= 1.0e6 {
        format!("{:.1}M", v / 1.0e6)
    } else if a >= 1.0e4 {
        format!("{:.0}k", v / 1.0e3)
    } else if a >= 1.0e3 {
        format!("{:.1}k", v / 1.0e3)
    } else if a == (a as i64) as f32 {
        format!("{}", v as i64)
    } else {
        format!("{:.1}", v)
    }
}

/// a "nice" axis maximum and step count (at most maxn, 1 ..= 5) for the largest value
fn nice_max(m: f32, maxn: i32) -> (f32, i32) {
    if m <= 0.0 {
        return (4.0, 4.min(maxn));
    }
    let mut p = 1.0f32;
    while p * 10.0 <= m {
        p *= 10.0;
    }
    while p > m {
        p /= 10.0;
    }
    for f in [1.0f32, 2.0, 2.5, 5.0, 10.0] {
        let step = p * f / 2.0;
        let mut n = (m / step) as i32;
        if (n as f32) * step < m {
            n += 1;
        }
        if n <= maxn {
            return (step * n.max(1) as f32, n.max(1));
        }
    }
    (p * 10.0, 4.min(maxn))
}

/// the column labels shown (index, x) when every k-th one is: the first, every k-th after
/// it and the last (in place of the k-th one before it), centred under their column (x0 +
/// cw * i) but kept in lo .. hi; None when two of them come closer than sep
fn axis_labels(ws: &[i32], x0: i32, cw: i32, lo: i32, hi: i32, k: usize, sep: i32) -> Option<Vec<(usize, i32)>> {
    let n = ws.len();
    let mut cols: Vec<usize> = (0..n).step_by(k).collect();
    if n > 1 && cols.last() != Some(&(n - 1)) {
        if cols.len() > 1 {
            cols.pop();
        }
        cols.push(n - 1);
    }
    let mut out: Vec<(usize, i32)> = Vec::with_capacity(cols.len());
    for i in cols {
        let x = (x0 + cw * i as i32 + ((cw - ws[i]) >> 1)).min(hi - ws[i]).max(lo);
        if let Some(&(j, px)) = out.last() {
            if x < px + ws[j] + sep {
                return None;
            }
        }
        out.push((i, x));
    }
    Some(out)
}

// ------------------------------------------------------------------ time series

static SERIES_VER: AtomicU32 = AtomicU32::new(1);

// a version for a changed series: one counter for every SeriesBuf, so two buffers (or a
// buffer and its clone after either changed) never share one
fn series_ver() -> u32 {
    SERIES_VER.fetch_add(1, Ordering::Relaxed)
}

/// a time series for line_chart: (t, value) points in a ring that keeps the newest
/// `capacity` (a full buffer drops its oldest point on push).  Times never decrease (push
/// refuses an earlier one, so every lookup is a binary search); a value that is not finite
/// (NaN) is a gap where the line breaks.  ver() changes with every change: a chart redraws
/// only then.
#[derive(Clone, Debug)]
pub struct SeriesBuf {
    pts: VecDeque<(f64, f64)>,
    cap: usize,
    ver: u32,
}

/// one pixel column of a downsampled series (SeriesBuf::columns): the points with a finite
/// value whose time falls into it
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesCol {
    /// the column (0 .. n)
    pub col: u32,
    /// its points with a value
    pub n: u32,
    pub min: f64,
    pub max: f64,
    /// the values of its first and its last point
    pub first: f64,
    pub last: f64,
    /// a gap came before it (after the previous point with a value inside t0 .. t1): the
    /// line breaks there; a gap inside a column starts a second entry for it
    pub gap: bool,
}

// the smallest and the largest finite value of points (slices: plain loops, the fast path)
fn min_max_of(parts: [&[(f64, f64)]; 2]) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for part in parts {
        for &(_, v) in part {
            if v.is_finite() {
                if v < lo {
                    lo = v;
                }
                if v > hi {
                    hi = v;
                }
            }
        }
    }
    if lo <= hi { Some((lo, hi)) } else { None }
}

impl SeriesBuf {
    /// an empty series that keeps the newest `capacity` points (at least one)
    pub fn new(capacity: usize) -> SeriesBuf {
        SeriesBuf { pts: VecDeque::new(), cap: capacity.max(1), ver: series_ver() }
    }

    /// append a point (t: the time, or any x that never decreases; v NaN: a gap); false
    /// (nothing added) when t is not finite or earlier than the last point's
    pub fn push(&mut self, t: f64, v: f64) -> bool {
        if !t.is_finite() || self.pts.back().is_some_and(|p| t < p.0) {
            return false;
        }
        let n = self.pts.len();
        if n >= self.cap {
            self.pts.pop_front();
        } else if n == self.pts.capacity() {
            // grow towards the capacity, never beyond it
            self.pts.reserve_exact(n.max(16).min(self.cap - n));
        }
        self.pts.push_back((t, v));
        self.ver = series_ver();
        true
    }

    /// remove every point
    pub fn clear(&mut self) {
        self.pts.clear();
        self.ver = series_ver();
    }

    /// the number of points
    pub fn len(&self) -> usize {
        self.pts.len()
    }

    /// no points
    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// the most points it keeps
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// changes with every push and clear (equal versions: equal points)
    pub fn ver(&self) -> u32 {
        self.ver
    }

    /// the i-th point, the oldest first
    pub fn get(&self, i: usize) -> Option<(f64, f64)> {
        self.pts.get(i).copied()
    }

    /// the oldest point
    pub fn first(&self) -> Option<(f64, f64)> {
        self.pts.front().copied()
    }

    /// the newest point
    pub fn last(&self) -> Option<(f64, f64)> {
        self.pts.back().copied()
    }

    /// the points, the oldest first
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (f64, f64)> + ExactSizeIterator + '_ {
        self.pts.iter().copied()
    }

    /// the index of the first point at t or later (len() when there is none)
    pub fn lower_bound(&self, t: f64) -> usize {
        self.pts.partition_point(|p| p.0 < t)
    }

    // the index after the last point at t or earlier
    fn upper_bound(&self, t: f64) -> usize {
        self.pts.partition_point(|p| p.0 <= t)
    }

    // the points a .. b as (at most) two slices of the ring
    fn slices(&self, a: usize, b: usize) -> [&[(f64, f64)]; 2] {
        let (s0, s1) = self.pts.as_slices();
        let n0 = s0.len();
        let b = b.min(self.pts.len()).max(a);
        if b <= n0 {
            [&s0[a..b], &[]]
        } else if a >= n0 {
            [&s1[a - n0..b - n0], &[]]
        } else {
            [&s0[a..], &s1[..b - n0]]
        }
    }

    /// the index of the point with a finite value nearest to t (of two as near the earlier)
    pub fn nearest(&self, t: f64) -> Option<usize> {
        let i = self.lower_bound(t);
        let after = (i..self.pts.len()).find(|&k| self.pts[k].1.is_finite());
        let before = (0..i).rev().find(|&k| self.pts[k].1.is_finite());
        match (before, after) {
            (Some(b), Some(a)) => Some(if t - self.pts[b].0 <= self.pts[a].0 - t { b } else { a }),
            (b, a) => b.or(a),
        }
    }

    /// the smallest and the largest finite value (None without one); looks at every point
    pub fn min_max(&self) -> Option<(f64, f64)> {
        min_max_of(self.slices(0, self.pts.len()))
    }

    /// the smallest and the largest finite value of the points with t0 <= t <= t1
    pub fn min_max_in(&self, t0: f64, t1: f64) -> Option<(f64, f64)> {
        let (a, b) = (self.lower_bound(t0), self.upper_bound(t1));
        if a >= b {
            return None;
        }
        min_max_of(self.slices(a, b))
    }

    /// the points with t0 <= t <= t1 in n columns of equal width (t1 belongs to the last):
    /// per column the range of its values, its first and its last value.  Columns without
    /// a point with a value are left out; the work is one pass over the points, the result
    /// at most n entries (plus one per gap)
    pub fn columns(&self, t0: f64, t1: f64, n: usize) -> Vec<SeriesCol> {
        let mut out: Vec<SeriesCol> = Vec::new();
        if n == 0 || !(t1 > t0) {
            return out;
        }
        let (a, b) = (self.lower_bound(t0), self.upper_bound(t1));
        out.reserve(n.min(b.saturating_sub(a)));
        let k = n as f64 / (t1 - t0);
        let last = (n - 1).min(u32::MAX as usize - 1) as u32;
        let lastf = last as f64;
        // the column being filled in locals (c.col == u32::MAX: none yet)
        let mut c = SeriesCol { col: u32::MAX, n: 0, min: 0.0, max: 0.0, first: 0.0, last: 0.0, gap: false };
        let mut gap = false;
        for part in self.slices(a, b) {
            for &(t, v) in part {
                if !v.is_finite() {
                    gap = true;
                    continue;
                }
                let f = (t - t0) * k;
                let col = if f >= lastf { last } else if f > 0.0 { f as u32 } else { 0 };
                if col == c.col && !gap {
                    c.n += 1;
                    if v < c.min {
                        c.min = v;
                    }
                    if v > c.max {
                        c.max = v;
                    }
                    c.last = v;
                    continue;
                }
                if c.n > 0 {
                    out.push(c);
                }
                c = SeriesCol { col, n: 1, min: v, max: v, first: v, last: v, gap };
                gap = false;
            }
        }
        if c.n > 0 {
            out.push(c);
        }
        out
    }
}

// ------------------------------------------------------------------ axis maths and labels

/// how a line chart labels values (axis ticks, the hover card, the summary)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LcUnit<'a> {
    /// numbers, large ones with k / M / G / T (1,5k), then the suffix after a space
    /// ("req/s", "%"; "" none)
    Num(&'a str),
    /// SI prefixes on a unit: Si("B") 950 B / 1,5 kB / 20 MB, Si("bit/s") 100 Mbit/s
    Si(&'a str),
    /// seconds shown as ns / µs / ms / s (latencies)
    Secs,
}

// the scales of the units, ascending (PREFIX[0] and DURATION[3] are the unit itself)
const PREFIX: [(f64, &str); 5] = [(1.0, ""), (1e3, "k"), (1e6, "M"), (1e9, "G"), (1e12, "T")];
const DURATION: [(f64, &str); 4] = [(1e-9, "ns"), (1e-6, "µs"), (1e-3, "ms"), (1.0, "s")];
static POW10F: [f64; 10] = [1.0, 10.0, 100.0, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9];
static POW10U: [u64; 10] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000, 1_000_000_000];

fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}

fn floor_f(x: f64) -> f64 {
    let f = x as i64 as f64;
    if f > x { f - 1.0 } else { f }
}

fn ceil_f(x: f64) -> f64 {
    -floor_f(-x)
}

// 10^e (exact up to 10^22)
fn pow10(e: i32) -> f64 {
    let mut p = 1.0f64;
    for _ in 0..e.unsigned_abs().min(320) {
        p *= 10.0;
    }
    if e < 0 { 1.0 / p } else { p }
}

// the decimals of three significant digits of x >= 0 (at most 6)
fn sig_decimals(x: f64) -> u32 {
    if !(x > 0.0) || x >= 100.0 {
        0
    } else if x >= 10.0 {
        1
    } else if x >= 1.0 {
        2
    } else {
        let (mut d, mut s) = (2, x);
        while s < 1.0 && d < 6 {
            s *= 10.0;
            d += 1;
        }
        d
    }
}

// the decimals that show multiples of x (at most 9)
fn step_decimals(x: f64) -> u32 {
    let mut s = abs(x);
    let mut d = 0;
    while d < 9 && s > 0.0 && abs(s - floor_f(s + 0.5)) > s * 1e-9 {
        s *= 10.0;
        d += 1;
    }
    d
}

// a formatted number without the zeros at the end of its decimals
fn trim_zeros(s: String, sep: u32) -> String {
    let Some(c) = char::from_u32(sep) else { return s };
    if !s.contains(c) {
        return s;
    }
    String::from(s.trim_end_matches('0').trim_end_matches(c))
}

impl LcUnit<'_> {
    fn scales(&self) -> &'static [(f64, &'static str)] {
        if *self == LcUnit::Secs { &DURATION } else { &PREFIX }
    }

    // the scale of the unit itself
    fn base(&self) -> usize {
        if *self == LcUnit::Secs { 3 } else { 0 }
    }

    // the largest scale not above m (the smallest for less, the unit itself for 0)
    fn scale_of(&self, m: f64) -> usize {
        if !(m > 0.0) {
            return self.base();
        }
        let s = self.scales();
        let mut k = 0;
        while k + 1 < s.len() && m >= s[k + 1].0 * 0.999_999_999 {
            k += 1;
        }
        k
    }

    // a number (scaled already) with the unit at scale k
    fn unit_text(&self, num: &str, k: usize) -> String {
        let p = self.scales()[k].1;
        match *self {
            LcUnit::Num("") => format!("{num}{p}"),
            LcUnit::Num(sfx) => format!("{num}{p} {sfx}"),
            LcUnit::Si(u) if p.is_empty() && u.is_empty() => String::from(num),
            LcUnit::Si(u) => format!("{num} {p}{u}"),
            LcUnit::Secs => format!("{num} {p}"),
        }
    }

    /// v as one value: three significant digits with the scale that fits ("48,2 ms",
    /// "1,25k req/s", "12 %", "1,5 MB"); sep = the decimal separator (ui.decimal_sep());
    /// "–" for a value that is not finite
    pub fn text(&self, v: f64, sep: u32) -> String {
        if !v.is_finite() {
            return String::from("–");
        }
        let a = abs(v);
        let s = self.scales();
        let mut k = self.scale_of(a);
        loop {
            let x = a / s[k].0;
            let d = sig_decimals(x);
            let p = POW10F[d as usize];
            let r = floor_f(x * p + 0.5) / p;
            if r >= 1000.0 && k + 1 < s.len() {
                k += 1;
                continue;
            }
            let num = trim_zeros(fmt_f64_sep(if v < 0.0 { -r } else { r }, d, sep), sep);
            return self.unit_text(&num, k);
        }
    }

    /// the labels of the axis ticks first + k * step (k = 0 ..= n): one scale for all (the
    /// largest tick's; plain numbers get k / M only from 10 000 on), the decimals the step
    /// needs; a zero tick reads "0" with the unit ("0 ms", "0 req/s")
    pub fn ticks(&self, first: f64, step: f64, n: u32, sep: u32) -> Vec<String> {
        let m = abs(first).max(abs(first + step * n as f64));
        let m = if m > 0.0 { m } else { abs(step) };
        let k = self.scale_of(if matches!(self, LcUnit::Num(_)) { m / 10.0 } else { m });
        let sc = self.scales()[k].0;
        let d = step_decimals(step / sc).min(6);
        (0..=n)
            .map(|i| {
                let v = first + step * i as f64;
                if abs(v) <= abs(step) * 1e-6 {
                    let z = if matches!(self, LcUnit::Num(_)) { self.base() } else { k };
                    return self.unit_text("0", z);
                }
                self.unit_text(&fmt_f64_sep(v / sc, d, sep), k)
            })
            .collect()
    }
}

/// "nice" ticks over lo ..= hi: (first, step, n) -- the step 1, 2, 2.5 or 5 x 10^k, first
/// and first + n * step the multiples of it at or around lo and hi, n <= max(steps, 1) (at
/// least 1; lo == hi spans |lo|, or 1)
pub fn lc_nice(lo: f64, hi: f64, steps: u32) -> (f64, f64, u32) {
    let m = steps.max(1) as f64;
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let span = hi - lo;
    if !span.is_finite() || !lo.is_finite() {
        return (0.0, 1.0, 1);
    }
    let span = if span > 0.0 { span } else if lo != 0.0 { abs(lo) } else { 1.0 };
    let raw = span / m;
    let mut e = 0i32;
    while e < 300 && pow10(e + 1) <= raw {
        e += 1;
    }
    while e > -300 && pow10(e) > raw {
        e -= 1;
    }
    let p = pow10(e);
    for f in [1.0, 2.0, 2.5, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0] {
        let step = f * p;
        let a = floor_f(lo / step + 1e-9);
        let b = ceil_f(hi / step - 1e-9);
        let n = (b - a).max(1.0);
        if n <= m {
            return (a * step, step, n as u32);
        }
    }
    let step = 100.0 * p;
    let a = floor_f(lo / step + 1e-9);
    (a * step, step, (ceil_f(hi / step - 1e-9) - a).max(1.0) as u32)
}

// the steps of a time axis from a second on (s): 1 2 5 10 15 30 s, 1 2 5 10 15 30 min, 1 2
// 3 6 12 h, a day
const CLOCK: [f64; 18] =
    [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 21600.0, 43200.0, 86400.0];

/// the step of a time axis (s): the smallest of 1 / 2 / 5 x 10^k below a second (down to a
/// nanosecond), 1 2 5 10 15 30 s, 1 2 5 10 15 30 min, 1 2 3 6 12 h, then days (1 / 2 / 5 x
/// 10^k) that is at least `min`
pub fn lc_time_step(min: f64) -> f64 {
    if !(min > 1e-9) {
        return 1e-9;
    }
    if min <= 1.0 {
        for e in -9..0 {
            for f in [1.0, 2.0, 5.0] {
                let s = f * pow10(e);
                if s >= min * (1.0 - 1e-12) {
                    return s;
                }
            }
        }
        return 1.0;
    }
    for s in CLOCK {
        if s >= min {
            return s;
        }
    }
    let d = min / 86400.0;
    for e in 0..300 {
        for f in [1.0, 2.0, 5.0] {
            if f * pow10(e) >= d {
                return f * pow10(e) * 86400.0;
            }
        }
    }
    min
}

/// a time t (s) as a clock: "0:05", "1:30", "12:00", with hours "1:00:00"; dec decimals of
/// the second (up to 9) after sep (the decimal separator): "0:01,5"; negative with a minus
pub fn lc_time_text(t: f64, dec: u32, hours: bool, sep: u32) -> String {
    let dec = dec.min(9);
    let p = POW10U[dec as usize];
    let neg = t < 0.0;
    let u = (abs(t) * p as f64 + 0.5) as u64;
    let (whole, frac) = (u / p, u % p);
    let mut s = String::new();
    if neg && u != 0 {
        s.push('-');
    }
    if hours {
        s.push_str(&format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60));
    } else {
        s.push_str(&format!("{}:{:02}", whole / 60, whole % 60));
    }
    if dec > 0 {
        if let Some(c) = char::from_u32(sep) {
            s.push(c);
        }
        s.push_str(&format!("{:0w$}", frac, w = dec as usize));
    }
    s
}

// ------------------------------------------------------------------ line chart

/// x axes of a line chart (LineChart::x)
pub const LX_SECS: u8 = 0; // time in seconds: 0:05, 1:00, 1:30
pub const LX_NANOS: u8 = 1; // time in nanoseconds (a simulation clock), labelled the same
pub const LX_NUM: u8 = 2; // plain numbers
/// LineChart::flags
pub const LC_NO_LEGEND: u32 = 1; // no legend below the plot
pub const LC_NO_GRID: u32 = 2; // no grid lines
pub const LC_NO_HOVER: u32 = 4; // no crosshair, card and keyboard cursor (no focus stop)

/// one series of a line chart: a label, its points, colour, line or area, unit
#[derive(Clone, Copy, Debug)]
pub struct LcSeries<'a> {
    pub label: &'a str,
    pub data: &'a SeriesBuf,
    /// 0xAARRGGBB; 0 = the chart palette's colour `pal`, without one the series' index
    pub color: u32,
    pub pal: Option<u8>,
    /// a translucent fill under the line, down to zero (or to the edge of the plot)
    pub area: bool,
    /// how its values read (the y axis takes the first series')
    pub unit: LcUnit<'a>,
}

impl<'a> LcSeries<'a> {
    /// a line in the palette colour of its index, plain numbers
    pub const fn new(label: &'a str, data: &'a SeriesBuf) -> LcSeries<'a> {
        LcSeries { label, data, color: 0, pal: None, area: false, unit: LcUnit::Num("") }
    }
    /// the same series in a colour (0xAARRGGBB)
    pub const fn color(mut self, c: u32) -> LcSeries<'a> {
        self.color = c;
        self
    }
    /// the same series in the chart palette's colour k (0 ..= 7)
    pub const fn pal(mut self, k: u8) -> LcSeries<'a> {
        self.pal = Some(k);
        self
    }
    /// the same series as an area
    pub const fn area(mut self) -> LcSeries<'a> {
        self.area = true;
        self
    }
    /// the same series with a unit
    pub const fn unit(mut self, u: LcUnit<'a>) -> LcSeries<'a> {
        self.unit = u;
        self
    }
}

/// a marker on the x axis: a line at t0, or the range t0 .. t1 shaded (fault injections)
#[derive(Clone, Copy, Debug)]
pub struct LcMarker<'a> {
    pub t0: f64,
    /// <= t0: a line
    pub t1: f64,
    /// a short label in a tag at the top of the plot ("" none)
    pub label: &'a str,
    /// 0xAARRGGBB, 0 = T_TEXT2
    pub color: u32,
}

impl<'a> LcMarker<'a> {
    /// a line at t
    pub const fn line(t: f64, label: &'a str) -> LcMarker<'a> {
        LcMarker { t0: t, t1: t, label, color: 0 }
    }
    /// the range t0 .. t1
    pub const fn range(t0: f64, t1: f64, label: &'a str) -> LcMarker<'a> {
        LcMarker { t0, t1, label, color: 0 }
    }
    /// the same marker in a colour
    pub const fn color(mut self, c: u32) -> LcMarker<'a> {
        self.color = c;
        self
    }
}

/// a threshold: a dashed horizontal line at a value with a label at its right end
#[derive(Clone, Copy, Debug)]
pub struct LcThreshold<'a> {
    pub value: f64,
    pub label: &'a str,
    /// 0xAARRGGBB, 0 = T_TEXT2
    pub color: u32,
}

impl<'a> LcThreshold<'a> {
    /// a threshold at value with its label, in T_TEXT2
    pub const fn new(value: f64, label: &'a str) -> LcThreshold<'a> {
        LcThreshold { value, label, color: 0 }
    }
    /// the same threshold in a colour
    pub const fn color(mut self, c: u32) -> LcThreshold<'a> {
        self.color = c;
        self
    }
}

/// what ui.line_chart shows: the series and how (see the module header of chart.rs); build
/// it with LineChart::new(&series) and the setters, or change the fields
#[derive(Clone, Copy, Debug)]
pub struct LineChart<'a> {
    pub series: &'a [LcSeries<'a>],
    /// the x axis: LX_SECS (default), LX_NANOS or LX_NUM
    pub x: u8,
    /// the visible x range (the points' units); empty (the default) = the data's
    pub x_range: (f64, f64),
    /// follow live: without x_range the newest x_span of the data (0 = all of it)
    pub x_span: f64,
    /// the y range; a NaN end (the default) is automatic: the visible values and the
    /// thresholds, out to the next tick.  (0.0, f64::NAN): from zero (metrics)
    pub y_range: (f64, f64),
    pub markers: &'a [LcMarker<'a>],
    pub thresholds: &'a [LcThreshold<'a>],
    /// LC_*
    pub flags: u32,
    /// the chart's name: the start of its summary (screen readers)
    pub name: &'a str,
    /// the text without points ("" = "Keine Daten" in the UI language)
    pub empty: &'a str,
}

impl<'a> LineChart<'a> {
    /// the series over time in seconds, both ranges automatic
    pub const fn new(series: &'a [LcSeries<'a>]) -> LineChart<'a> {
        LineChart {
            series,
            x: LX_SECS,
            x_range: (0.0, 0.0),
            x_span: 0.0,
            y_range: (f64::NAN, f64::NAN),
            markers: &[],
            thresholds: &[],
            flags: 0,
            name: "",
            empty: "",
        }
    }
    /// the x axis: LX_SECS, LX_NANOS or LX_NUM
    pub const fn x_axis(mut self, x: u8) -> LineChart<'a> {
        self.x = x;
        self
    }
    /// a fixed x range (a paused view)
    pub const fn x_range(mut self, lo: f64, hi: f64) -> LineChart<'a> {
        self.x_range = (lo, hi);
        self
    }
    /// follow live: the newest `span` of the data
    pub const fn follow(mut self, span: f64) -> LineChart<'a> {
        self.x_span = span;
        self
    }
    /// the y range (a NaN end: automatic)
    pub const fn y_range(mut self, lo: f64, hi: f64) -> LineChart<'a> {
        self.y_range = (lo, hi);
        self
    }
    /// markers on the x axis (lines and shaded ranges with tags)
    pub const fn markers(mut self, m: &'a [LcMarker<'a>]) -> LineChart<'a> {
        self.markers = m;
        self
    }
    /// thresholds (dashed lines with tags)
    pub const fn thresholds(mut self, t: &'a [LcThreshold<'a>]) -> LineChart<'a> {
        self.thresholds = t;
        self
    }
    /// LC_* flags
    pub const fn flags(mut self, f: u32) -> LineChart<'a> {
        self.flags = f;
        self
    }
    /// the name its summary starts with
    pub const fn name(mut self, n: &'a str) -> LineChart<'a> {
        self.name = n;
        self
    }
    /// the text of the empty chart
    pub const fn empty(mut self, s: &'a str) -> LineChart<'a> {
        self.empty = s;
        self
    }

    /// the x range the chart shows (the points' units): x_range when it is set, else the
    /// data's -- with x_span the newest x_span of it (from the first point on while the
    /// data is shorter: it grows from the left); without points 0 .. x_span (or one second)
    pub fn view(&self) -> (f64, f64) {
        let (a, b) = self.x_range;
        if a.is_finite() && b.is_finite() && b > a {
            return (a, b);
        }
        let one = match self.x {
            LX_NANOS => 1e9,
            _ => 1.0,
        };
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in self.series {
            if let (Some(f), Some(l)) = (s.data.first(), s.data.last()) {
                lo = lo.min(f.0);
                hi = hi.max(l.0);
            }
        }
        let span = if self.x_span.is_finite() && self.x_span > 0.0 { self.x_span } else { 0.0 };
        let v = if lo > hi {
            (0.0, if span > 0.0 { span } else { one })
        } else if span > 0.0 {
            let end = hi.max(lo + span);
            (end - span, end)
        } else if hi > lo {
            (lo, hi)
        } else {
            (lo - one / 2.0, lo + one / 2.0)
        };
        if v.1 > v.0 { v } else { (v.0, v.0 + one) }
    }
}

/// the state of a line chart between passes: what the view shows of each series, the
/// summary and the plot's drawing object, each with the key of what it was made from
#[derive(Default)]
pub(crate) struct LcState {
    stats_key: u64,
    stats: Vec<Stat>,
    sum_key: u64,
    summary: String,
    obj_key: u64,
    obj: Option<DrawRef>,
    // the last pass's hover (series, x bits)
    hover: Option<(usize, u64)>,
    // the keyboard's cursor: series, x of its point
    cursor: Option<(usize, f64)>,
    // the plot's rectangle in the last pass (px)
    rect: [i32; 4],
}

// what a series shows in the view: the range of its values, its last value, its points
#[derive(Clone, Copy, Default)]
struct Stat {
    lo: f64,
    hi: f64,
    last: f64,
    has: bool,
    n: usize,
}

fn stat_of(d: &SeriesBuf, v: (f64, f64)) -> Stat {
    let (a, b) = (d.lower_bound(v.0), d.upper_bound(v.1));
    let mut s = Stat { n: b.saturating_sub(a), ..Stat::default() };
    if let Some((lo, hi)) = d.min_max_in(v.0, v.1) {
        s.lo = lo;
        s.hi = hi;
        s.has = true;
        s.last = (a..b).rev().filter_map(|i| d.get(i)).map(|p| p.1).find(|x| x.is_finite()).unwrap_or(f64::NAN);
    }
    s
}

// how a series is drawn: its colour, dash (high contrast), a fill under it
#[derive(Clone, Copy)]
struct Look {
    color: u32,
    dash: u32,
    area: bool,
}

// the dash pattern of series i in high contrast (with the colour of i % 3 every series up
// to the 12th differs)
const HC_DASH: [u32; 4] = [DASH_SOLID, DASH_DASH, DASH_DOT, DASH_DASHDOT];

// a drawn series: polylines to stroke, or bands to fill; colour, width (24.8), DASH_*
struct Trace {
    polys: Vec<Vec<(f32, f32)>>,
    band: bool,
    color: u32,
    width: i32,
    dash: u32,
}

// the plot of a line chart (px relative to its top left, m inside the rectangle of its
// command; drawn at render time): grid, shaded ranges, areas, lines, thresholds, markers
struct Plot {
    m: i32,
    w: i32,
    h: i32,
    hair: i32,
    // grid lines: position, vertical
    grid: Vec<(i32, bool)>,
    grid_c: u32,
    dotted: bool,
    // the shaded ranges of markers: x0, x1, fill
    ranges: Vec<(i32, i32, u32)>,
    areas: Vec<(Vec<Vec<(f32, f32)>>, u32)>,
    traces: Vec<Trace>,
    // thresholds: y, colour; dashes (thickness, on, off px)
    limits: Vec<(i32, u32)>,
    lim: (i32, i32, i32),
    // marker lines and the edges of ranges: x, width, colour
    rules: Vec<(i32, i32, u32)>,
}

// polylines (or polygons) moved by (ox, oy) into the path
fn trace_path(path: &mut Path, polys: &[Vec<(f32, f32)>], ox: f32, oy: f32, close: bool) {
    path.reset();
    path.set_origin(0, 0);
    for p in polys {
        for (k, &(x, y)) in p.iter().enumerate() {
            if k == 0 {
                path.move_f(ox + x, oy + y);
            } else {
                path.line_f(ox + x, oy + y);
            }
        }
        if close {
            path.close();
        }
    }
}

impl Draw for Plot {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let (ox, oy) = (r[0] + self.m, r[1] + self.m);
        let (w, h, t) = (self.w, self.h, self.hair);
        g.dc.set_color(self.grid_c);
        for &(p, vert) in &self.grid {
            if self.dotted {
                // high contrast: dotted, so the grid stays behind the series
                let n = if vert { h } else { w };
                let mut k = 0;
                while k < n {
                    if vert {
                        g.dc.fill(g.sf, ox + p, oy + k, t, t);
                    } else {
                        g.dc.fill(g.sf, ox + k, oy + p, t, t);
                    }
                    k += t * 3;
                }
            } else if vert {
                g.dc.fill(g.sf, ox + p, oy, t, h);
            } else {
                g.dc.fill(g.sf, ox, oy + p, w, t);
            }
        }
        for &(x0, x1, c) in &self.ranges {
            g.dc.set_color(c);
            g.dc.fill(g.sf, ox + x0, oy, x1 - x0, h);
        }
        let (fx, fy) = (ox as f32, oy as f32);
        for (polys, c) in &self.areas {
            trace_path(&mut g.text.path, polys, fx, fy, true);
            g.dc.set_color(*c);
            g.text.path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
        }
        for tr in &self.traces {
            trace_path(&mut g.text.path, &tr.polys, fx, fy, tr.band);
            g.dc.set_color(tr.color);
            if tr.band {
                g.text.path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
            } else if tr.dash == DASH_SOLID {
                g.text.path.stroke(&mut g.text.ras, g.dc, g.sf, tr.width, gfx::JOIN_ROUND | gfx::CAP_ROUND << 4);
            } else {
                let st = DStyle {
                    width: tr.width as f64 / 256.0,
                    pat: dash_pattern(tr.dash),
                    join: gfx::JOIN_ROUND,
                    cap0: LCAP_FLAT,
                    cap1: LCAP_FLAT,
                    dcap: LCAP_FLAT,
                    offset: 0.0,
                    miter: 0.0,
                };
                dash_stroke(&mut g.text.path, &st);
                g.text.path.fill(&mut g.text.ras, g.dc, g.sf, gfx::RULE_NONZERO);
            }
        }
        let (lt, on, off) = self.lim;
        for &(y, c) in &self.limits {
            g.dc.set_color(c);
            let mut x = 0;
            while x < w {
                g.dc.fill(g.sf, ox + x, oy + y, on.min(w - x), lt);
                x += on + off;
            }
        }
        for &(x, lw, c) in &self.rules {
            g.dc.set_color(c);
            g.dc.fill(g.sf, ox + x, oy, lw, h);
        }
    }
}

// a polyline of points (px, f64) into f32, its ends cut back to 64 px outside the plot
// (the points before and after the view may be far away)
fn polyline(run: &mut [(f64, f64)], w: f64) -> Vec<(f32, f32)> {
    const M: f64 = 64.0;
    let n = run.len();
    if n >= 2 {
        let (a, b) = (run[0], run[1]);
        if a.0 < -M && b.0 > a.0 {
            let f = (-M - a.0) / (b.0 - a.0);
            run[0] = (-M, a.1 + (b.1 - a.1) * f);
        }
        let (a, b) = (run[n - 2], run[n - 1]);
        if b.0 > w + M && b.0 > a.0 {
            let f = (w + M - a.0) / (b.0 - a.0);
            run[n - 1] = (w + M, a.1 + (b.1 - a.1) * f);
        }
    }
    if n == 1 {
        // a point alone: a dot (the round caps of a pixel long line)
        let (x, y) = run[0];
        return alloc::vec![((x - 0.5) as f32, y as f32), ((x + 0.5) as f32, y as f32)];
    }
    run.iter().map(|&(x, y)| (x as f32, y as f32)).collect()
}

type Polys = Vec<Vec<(f32, f32)>>;

// the lines (or bands) and the areas of a series over the view in px relative to a w x h
// plot whose values run from y.0 (bottom) to y.1: up to two points per pixel column the
// points themselves (with the one before and after the view), more as one band per run of
// columns (each column the range of its values, joined to the next one), lw thick; dashed
// bands are cut into the pattern's pieces along x.  -> (polylines or bands, band, areas)
#[allow(clippy::too_many_arguments)]
fn trace_of(d: &SeriesBuf, v: (f64, f64), (w, h): (i32, i32), y: (f64, f64), lw: f64, dense: bool, area: bool, dash: &[f64]) -> (Polys, bool, Polys) {
    let sx = w as f64 / (v.1 - v.0);
    let sy = h as f64 / (y.1 - y.0);
    let hf = h as f64;
    let fy = |val: f64| (hf - (val - y.0) * sy).clamp(-hf - 64.0, hf * 2.0 + 64.0);
    let base = fy(0.0f64.clamp(y.0, y.1));
    let mut lines: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut areas: Vec<Vec<(f32, f32)>> = Vec::new();
    if !dense {
        let a = d.lower_bound(v.0).saturating_sub(1);
        let b = (d.upper_bound(v.1) + 1).min(d.len());
        let mut run: Vec<(f64, f64)> = Vec::new();
        let mut flush = |run: &mut Vec<(f64, f64)>| {
            if run.is_empty() {
                return;
            }
            let p = polyline(run, w as f64);
            if area && p.len() >= 2 {
                let mut q = p.clone();
                q.push((p[p.len() - 1].0, base as f32));
                q.push((p[0].0, base as f32));
                areas.push(q);
            }
            lines.push(p);
            run.clear();
        };
        for i in a..b {
            let Some((t, val)) = d.get(i) else { break };
            if !val.is_finite() {
                flush(&mut run);
                continue;
            }
            run.push(((t - v.0) * sx, fy(val)));
        }
        flush(&mut run);
        return (lines, false, areas);
    }
    let cols = d.columns(v.0, v.1, w.max(1) as usize);
    let half = lw / 2.0;
    // the dash pattern along x (px): its period and whether x is in a dash
    let pat: Vec<f64> = dash.iter().map(|k| k * lw).collect();
    let period: f64 = pat.iter().sum::<f64>() * if pat.len() % 2 == 1 { 2.0 } else { 1.0 };
    let on = |x: f64| -> bool {
        if pat.is_empty() || !(period > 0.0) {
            return true;
        }
        let mut p = x - floor_f(x / period) * period;
        let mut k = 0;
        loop {
            let len = pat[k % pat.len()];
            if p < len {
                return k % 2 == 0;
            }
            p -= len;
            k += 1;
            if k > 64 {
                return true;
            }
        }
    };
    let mut i = 0;
    while i < cols.len() {
        let mut j = i + 1;
        while j < cols.len() && !cols[j].gap {
            j += 1;
        }
        // columns i .. j are one run: top and bottom of each column joined to the next one
        let mut ext: Vec<(f64, f64, f64)> = Vec::with_capacity(j - i);
        for k in i..j {
            let c = cols[k];
            let (mut lo, mut hi) = (c.min, c.max);
            if k + 1 < j {
                lo = lo.min(cols[k + 1].first);
                hi = hi.max(cols[k + 1].first);
            }
            ext.push((c.col as f64 + 0.5, fy(hi), fy(lo)));
        }
        if area {
            let mut q: Vec<(f32, f32)> = Vec::with_capacity(ext.len() + 4);
            q.push(((ext[0].0 - 0.5) as f32, ext[0].1 as f32));
            q.extend(ext.iter().map(|e| (e.0 as f32, e.1 as f32)));
            let l = ext[ext.len() - 1];
            q.push(((l.0 + 0.5) as f32, l.1 as f32));
            q.push(((l.0 + 0.5) as f32, base as f32));
            q.push(((ext[0].0 - 0.5) as f32, base as f32));
            areas.push(q);
        }
        // the bands (one per dash piece)
        let mut k = 0;
        while k < ext.len() {
            if !on(ext[k].0) {
                k += 1;
                continue;
            }
            let mut e = k + 1;
            while e < ext.len() && on(ext[e].0) {
                e += 1;
            }
            let part = &ext[k..e];
            let mut q: Vec<(f32, f32)> = Vec::with_capacity(part.len() * 2 + 4);
            q.push(((part[0].0 - 0.5) as f32, (part[0].1 - half) as f32));
            q.extend(part.iter().map(|p| (p.0 as f32, (p.1 - half) as f32)));
            let l = part[part.len() - 1];
            q.push(((l.0 + 0.5) as f32, (l.1 - half) as f32));
            q.push(((l.0 + 0.5) as f32, (l.2 + half) as f32));
            q.extend(part.iter().rev().map(|p| (p.0 as f32, (p.2 + half) as f32)));
            q.push(((part[0].0 - 0.5) as f32, (part[0].2 + half) as f32));
            lines.push(q);
            k = e;
        }
        i = j;
    }
    (lines, true, areas)
}

// two rectangles closer than sep
fn near(a: [i32; 4], b: [i32; 4], sep: i32) -> bool {
    a[0] < b[0] + b[2] + sep && b[0] < a[0] + a[2] + sep && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

// an x as the hover card's title: the clock to the last decimal that is not 0 (x in s or
// ns), or the number
fn lc_x_label(x: u8, t: f64, hours: bool, sep: u32) -> String {
    if x == LX_NUM {
        return LcUnit::Num("").text(t, sep);
    }
    let s = if x == LX_NANOS { t * 1e-9 } else { t };
    let full = lc_time_text(s, 9, hours, sep);
    match char::from_u32(sep) {
        Some(c) => String::from(full.trim_end_matches('0').trim_end_matches(c)),
        None => full,
    }
}

// where a marker ends (a line: where it starts)
fn mark_end(m: &LcMarker) -> f64 {
    if m.t1 > m.t0 { m.t1 } else { m.t0 }
}

// the x of the first point with a value inside v from index i on (back: before i)
fn lc_step(d: &SeriesBuf, i: usize, back: bool, v: (f64, f64)) -> Option<f64> {
    let mut k = i;
    loop {
        if back {
            k = k.checked_sub(1)?;
        }
        let (t, val) = d.get(k)?;
        if (back && t < v.0) || (!back && t > v.1) {
            return None;
        }
        if val.is_finite() && t >= v.0 && t <= v.1 {
            return Some(t);
        }
        if !back {
            k += 1;
        }
    }
}

impl Ui {
    fn chart_color(&self, c: u32, i: usize) -> u32 {
        if c != 0 {
            return c;
        }
        let p = self.style.data().chart_palette(self);
        p[i % 8]
    }

    // the cached drawing object for id with this content hash (built by make otherwise)
    fn chart_obj(&mut self, id: u32, hash: u64, make: impl FnOnce() -> DrawRef) -> DrawRef {
        if let Some((h, d)) = self.m.chart.cache.get(&id) {
            if *h == hash {
                return d.clone();
            }
        }
        let d = make();
        if self.m.chart.cache.len() > 256 {
            self.m.chart.cache.clear();
        }
        self.m.chart.cache.insert(id, (hash, d.clone()));
        d
    }

    /// ui_sparkline(id, values, color): area + line over the values (min .. max scaled)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn sparkline(&mut self, id: u32, values: &[f32], color: u32) {
        let _src = self.src_scope();
        let r = self.place(self.dp(120), self.dp(32));
        let line = self.chart_color(color, 0);
        let area = (line & 0x00FF_FFFF) | 0x2E00_0000;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &v in values {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if hi <= lo {
            hi = lo + 1.0;
            lo -= 1.0;
        }
        let width = self.dp_fx(2 * 256).max(256);
        let mut h = Fnv::new();
        for &v in values {
            h.f32(v);
        }
        h.u32(line);
        h.u32(width as u32);
        let n = values.len().max(2) - 1;
        let hash = h.0;
        let d = self.chart_obj(id, hash, || {
            let pts = values.iter().enumerate().map(|(i, &v)| (i as f32 / n as f32, (v - lo) / (hi - lo))).collect();
            Rc::new(Spark { pts, line, area, width })
        });
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(d), 0, hash as u32);
        self.wp.ctx.last_state = 0;
        self.sem(K_CANVAS, id, None, values.len() as i32);
    }

    /// ui_bar_chart(id, labels, series, stacked) -> the hovered column (-1 = none)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn bar_chart(&mut self, id: u32, labels: &[&str], series: &[BarSeries], stacked: bool) -> i32 {
        let _src = self.src_scope();
        let r = self.place(self.dp(360), self.dp(180));
        let n = labels.len();
        // the largest column (stacked: the sums)
        let mut m = 0.0f32;
        for i in 0..n {
            if stacked {
                let s: f32 = series.iter().map(|s| s.values.get(i).copied().unwrap_or(0.0).max(0.0)).sum();
                m = m.max(s);
            } else {
                for s in series {
                    m = m.max(s.values.get(i).copied().unwrap_or(0.0));
                }
            }
        }
        // value labels a line apart at least: fewer grid steps in a low plot (large text)
        let lh = self.ts_line(TS_CAPTION);
        let ph = r[3] - lh - self.dp(6) - (lh >> 1);
        let (top, steps) = nice_max(m, (ph / lh.max(1)).clamp(1, 5));
        let vals: Vec<String> = (0..=steps).map(|k| short_num(top * k as f32 / steps as f32)).collect();
        // axis label width: the widest value label
        self.ts_font(TS_CAPTION);
        let vw = vals.iter().map(|s| self.text.width(s)).max().unwrap_or(0);
        let lw = vw.max(self.dp(16)) + self.dp(8);
        let plot = [r[0] + lw, r[1] + (lh >> 1), r[2] - lw, ph];
        // grid lines and value labels
        let hair = self.hair();
        let grid = self.color(T_DIVIDER);
        let t3 = self.color(T_TEXT3);
        for (k, s) in vals.iter().enumerate() {
            let y = plot[1] + plot[3] - (plot[3] as i64 * k as i64 / steps as i64) as i32;
            self.draw_color(grid);
            self.draw_rect(plot[0], y, plot[2], hair);
            self.ts_font(TS_CAPTION);
            let tw = self.text.width(s);
            text_at(self, TS_CAPTION, plot[0] - tw - self.dp(6), [r[0], y - (lh >> 1), lw, lh], s, t3, 0);
        }
        if n == 0 {
            return -1;
        }
        // columns
        let cw = plot[2] / n as i32;
        // column labels that do not fit side by side: every k-th one (the smallest k that
        // keeps them apart), the first and the last kept; one alone is cut with "…"
        self.ts_font(TS_CAPTION);
        let lws: Vec<i32> = labels.iter().map(|l| self.text.width(crate::basic::cstr(l))).collect();
        let (lo, hi) = (plot[0], r[0] + r[2]);
        let sep = self.dp(4);
        let shown = (1..=n)
            .find_map(|k| axis_labels(&lws, plot[0], cw, lo, hi, k, sep))
            .unwrap_or_else(|| alloc::vec![(0, lo)]);
        let ly = plot[1] + plot[3] + self.dp(6);
        let gw = ((cw * 3) / 5).max(2);
        let ns = series.len().max(1) as i32;
        let mut hovered = -1;
        let rad = self.dp(2);
        let mut next = 0; // the next of `shown`
        for (i, label) in labels.iter().enumerate() {
            let cx = plot[0] + cw * i as i32;
            // hover area of the column
            let ev = self.hit_rect(id.wrapping_add(1 + i as u32), cx, plot[1], cw, plot[3] + lh + self.dp(6), 0);
            if ev & EV_HOVER != 0 {
                hovered = i as i32;
                let c = self.color(T_SUBTLE_HOVER);
                if c & 0xFF00_0000 != 0 {
                    self.cmd_rrect(cx + self.dp(2), plot[1], cw - self.dp(4), plot[3], self.dp(4), 0, c, 0, 0);
                }
            }
            let x0 = cx + ((cw - gw) >> 1);
            let mut base = plot[1] + plot[3];
            for (si, s) in series.iter().enumerate() {
                let v = s.values.get(i).copied().unwrap_or(0.0).max(0.0);
                let h = ((v / top) * plot[3] as f32) as i32;
                let c = self.chart_color(s.color, si);
                if stacked {
                    if h > 0 {
                        self.cmd_rrect(x0, base - h, gw, h, if si as i32 == ns - 1 { rad } else { 0 }, 0, c, 0, 0);
                    }
                    base -= h;
                } else {
                    let bw = (gw / ns).max(1);
                    let bx = x0 + bw * si as i32;
                    if h > 0 {
                        self.cmd_rrect(bx, base - h, (bw - hair).max(1), h, rad.min(bw / 2), 0, c, 0, 0);
                    }
                }
            }
            // column label (a text record each, value 1 = cut)
            let lab = crate::basic::cstr(label);
            if let Some(&(_, x)) = shown.get(next).filter(|s| s.0 == i) {
                next += 1;
                let room = hi - x;
                self.ts_font(TS_CAPTION);
                text_at(self, TS_CAPTION, x, [x, ly, room, lh], lab, t3, room.max(1));
                self.wp.ctx.last_rect = [x, ly, lws[i].min(room), lh];
                self.wp.ctx.last_state = 0;
                self.sem(K_TEXT, 0, Some(lab), (lws[i] > room) as i32);
            }
            if ev & EV_HOVER != 0 {
                let mut tip = String::from(lab);
                for s in series {
                    let v = s.values.get(i).copied().unwrap_or(0.0);
                    tip.push_str(&format!("\n{}: {}", s.label, short_num(v)));
                }
                self.tooltip(&tip);
            }
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = 0;
        self.sem(K_CANVAS, id, None, n as i32);
        hovered
    }

    /// ui_donut(id, slices, center, sub): ring of slices (fractions of their sum) with a
    /// centre text (TS_SUBTITLE; TS_BODY_STRONG where that is wider than the hole) and a
    /// caption below it (left out where the hole is too low for both)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn donut(&mut self, id: u32, slices: &[DonutSlice], center: &str, sub: &str) {
        let _src = self.src_scope();
        let s = self.dp(160);
        let r = self.place(s, s);
        let d = r[2].min(r[3]);
        let sq = [r[0] + ((r[2] - d) >> 1), r[1] + ((r[3] - d) >> 1), d, d];
        let total: f32 = slices.iter().map(|s| s.value.max(0.0)).sum();
        let track = self.color(T_TRACK);
        let mut h = Fnv::new();
        let mut parts = Vec::new();
        let mut a = 0.0f32;
        for (i, sl) in slices.iter().enumerate() {
            let f = if total > 0.0 { sl.value.max(0.0) / total } else { 0.0 };
            let c = self.chart_color(sl.color, i);
            parts.push((a, a + f, c));
            h.f32(f);
            h.u32(c);
            a += f;
        }
        h.u32(track);
        h.u32(d as u32);
        let hash = h.0;
        let gap = if slices.len() > 1 { 0.006 } else { 0.0 };
        let obj = self.chart_obj(id, hash, || Rc::new(Ring { slices: parts, thick: 0.22, track, gap }));
        self.cmd_custom(sq[0], sq[1], sq[2], sq[3], Some(obj), 0, hash as u32);
        // centre texts inside the hole (large text: the centre in a smaller style, the caption
        // left out, else cut with "…")
        let t = self.color(T_TEXT);
        let t2 = self.color(T_TEXT2);
        let hole = (d as f32 * 0.78) as i32 - self.dp(8);
        let mut ts1 = TS_SUBTITLE;
        self.ts_font(ts1);
        if self.text.width(center) > hole {
            ts1 = TS_BODY_STRONG;
        }
        let l1 = self.ts_line(ts1);
        let cap = self.ts_line(TS_CAPTION);
        let sub = if sub.is_empty() || l1 + cap > hole { "" } else { sub };
        let l2 = if sub.is_empty() { 0 } else { cap };
        let y = sq[1] + ((d - l1 - l2) >> 1);
        self.ts_font(ts1);
        let w1 = self.text.width(center).min(hole);
        text_at(self, ts1, sq[0] + ((d - w1) >> 1), [sq[0], y, d, l1], center, t, hole.max(1));
        if !sub.is_empty() {
            self.ts_font(TS_CAPTION);
            let w2 = self.text.width(sub).min(hole);
            text_at(self, TS_CAPTION, sq[0] + ((d - w2) >> 1), [sq[0], y + l1, d, l2], sub, t2, hole.max(1));
        }
        // hover: the slice under the pointer as a tooltip
        let ev = self.hit_rect(id, sq[0], sq[1], sq[2], sq[3], 0);
        if ev & EV_HOVER != 0 && total > 0.0 {
            let (mx, my) = self.mouse();
            let dx = (mx - (sq[0] + d / 2)) as f32;
            let dy = (my - (sq[1] + d / 2)) as f32;
            let dist = rt::fx::sqrt(dx * dx + dy * dy);
            let ro = d as f32 / 2.0;
            if dist <= ro && dist >= ro * 0.78 {
                let ang = rt::x87::atan2(dx as f64, -dy as f64) as f32 / core::f32::consts::TAU;
                let ang = if ang < 0.0 { ang + 1.0 } else { ang };
                let mut acc = 0.0f32;
                for sl in slices {
                    let f = sl.value.max(0.0) / total;
                    if ang >= acc && ang < acc + f {
                        let tip = format!("{}: {} ({:.0} %)", sl.label, short_num(sl.value), f * 100.0);
                        self.tooltip(&tip);
                        break;
                    }
                    acc += f;
                }
            }
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = 0;
        self.sem(K_CANVAS, id, Some(center), slices.len() as i32);
    }

    /// ui_legend(items): a colour dot and a label per item (colour 0 = palette by index),
    /// laid out in a wrapping row; an item wider than the row has its label cut with "…"
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn legend(&mut self, items: &[(&str, u32)]) {
        let _src = self.src_scope();
        self.legend_marks(items, &[], 0);
    }

    // the legend; the items with an entry in `dashes` get a line sample in that DASH_*
    // pattern (lw thick, 24.8) instead of the dot (line charts in high contrast)
    fn legend_marks(&mut self, items: &[(&str, u32)], dashes: &[u32], lw: i32) {
        let s = self.dp(10);
        let sl = self.dp(20);
        let (gap, igap) = (self.dp(12), self.dp(6));
        // the width the row gets (from the items' own widths, so a cut item does not narrow
        // the row it is measured against)
        self.ts_font(TS_CAPTION);
        let ws: Vec<i32> = items
            .iter()
            .enumerate()
            .map(|(i, &(label, _))| if i < dashes.len() { sl } else { s } + igap + self.text.width(crate::basic::cstr(label)))
            .collect();
        let one_row = ws.iter().sum::<i32>() + gap * (ws.len() as i32 - 1).max(0);
        let (room, _) = self.avail_width(one_row);
        self.box_begin(0, BOX_H | BOX_WRAP | BOX_XCENTER, 12);
        for (i, &(label, c)) in items.iter().enumerate() {
            let col = self.chart_color(c, i);
            let cut = room > 0 && ws[i] > room;
            if cut {
                self.next_size(self.px2dip(room), 0);
            }
            self.box_begin(0, BOX_H | BOX_XCENTER, 6);
            if let Some(&dash) = dashes.get(i) {
                let r = self.place(sl, s);
                self.lc_sample(r, col, dash, lw);
            } else {
                let r = self.place(s, s);
                self.draw_color(col);
                self.draw_circle((r[0] << 8) + (s << 7), (r[1] << 8) + (r[3] << 7), s << 7, 0);
            }
            if cut {
                self.next_size(-1, 0);
            }
            self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
            self.box_end();
        }
        self.box_end();
    }

    // a line sample across the middle of r: lw thick (24.8), in the pieces of a DASH_* pattern
    fn lc_sample(&mut self, r: [i32; 4], c: u32, dash: u32, lw: i32) {
        let t = ((lw + 128) >> 8).max(1);
        let y = r[1] + ((r[3] - t) >> 1);
        self.draw_color(c);
        let pat = dash_pattern(dash);
        if pat.is_empty() {
            self.draw_rect(r[0], y, r[2], t);
            return;
        }
        let (mut x, mut k) = (0, 0);
        while x < r[2] {
            let len = ((pat[k % pat.len()] * t as f64) as i32).max(1);
            if k % 2 == 0 {
                self.draw_rect(r[0] + x, y, len.min(r[2] - x), t);
            }
            x += len;
            k += 1;
        }
    }

    // the ticks of the x axis over pw px: (x in the points' units, label); labels at least
    // 72 DIP apart and never closer than their width and 12 DIP
    fn lc_x_ticks(&mut self, x: u8, v: (f64, f64), pw: i32, sep: u32) -> Vec<(f64, String)> {
        self.ts_font(TS_CAPTION);
        let room = self.dp(12);
        if x == LX_NUM {
            let pps = pw as f64 / (v.1 - v.0);
            let mut steps = (pw / self.dp(72).max(1)).clamp(1, 12) as u32;
            loop {
                let (_, step, _) = lc_nice(v.0, v.1, steps);
                let (k0, k1) = (ceil_f(v.0 / step - 1e-9), floor_f(v.1 / step + 1e-9));
                if k1 < k0 || k1 - k0 > 64.0 {
                    return Vec::new();
                }
                let n = (k1 - k0) as u32;
                let labels = LcUnit::Num("").ticks(k0 * step, step, n, sep);
                let wmax = labels.iter().map(|s| self.text.width(s)).max().unwrap_or(0);
                if step * pps >= (wmax + room) as f64 || steps == 1 {
                    return labels.into_iter().enumerate().map(|(i, s)| ((k0 + i as f64) * step, s)).collect();
                }
                steps -= 1;
            }
        }
        let xs = if x == LX_NANOS { 1e-9 } else { 1.0 };
        let (a, b) = (v.0 * xs, v.1 * xs);
        let pps = pw as f64 / (b - a);
        let hours = abs(a).max(abs(b)) >= 3600.0;
        let mut step = lc_time_step(self.dp(72) as f64 / pps);
        for _ in 0..12 {
            let dec = if step >= 1.0 { 0 } else { step_decimals(step) };
            let (k0, k1) = (ceil_f(a / step - 1e-9), floor_f(b / step + 1e-9));
            if k1 < k0 || k1 - k0 > 64.0 {
                break;
            }
            let ticks: Vec<(f64, String)> = (0..=(k1 - k0) as i64)
                .map(|i| {
                    let t = (k0 + i as f64) * step;
                    (t / xs, lc_time_text(t, dec, hours, sep))
                })
                .collect();
            let wmax = ticks.iter().map(|t| self.text.width(&t.1)).max().unwrap_or(0);
            if step * pps >= (wmax + room) as f64 {
                return ticks;
            }
            step = lc_time_step(step * 1.0001);
        }
        Vec::new()
    }

    // the summary of a line chart for screen readers: "Latenz – p50: aktuell 12 ms, min.
    // 9,8 ms, max. 41 ms; p95: ..."
    fn lc_summary(c: &LineChart, stats: &[Stat], empty: bool, sep: u32) -> String {
        let mut s = String::from(c.name);
        if !s.is_empty() {
            s.push_str(" – ");
        }
        let none = if c.empty.is_empty() { tr("Keine Daten") } else { c.empty };
        if empty || c.series.is_empty() {
            s.push_str(none);
            return s;
        }
        for (i, (ser, st)) in c.series.iter().zip(stats).enumerate() {
            if i > 0 {
                s.push_str("; ");
            }
            s.push_str(crate::basic::cstr(ser.label));
            s.push_str(": ");
            if st.has {
                let u = ser.unit;
                s.push_str(&format!(
                    "{} {}, {} {}, {} {}",
                    tr("aktuell"),
                    u.text(st.last, sep),
                    tr("min."),
                    u.text(st.lo, sep),
                    tr("max."),
                    u.text(st.hi, sep)
                ));
            } else {
                s.push_str(none);
            }
        }
        s
    }

    // a tag with a label: the colour's tint (high contrast: the background, framed) under
    // the text, a text record
    fn lc_tag(&mut self, rect: [i32; 4], label: &str, c: u32, lx: impl Fn(i32, i32) -> i32) {
        let hc = self.high_contrast;
        let rad = self.dp(3);
        let (fill, stroke, t) = if hc {
            (self.color(T_BG), self.color(T_TEXT), self.hair())
        } else {
            (color_mix(card_opaque(self), c | 0xFF00_0000, 52), 0, 0)
        };
        self.cmd_rrect(rect[0], rect[1], rect[2], rect[3], rad, t, fill, stroke, stroke);
        let tc = self.color(T_TEXT);
        let pad = self.dp(5);
        text_at(self, TS_CAPTION, rect[0] + pad, rect, label, tc, rect[2] - pad * 2);
        self.wp.ctx.last_rect = [lx(rect[0], rect[2]), rect[1], rect[2], rect[3]];
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(label), 0);
    }

    // the keys of a focused line chart -> its cursor (series, x of a point in the view v):
    // Left / Right the previous / next point of its series (as drawn: the plot stays left to
    // right in mirrored windows), Up / Down the series before / after, Home / End the first /
    // last point, Esc ends it; the first key starts it at the first series' last point
    fn lc_keys(&mut self, c: &LineChart, v: (f64, f64), cur: Option<(usize, f64)>, rtl: bool) -> Option<(usize, f64)> {
        let swap = rtl && self.mods & (MOD_CTRL | MOD_ALT) == 0;
        let (kp, kn) = if swap { (VK_RIGHT, VK_LEFT) } else { (VK_LEFT, VK_RIGHT) };
        let n = c.series.len();
        let last = |s: usize| {
            let d = c.series[s].data;
            lc_step(d, d.upper_bound(v.1), true, v).map(|t| (s, t))
        };
        let mut q = cur;
        let mut moved = false;
        for (key, back) in [(kp, true), (kn, false)] {
            if self.key(key) {
                moved = true;
                q = match q {
                    Some((s, t)) => {
                        let d = c.series[s].data;
                        let i = if back { d.lower_bound(t) } else { d.upper_bound(t) };
                        lc_step(d, i, back, v).map(|t| (s, t)).or(q)
                    }
                    None => last(0),
                };
            }
        }
        if self.key(VK_HOME) {
            moved = true;
            let s = q.map_or(0, |x| x.0);
            let d = c.series[s].data;
            q = lc_step(d, d.lower_bound(v.0), false, v).map(|t| (s, t)).or(q);
        }
        if self.key(VK_END) {
            moved = true;
            q = last(q.map_or(0, |x| x.0)).or(q);
        }
        for (key, up) in [(VK_UP, true), (VK_DOWN, false)] {
            if self.key(key) {
                moved = true;
                let (s, t) = q.unwrap_or((0, v.1));
                let s2 = if up { s.saturating_sub(1) } else { (s + 1).min(n - 1) };
                let d = c.series[s2].data;
                let p = d.nearest(t).and_then(|i| d.get(i)).filter(|p| p.0 >= v.0 && p.0 <= v.1);
                q = p.map(|p| (s2, p.0)).or(q);
            }
        }
        if q.is_some() && self.key(VK_ESCAPE) {
            q = None;
        }
        if moved {
            self.wp.ctx.focus_visible = true;
        }
        q
    }

    /// ui_line_chart(id, chart) -> the hovered series and the x it snapped to: series over
    /// time (or plain numbers) with axes, grid, legend, markers, thresholds and a hover card
    /// (see the module header of chart.rs)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn line_chart(&mut self, id: u32, c: &LineChart) -> Option<(usize, f64)> {
        let _src = self.src_scope();
        let hc = self.high_contrast;
        let sep = self.m.value.decimal_sep;
        let lh = self.ts_line(TS_CAPTION);
        let hair = self.hair();
        let n = c.series.len();
        let empty = c.series.iter().all(|s| s.data.is_empty());
        let xs = if c.x == LX_NANOS { 1e-9 } else { 1.0 }; // seconds per x (time axes)
        // the box of the plot and the legend below it
        self.box_begin(0, BOX_V, 6);
        self.next_size(-1, -1);
        let r = self.place(self.dp(480), self.dp(200));
        // how each series looks: high contrast takes the scheme's colours with dashes
        let pal = self.style.data().chart_palette(self);
        let hcc = [self.color(T_ACCENT), self.color(T_TEXT), self.color(T_ACCENT_TEXT)];
        let looks: Vec<Look> = c
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if hc {
                    Look { color: hcc[i % 3], dash: HC_DASH[i % 4], area: false }
                } else {
                    let color = if s.color != 0 { s.color } else { pal[s.pal.map_or(i, |k| k as usize) % 8] };
                    Look { color, dash: DASH_SOLID, area: s.area }
                }
            })
            .collect();
        let lw = (if hc { self.dp_fx(512) } else { self.dp_fx(384) }).max(256);
        let (t2, tx) = (self.color(T_TEXT2), self.color(T_TEXT));
        let mut st = self.m.chart.lines.remove(&id).unwrap_or_default();
        // the view and what each series shows of it
        let view = c.view();
        let mut h = Fnv::new();
        for s in c.series {
            h.u32(s.data.ver());
        }
        h.f64(view.0);
        h.f64(view.1);
        let skey = h.0;
        if st.stats_key != skey || st.stats.len() != n {
            st.stats = c.series.iter().map(|s| stat_of(s.data, view)).collect();
            st.stats_key = skey;
        }
        // the summary for screen readers (kept while the statistics and texts stay)
        let mut h = Fnv::new();
        h.u64(skey);
        h.u32(sep);
        h.u32(crate::i18n::language());
        h.str(c.name);
        h.str(c.empty);
        h.u32(empty as u32);
        for s in c.series {
            h.str(s.label);
            match s.unit {
                LcUnit::Num(u) => h.str(u),
                LcUnit::Si(u) => {
                    h.u32(1);
                    h.str(u);
                }
                LcUnit::Secs => h.u32(2),
            }
        }
        if st.sum_key != h.0 {
            st.summary = Ui::lc_summary(c, &st.stats, empty, sep);
            st.sum_key = h.0;
        }
        // the y range: the visible values and the thresholds, fixed ends as they are
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in st.stats.iter().filter(|s| s.has) {
            lo = lo.min(s.lo);
            hi = hi.max(s.hi);
        }
        for t in c.thresholds.iter().filter(|t| t.value.is_finite()) {
            lo = lo.min(t.value);
            hi = hi.max(t.value);
        }
        let (flo, fhi) = (c.y_range.0.is_finite(), c.y_range.1.is_finite());
        if flo {
            lo = c.y_range.0;
        }
        if fhi {
            hi = c.y_range.1;
        }
        if !(lo <= hi) {
            (lo, hi) = if flo {
                (lo, lo + 1.0)
            } else if fhi {
                (hi - 1.0, hi)
            } else {
                (0.0, 1.0)
            };
        }
        if hi == lo {
            let d = abs(lo).max(1.0) * if flo || fhi { 1.0 } else { 0.1 };
            if !fhi {
                hi = lo + d;
            } else if !flo {
                lo = hi - d;
            } else {
                hi = lo + 1.0;
            }
        }
        // the plot: the y labels at its left, half a line above it (the top label), the x
        // labels below; at most a y step per two lines
        let top = lh / 2;
        let ph = (r[3] - top - lh - self.dp(4)).max(1);
        let steps = (ph / (lh * 2).max(self.dp(28)).max(1)).clamp(1, 8) as u32;
        let (first, step, nst) = lc_nice(lo, hi, steps);
        let ylo = if flo { lo } else { first };
        let yhi = if fhi { hi } else { first + step * nst as f64 };
        let (ylo, yhi) = if yhi > ylo { (ylo, yhi) } else { (ylo, ylo + 1.0) };
        let (k0, k1) = (ceil_f(ylo / step - 1e-9), floor_f(yhi / step + 1e-9));
        let unit = c.series.first().map_or(LcUnit::Num(""), |s| s.unit);
        let ylab = if empty || k1 < k0 || k1 - k0 > 64.0 { Vec::new() } else { unit.ticks(k0 * step, step, (k1 - k0) as u32, sep) };
        self.ts_font(TS_CAPTION);
        let yw = ylab.iter().map(|s| self.text.width(s)).max().unwrap_or(0);
        let ylw = if yw > 0 { yw + self.dp(8) } else { 0 };
        let px0 = r[0] + ylw;
        let pw = (r[2] - ylw - self.dp(4)).max(1);
        let py0 = r[1] + top;
        let xt = if empty { Vec::new() } else { self.lc_x_ticks(c.x, view, pw, sep) };
        let sx = pw as f64 / (view.1 - view.0);
        let sy = ph as f64 / (yhi - ylo);
        let xpx = |x: f64| ((x - view.0) * sx).clamp(-1e6, 1e6);
        let ypx = |v: f64| (ph as f64 - (v - ylo) * sy).clamp(-1e6, 1e6);
        // the parts of the plot: grid, markers, thresholds (px relative to the plot)
        let mut grid: Vec<(i32, bool)> = Vec::new();
        let gy: Vec<i32> =
            (0..ylab.len()).map(|k| (floor_f(ypx((k0 + k as f64) * step) + 0.5) as i32).min(ph - hair).max(0)).collect();
        let gx: Vec<i32> = xt.iter().map(|t| (floor_f(xpx(t.0) + 0.5) as i32).min(pw - hair).max(0)).collect();
        if c.flags & LC_NO_GRID == 0 {
            grid.extend(gy.iter().map(|&y| (y, false)));
            grid.extend(gx.iter().map(|&x| (x, true)));
        }
        let mlw = self.dp(1).max(1);
        let mcolor = |m: &LcMarker| if hc { tx } else if m.color != 0 { m.color } else { t2 };
        let mut ranges: Vec<(i32, i32, u32)> = Vec::new();
        let mut rules: Vec<(i32, i32, u32)> = Vec::new();
        let mut tags: Vec<(i32, &str, u32)> = Vec::new(); // marker tags: x, label, colour
        for m in c.markers.iter().filter(|m| !empty && m.t0.is_finite()) {
            let (a, b) = (m.t0, mark_end(m));
            if b < view.0 || a > view.1 {
                continue;
            }
            let col = mcolor(m);
            let xa = floor_f(xpx(a) + 0.5) as i32;
            let rule = |x: i32| (x - mlw / 2).min(pw - mlw).max(0);
            if b > a {
                let xb = floor_f(xpx(b) + 0.5) as i32;
                let (ca, cb) = (xa.clamp(0, pw), xb.clamp(0, pw));
                if !hc && cb > ca {
                    ranges.push((ca, cb, (col & 0x00FF_FFFF) | 0x1C00_0000));
                }
                let edge = if hc { col } else { (col & 0x00FF_FFFF) | 0x9000_0000 };
                for x in [xa, xb] {
                    if x >= 0 && x <= pw {
                        rules.push((rule(x), mlw, edge));
                    }
                }
                tags.push((ca, m.label, col));
            } else {
                rules.push((rule(xa), mlw, col));
                tags.push((xa, m.label, col));
            }
        }
        let lt = self.dp(1).max(1);
        let mut limits: Vec<(i32, u32)> = Vec::new();
        let mut ltags: Vec<(i32, &str, u32)> = Vec::new(); // threshold tags: y, label, colour
        for t in c.thresholds.iter().filter(|t| !empty && t.value.is_finite() && t.value >= ylo && t.value <= yhi) {
            let col = if hc { tx } else if t.color != 0 { t.color } else { t2 };
            let y = (floor_f(ypx(t.value) + 0.5) as i32 - lt / 2).min(ph - lt).max(0);
            limits.push((y, col));
            ltags.push((y + lt / 2, t.label, col));
        }
        // the plot's drawing object: rebuilt when anything it shows changed
        let m = ((lw + 255) >> 8) + 1;
        let grid_c = self.color(T_DIVIDER);
        let mut h = Fnv::new();
        h.u64(skey);
        h.f64(ylo);
        h.f64(yhi);
        for v in [pw, ph, lw, hair, m, hc as i32, grid_c as i32, lt, mlw] {
            h.u32(v as u32);
        }
        for l in &looks {
            h.u32(l.color);
            h.u32(l.dash);
            h.u32(l.area as u32);
        }
        for &(p, v) in &grid {
            h.u32(p as u32 | (v as u32) << 31);
        }
        for &(a, b, col) in ranges.iter().chain(rules.iter()) {
            h.u32(a as u32);
            h.u32(b as u32);
            h.u32(col);
        }
        for &(y, col) in &limits {
            h.u32(y as u32);
            h.u32(col);
        }
        let okey = h.0;
        if st.obj_key != okey || st.obj.is_none() {
            let lwf = lw as f64 / 256.0;
            let mut areas = Vec::new();
            let mut traces = Vec::new();
            if !empty {
                for (i, s) in c.series.iter().enumerate() {
                    let l = looks[i];
                    let dense = st.stats[i].n > pw as usize * 2;
                    let (polys, band, area) = trace_of(s.data, view, (pw, ph), (ylo, yhi), lwf, dense, l.area, dash_pattern(l.dash));
                    if l.area && !area.is_empty() {
                        areas.push((area, (l.color & 0x00FF_FFFF) | 0x2E00_0000));
                    }
                    traces.push(Trace { polys, band, color: l.color, width: lw, dash: l.dash });
                }
            }
            let lim = (lt, self.dp(4), self.dp(3));
            let plot = Plot { m, w: pw, h: ph, hair, grid, grid_c, dotted: hc, ranges, areas, traces, limits, lim, rules };
            st.obj = Some(Rc::new(plot));
            st.obj_key = okey;
        }
        // ---- commands: the plot keeps its left-to-right order in mirrored windows
        self.ltr_begin(r[0], r[2]);
        let rtl = self.rtl;
        let lx = move |x: i32, w: i32| if rtl { r[0] + r[0] + r[2] - x - w } else { x };
        let ev = if c.flags & LC_NO_HOVER == 0 && !empty { self.hit_rect(id, lx(px0, pw), py0, pw, ph, HB_FOCUS | HB_MOVES) } else { 0 };
        // the keyboard's cursor on the points while the plot has the focus
        let focused = ev & EV_FOCUS != 0;
        let mut cursor = st.cursor.filter(|q| focused && q.0 < n && q.1 >= view.0 && q.1 <= view.1);
        if focused && n > 0 {
            cursor = self.lc_keys(c, view, cursor, rtl);
        }
        // the record first (a screen reader meets the summary before the details); with the
        // cursor its point before it ("p95, 1:45: 88,1 ms. Latenz – ...")
        let hours = abs(view.0 * xs).max(abs(view.1 * xs)) >= 3600.0;
        let point = cursor.and_then(|(s, t)| {
            let ser = &c.series[s];
            let v = ser.data.nearest(t).and_then(|i| ser.data.get(i))?;
            Some(format!("{}, {}: {}. ", crate::basic::cstr(ser.label), lc_x_label(c.x, t, hours, sep), ser.unit.text(v.1, sep)))
        });
        let label = match &point {
            Some(p) => format!("{}{}", p, st.summary),
            None => st.summary.clone(),
        };
        self.wp.ctx.last_rect = r;
        let mut ss = if ev & EV_HOVER != 0 { SS_HOVER as u32 } else { 0 };
        if focused {
            ss |= SS_FOCUSED as u32;
        }
        self.wp.ctx.last_state = ss;
        self.sem(K_CANVAS, id, Some(&label), n as i32);
        self.cmd_custom(px0 - m, py0 - m, pw + m * 2, ph + m * 2, st.obj.clone(), 0, okey as u32);
        // the y labels before the plot, centred on their grid lines; the x labels below,
        // centred under their ticks inside the chart, never closer than 4 DIP
        for (k, s) in ylab.iter().enumerate() {
            self.ts_font(TS_CAPTION);
            let w = self.text.width(s);
            let y = py0 + gy[k] + hair / 2 - lh / 2;
            let x = px0 - self.dp(6) - w;
            text_at(self, TS_CAPTION, x, [x, y, w, lh], s, t2, 0);
            self.wp.ctx.last_rect = [lx(x, w), y, w, lh];
            self.wp.ctx.last_state = 0;
            self.sem(K_TEXT, 0, Some(s), 0);
        }
        let ly = py0 + ph + self.dp(4);
        let mut end = i32::MIN;
        for (k, (_, s)) in xt.iter().enumerate() {
            self.ts_font(TS_CAPTION);
            let w = self.text.width(s);
            let x = (px0 + gx[k] - w / 2).min(r[0] + r[2] - w).max(px0);
            if x < end + self.dp(4) {
                continue;
            }
            end = x + w;
            text_at(self, TS_CAPTION, x, [x, ly, w, lh], s, t2, 0);
            self.wp.ctx.last_rect = [lx(x, w), ly, w, lh];
            self.wp.ctx.last_state = 0;
            self.sem(K_TEXT, 0, Some(s), 0);
        }
        // the tags: thresholds at the right end of their line, markers in rows from the top
        // (right of their line, left of it where there is no room); never over each other
        let pad = self.dp(5);
        let tsep = self.dp(4);
        let mut placed: Vec<[i32; 4]> = Vec::new();
        for &(y, label, col) in &ltags {
            if label.is_empty() {
                continue;
            }
            self.ts_font(TS_CAPTION);
            let w = (self.text.width(label) + pad * 2).min(pw);
            let ty = (py0 + y - lh / 2).min(py0 + ph - lh).max(py0);
            let rect = [px0 + pw - w - self.dp(4), ty, w, lh];
            if rect[0] < px0 || placed.iter().any(|&p| near(p, rect, tsep)) {
                continue;
            }
            placed.push(rect);
            self.lc_tag(rect, label, col, lx);
        }
        let rows = ((ph / 3) / (lh + self.dp(2)).max(1)).clamp(1, 3);
        for &(x, label, col) in &tags {
            if label.is_empty() {
                continue;
            }
            self.ts_font(TS_CAPTION);
            let w = (self.text.width(label) + pad * 2).min(pw);
            let xm = px0 + x;
            let x0 = if xm + self.dp(4) + w <= px0 + pw { xm + self.dp(4) } else { (xm - self.dp(4) - w).max(px0) };
            for row in 0..rows {
                let rect = [x0, py0 + self.dp(2) + row * (lh + self.dp(2)), w, lh];
                if !placed.iter().any(|&p| near(p, rect, tsep)) {
                    placed.push(rect);
                    self.lc_tag(rect, label, col, lx);
                    break;
                }
            }
        }
        // no points: the empty text in the middle
        if empty {
            let s = if c.empty.is_empty() { tr("Keine Daten") } else { c.empty };
            self.ts_font(TS_BODY);
            let lb = self.ts_line(TS_BODY);
            let w = self.text.width(s).min(r[2]);
            let (x, y) = (r[0] + (r[2] - w) / 2, r[1] + (r[3] - lb) / 2);
            text_at(self, TS_BODY, x, [x, y, w, lb], s, t2, r[2]);
            self.wp.ctx.last_rect = [lx(x, w), y, w, lb];
            self.wp.ctx.last_state = 0;
            self.sem(K_TEXT, 0, Some(s), 0);
        }
        // hover (else the keyboard's cursor): the crosshair at the nearest point, a dot on
        // every series, the card
        let mut out = None;
        let probe = if ev & EV_HOVER != 0 {
            let mx = self.ltr_x(self.wp.ctx.mx);
            Some((view.0 + ((mx - px0) as f64 + 0.5) / sx, None, Some(self.wp.ctx.my)))
        } else {
            cursor.map(|(s, t)| (t, Some(s), None))
        };
        if let Some((tp, fixed, my)) = probe {
            let inside = |p: &(f64, f64)| p.0 >= view.0 && p.0 <= view.1;
            let snap = if fixed.is_some() {
                Some(tp)
            } else {
                c.series
                    .iter()
                    .filter_map(|s| s.data.nearest(tp).and_then(|i| s.data.get(i)).filter(inside))
                    .map(|p| p.0)
                    .min_by(|a, b| abs(a - tp).total_cmp(&abs(b - tp)))
            };
            if let Some(ts) = snap {
                // each series' point there (its nearest one, within 24 DIP)
                let tol = self.dp(24) as f64 / sx;
                let at: Vec<Option<(f64, f64)>> = c
                    .series
                    .iter()
                    .map(|s| s.data.nearest(ts).and_then(|i| s.data.get(i)).filter(|p| inside(p) && abs(p.0 - ts) <= tol))
                    .collect();
                // the series nearest to the pointer (the cursor's own); a click puts the
                // cursor there
                let best = fixed.or_else(|| {
                    let my = my.unwrap_or(0) as f64;
                    at.iter()
                        .enumerate()
                        .filter_map(|(i, p)| p.map(|p| (i, abs(py0 as f64 + ypx(p.1) - my))))
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|b| b.0)
                });
                if let Some(bi) = best {
                    out = Some((bi, ts));
                    if ev & EV_ACT != 0 && fixed.is_none() {
                        cursor = Some((bi, ts));
                    }
                }
                // the card at the pointer's height, else at the cursor's point
                let my = my.unwrap_or_else(|| {
                    let v = best.and_then(|b| at[b]).map_or(0.0, |p| ypx(p.1).clamp(0.0, ph as f64));
                    py0 + v as i32
                });
                let cx = px0 + floor_f(xpx(ts) + 0.5) as i32;
                let cross = self.color(T_TEXT3);
                self.draw_color(cross);
                self.draw_rect(cx, py0, hair, ph);
                let ring = if hc { self.color(T_BG) } else { card_opaque(self) };
                let (ro, ri, grow) = (self.dp_fx(1152), self.dp_fx(768), self.dp_fx(256));
                for (i, p) in at.iter().enumerate() {
                    let Some(p) = p else { continue };
                    let y = ypx(p.1);
                    if !(y >= 0.0 && y <= ph as f64) {
                        continue;
                    }
                    let (fx, fy) = (((px0 as f64 + xpx(p.0)) * 256.0) as i32, ((py0 as f64 + y) * 256.0) as i32);
                    let g = if Some(i) == best { grow } else { 0 };
                    self.draw_color(ring);
                    self.draw_circle(fx, fy, ro + g, 0);
                    self.draw_color(looks[i].color);
                    self.draw_circle(fx, fy, ri + g, 0);
                }
                // the card: the x, every series' value there, the markers there
                let head = lc_x_label(c.x, ts, hours, sep);
                let mut rows: Vec<(u32, bool, &str, String)> = c
                    .series
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (looks[i].color, false, crate::basic::cstr(s.label), at[i].map_or(String::from("–"), |p| s.unit.text(p.1, sep))))
                    .collect();
                // the markers there with their time ("1:10 – 1:22")
                let half = self.dp(3) as f64 / sx;
                for mk in c.markers.iter().filter(|m| !m.label.is_empty() && m.t0.is_finite() && ts >= m.t0 - half && ts <= mark_end(m) + half) {
                    let mut when = lc_x_label(c.x, mk.t0, hours, sep);
                    if mk.t1 > mk.t0 {
                        when = format!("{} – {}", when, lc_x_label(c.x, mk.t1, hours, sep));
                    }
                    rows.push((mcolor(mk), true, crate::basic::cstr(mk.label), when));
                }
                self.lc_card(r, cx, my, &head, &rows);
            }
        }
        if self.focus_ring(ev) {
            let s = self.style;
            s.focus_ring(self, [px0, py0, pw, ph], 4);
        }
        self.ltr_end();
        // a new hover or cursor: once more, so what the app shows of it before the chart
        // (and the record's text) is current -- not while the layout still moves the plot
        // (that pass comes anyway, and the pointer means another point in each of them)
        let hk = out.map(|(i, t): (usize, f64)| (i, t.to_bits()));
        let moved = cursor.map(|q| (q.0, q.1.to_bits())) != st.cursor.map(|q| (q.0, q.1.to_bits()));
        let rect = [px0, py0, pw, ph];
        if (st.hover != hk || moved) && st.rect == rect {
            self.repass();
        }
        st.hover = hk;
        st.cursor = cursor;
        st.rect = rect;
        if self.m.chart.lines.len() > 64 {
            self.m.chart.lines.clear();
        }
        self.m.chart.lines.insert(id, st);
        // the legend: the series' labels (high contrast: line samples with their dashes)
        if c.flags & LC_NO_LEGEND == 0 && n > 0 {
            let items: Vec<(&str, u32)> = c.series.iter().enumerate().map(|(i, s)| (s.label, looks[i].color)).collect();
            let dashes: Vec<u32> = if hc { looks.iter().map(|l| l.dash).collect() } else { Vec::new() };
            self.legend_marks(&items, &dashes, lw);
        }
        self.box_end();
        out
    }

    // the hover card of a line chart beside the crosshair at cx (px, left to right inside
    // the chart's region r), at the pointer's height: the x, then rows (colour, marker,
    // label, value) -- a series' value after a dot, a marker's time after a bar; on the
    // tooltip layer, inside the window
    fn lc_card(&mut self, r: [i32; 4], cx: i32, my: i32, head: &str, rows: &[(u32, bool, &str, String)]) {
        let prev = self.layer_set(L_TIP);
        self.ltr_begin(r[0], r[2]);
        let rtl = self.rtl;
        let lx = move |x: i32, w: i32| if rtl { r[0] + r[0] + r[2] - x - w } else { x };
        let lh = self.ts_line(TS_CAPTION);
        let (padx, pady) = (self.dp(10), self.dp(6));
        let (dot, dgap, cgap) = (self.dp(8), self.dp(6), self.dp(16));
        self.ts_font(TS_CAPTION);
        let hw = self.text.width(head);
        let lws: Vec<i32> = rows.iter().map(|q| self.text.width(q.2)).collect();
        let vws: Vec<i32> = rows.iter().map(|q| self.text.width(&q.3)).collect();
        let (lmax, vmax) = (lws.iter().copied().max().unwrap_or(0), vws.iter().copied().max().unwrap_or(0));
        let w = (padx * 2 + hw.max(dot + dgap + lmax + cgap + vmax)).min(self.dp(360));
        let h = pady * 2 + lh * (1 + rows.len()) as i32;
        // beside the crosshair where there is room, inside the window (its edges as the
        // region left to right sees them)
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let d = if rtl { fw - r[0] - r[0] - r[2] } else { 0 };
        let (lo, hi) = (-d, fw - d);
        let mut x = cx + self.dp(14);
        if x + w > hi {
            x = cx - self.dp(14) - w;
        }
        let x = x.min(hi - w).max(lo);
        let y = (my - h / 2).min(fh - h).max(0);
        // the style's tooltip surface
        let st = self.style;
        let mut sp = Sp { label: Some(""), ..Sp::default() };
        sp.place([x, y, w, h]);
        st.tooltip(self, &sp);
        let (tx, t2) = (self.color(T_TEXT), self.color(T_TEXT2));
        let inner = w - padx * 2;
        let mut ty = y + pady;
        text_at(self, TS_CAPTION, x + padx, [x + padx, ty, inner, lh], head, t2, inner);
        self.wp.ctx.last_rect = [lx(x + padx, hw.min(inner)), ty, hw.min(inner), lh];
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(head), 0);
        ty += lh;
        let dy = (lh - dot) / 2;
        for (k, (col, mark, label, value)) in rows.iter().enumerate() {
            self.draw_color(*col);
            if *mark {
                self.draw_rect(x + padx + dot / 2 - self.dp(2), ty + dy, self.dp(4), dot);
            } else {
                self.draw_circle(((x + padx) << 8) + (dot << 7), ((ty + dy) << 8) + (dot << 7), dot << 7, 0);
            }
            let lx0 = x + padx + dot + dgap;
            let vw = vws[k].min(inner);
            let room = (inner - dot - dgap - vw - cgap).max(1);
            text_at(self, TS_CAPTION, lx0, [lx0, ty, room, lh], label, tx, room);
            let vx = x + w - padx - vw;
            text_at(self, TS_CAPTION, vx, [vx, ty, vw, lh], value, if *mark { t2 } else { tx }, inner);
            let rec = format!("{}: {}", label, value);
            self.wp.ctx.last_rect = [lx(lx0, inner - dot - dgap), ty, inner - dot - dgap, lh];
            self.wp.ctx.last_state = 0;
            self.sem(K_TEXT, 0, Some(&rec), 0);
            ty += lh;
        }
        self.ltr_end();
        self.layer_set(prev);
    }
}
