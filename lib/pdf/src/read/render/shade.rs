// Shadings and patterns (ISO 32000-1, 8.7): function-based, axial and radial shadings by pixel
// (axial and radial through a table of 1024 colours over their domain, with Extend), the mesh
// shadings (free-form and lattice triangles, Coons and tensor-product patches, the patches
// cut into a grid of triangles fine enough for the device; each vertex, and each patch, begins
// on a byte as Poppler and pdf.js read them) drawn with Gouraud colours and without
// antialiasing between their triangles; sh fills the clip (and the shading's BBox),
// a shading pattern fills a path, with its Background where the shading does not reach.
// Tiling patterns draw one cell at device resolution into a tile (coloured, or uncoloured in
// the colour of the scn) that repeats with the pattern's steps.

use super::super::object::{Dict, Object, Stream};
use super::super::Document;
use super::canvas::{self, Clip, Layer, Put, Rect, Shader, Src};
use super::color::{ceil, floor, sqrt, Cs, Func};
use super::{apply, inv, mul, to8, Paint, Res, Run, M, MAX_DEPTH};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{OnceCell, RefCell};
use gfx::path::Path;
use gfx::RULE_NONZERO;

const LUT: usize = 1024;
const MAX_TRIANGLES: usize = 1 << 20;
const MAX_TILE: f64 = 2048.0;

#[derive(Clone, Copy)]
struct Vtx {
    x: f64,
    y: f64,
    c: [f64; 8],
}

enum Kind {
    Func { domain: [f64; 4], matrix: M },
    Axial { c: [f64; 4], d: [f64; 2], ext: [bool; 2] },
    Radial { c: [f64; 6], d: [f64; 2], ext: [bool; 2] },
    Mesh { tris: Vec<[Vtx; 3]> },
    Patches { patches: Vec<([(f64, f64); 16], [[f64; 8]; 4])> },
}

pub(super) struct Shading {
    kind: Kind,
    cs: Rc<Cs>,
    f: Option<Rc<Func>>,
    ncomp: usize,
    pub bbox: Option<[f64; 4]>,
    background: Option<[u8; 3]>,
    lut: OnceCell<Vec<[u8; 4]>>,
}

/// reads n-bit values MSB first
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: usize) -> Option<u64> {
        if n == 0 || n > 32 {
            return if n == 0 { Some(0) } else { None };
        }
        if self.pos + n > self.d.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            let b = self.d[self.pos / 8] >> (7 - self.pos % 8) & 1;
            v = v << 1 | b as u64;
            self.pos += 1;
        }
        Some(v)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

impl Shading {
    pub fn parse(doc: &Document, o: &Object, res: &Dict) -> Option<Shading> {
        let o = doc.resolve(o);
        let (d, stream): (Dict, Option<Stream>) = match &o {
            Object::Dict(d) => (d.clone(), None),
            Object::Stream(s) => (s.dict.clone(), Some((**s).clone())),
            _ => return None,
        };
        let cs = Cs::parse(doc, d.get("ColorSpace")?, res)?;
        let f = d.get("Function").and_then(|f| Func::parse(doc, f));
        let ncomp = if f.is_some() { 1 } else { cs.n() };
        let bbox = doc.key(&d, "BBox").as_rect();
        let background = {
            let b = super::color::nums(doc, &doc.key(&d, "Background"));
            (b.len() == cs.n()).then(|| {
                let c = cs.rgb(&b);
                [to8(c[0]), to8(c[1]), to8(c[2])]
            })
        };
        let nums = |k: &str| super::color::nums(doc, &doc.key(&d, k));
        let ext = {
            let e = doc.key(&d, "Extend");
            let a = e.as_array().unwrap_or(&[]);
            [a.first().and_then(|v| doc.resolve(v).as_bool()).unwrap_or(false), a.get(1).and_then(|v| doc.resolve(v).as_bool()).unwrap_or(false)]
        };
        let dom2 = {
            let v = nums("Domain");
            if v.len() >= 2 { [v[0], v[1]] } else { [0.0, 1.0] }
        };
        let kind = match doc.key(&d, "ShadingType").as_int()? {
            1 => {
                f.as_ref()?;
                let v = nums("Domain");
                let domain = if v.len() >= 4 { [v[0], v[1], v[2], v[3]] } else { [0.0, 1.0, 0.0, 1.0] };
                let matrix = super::m6(&nums("Matrix")).unwrap_or(super::ID);
                Kind::Func { domain, matrix }
            }
            2 => {
                f.as_ref()?;
                let c = nums("Coords");
                if c.len() < 4 {
                    return None;
                }
                Kind::Axial { c: [c[0], c[1], c[2], c[3]], d: dom2, ext }
            }
            3 => {
                f.as_ref()?;
                let c = nums("Coords");
                if c.len() < 6 {
                    return None;
                }
                Kind::Radial { c: [c[0], c[1], c[2], c[3], c[4], c[5]], d: dom2, ext }
            }
            t @ 4..=7 => {
                let data = doc.decode(&stream?).ok()?;
                let bpcoord = doc.key(&d, "BitsPerCoordinate").as_int()? as usize;
                let bpcomp = doc.key(&d, "BitsPerComponent").as_int()? as usize;
                let bpflag = if t == 5 { 0 } else { doc.key(&d, "BitsPerFlag").as_int().unwrap_or(8) as usize };
                let dec = nums("Decode");
                if dec.len() < 4 + 2 * ncomp || ncomp > 8 || bpcoord > 32 || bpcomp > 16 {
                    return None;
                }
                let mut b = Bits { d: &data, pos: 0 };
                let cmax = ((1u64 << bpcoord) - 1) as f64;
                let kmax = ((1u64 << bpcomp) - 1) as f64;
                let read_pt = |b: &mut Bits| -> Option<(f64, f64)> {
                    let x = b.get(bpcoord)? as f64;
                    let y = b.get(bpcoord)? as f64;
                    Some((dec[0] + x * (dec[1] - dec[0]) / cmax, dec[2] + y * (dec[3] - dec[2]) / cmax))
                };
                let read_c = |b: &mut Bits| -> Option<[f64; 8]> {
                    let mut c = [0f64; 8];
                    for (i, v) in c.iter_mut().enumerate().take(ncomp) {
                        let s = b.get(bpcomp)? as f64;
                        *v = dec[4 + 2 * i] + s * (dec[5 + 2 * i] - dec[4 + 2 * i]) / kmax;
                    }
                    Some(c)
                };
                match t {
                    4 => {
                        let mut tris = Vec::new();
                        let mut prev: Vec<Vtx> = Vec::new();
                        while tris.len() < MAX_TRIANGLES {
                            let Some(flag) = b.get(bpflag) else { break };
                            let Some((x, y)) = read_pt(&mut b) else { break };
                            let Some(c) = read_c(&mut b) else { break };
                            b.align();
                            let v = Vtx { x, y, c };
                            match flag {
                                0 => {
                                    let mut vs = vec![v];
                                    let mut ok = true;
                                    for _ in 0..2 {
                                        let _ = b.get(bpflag);
                                        match (read_pt(&mut b), read_c(&mut b)) {
                                            (Some((x, y)), Some(c)) => vs.push(Vtx { x, y, c }),
                                            _ => ok = false,
                                        }
                                        b.align();
                                    }
                                    if !ok {
                                        break;
                                    }
                                    tris.push([vs[0], vs[1], vs[2]]);
                                    prev = vs;
                                }
                                1 | 2 if prev.len() == 3 => {
                                    let t3 = if flag == 1 { [prev[1], prev[2], v] } else { [prev[0], prev[2], v] };
                                    tris.push(t3);
                                    prev = t3.to_vec();
                                }
                                _ => break,
                            }
                        }
                        Kind::Mesh { tris }
                    }
                    5 => {
                        let per_row = doc.key(&d, "VerticesPerRow").as_int().unwrap_or(0);
                        if !(2..=1 << 16).contains(&per_row) {
                            return None;
                        }
                        let per_row = per_row as usize;
                        let mut rows: Vec<Vec<Vtx>> = Vec::new();
                        'rows: loop {
                            let mut row = Vec::with_capacity(per_row);
                            for _ in 0..per_row {
                                let (Some((x, y)), Some(c)) = (read_pt(&mut b), read_c(&mut b)) else { break 'rows };
                                b.align();
                                row.push(Vtx { x, y, c });
                            }
                            rows.push(row);
                            if rows.len() * per_row * 2 > MAX_TRIANGLES {
                                break;
                            }
                        }
                        let mut tris = Vec::new();
                        for r in rows.windows(2) {
                            for i in 0..per_row - 1 {
                                tris.push([r[0][i], r[0][i + 1], r[1][i]]);
                                tris.push([r[0][i + 1], r[1][i + 1], r[1][i]]);
                            }
                        }
                        Kind::Mesh { tris }
                    }
                    _ => {
                        // patches: the points in the order of 8.7.4.5.7, as a 4x4 grid p[i][j]
                        let order: [(usize, usize); 16] =
                            [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3), (3, 2), (3, 1), (3, 0), (2, 0), (1, 0), (1, 1), (1, 2), (2, 2), (2, 1)];
                        let npts = if t == 6 { 12 } else { 16 };
                        let mut patches: Vec<([(f64, f64); 16], [[f64; 8]; 4])> = Vec::new();
                        let mut prev: Option<([[(f64, f64); 4]; 4], [[f64; 8]; 4])> = None;
                        while patches.len() < MAX_TRIANGLES / 32 {
                            let Some(flag) = b.get(bpflag) else { break };
                            let mut g = [[(0.0, 0.0); 4]; 4];
                            // the corner colours c00, c03, c33, c30
                            let mut cc = [[0f64; 8]; 4];
                            let (from, cfrom) = if flag == 0 {
                                (0, 0)
                            } else {
                                let Some((pg, pc)) = prev else { break };
                                // the shared edge of the previous patch becomes p00..p03
                                let (edge, c0, c1) = match flag {
                                    1 => ([pg[0][3], pg[1][3], pg[2][3], pg[3][3]], pc[1], pc[2]),
                                    2 => ([pg[3][3], pg[3][2], pg[3][1], pg[3][0]], pc[2], pc[3]),
                                    3 => ([pg[3][0], pg[2][0], pg[1][0], pg[0][0]], pc[3], pc[0]),
                                    _ => break,
                                };
                                for (k, p) in edge.iter().enumerate() {
                                    g[0][k] = *p;
                                }
                                cc[0] = c0;
                                cc[1] = c1;
                                (4, 2)
                            };
                            let mut ok = true;
                            for &(i, j) in order.iter().take(npts).skip(from) {
                                match read_pt(&mut b) {
                                    Some(p) => g[i][j] = p,
                                    None => ok = false,
                                }
                            }
                            for c in cc.iter_mut().skip(cfrom) {
                                match read_c(&mut b) {
                                    Some(v) => *c = v,
                                    None => ok = false,
                                }
                            }
                            if !ok {
                                break;
                            }
                            if t == 6 {
                                coons_inner(&mut g);
                            }
                            let mut flat = [(0.0, 0.0); 16];
                            for i in 0..4 {
                                for j in 0..4 {
                                    flat[i * 4 + j] = g[i][j];
                                }
                            }
                            patches.push((flat, cc));
                            prev = Some((g, cc));
                            b.align();
                        }
                        Kind::Patches { patches }
                    }
                }
            }
            _ => return None,
        };
        Some(Shading { kind, cs, f, ncomp, bbox, background, lut: OnceCell::new() })
    }

    /// the colour of components c (the function's input when there is one)
    fn color(&self, c: &[f64], out: &mut Vec<f64>) -> [u8; 3] {
        let v = match &self.f {
            Some(f) => {
                f.eval(&c[..self.ncomp.min(c.len())], out);
                out.resize(self.cs.n(), 0.0);
                self.cs.rgb(out)
            }
            None => self.cs.rgb(&c[..self.ncomp.min(c.len())]),
        };
        [to8(v[0]), to8(v[1]), to8(v[2])]
    }

    /// axial and radial: the colours over the domain
    fn table(&self) -> &[[u8; 4]] {
        self.lut.get_or_init(|| {
            let d = match &self.kind {
                Kind::Axial { d, .. } | Kind::Radial { d, .. } => *d,
                _ => [0.0, 1.0],
            };
            let mut out = Vec::new();
            (0..LUT)
                .map(|i| {
                    let t = d[0] + (d[1] - d[0]) * i as f64 / (LUT - 1) as f64;
                    let c = self.color(&[t], &mut out);
                    [c[0], c[1], c[2], 255]
                })
                .collect()
        })
    }
}

/// the inner points of a Coons patch as a tensor-product patch has them
fn coons_inner(p: &mut [[(f64, f64); 4]; 4]) {
    let f = |a: (f64, f64), k: f64| (a.0 * k, a.1 * k);
    let add = |a: (f64, f64), b: (f64, f64)| (a.0 + b.0, a.1 + b.1);
    let sum = |t: &[((f64, f64), f64)]| t.iter().fold((0.0, 0.0), |acc, &(v, k)| add(acc, f(v, k)));
    let q = *p;
    // (-4 corner + 6 (its two neighbours) - 2 (the far ends of their edges) + 3 (the two points
    // next to those) - the opposite corner) / 9, as 8.7.4.5.7 gives them
    let x = |c: [(f64, f64); 8]| -> (f64, f64) {
        let s = sum(&[(c[0], -4.0), (c[1], 6.0), (c[2], 6.0), (c[3], -2.0), (c[4], -2.0), (c[5], 3.0), (c[6], 3.0), (c[7], -1.0)]);
        f(s, 1.0 / 9.0)
    };
    p[1][1] = x([q[0][0], q[0][1], q[1][0], q[0][3], q[3][0], q[3][1], q[1][3], q[3][3]]);
    p[1][2] = x([q[0][3], q[0][2], q[1][3], q[0][0], q[3][3], q[3][2], q[1][0], q[3][0]]);
    p[2][2] = x([q[3][3], q[3][2], q[2][3], q[3][0], q[0][3], q[2][0], q[0][2], q[0][0]]);
    p[2][1] = x([q[3][0], q[3][1], q[2][0], q[3][3], q[0][0], q[0][1], q[2][3], q[0][3]]);
}

fn bern(t: f64) -> [f64; 4] {
    let u = 1.0 - t;
    [u * u * u, 3.0 * t * u * u, 3.0 * t * t * u, t * t * t]
}

/// a shading's colours by pixel, for the pixels of `area`
struct Analytic {
    sh: Rc<Shading>,
    /// device -> shading space
    inv: M,
    background: Option<[u8; 3]>,
}

impl Analytic {
    fn at(&self, x: f64, y: f64, tmp: &mut Vec<f64>) -> [u8; 4] {
        let sh = &*self.sh;
        let bg = |b: Option<[u8; 3]>| b.map_or([0; 4], |c| [c[0], c[1], c[2], 255]);
        match &sh.kind {
            Kind::Axial { c, ext, .. } => {
                let (dx, dy) = (c[2] - c[0], c[3] - c[1]);
                let den = dx * dx + dy * dy;
                let mut t = if den > 0.0 { ((x - c[0]) * dx + (y - c[1]) * dy) / den } else { 0.0 };
                if t < 0.0 {
                    if !ext[0] {
                        return bg(self.background);
                    }
                    t = 0.0;
                } else if t > 1.0 {
                    if !ext[1] {
                        return bg(self.background);
                    }
                    t = 1.0;
                }
                sh.table()[(t * (LUT - 1) as f64 + 0.5) as usize]
            }
            Kind::Radial { c, ext, .. } => {
                let (x0, y0, r0, x1, y1, r1) = (c[0], c[1], c[2], c[3], c[4], c[5]);
                let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
                let (px, py) = (x - x0, y - y0);
                let a = cdx * cdx + cdy * cdy - dr * dr;
                let b = px * cdx + py * cdy + r0 * dr;
                let cc = px * px + py * py - r0 * r0;
                let mut ts = [f64::NAN; 2];
                if a.abs() < 1e-9 {
                    if b.abs() > 1e-12 {
                        ts[0] = cc / (2.0 * b);
                    }
                } else {
                    let disc = b * b - a * cc;
                    if disc >= 0.0 {
                        let s = sqrt(disc);
                        let (t1, t2) = ((b + s) / a, (b - s) / a);
                        ts = if t1 > t2 { [t1, t2] } else { [t2, t1] };
                    }
                }
                for t in ts {
                    if !t.is_finite() || r0 + t * dr < 0.0 {
                        continue;
                    }
                    let tt = if t < 0.0 {
                        if !ext[0] {
                            continue;
                        }
                        0.0
                    } else if t > 1.0 {
                        if !ext[1] {
                            continue;
                        }
                        1.0
                    } else {
                        t
                    };
                    return sh.table()[(tt * (LUT - 1) as f64 + 0.5) as usize];
                }
                bg(self.background)
            }
            Kind::Func { domain, matrix } => {
                let Some(mi) = inv(matrix) else { return [0; 4] };
                let (u, v) = apply(&mi, x, y);
                if u < domain[0] || u > domain[1] || v < domain[2] || v > domain[3] {
                    return bg(self.background);
                }
                let Some(f) = &sh.f else { return [0; 4] };
                f.eval(&[u, v], tmp);
                tmp.resize(sh.cs.n(), 0.0);
                let c = sh.cs.rgb(tmp);
                [to8(c[0]), to8(c[1]), to8(c[2]), 255]
            }
            _ => [0; 4],
        }
    }
}

impl Shader for Analytic {
    fn row(&self, y: i32, x: i32, out: &mut [[u8; 4]]) {
        let mut tmp = Vec::new();
        for (i, o) in out.iter_mut().enumerate() {
            let (sx, sy) = apply(&self.inv, x as f64 + i as f64 + 0.5, y as f64 + 0.5);
            *o = self.at(sx, sy, &mut tmp);
        }
    }
}

/// colours drawn beforehand into a layer over the area (meshes, tiles)
struct Prepared {
    layer: Layer,
}

impl Shader for Prepared {
    fn row(&self, y: i32, x: i32, out: &mut [[u8; 4]]) {
        let r = self.layer.rect;
        for (i, o) in out.iter_mut().enumerate() {
            let xx = x + i as i32;
            if xx < r[0] || xx >= r[2] || y < r[1] || y >= r[3] {
                *o = [0; 4];
                continue;
            }
            let p = &self.layer.px[self.layer.at(xx, y)..][..4];
            *o = [p[0], p[1], p[2], p[3]];
        }
    }
}

/// a tile repeating over the plane
struct Tiled {
    tile: Rc<Layer>,
    /// device -> tile pixels (before wrapping)
    m: M,
}

impl Shader for Tiled {
    fn row(&self, y: i32, x: i32, out: &mut [[u8; 4]]) {
        let (tw, th) = (self.tile.rect[2] as f64, self.tile.rect[3] as f64);
        for (i, o) in out.iter_mut().enumerate() {
            let (u, v) = apply(&self.m, x as f64 + i as f64 + 0.5, y as f64 + 0.5);
            let u = u - floor(u / tw) * tw;
            let v = v - floor(v / th) * th;
            let tx = (u as i32).clamp(0, self.tile.rect[2] - 1);
            let ty = (v as i32).clamp(0, self.tile.rect[3] - 1);
            let p = &self.tile.px[self.tile.at(tx, ty)..][..4];
            let a = p[3] as u32;
            *o = if a == 0 {
                [0; 4]
            } else if a == 255 {
                [p[0], p[1], p[2], 255]
            } else {
                let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
                [un(p[0]), un(p[1]), un(p[2]), p[3]]
            };
        }
    }
}

/// the triangles of a mesh shading (patches cut into a grid), in shading space
fn triangles(sh: &Shading, m: &M) -> Vec<[Vtx; 3]> {
    match &sh.kind {
        Kind::Mesh { tris } => tris.clone(),
        Kind::Patches { patches } => {
            let mut out = Vec::new();
            for (p, cc) in patches {
                // the grid: fine enough that a cell is about 3 device pixels
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, y) in p {
                    let (a, b) = apply(m, x, y);
                    x0 = x0.min(a);
                    y0 = y0.min(b);
                    x1 = x1.max(a);
                    y1 = y1.max(b);
                }
                let size = (x1 - x0).max(y1 - y0);
                let n = (ceil(size / 3.0) as usize).clamp(2, 64);
                let mut grid: Vec<Vtx> = Vec::with_capacity((n + 1) * (n + 1));
                for i in 0..=n {
                    let s = i as f64 / n as f64;
                    let bs = bern(s);
                    for j in 0..=n {
                        let t = j as f64 / n as f64;
                        let bt = bern(t);
                        let (mut x, mut y) = (0.0, 0.0);
                        for a in 0..4 {
                            for b in 0..4 {
                                let w = bs[a] * bt[b];
                                x += p[a * 4 + b].0 * w;
                                y += p[a * 4 + b].1 * w;
                            }
                        }
                        // the corner colours bilinear: c00 (s 0, t 0), c03 (0, 1), c33 (1, 1), c30 (1, 0)
                        let mut c = [0f64; 8];
                        for (k, v) in c.iter_mut().enumerate() {
                            *v = (1.0 - s) * (1.0 - t) * cc[0][k] + (1.0 - s) * t * cc[1][k] + s * t * cc[2][k] + s * (1.0 - t) * cc[3][k];
                        }
                        grid.push(Vtx { x, y, c });
                    }
                }
                for i in 0..n {
                    for j in 0..n {
                        let at = |a: usize, b: usize| grid[a * (n + 1) + b];
                        out.push([at(i, j), at(i, j + 1), at(i + 1, j)]);
                        out.push([at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
                    }
                }
                if out.len() > MAX_TRIANGLES {
                    break;
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// a mesh drawn into a layer over `area` (alpha 0 where no triangle is, or the background)
fn mesh_layer(run: &mut Run, sh: &Shading, m: &M, area: Rect, background: Option<[u8; 3]>) -> Option<Layer> {
    let mut layer = Layer::new(area)?;
    if let Some(b) = background {
        layer.fill([b[0], b[1], b[2], 255]);
    }
    let tris = triangles(sh, m);
    let mut tmp = Vec::new();
    for t in &tris {
        let v: Vec<(f64, f64)> = t.iter().map(|v| apply(m, v.x, v.y)).collect();
        let (minx, maxx) = (v[0].0.min(v[1].0).min(v[2].0), v[0].0.max(v[1].0).max(v[2].0));
        let (miny, maxy) = (v[0].1.min(v[1].1).min(v[2].1), v[0].1.max(v[1].1).max(v[2].1));
        let x0 = (floor(minx.max(area[0] as f64)) as i32).max(area[0]);
        let x1 = (ceil(maxx.min(area[2] as f64)) as i32).min(area[2]);
        let y0 = (floor(miny.max(area[1] as f64)) as i32).max(area[1]);
        let y1 = (ceil(maxy.min(area[3] as f64)) as i32).min(area[3]);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        if !run.spend(((x1 - x0) as u64) * ((y1 - y0) as u64)) {
            return None;
        }
        let det = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if det.abs() < 1e-12 {
            continue;
        }
        let n = sh.ncomp;
        for py in y0..y1 {
            let cy = py as f64 + 0.5;
            for px in x0..x1 {
                let cx = px as f64 + 0.5;
                let l1 = ((v[1].0 - cx) * (v[2].1 - cy) - (v[2].0 - cx) * (v[1].1 - cy)) / det;
                let l2 = ((v[2].0 - cx) * (v[0].1 - cy) - (v[0].0 - cx) * (v[2].1 - cy)) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 < -1e-9 || l2 < -1e-9 || l3 < -1e-9 {
                    continue;
                }
                let mut c = [0f64; 8];
                for k in 0..n {
                    c[k] = l1 * t[0].c[k] + l2 * t[1].c[k] + l3 * t[2].c[k];
                }
                let col = sh.color(&c[..n], &mut tmp);
                let o = layer.at(px, py);
                layer.px[o..o + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
            }
        }
    }
    Some(layer)
}

/// a shader for a shading drawn through matrix m (shading space -> device) over area
fn shading_shader(run: &mut Run, sh: &Rc<Shading>, m: &M, area: Rect, background: Option<[u8; 3]>) -> Option<Box<dyn Shader>> {
    match sh.kind {
        Kind::Mesh { .. } | Kind::Patches { .. } => {
            let layer = mesh_layer(run, sh, m, area, background)?;
            Some(Box::new(Prepared { layer }))
        }
        _ => Some(Box::new(Analytic { sh: sh.clone(), inv: inv(m)?, background })),
    }
}

/// sh: the shading over the clip (and its BBox)
pub(super) fn sh(run: &mut Run, o: &Object, res: &Rc<Res>) {
    let r = o.as_ref();
    let sh = match r.and_then(|r| run.shadings.get(&r).cloned()) {
        Some(s) => s,
        None => {
            let s = Shading::parse(run.doc, o, &res.all).map(Rc::new);
            if let Some(r) = r {
                run.shadings.insert(r, s.clone());
            }
            s
        }
    };
    let Some(sh) = sh else { return };
    let ctm = run.gs.ctm;
    let area = canvas::intersect(run.gs.clip.rect, run.layer_rect());
    let area = match sh.bbox {
        Some(b) => canvas::intersect(area, super::bbox_rect(&b, &ctm)),
        None => area,
    };
    if canvas::is_empty(area) {
        return;
    }
    let Some(shader) = shading_shader(run, &sh, &ctm, area, None) else { return };
    let mut p = Path::default();
    match sh.bbox {
        Some(b) => {
            for (i, (x, y)) in [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].into_iter().enumerate() {
                let (a, c) = apply(&ctm, x, y);
                if i == 0 {
                    p.move_f(a as f32, c as f32);
                } else {
                    p.line_f(a as f32, c as f32);
                }
            }
        }
        None => {
            p.move_f(area[0] as f32, area[1] as f32);
            p.line_f(area[2] as f32, area[1] as f32);
            p.line_f(area[2] as f32, area[3] as f32);
            p.line_f(area[0] as f32, area[3] as f32);
        }
    }
    p.close();
    p.fill_lines(&mut run.ras);
    let Run { layers, ras, gs, .. } = run;
    let Some(layer) = layers.last_mut() else { return };
    let put = Put { clip: &gs.clip, alpha: to8(gs.fill_alpha), blend: gs.blend, smask: gs.smask.as_deref() };
    let n = canvas::fill(layer, ras, RULE_NONZERO, &Src::Shade(&*shader), &put);
    run.spend(n);
}

// ---------------------------------------------------------------- patterns

pub(super) struct Tiling {
    data: Vec<u8>,
    res: Rc<Res>,
    bbox: [f64; 4],
    xstep: f64,
    ystep: f64,
    /// pattern space -> device
    matrix: M,
    coloured: bool,
    tiles: RefCell<Vec<([u8; 3], Rc<Layer>)>>,
}

pub(super) enum Pattern {
    Shading { sh: Rc<Shading>, matrix: M },
    Tiling(Tiling),
}

impl Pattern {
    pub fn parse(doc: &Document, o: &Object, base: &M, run: &mut Run) -> Option<Rc<Pattern>> {
        let ob = doc.resolve(o);
        let d = match &ob {
            Object::Dict(d) => d.clone(),
            Object::Stream(s) => s.dict.clone(),
            _ => return None,
        };
        let pm = super::m6(&super::color::nums(doc, &doc.key(&d, "Matrix"))).unwrap_or(super::ID);
        let matrix = mul(&pm, base);
        match doc.key(&d, "PatternType").as_int()? {
            2 => {
                let so = d.get("Shading")?.clone();
                let r = so.as_ref();
                let sh = match r.and_then(|r| run.shadings.get(&r).cloned()) {
                    Some(s) => s?,
                    None => {
                        let s = Shading::parse(doc, &so, &Dict::new()).map(Rc::new);
                        if let Some(r) = r {
                            run.shadings.insert(r, s.clone());
                        }
                        s?
                    }
                };
                Some(Rc::new(Pattern::Shading { sh, matrix }))
            }
            1 => {
                let Object::Stream(s) = ob else { return None };
                let data = doc.decode(&s).ok()?;
                let res = Rc::new(Res::of(doc, &d.get("Resources").map(|r| doc.dict(r)).unwrap_or_default()));
                let bbox = doc.key(&d, "BBox").as_rect()?;
                let xstep = doc.key(&d, "XStep").as_f64()?;
                let ystep = doc.key(&d, "YStep").as_f64()?;
                if !(xstep.abs() > 1e-9 && ystep.abs() > 1e-9 && xstep.is_finite() && ystep.is_finite()) {
                    return None;
                }
                let coloured = doc.key(&d, "PaintType").as_int() != Some(2);
                Some(Rc::new(Pattern::Tiling(Tiling { data, res, bbox, xstep, ystep, matrix, coloured, tiles: RefCell::new(Vec::new()) })))
            }
            _ => None,
        }
    }
}

/// the cell of a tiling pattern drawn into a tile at device resolution
fn tile(run: &mut Run, t: &Tiling, colour: [u8; 3]) -> Option<(Rc<Layer>, M)> {
    let m = &t.matrix;
    let len = |x: f64, y: f64| {
        let (a, b) = (m[0] * x + m[2] * y, m[1] * x + m[3] * y);
        sqrt(a * a + b * b)
    };
    let tw = ceil(len(t.xstep, 0.0)).clamp(1.0, MAX_TILE);
    let th = ceil(len(0.0, t.ystep)).clamp(1.0, MAX_TILE);
    // pattern space -> tile pixels: the cell [x0, x0 + xstep] x [y0, y0 + ystep]
    let (x0, y0) = (t.bbox[0], t.bbox[1]);
    let to_tile: M = [tw / t.xstep, 0.0, 0.0, th / t.ystep, -x0 * tw / t.xstep, -y0 * th / t.ystep];
    let key = if t.coloured { [0; 3] } else { colour };
    let cached = t.tiles.borrow().iter().find(|e| e.0 == key).map(|e| e.1.clone());
    let layer = match cached {
        Some(l) => l,
        None => {
            if run.depth >= MAX_DEPTH {
                return None;
            }
            let rect = [0, 0, tw as i32, th as i32];
            if !run.spend((tw * th) as u64) {
                return None;
            }
            let layer = Layer::new(rect)?;
            let saved = run.gs.clone();
            let saved_path = core::mem::take(&mut run.path);
            let saved_base = run.base;
            let saved_unc = run.uncolored;
            let saved_hidden = run.hidden;
            run.layers.push(layer);
            run.gs.ctm = to_tile;
            run.base = to_tile;
            run.gs.clip = Clip { rect, mask: None };
            run.gs.fill_alpha = 1.0;
            run.gs.stroke_alpha = 1.0;
            run.gs.blend = canvas::Blend::Normal;
            run.gs.smask = None;
            run.hidden = 0;
            if !t.coloured {
                let gray = Rc::new(Cs::Rgb);
                let mut p = Paint::new(gray);
                p.rgb = colour;
                run.gs.fill = p.clone();
                run.gs.stroke = p;
                run.uncolored = true;
            }
            run.clip_rect_user(&t.bbox);
            run.depth += 1;
            let res = t.res.clone();
            run.run(&t.data, &res);
            run.depth -= 1;
            let layer = Rc::new(run.layers.pop()?);
            run.gs = saved;
            run.path = saved_path;
            run.base = saved_base;
            run.uncolored = saved_unc;
            run.hidden = saved_hidden;
            let mut tiles = t.tiles.borrow_mut();
            if tiles.len() < 8 {
                tiles.push((key, layer.clone()));
            }
            layer
        }
    };
    // device -> pattern space -> tile pixels
    let dm = mul(&inv(m)?, &to_tile);
    Some((layer, dm))
}

/// a shader for a pattern paint over area
pub(super) fn shader(run: &mut Run, pat: &Rc<Pattern>, paint: &Paint, area: Rect) -> Option<Box<dyn Shader>> {
    match &**pat {
        Pattern::Shading { sh, matrix } => {
            let bg = sh.background;
            shading_shader(run, sh, matrix, area, bg)
        }
        Pattern::Tiling(t) => {
            let (tile, m) = tile(run, t, paint.rgb)?;
            Some(Box::new(Tiled { tile, m }))
        }
    }
}
