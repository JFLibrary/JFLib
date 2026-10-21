// color -- Farbe: Quantize                                                 [src/fx/fx_color.s]
//
//   FXTAB            the group table
//
// quantize: colors 2..256, algorithm octree | median_cut, dithering 0..8, alpha keep | binary.
// prepare (single thread, polls the cancel flag per row) does everything over the ROI and keeps
// the result image in aux; render copies rows (so tiles / threads cannot change the result).
//   considered pixels (palette statistics): inside the ROI, selection coverage > 0, alpha > 0
//     (keep) / alpha >= 128 (binary); row-major order
//   palette: <= K distinct colours -> exactly those (first appearance); else
//     octree  Gervautz-Purgathofer: 8 levels, child = bit (7-L) of R, G, B; after every pixel,
//             while leaves > K the most recently created internal node of the deepest level
//             with internal nodes absorbs its children; palette = leaves depth first
//     median_cut  Heckbert on a 32^3 histogram: split the box with the most pixels (ties:
//             first) along its longest cell extent after the first slice s with 2 * count(lo..s)
//             >= count, boxes shrunk to their occupied cells; colour = round(sum / count)
//   mapping: nearest palette colour (squared RGB distance, ties -> lowest index, through lazily
//     built candidate lists per 16^3 cell), Floyd-Steinberg (7 / 3 / 5 / 1 in 16ths) when
//     dithering L > 0: v = clamp(c + ((E L + 64) >> 7)); only "opaque" pixels take and pass
//     on error
//   alpha: keep -> unchanged; binary -> 255 when alpha >= 128, else 0

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::*;

const QMAX: usize = 256;
const QNODES: usize = 4096;
const QHASH: usize = 1024;
const NCELL: usize = 32768;

/// the quantised image of the ROI (render copies it)
struct QAux {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    px: Vec<u32>,
}

#[derive(Clone, Copy, Default)]
struct Node {
    ch: [u16; 8],
    next: u16,
    leaf: u8,
    level: u8,
    count: u32,
    sr: u64,
    sg: u64,
    sb: u64,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    count: u32,
    sr: u64,
    sg: u64,
    sb: u64,
}

#[derive(Clone, Copy, Default)]
struct QBox {
    lo: [i32; 3], // R, G, B cell
    hi: [i32; 3],
    count: u32,
}

struct Work<'a> {
    a: &'a FxArgs<'a>,
    pal: [u32; QMAX],
    npal: usize,
    k: usize,
    alpha: u32, // 0 keep, 1 binary
    level: i32,
    alg: u32,
    // exact palette
    hash: Vec<u32>,
    // octree
    nodes: Vec<Node>,
    red: [u32; 9],
    leaves: u32,
    free: u32,
    nnodes: u32,
    // median cut
    hist: Vec<Cell>,
    boxes: Vec<QBox>,
    nbox: usize,
    // mapping
    celloff: Vec<u32>,
    celln: Vec<u32>,
    cand: Vec<u8>,
    candn: u32,
}

/// q_mean: round(sum / count)
#[inline]
fn q_mean(sum: u64, count: u32) -> u32 {
    let c = count as u64;
    ((sum + (c >> 1)) / c) as u32
}

impl Work<'_> {
    /// fn(w, pixel) -> stop for every considered pixel of the ROI, row-major -> 0 | 1 (stopped)
    /// | ERR_CANCEL  [q_foreach]
    fn foreach(&mut self, which: u32) -> i32 {
        let a = self.a;
        let mut sbuf = [0u8; 256];
        for y in a.y0..a.y1 {
            if a.cancelled() {
                return ERR_CANCEL;
            }
            let row = a.src_row(y);
            let mut x = a.x0;
            while a.x1 - x > 0 {
                let n = (a.x1 - x).min(256);
                let sel = match a.sel {
                    Some(m) => Some(m.row(x, y, n, &mut sbuf).0),
                    None => None,
                };
                for i in 0..n as usize {
                    if let Some(s) = sel {
                        if s[i] == 0 {
                            continue;
                        }
                    }
                    let p = px(row, x as usize + i);
                    let al = p >> 24;
                    if al == 0 || (self.alpha != 0 && al < 128) {
                        continue;
                    }
                    let stop = match which {
                        0 => self.exact_fn(p),
                        1 => self.oct_fn(p),
                        _ => self.mc_fn(p),
                    };
                    if stop {
                        return 1;
                    }
                }
                x += n;
            }
        }
        0
    }

    /// add a colour to the exact palette -> true when there are more than K  [q_exact_fn]
    fn exact_fn(&mut self, p: u32) -> bool {
        let key = (p & 0x00FF_FFFF) + 1;
        let mut h = (key.wrapping_mul(0x9E3779B1) >> 22) as usize;
        loop {
            let e = self.hash[h];
            if e == key {
                return false;
            }
            if e == 0 {
                break;
            }
            h = (h + 1) & (QHASH - 1);
        }
        if self.npal >= self.k {
            return true;
        }
        self.hash[h] = key;
        self.pal[self.npal] = p | 0xFF00_0000;
        self.npal += 1;
        false
    }

    // ---- octree

    /// a zeroed node from the free list or the pool -> index | 0  [q_node_new]
    fn node_new(&mut self) -> u32 {
        let i = if self.free != 0 {
            let i = self.free;
            self.free = self.nodes[i as usize].next as u32;
            i
        } else {
            if self.nnodes as usize >= QNODES {
                return 0;
            }
            self.nnodes += 1;
            self.nnodes - 1
        };
        self.nodes[i as usize] = Node::default();
        i
    }

    /// insert one colour, reduce while leaves > K  [q_oct_fn]
    fn oct_fn(&mut self, p: u32) -> bool {
        let mut node = 1usize;
        let mut lv = 0u32;
        while self.nodes[node].leaf == 0 {
            let bit = 7 - lv;
            let slot = ((p >> 16 >> bit & 1) << 2 | (p >> 8 >> bit & 1) << 1 | (p >> bit & 1)) as usize;
            let mut c = self.nodes[node].ch[slot] as u32;
            if c == 0 {
                c = self.node_new();
                if c == 0 {
                    break; // pool exhausted: accumulate here
                }
                let l = lv + 1;
                self.nodes[c as usize].level = l as u8;
                if l >= 8 {
                    self.nodes[c as usize].leaf = 1;
                    self.leaves += 1;
                } else {
                    self.nodes[c as usize].next = self.red[l as usize] as u16;
                    self.red[l as usize] = c;
                }
                self.nodes[node].ch[slot] = c as u16;
            }
            node = c as usize;
            lv += 1;
        }
        let n = &mut self.nodes[node];
        n.count += 1;
        n.sr += (p >> 16 & 255) as u64;
        n.sg += (p >> 8 & 255) as u64;
        n.sb += (p & 255) as u64;
        while self.leaves as usize > self.k {
            self.reduce();
        }
        false
    }

    /// merge the children of the most recent internal node of the deepest level  [q_reduce]
    fn reduce(&mut self) {
        let mut l = 7usize;
        while l != 0 && self.red[l] == 0 {
            l -= 1;
        }
        let ni = self.red[l] as usize;
        if ni == 0 {
            return;
        }
        self.red[l] = self.nodes[ni].next as u32;
        let mut merged = 0u32;
        for s in 0..8 {
            let c = self.nodes[ni].ch[s] as usize;
            if c == 0 {
                continue;
            }
            let ch = self.nodes[c];
            let n = &mut self.nodes[ni];
            n.count = n.count.wrapping_add(ch.count);
            n.sr = n.sr.wrapping_add(ch.sr);
            n.sg = n.sg.wrapping_add(ch.sg);
            n.sb = n.sb.wrapping_add(ch.sb);
            n.ch[s] = 0;
            self.nodes[c].next = self.free as u16;
            self.free = c as u32;
            merged += 1;
        }
        self.nodes[ni].leaf = 1;
        self.leaves = self.leaves.wrapping_sub(merged.wrapping_sub(1));
    }

    /// append the leaves below a node (depth first, child order)  [q_oct_pal]
    fn oct_pal(&mut self, node: usize) {
        let n = self.nodes[node];
        if n.leaf != 0 {
            if n.count == 0 || self.npal >= QMAX {
                return;
            }
            self.pal[self.npal] = 0xFF00_0000 | q_mean(n.sr, n.count) << 16 | q_mean(n.sg, n.count) << 8 | q_mean(n.sb, n.count);
            self.npal += 1;
            return;
        }
        for s in 0..8 {
            let c = self.nodes[node].ch[s];
            if c != 0 {
                self.oct_pal(c as usize);
            }
        }
    }

    fn octree(&mut self) -> i32 {
        self.nodes = vec![Node::default(); QNODES];
        self.red = [0; 9];
        self.red[0] = 1; // root in the level-0 list
        self.nnodes = 2;
        self.free = 0;
        self.leaves = 0;
        let r = self.foreach(1);
        if r < 0 {
            return r;
        }
        self.npal = 0;
        self.oct_pal(1);
        0
    }

    // ---- median cut

    fn mc_fn(&mut self, p: u32) -> bool {
        let i = ((p >> 19 & 31) << 10 | (p >> 11 & 31) << 5 | (p >> 3 & 31)) as usize;
        let c = &mut self.hist[i];
        c.count += 1;
        c.sr += (p >> 16 & 255) as u64;
        c.sg += (p >> 8 & 255) as u64;
        c.sb += (p & 255) as u64;
        false
    }

    #[inline]
    fn cell(&self, r: i32, g: i32, b: i32) -> &Cell {
        &self.hist[((r << 10) | (g << 5) | b) as usize]
    }

    /// bounds := occupied cells of the box, count := their pixels  [q_shrink]
    fn shrink(&mut self, bi: usize) {
        let bx = self.boxes[bi];
        let mut lo = [32i32; 3];
        let mut hi = [-1i32; 3];
        let mut count = 0u32;
        for r in bx.lo[0]..=bx.hi[0] {
            for g in bx.lo[1]..=bx.hi[1] {
                for b in bx.lo[2]..=bx.hi[2] {
                    let c = self.cell(r, g, b).count;
                    if c == 0 {
                        continue;
                    }
                    count = count.wrapping_add(c);
                    let v = [r, g, b];
                    for k in 0..3 {
                        if v[k] < lo[k] {
                            lo[k] = v[k];
                        }
                        if v[k] > hi[k] {
                            hi[k] = v[k];
                        }
                    }
                }
            }
        }
        let bx = &mut self.boxes[bi];
        bx.count = count;
        if count != 0 {
            bx.lo = lo;
            bx.hi = hi;
        }
    }

    /// split box i into box i and box nbox  [q_split]
    fn split(&mut self, i: usize) {
        let bx = self.boxes[i];
        let mut ext = bx.hi[0] - bx.lo[0];
        let mut axis = 0usize;
        let e = bx.hi[1] - bx.lo[1];
        if e > ext {
            ext = e;
            axis = 1;
        }
        let e = bx.hi[2] - bx.lo[2];
        if e > ext {
            axis = 2;
        }
        let mut slice = [0u32; 32];
        for r in bx.lo[0]..=bx.hi[0] {
            for g in bx.lo[1]..=bx.hi[1] {
                for b in bx.lo[2]..=bx.hi[2] {
                    let v = [r, g, b][axis];
                    slice[v as usize] = slice[v as usize].wrapping_add(self.cell(r, g, b).count);
                }
            }
        }
        let (lo, hi) = (bx.lo[axis], bx.hi[axis]);
        let mut split = hi - 1;
        let mut cum = 0u32;
        let mut s = lo;
        while s < hi {
            cum = cum.wrapping_add(slice[s as usize]);
            if cum.wrapping_add(cum) >= bx.count {
                split = s;
                break;
            }
            s += 1;
        }
        let ni = self.nbox;
        let mut nb = bx;
        nb.lo[axis] = split + 1;
        self.boxes[ni] = nb;
        self.boxes[i].hi[axis] = split;
        self.shrink(i);
        self.shrink(ni);
    }

    /// rounded mean of the box's pixels  [q_box_color]
    fn box_color(&self, bi: usize) -> u32 {
        let bx = self.boxes[bi];
        let (mut n, mut sr, mut sg, mut sb) = (0u32, 0u64, 0u64, 0u64);
        for r in bx.lo[0]..=bx.hi[0] {
            for g in bx.lo[1]..=bx.hi[1] {
                for b in bx.lo[2]..=bx.hi[2] {
                    let c = self.cell(r, g, b);
                    n = n.wrapping_add(c.count);
                    sr = sr.wrapping_add(c.sr);
                    sg = sg.wrapping_add(c.sg);
                    sb = sb.wrapping_add(c.sb);
                }
            }
        }
        if n == 0 {
            return 0xFF00_0000;
        }
        0xFF00_0000 | q_mean(sr, n) << 16 | q_mean(sg, n) << 8 | q_mean(sb, n)
    }

    fn mcut(&mut self) -> i32 {
        self.hist = vec![Cell::default(); NCELL];
        let r = self.foreach(2);
        if r < 0 {
            return r;
        }
        self.boxes = vec![QBox::default(); QMAX + 1];
        self.boxes[0] = QBox { lo: [0; 3], hi: [31; 3], count: 0 };
        self.shrink(0);
        self.nbox = if self.boxes[0].count != 0 { 1 } else { 0 };
        while self.nbox != 0 && self.nbox < self.k {
            let mut best: i32 = -1;
            let mut bc = 0u32;
            for i in 0..self.nbox {
                let b = &self.boxes[i];
                if (b.hi[0] - b.lo[0]) | (b.hi[1] - b.lo[1]) | (b.hi[2] - b.lo[2]) == 0 {
                    continue; // a single cell
                }
                if b.count > bc {
                    bc = b.count;
                    best = i as i32;
                }
            }
            if best < 0 {
                break;
            }
            self.split(best as usize);
            self.nbox += 1;
        }
        self.npal = 0;
        for i in 0..self.nbox {
            self.pal[self.npal] = self.box_color(i);
            self.npal += 1;
        }
        0
    }

    // ---- mapping

    /// (min, max) squared distance of a palette colour to cell `cell`  [q_cell_dist]
    fn cell_dist(p: u32, cell: u32) -> (u32, u32) {
        let (mut mn, mut mx) = (0i32, 0i32);
        for c in 0..3 {
            let v = (p >> (8 * c) & 255) as i32;
            let lo = ((cell >> (4 * c) & 15) << 4) as i32;
            let hi = lo + 15;
            let mut d = 0;
            if v < lo {
                d = lo - v;
            }
            if v > hi {
                d = v - hi;
            }
            mn = mn.wrapping_add(d * d);
            let a = (v - lo) * (v - lo);
            let b = (hi - v) * (hi - v);
            mx = mx.wrapping_add(if b > a { b } else { a });
        }
        (mn as u32, mx as u32)
    }

    /// candidate list of a 16^3 cell  [q_cell_build]
    fn cell_build(&mut self, cell: u32) {
        let mut t = u32::MAX;
        for i in 0..self.npal {
            let (_, mx) = Self::cell_dist(self.pal[i], cell);
            if mx < t {
                t = mx;
            }
        }
        let off = self.candn;
        self.celloff[cell as usize] = off;
        let mut n = 0u32;
        for i in 0..self.npal {
            let (mn, _) = Self::cell_dist(self.pal[i], cell);
            if mn <= t {
                self.cand[(off + n) as usize] = i as u8;
                n += 1;
            }
        }
        self.celln[cell as usize] = n;
        self.candn += n;
    }

    /// the nearest palette entry (lowest index on ties)  [q_nearest]
    fn nearest(&mut self, rgb: u32) -> usize {
        let cell = (rgb >> 12 & 0xF00) | (rgb >> 8 & 0xF0) | (rgb >> 4 & 0xF);
        if self.celloff[cell as usize] == u32::MAX {
            self.cell_build(cell);
        }
        let off = self.celloff[cell as usize] as usize;
        let n = self.celln[cell as usize] as usize;
        let (r, g, b) = ((rgb >> 16 & 255) as i32, (rgb >> 8 & 255) as i32, (rgb & 255) as i32);
        let mut best = u32::MAX;
        let mut bi = 0usize;
        for &ci in &self.cand[off..off + n] {
            let p = self.pal[ci as usize];
            let dr = (p >> 16 & 255) as i32 - r;
            let dg = (p >> 8 & 255) as i32 - g;
            let db = (p & 255) as i32 - b;
            let d = (dr * dr + dg * dg + db * db) as u32;
            if d < best {
                best = d;
                bi = ci as usize;
            }
        }
        bi
    }

    /// nearest colour + Floyd-Steinberg into the aux image  [q_map]
    fn map(&mut self, aux: &mut QAux) -> i32 {
        let a = self.a;
        let (w, h) = (aux.w as usize, aux.h as usize);
        self.celloff = vec![u32::MAX; 4096];
        self.celln = vec![0; 4096];
        self.cand = vec![0u8; 4096 * 256];
        self.candn = 0;
        let ew = (w + 2) * 3;
        let mut cur = vec![0i32; ew];
        let mut next = vec![0i32; ew];
        let level = self.level;
        for y in 0..h {
            if a.cancelled() {
                return ERR_CANCEL;
            }
            let srow = &a.src_row(aux.y0 + y as i32)[aux.x0 as usize * 4..];
            for x in 0..w {
                let p = px(srow, x);
                let out = if self.npal == 0 {
                    p
                } else {
                    let al = p >> 24;
                    let part = if self.alpha != 0 { al >= 128 } else { al != 0 };
                    let mut v = p & 0x00FF_FFFF;
                    if part && level != 0 {
                        v = 0;
                        for c in 0..3 {
                            let e = cur[(x + 1) * 3 + c];
                            let d = (e.wrapping_mul(level).wrapping_add(64) >> 7) + (p >> (8 * c) & 255) as i32;
                            v |= (d.clamp(0, 255) as u32) << (8 * c);
                        }
                    }
                    let q = self.pal[self.nearest(v)];
                    if part && level != 0 {
                        for c in 0..3 {
                            let d = (v >> (8 * c) & 255) as i32 - (q >> (8 * c) & 255) as i32;
                            cur[(x + 2) * 3 + c] += d * 7;
                            next[x * 3 + c] += d * 3;
                            next[(x + 1) * 3 + c] += d * 5;
                            next[(x + 2) * 3 + c] += d;
                        }
                    }
                    let al = if self.alpha != 0 {
                        if al >= 128 { 255 } else { 0 }
                    } else {
                        al
                    };
                    q & 0x00FF_FFFF | al << 24
                };
                aux.px[y * w + x] = out;
            }
            core::mem::swap(&mut cur, &mut next);
            if level != 0 {
                next.iter_mut().for_each(|e| *e = 0);
            }
        }
        0
    }
}

fn quant_prepare(a: &FxArgs) -> Result<Option<Aux>, i32> {
    let w = (a.x1 - a.x0).max(0);
    let h = (a.y1 - a.y0).max(0);
    let mut aux = QAux { x0: a.x0, y0: a.y0, w, h, px: vec![0u32; w as usize * h as usize] };
    let p = a.params;
    let mut wk = Work {
        a,
        pal: [0; QMAX],
        npal: 0,
        k: p.i(0).clamp(2, QMAX as i64) as usize,
        alg: (p.i(1) != 0) as u32,
        level: p.i(2).clamp(0, 8) as i32,
        alpha: (p.i(3) != 0) as u32,
        hash: vec![0; QHASH],
        nodes: Vec::new(),
        red: [0; 9],
        leaves: 0,
        free: 0,
        nnodes: 0,
        hist: Vec::new(),
        boxes: Vec::new(),
        nbox: 0,
        celloff: Vec::new(),
        celln: Vec::new(),
        cand: Vec::new(),
        candn: 0,
    };
    // 1. exact palette?
    let r = wk.foreach(0);
    if r < 0 {
        return Err(r);
    }
    if r != 0 {
        wk.npal = 0;
        // 2. octree or median cut
        let r = if wk.alg == 0 { wk.octree() } else { wk.mcut() };
        if r < 0 {
            return Err(r);
        }
    }
    let r = wk.map(&mut aux);
    if r < 0 {
        return Err(r);
    }
    Ok(Some(Box::new(aux)))
}

/// copy the prepared rows (the source outside the prepared area)  [fx_quant_render]
fn quant_render(a: &FxArgs, dst: &mut Dst) -> i32 {
    let q = a.aux::<QAux>();
    rows(a, dst, |d, s, x0, y| {
        for i in 0..s.len() / 4 {
            let mut v = px(s, i);
            if let Some(q) = q {
                let cx = (x0 as i64 + i as i64) - q.x0 as i64;
                let cy = y as i64 - q.y0 as i64;
                if cx >= 0 && cx < q.w as i64 && cy >= 0 && cy < q.h as i64 {
                    v = q.px[(cy * q.w as i64 + cx) as usize];
                }
            }
            put(d, i, v);
        }
    })
}

static PROPS_QUANTIZE: [FxProp; 4] = [
    FxProp::new(FP_INT, 0, 0, 0, "colors", 2.0, 256.0, 16.0),
    FxProp::choice(1, "algorithm", 1.0, 0.0, &["octree", "median_cut"]),
    FxProp::new(FP_INT, 0, 2, 0, "dithering", 0.0, 8.0, 7.0),
    FxProp::choice(3, "alpha", 1.0, 0.0, &["keep", "binary"]),
];

/// the group table  [fxtab_color]
pub static FXTAB: [FxDesc; 1] = [FxDesc {
    id: FX_QUANTIZE,
    flags: FXF_PREPARE | FXF_SLOW,
    name: "quantize",
    prepare: Some(quant_prepare),
    render: quant_render,
    props: &PROPS_QUANTIZE,
    category: FXC_COLOR,
    user: 0,
}];
