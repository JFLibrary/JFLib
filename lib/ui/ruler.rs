// ruler -- a ruler along a canvas (ZF-02): ticks and numbers in the document's units for the
// view of a ui.diagram canvas, the page's extent lighter, marks (the selection's edges and
// middle while it moves or changes size), the pointer's position; dragging out of it makes a
// guide (SP-10)
//
//   ui.ruler(id, &vp, &RulerSpec) -> RulerEv   a horizontal ruler (above the canvas) or a
//                                   vertical one (left of it) in the next layout slot (size it
//                                   with next_size: the canvas' width or height, 20 DIP across)
//   RulerSpec { vertical, origin, per_world, page, marks, pointer }
//   RulerEv::None | Guide { at, done }   a guide dragged out of the ruler: its world position
//                                   (y of a horizontal guide from the top ruler, x of a vertical
//                                   one from the left ruler) while the button is held, done when
//                                   released over the canvas (Esc: none)
//
// Units: a world coordinate c (the canvas', y down) shows as (c - origin) * per_world; a vertical
// ruler whose numbers grow upwards (a page with its origin at the bottom) has per_world < 0.  The
// step between numbered ticks is 1, 2 or 5 times a power of ten, at least 48 DIP apart; between
// them 10, 5 or 2 smaller ticks as there is room.  The ruler has a semantic record (K_SLIDER-like
// K_BOX "Lineal") so scripts drag from it.

use alloc::format;
use alloc::string::String;

use crate::consts::*;
use crate::diagram::{fl, fx, Viewport};
use crate::keys::VK_ESCAPE;
use crate::Ui;

/// what a ruler shows
#[derive(Clone, Copy, Debug)]
pub struct RulerSpec<'a> {
    /// a vertical ruler (left of the canvas) instead of a horizontal one
    pub vertical: bool,
    /// the world coordinate of 0
    pub origin: f32,
    /// units per world unit (negative: the numbers grow up / to the left)
    pub per_world: f64,
    /// the page's extent along the ruler (world, from .. to), drawn lighter
    pub page: [f32; 2],
    /// world coordinates to mark (edges and middle of the selection)
    pub marks: &'a [f32],
    /// the pointer's world coordinate along the ruler
    pub pointer: Option<f32>,
}

/// what the user did with a ruler
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum RulerEv {
    #[default]
    None,
    /// a guide is dragged out of the ruler: its world coordinate across the ruler; done =
    /// released over the canvas
    Guide { at: f32, done: bool },
    /// the drag of a guide ended outside the canvas or with Esc
    GuideCancel,
}

// a step of 1, 2 or 5 times a power of ten of at least `min` units
fn nice_step(min: f64) -> f64 {
    if !(min > 0.0) || !min.is_finite() {
        return 1.0;
    }
    let mut p = 1.0f64;
    while p > min {
        p /= 10.0;
    }
    while p * 10.0 <= min {
        p *= 10.0;
    }
    for k in [1.0, 2.0, 5.0, 10.0] {
        if p * k >= min {
            return p * k;
        }
    }
    p * 10.0
}

// floor of a finite f64 (clamped)
fn floor64(x: f64) -> i64 {
    let x = x.clamp(-1.0e15, 1.0e15);
    let t = x as i64;
    if (t as f64) > x { t - 1 } else { t }
}

// a number as short as it can be at this step
fn label(v: f64, step: f64) -> String {
    let dec = if step >= 1.0 { 0 } else if step >= 0.1 { 1 } else if step >= 0.01 { 2 } else { 3 };
    let v = if v.abs() < step * 1e-6 { 0.0 } else { v };
    let s = format!("{:.*}", dec, v);
    if s == "-0" { String::from("0") } else { s }
}

impl Ui {
    /// ui_ruler(id, vp, spec) -> what the user did: a ruler for the view vp of a canvas (see the
    /// file header)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ruler(&mut self, id: u32, vp: &Viewport, spec: &RulerSpec) -> RulerEv {
        let _src = self.src_scope();
        let _ = self.take_flags();
        let r = self.place(0, 0);
        let mut ev = RulerEv::None;
        if r[2] <= 0 || r[3] <= 0 {
            return ev;
        }
        let v = spec.vertical;
        let cur = if v { CUR_SIZE_WE } else { CUR_SIZE_NS };
        let e = self.behave(id, HB_GRAB_H | HB_GRAB_V | cur << HB_CURSOR_SHIFT);
        let c = &self.wp.ctx;
        let (mx, my) = (c.mx, c.my);
        let held = c.mbtn & (1 << BTN_LEFT) != 0;
        let mine = c.active == id;
        // a guide being dragged out: the world coordinate across the ruler
        if mine && (e & (EV_HELD | EV_DRAG) != 0 || held) {
            let w = vp.to_world(mx as f32 + 0.5, my as f32 + 0.5);
            let inside = mx >= vp.rect[0] && my >= vp.rect[1] && mx < vp.rect[0] + vp.rect[2] && my < vp.rect[1] + vp.rect[3];
            if inside {
                ev = RulerEv::Guide { at: if v { w[0] } else { w[1] }, done: false };
            }
            if self.key_peek(VK_ESCAPE) {
                ev = RulerEv::GuideCancel;
                self.wp.ctx.active = 0;
            }
        } else if e & EV_COMMIT != 0 || (mine && !held) {
            let inside = mx >= vp.rect[0] && my >= vp.rect[1] && mx < vp.rect[0] + vp.rect[2] && my < vp.rect[1] + vp.rect[3];
            let w = vp.to_world(mx as f32 + 0.5, my as f32 + 0.5);
            ev = if inside { RulerEv::Guide { at: if v { w[0] } else { w[1] }, done: true } } else { RulerEv::GuideCancel };
        }
        // the end of a drag changes the app's drawing: one more pass (the moves bring their own)
        if matches!(ev, RulerEv::Guide { done: true, .. } | RulerEv::GuideCancel) {
            self.repass();
        }
        // drawing
        let bg = self.color(T_LAYER) | 0xFF00_0000;
        let paper = self.color(T_CARD) | 0xFF00_0000;
        let line = self.color(T_DIVIDER) | 0xFF00_0000;
        let ink = self.color(T_TEXT2) | 0xFF00_0000;
        let accent = self.color(T_ACCENT);
        self.cmd_rrect(r[0], r[1], r[2], r[3], 0, 0, bg, 0, 0);
        // the page's extent
        let (a, b) = (vp.to_screen([spec.page[0], spec.page[0]]), vp.to_screen([spec.page[1], spec.page[1]]));
        let (p0, p1) = if v { (a[1].min(b[1]), a[1].max(b[1])) } else { (a[0].min(b[0]), a[0].max(b[0])) };
        self.clip_push(r[0], r[1], r[2], r[3]);
        if v {
            let (y0, y1) = (fl(p0).max(r[1]), fl(p1).min(r[1] + r[3]));
            if y1 > y0 {
                self.cmd_rrect(r[0], y0, r[2], y1 - y0, 0, 0, paper, 0, 0);
            }
        } else {
            let (x0, x1) = (fl(p0).max(r[0]), fl(p1).min(r[0] + r[2]));
            if x1 > x0 {
                self.cmd_rrect(x0, r[1], x1 - x0, r[3], 0, 0, paper, 0, 0);
            }
        }
        // the edge towards the canvas
        self.draw_color(line);
        let hair = self.hair().max(1);
        if v {
            self.draw_rect(r[0] + r[2] - hair, r[1], hair, r[3]);
        } else {
            self.draw_rect(r[0], r[1] + r[3] - hair, r[2], hair);
        }
        // ticks: px per unit, the step between numbers
        let scale = vp.scale(); // px per world unit
        let k = spec.per_world;
        if k != 0.0 && scale > 0.0 {
            let px_per_unit = scale / k.abs();
            let step = nice_step(self.dp(48) as f64 / px_per_unit);
            let sub = [10, 5, 2].into_iter().find(|&n| step * px_per_unit / n as f64 >= self.dp(5) as f64).unwrap_or(1);
            // the visible range in units
            let (s0, s1) = if v { (r[1] as f32, (r[1] + r[3]) as f32) } else { (r[0] as f32, (r[0] + r[2]) as f32) };
            let w0 = if v { vp.to_world(0.0, s0)[1] } else { vp.to_world(s0, 0.0)[0] };
            let w1 = if v { vp.to_world(0.0, s1)[1] } else { vp.to_world(s1, 0.0)[0] };
            let (u0, u1) = (((w0 - spec.origin) as f64) * k, ((w1 - spec.origin) as f64) * k);
            let (lo, hi) = (u0.min(u1), u0.max(u1));
            let minor = step / sub as f64;
            let n0 = floor64(lo / minor) - 1;
            let n1 = floor64(hi / minor) + 2;
            self.ts_font(TS_CAPTION);
            let (font, size) = self.text.get_font();
            let asc = self.text.ascent();
            let across = if v { r[2] } else { r[3] };
            for n in n0..=n1.min(n0 + 4000) {
                let u = n as f64 * minor;
                let wc = spec.origin as f64 + u / k;
                let q = vp.to_screen([wc as f32, wc as f32]);
                let at = if v { q[1] } else { q[0] };
                let major = n.rem_euclid(sub as i64) == 0;
                let half = sub % 2 == 0 && n.rem_euclid(sub as i64) == (sub as i64) / 2;
                let len = if major { across * 3 / 5 } else if half { across * 2 / 5 } else { across / 4 };
                self.draw_color(ink);
                if v {
                    let x1 = r[0] + r[2] - hair;
                    self.draw_line(fx((x1 - len) as f32), fx(at), fx(x1 as f32), fx(at), fx(hair as f32));
                } else {
                    let y1 = r[1] + r[3] - hair;
                    self.draw_line(fx(at), fx((y1 - len) as f32), fx(at), fx(y1 as f32), fx(hair as f32));
                }
                if major {
                    let s = label(u, step);
                    let tw = self.text.width(&s);
                    if v {
                        // the number turned would need a turned font: written small beside the tick
                        let x = r[0] + self.dp(2);
                        let y = fl(at) - self.dp(2);
                        if tw < r[2] - self.dp(4) || s.len() <= 3 {
                            self.cmd_text(x, y, s.as_bytes(), font, size, ink, r[2] - self.dp(3));
                        }
                    } else {
                        let x = fl(at) + self.dp(3);
                        let y = r[1] + asc + self.dp(1);
                        self.cmd_text(x, y, s.as_bytes(), font, size, ink, 0);
                    }
                }
            }
        }
        // marks of the selection, the pointer
        for (i, &m) in spec.marks.iter().chain(spec.pointer.iter()).enumerate() {
            let q = vp.to_screen([m, m]);
            let at = if v { q[1] } else { q[0] };
            let pointer = i >= spec.marks.len();
            self.draw_color(if pointer { ink } else { accent });
            let t = fx(if pointer { hair as f32 } else { (hair * 2) as f32 });
            if v {
                self.draw_line(fx(r[0] as f32), fx(at), fx((r[0] + r[2]) as f32), fx(at), t);
            } else {
                self.draw_line(fx(at), fx(r[1] as f32), fx(at), fx((r[1] + r[3]) as f32), t);
            }
        }
        self.clip_pop();
        self.sem(K_BOX, id, Some(crate::i18n::tr("Lineal")), 0);
        ev
    }
}
