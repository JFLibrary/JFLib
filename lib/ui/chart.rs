// chart -- small charts for dashboards: sparkline, bar chart (grouped or stacked), donut,
// legend.  Colours: 0 = the style's chart palette (StyleData::chart_palette) by index.
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
//
// Area, line and ring are vector paths filled at render time (cmd_custom); their drawing
// object is kept per id and replaced only when the data or the size change, so an unchanged
// chart does not redraw (dirty.rs hashes the object's identity and version).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::stock::{sincos, text_at};
use crate::Ui;

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

/// shared state: the drawing objects of the charts by id (hash, object)
#[derive(Default)]
pub(crate) struct S {
    cache: BTreeMap<u32, (u64, DrawRef)>,
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
    pub fn sparkline(&mut self, id: u32, values: &[f32], color: u32) {
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
    pub fn bar_chart(&mut self, id: u32, labels: &[&str], series: &[BarSeries], stacked: bool) -> i32 {
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
    pub fn donut(&mut self, id: u32, slices: &[DonutSlice], center: &str, sub: &str) {
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
    pub fn legend(&mut self, items: &[(&str, u32)]) {
        let s = self.dp(10);
        let (gap, igap) = (self.dp(12), self.dp(6));
        // the width the row gets (from the items' own widths, so a cut item does not narrow
        // the row it is measured against)
        self.ts_font(TS_CAPTION);
        let ws: Vec<i32> = items.iter().map(|&(label, _)| s + igap + self.text.width(crate::basic::cstr(label))).collect();
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
            let r = self.place(s, s);
            self.draw_color(col);
            self.draw_circle((r[0] << 8) + (s << 7), (r[1] << 8) + (r[3] << 7), s << 7, 0);
            if cut {
                self.next_size(-1, 0);
            }
            self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
            self.box_end();
        }
        self.box_end();
    }
}
