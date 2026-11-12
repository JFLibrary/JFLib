// block -- blocks and their inserts on the CPU (G6b2; docs/RENDER2D.md "Blöcke"): an insert's transform as an affine map
// in f64, composed for nested inserts; what the GPU gets per instance (cull.wgsl Inst); a hatch's lines through a
// transform.  The scene's side (blocks, variants, instances, entries) is in scene.rs.

use crate::math::{sincos, split, sqrt};
use crate::scene::{split3, HatchLine, Insert, Style, HATCH_DASH_MAX, HATCH_MAX};
use crate::INST_WORDS;

/// an affine map: world = t + m local (m[row][column])
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Xf {
    pub t: [f64; 2],
    pub m: [[f64; 2]; 2],
}

impl Xf {
    pub fn lin(&self, v: [f64; 2]) -> [f64; 2] {
        [self.m[0][0] * v[0] + self.m[0][1] * v[1], self.m[1][0] * v[0] + self.m[1][1] * v[1]]
    }

    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let v = self.lin(p);
        [self.t[0] + v[0], self.t[1] + v[1]]
    }

    /// self after inner: world = self(inner(p))
    pub fn then(&self, inner: &Xf) -> Xf {
        let a = &self.m;
        let b = &inner.m;
        Xf {
            t: self.apply(inner.t),
            m: [[a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]], [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]]],
        }
    }

    pub fn det(&self) -> f64 {
        self.m[0][0] * self.m[1][1] - self.m[0][1] * self.m[1][0]
    }

    /// m's inverse
    pub fn inv(&self) -> [[f64; 2]; 2] {
        let d = self.det();
        [[self.m[1][1] / d, -self.m[0][1] / d], [-self.m[1][0] / d, self.m[0][0] / d]]
    }

    /// the scale of widths and patterns in units: sqrt |det| (an even scale's own)
    pub fn sw(&self) -> f64 {
        sqrt(self.det().abs())
    }

    /// m's largest and smallest singular values
    pub fn singular(&self) -> (f64, f64) {
        let (a, b, c, d) = (self.m[0][0], self.m[0][1], self.m[1][0], self.m[1][1]);
        let s1 = a * a + b * b + c * c + d * d;
        let dt = (a * d - b * c).abs();
        let r = sqrt((s1 * s1 - 4.0 * dt * dt).max(0.0));
        (sqrt(0.5 * (s1 + r)), sqrt((0.5 * (s1 - r)).max(0.0)))
    }

    /// a similarity: a rotation, mirrored or not, times an even scale (to 1e-9 of it); else the insert scales unevenly
    pub fn even(&self) -> bool {
        let m = &self.m;
        let n = m[0][0].abs() + m[0][1].abs() + m[1][0].abs() + m[1][1].abs();
        (m[0][0] - m[1][1]).abs() + (m[0][1] + m[1][0]).abs() <= 1e-9 * n || (m[0][0] + m[1][1]).abs() + (m[0][1] - m[1][0]).abs() <= 1e-9 * n
    }
}

/// the transforms of an insert's copies (an array: rows, then columns in each), from the local units of its block (points
/// less the block's base) to those of where it sits (`origin` taken off: the base of the block it sits in, else none)
pub(crate) fn insert_xfs(ins: &Insert, origin: [f64; 2], out: &mut alloc::vec::Vec<Xf>) {
    let (s, c) = sincos(ins.rotation);
    let m = [[c * ins.scale[0], -s * ins.scale[1]], [s * ins.scale[0], c * ins.scale[1]]];
    for row in 0..ins.rows.max(1) {
        for col in 0..ins.columns.max(1) {
            let (dx, dy) = (col as f64 * ins.spacing[0], row as f64 * ins.spacing[1]);
            let t = [ins.at[0] - origin[0] + (c * dx - s * dy), ins.at[1] - origin[1] + (s * dx + c * dy)];
            out.push(Xf { t, m });
        }
    }
}

/// cull.wgsl's Inst of an instance: its transform (t three f32 per axis, m's inverse three f32 per entry, m), its scale
/// for widths (hi, lo), its block's box (local, as points hi.x, hi.y, lo.x, lo.y), how far its strokes reach beyond it
/// (px, block units), mirrored, the style by block, the first row of its hatches, its fill edges' offset in epx
pub(crate) fn inst_words(xf: &Xf, bounds: [f64; 4], reach: (f32, f32), ins: u32, hatch: u32, eoff: u32) -> [u32; INST_WORDS] {
    let b = |v: f32| v.to_bits();
    let mut w = [0u32; INST_WORDS];
    let (x0, x1, x2) = split3(xf.t[0]);
    let (y0, y1, y2) = split3(xf.t[1]);
    let (sh, sl) = split(xf.sw());
    let n = xf.inv();
    let mut put = |k: usize, v: f32| w[k] = b(v);
    for (k, v) in [x0, x1, x2, y0, y1, y2, sh, sl].into_iter().enumerate() {
        put(k, v);
    }
    let mut k = 8;
    for e in [n[0][0], n[0][1], n[1][0], n[1][1]] {
        let (a, bb, c) = split3(e);
        put(k, a);
        put(k + 1, bb);
        put(k + 2, c);
        k += 3;
    }
    for (j, v) in [xf.m[0][0], xf.m[0][1], xf.m[1][0], xf.m[1][1]].into_iter().enumerate() {
        put(20 + j, v as f32);
    }
    let (ax, lax) = split(bounds[0]);
    let (ay, lay) = split(bounds[1]);
    let (bx, lbx) = split(bounds[2]);
    let (by, lby) = split(bounds[3]);
    for (j, v) in [ax, ay, lax, lay, bx, by, lbx, lby].into_iter().enumerate() {
        put(24 + j, v);
    }
    put(32, reach.0);
    put(33, reach.1);
    w[36] = if xf.det() < 0.0 { 2 } else { 0 };
    w[37] = ins;
    w[38] = hatch;
    w[39] = eoff;
    w
}

/// a hatch family as Style::hatch_words takes it: its direction (unit), base point, spacing, shift along its lines, dash
/// pattern (drawing units)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fam {
    pub dir: [f64; 2],
    pub base: [f64; 2],
    pub spacing: f64,
    pub shift: f64,
    pub dash: [f64; HATCH_DASH_MAX],
    pub dashes: u32,
}

impl Fam {
    pub fn of(h: &HatchLine) -> Fam {
        let (sa, ca) = sincos(h.angle);
        Fam { dir: [ca, sa], base: h.base, spacing: h.spacing, shift: h.shift, dash: h.dash, dashes: h.dashes }
    }

    /// the family through an insert's transform (its base point less the block's base first): parallel lines again, along m
    /// dir, as far apart as m takes the spacing across them, the pattern and the shift along them scaled with m dir (and
    /// by how far m pushes a line's base along it); a mirror turns the lines' order around
    pub fn through(&self, xf: &Xf, origin: [f64; 2]) -> Fam {
        let d = xf.lin(self.dir);
        let l = sqrt(d[0] * d[0] + d[1] * d[1]);
        let du = [d[0] / l, d[1] / l];
        let nn = [-du[1], du[0]];
        let v = xf.lin([-self.dir[1] * self.spacing, self.dir[0] * self.spacing]);
        let a = v[0] * nn[0] + v[1] * nn[1];
        let c = v[0] * du[0] + v[1] * du[1];
        let mut dash = self.dash;
        for x in &mut dash {
            *x *= l;
        }
        Fam { dir: du, base: xf.apply([self.base[0] - origin[0], self.base[1] - origin[1]]), spacing: a.abs(), shift: a.signum() * (c + l * self.shift), dash, dashes: self.dashes }
    }
}

/// a style's hatch families (at most HATCH_MAX)
pub(crate) fn fams(st: &Style) -> ([Fam; HATCH_MAX], usize) {
    let n = (st.hatches as usize).min(HATCH_MAX);
    let mut f = [Fam::of(&st.hatch[0]); HATCH_MAX];
    for k in 0..n {
        f[k] = Fam::of(&st.hatch[k]);
    }
    (f, n)
}
