// emod -- modular encoding of the new encoder: predictors and properties, tree data gathering,
// channel tokens, forward transforms (RCT, palette, squeeze), cost estimates, group headers,
// and the frame-level stream set with one global MA tree (libjxl 0.11.1 enc_modular.cc,
// modular/encoding/enc_encoding.cc, context_predict.h, transform/enc_rct.cc, enc_palette.cc,
// enc_squeeze.cc)                                                          [jxl-plus: no asm]
//
//   MOpts                      ModularOptions (+ the effort dependent choices)
//   Streams                    the modular streams of a frame (stream id = ModularStreamId):
//                              learn_tree / tokens / write_global / write_stream
//   fwd_rct / fwd_palette / fwd_squeeze, try_palettes, est_cost, est_wp_cost
//   write_group_header(jw, global, wp, transforms)
//   par_map(n, threads, f)     f(0..n) on the worker pool (results in index order); jxl-speed:
//                              par_map_with (a scratch value per worker), par_rows (bands of
//                              rows of planes), join(threads, a, b) (two parts at the same time)
//
// Channel samples are i32 rows (modular::Mc); arithmetic as in the decoder (modular.rs), so the
// residuals the decoder adds back are exactly the ones coded here.

use alloc::vec;
use alloc::vec::Vec;

pub use crate::par::{join, par_map, par_map_with, par_rows}; // jxl-module: in img_jxl (the decoder uses them)

use crate::eans::{self, pack_signed, Code, HParams, Histo, Huc, Token};
use crate::ema::{self, Kind, MulInfo, Node, Rng, Samples, Tree, NUM_NONREF, NUM_PREDICTORS, PRED_GRADIENT, PRED_WEIGHTED};
use crate::ewr::Jw;
use crate::modular::{jm_predict, Mc, Mi, Mt, Wp, MTR_PALETTE, MTR_RCT, MTR_SQUEEZE, WH_DEF};

/// WP header presets (PredictorMode): p1C, p2C, p3Ca..e, w0..w3
pub static WP_MODES: [[u32; 11]; 5] = [
    [16, 10, 7, 7, 7, 0, 0, 13, 12, 12, 12],
    [8, 8, 4, 0, 3, 23, 2, 13, 12, 12, 11],
    [10, 9, 7, 0, 0, 16, 9, 13, 12, 13, 12],
    [16, 8, 0, 16, 0, 23, 0, 13, 13, 12, 12],
    [10, 10, 5, 5, 5, 12, 4, 13, 12, 12, 12],
];

/// ModularOptions
#[derive(Clone)]
pub struct MOpts {
    pub max_chan: u32,
    pub nb_repeats: f32,
    pub props: Vec<u32>,
    pub threshold: f32,
    pub max_vals: usize,
    pub predictor: u32, // 0..13, 14 Best, 15 Variable
    pub wp_mode: usize,
    pub fast_mult: f32,
    pub wp_tree: u32, // 0 default, 1 WP only, 2 no WP, 3 gradient only
    pub kind: Kind,
    pub hp: HParams,
    pub max_props: i32,
}

impl Default for MOpts {
    fn default() -> Self {
        MOpts {
            max_chan: 0xFF_FFFF,
            nb_repeats: 0.5,
            props: vec![0, 1, 15, 9, 10, 11, 12, 13],
            threshold: 96.0,
            max_vals: 32,
            predictor: PRED_GRADIENT,
            wp_mode: 0,
            fast_mult: 1.01,
            wp_tree: 0,
            kind: Kind::Learn,
            hp: HParams::default(),
            max_props: 0,
        }
    }
}

// ------------------------------------------------------------------------- pixels

/// neighbours of (x, y): left, top, topleft, topright, leftleft, toptop, toprightright
#[inline(always)]
fn nbrs(d: &[i32], w: usize, x: usize, y: usize) -> [i32; 7] {
    let i = y * w + x;
    let left = if x > 0 {
        d[i - 1]
    } else if y > 0 {
        d[i - w]
    } else {
        0
    };
    let top = if y > 0 { d[i - w] } else { left };
    let tl = if x > 0 && y > 0 { d[i - w - 1] } else { left };
    let tr = if x + 1 < w && y > 0 { d[i - w + 1] } else { top };
    let ll = if x > 1 { d[i - 2] } else { left };
    let tt = if y > 1 { d[i - 2 * w] } else { top };
    let trr = if x + 2 < w && y > 0 { d[i - w + 2] } else { tr };
    [left, top, tl, tr, ll, tt, trr]
}

/// clamped gradient
#[inline(always)]
pub fn cgrad(n: i32, w: i32, l: i32) -> i32 {
    crate::modular::cgrad(n, w, l)
}

/// the reference channels of channel c (earlier channels of the same size and shifts),
/// at most n
fn refs_of(img: &Mi, c: usize, n: usize) -> Vec<usize> {
    let me = &img.ch[c];
    let mut v = Vec::new();
    let mut k = c;
    while k != 0 && v.len() < n {
        k -= 1;
        if img.ch[k].same(me) {
            v.push(k);
        }
    }
    v
}

/// properties 3..14 (+ references) of (x, y); props[2] = y is set by the caller, p9 is the
/// running "previous local gradient" of the row
#[inline(always)]
fn props_at(props: &mut [i32], nb: &[i32; 7], x: usize, p9: &mut i32) {
    let [left, top, tl, tr, ll, tt, _] = *nb;
    props[3] = x as i32;
    props[4] = top.wrapping_abs();
    props[5] = left.wrapping_abs();
    props[6] = top;
    props[7] = left;
    props[8] = left.wrapping_sub(*p9);
    *p9 = left.wrapping_add(top).wrapping_sub(tl);
    props[9] = *p9;
    props[10] = left.wrapping_sub(tl);
    props[11] = tl.wrapping_sub(top);
    props[12] = top.wrapping_sub(tr);
    props[13] = top.wrapping_sub(tt);
    props[14] = left.wrapping_sub(ll);
}

#[inline(always)]
fn ref_props(props: &mut [i32], img: &Mi, refs: &[usize], w: usize, x: usize, y: usize) {
    for (k, &rc) in refs.iter().enumerate() {
        let rd = &img.ch[rc].d;
        let i = y * w + x;
        let v = rd[i];
        let vl = if x > 0 { rd[i - 1] } else { 0 };
        let (mut vt, mut vtl) = (vl, vl);
        if y > 0 {
            vt = rd[i - w];
            if x > 0 {
                vtl = rd[i - w - 1];
            }
        }
        let pv = cgrad(vl, vt, vtl);
        let o = NUM_NONREF + 4 * k;
        props[o] = v.wrapping_abs();
        props[o + 1] = v;
        let dv = v.wrapping_sub(pv);
        props[o + 2] = dv.wrapping_abs();
        props[o + 3] = dv;
    }
}

/// the WP neighbour order of Wp::predict: N, W, NE, NW, NN
#[inline(always)]
fn wpnb(nb: &[i32; 7]) -> [i32; 5] {
    [nb[1], nb[0], nb[3], nb[2], nb[5]]
}

/// GatherTreeData of channel c of img (stream / group id gid)
pub fn gather(img: &Mi, c: usize, gid: u32, wp: &[u32; 11], o: &MOpts, s: &mut Samples, total: &mut usize) {
    let ch = &img.ch[c];
    let (w, h) = (ch.w as usize, ch.h as usize);
    if w == 0 || h == 0 {
        return;
    }
    let nrefs = o.max_props.max(0) as usize;
    let refs = refs_of(img, c, nrefs);
    let mut props = vec![0i32; NUM_NONREF + 4 * nrefs.max(1)];
    props[0] = c as i32;
    props[1] = gid as i32;
    let mut frac = o.nb_repeats.min(1.0) as f64;
    if frac > 0.0 {
        frac = frac.max((1024.0 / (w * h) as f64).min(1.0));
    }
    let threshold = ((u64::MAX >> 32) as f64 * frac) as u64;
    let mut st: [u64; 2] = [0x94D0_49BB_1331_11EB, 0xBF58_476D_1CE4_E5B9];
    let mut use_sample = || {
        let mut s1 = st[0];
        let s0 = st[1];
        let bits = s1.wrapping_add(s0);
        st[0] = s0;
        s1 ^= s1 << 23;
        s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5);
        st[1] = s1;
        (bits >> 32) <= threshold
    };
    let mut wps = Wp::new(wp, ch.w).ok();
    s.prepare((frac * (h * w) as f64) as usize + 64);
    let multi = s.preds.len() != 1;
    let single = s.preds[0];
    let d = &ch.d;
    let mut preds = [0i64; NUM_PREDICTORS];
    for y in 0..h {
        props[2] = y as i32;
        let mut p9 = 0i32;
        for x in 0..w {
            let nb = nbrs(d, w, x, y);
            props_at(&mut props, &nb, x, &mut p9);
            let mut wpp = 0i64;
            if let Some(wst) = wps.as_mut() {
                wpp = wst.predict(x as u32, y as u32, &wpnb(&nb), Some(&mut props[..]));
            }
            ref_props(&mut props, img, &refs, w, x, y);
            if multi {
                for (k, p) in preds.iter_mut().enumerate() {
                    *p = jm_predict(k as u32, &nb, wpp);
                }
            } else {
                preds[single as usize] = jm_predict(single, &nb, wpp);
            }
            *total += 1;
            let v = d[y * w + x];
            if use_sample() {
                s.add(v as i64, &props, &preds);
            }
            if let Some(wst) = wps.as_mut() {
                wst.update(v, x as u32, y as u32);
            }
        }
    }
}

/// a tree in decoder order: does it use the WP (property 15 or predictor 6), the largest
/// property
fn tree_info(t: &Tree) -> (bool, i32) {
    let mut wp = false;
    let mut mp = -1;
    for n in t {
        if n.prop > mp {
            mp = n.prop;
        }
        if n.prop == 15 || (n.prop < 0 && n.pred == PRED_WEIGHTED) {
            wp = true;
        }
    }
    (wp, mp)
}

/// EncodeModularChannelMAANS: the tokens of channel c with the decoder-order tree
pub fn chan_tokens(img: &Mi, c: usize, gid: u32, wp: &[u32; 11], tree: &Tree, out: &mut Vec<Token>) {
    let ch = &img.ch[c];
    let (w, h) = (ch.w as usize, ch.h as usize);
    if w == 0 || h == 0 {
        return;
    }
    let (use_wp, maxprop) = tree_info(tree);
    let mp1 = maxprop + 1;
    let np = if mp1 > 16 { (((mp1 - 13) as u32) & !3) + 16 } else { 16 };
    let nrefs = ((np - 16) / 4) as usize;
    let refs = refs_of(img, c, nrefs);
    let mut props = vec![0i32; np as usize + 4];
    props[0] = c as i32;
    props[1] = gid as i32;
    let mut wps = if use_wp { Wp::new(wp, ch.w).ok() } else { None };
    let d = &ch.d;
    // single leaf without WP: no properties needed
    let single = tree.len() == 1 && !use_wp;
    out.reserve(w * h);
    for y in 0..h {
        props[2] = y as i32;
        let mut p9 = 0i32;
        for x in 0..w {
            let nb = nbrs(d, w, x, y);
            let v = d[y * w + x];
            let nd: &Node = if single {
                &tree[0]
            } else {
                props_at(&mut props, &nb, x, &mut p9);
                ref_props(&mut props, img, &refs, w, x, y);
                &tree[0]
            };
            let mut wpp = 0i64;
            if let Some(wst) = wps.as_mut() {
                wpp = wst.predict(x as u32, y as u32, &wpnb(&nb), Some(&mut props[..]));
            }
            let mut nd = nd;
            while nd.prop >= 0 {
                let pv = props[nd.prop as usize];
                nd = &tree[if pv > nd.split { nd.l } else { nd.r } as usize];
            }
            let guess = jm_predict(nd.pred, &nb, wpp).wrapping_add(nd.off);
            let r = (v as i64).wrapping_sub(guess);
            let r = if nd.mul > 1 { r / nd.mul as i64 } else { r };
            out.push(Token::new(nd.l, pack_signed(r as i32)));
            if let Some(wst) = wps.as_mut() {
                wst.update(v, x as u32, y as u32);
            }
        }
    }
}

// ------------------------------------------------------------------------- cost estimates

/// EstimateCost: clamped gradient residuals in 18 contexts of the local spread
pub fn est_cost(img: &Mi) -> f32 {
    static CUT: [u32; 17] = [0, 1, 3, 5, 7, 11, 15, 23, 31, 47, 63, 95, 127, 191, 255, 392, 500];
    let cfg = Huc::default();
    let mut extra = 0usize;
    let mut cost = 0.0f32;
    let mut hs: Vec<Histo> = vec![Histo::default(); 18];
    for ch in &img.ch {
        let (w, h) = (ch.w as usize, ch.h as usize);
        if w == 0 || h == 0 {
            continue;
        }
        let d = &ch.d;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let left = if x > 0 {
                    d[i - 1]
                } else if y > 0 {
                    d[i - w]
                } else {
                    0
                };
                let top = if y > 0 { d[i - w] } else { left };
                let tl = if x > 0 && y > 0 { d[i - w - 1] } else { left };
                let mx = left.max(top).max(tl) as i64;
                let mn = left.min(top).min(tl) as i64;
                let maxdiff = (mx - mn) as u64;
                let mut ctx = 0usize;
                for &c in &CUT {
                    if c as u64 > maxdiff {
                        ctx += 1;
                    }
                }
                let r = d[i].wrapping_sub(cgrad(top, left, tl));
                let (tok, nb, _) = cfg.encode(pack_signed(r));
                hs[ctx].add(tok as usize);
                extra += nb as usize;
            }
        }
        for h in hs.iter_mut() {
            cost += h.shannon();
            h.clear();
        }
    }
    cost + extra as f32
}

/// EstimateWPCost: WP residuals in contexts of the WP property, WP preset `mode`
pub fn est_wp_cost(img: &Mi, mode: usize) -> f32 {
    static CUT: [i32; 33] = [
        -500, -392, -255, -191, -127, -95, -63, -47, -31, -23, -15, -11, -7, -4, -3, -1, 0, 1, 3, 5, 7, 11, 15, 23,
        31, 47, 63, 95, 127, 191, 255, 392, 500,
    ];
    let cfg = Huc::default();
    let mut extra = 0usize;
    let mut cost = 0.0f32;
    let mut hs: Vec<Histo> = vec![Histo::default(); 34];
    let wh = &WP_MODES[mode.min(4)];
    let mut props = [0i32; 16];
    for ch in &img.ch {
        let (w, h) = (ch.w as usize, ch.h as usize);
        if w == 0 || h == 0 {
            continue;
        }
        let Ok(mut wst) = Wp::new(wh, ch.w) else { continue };
        let d = &ch.d;
        for y in 0..h {
            for x in 0..w {
                let nb = nbrs(d, w, x, y);
                let g = wst.predict(x as u32, y as u32, &wpnb(&nb), Some(&mut props[..]));
                let mut ctx = 0usize;
                for &c in &CUT {
                    if c >= props[15] {
                        ctx += 1;
                    }
                }
                let v = d[y * w + x];
                let r = (v as i64).wrapping_sub(g) as i32;
                let (tok, nb2, _) = cfg.encode(pack_signed(r));
                hs[ctx].add(tok as usize);
                extra += nb2 as usize;
                wst.update(v, x as u32, y as u32);
            }
        }
        for h in hs.iter_mut() {
            cost += h.shannon();
            h.clear();
        }
    }
    cost + extra as f32
}

// ------------------------------------------------------------------------- transforms

fn equal_channels(img: &Mi, a: usize, b: usize) -> bool {
    if b >= img.ch.len() || a > b {
        return false;
    }
    if a < img.nbmeta as usize && img.nbmeta as usize <= b {
        return false;
    }
    let f = &img.ch[a];
    img.ch[a..=b].iter().all(|c| c.same(f))
}

/// FwdRCT: false for type 0 (nothing done)
pub fn fwd_rct(img: &mut Mi, begin: usize, typ: u32) -> bool {
    if !equal_channels(img, begin, begin + 2) || typ == 0 {
        return false;
    }
    let perm = (typ / 7) as usize;
    let custom = typ % 7;
    let second = custom >> 1;
    let third = custom & 1;
    let n = img.ch[begin].d.len();
    let i0 = begin + perm % 3;
    let i1 = begin + (perm + 1 + perm / 3) % 3;
    let i2 = begin + (perm + 2 - perm / 3) % 3;
    let a = img.ch[i0].d.clone();
    let b = img.ch[i1].d.clone();
    let c = img.ch[i2].d.clone();
    let mut o0 = vec![0i32; n];
    let mut o1 = vec![0i32; n];
    let mut o2 = vec![0i32; n];
    for i in 0..n {
        if custom == 6 {
            let (r, g, bb) = (a[i], b[i], c[i]);
            let co = r.wrapping_sub(bb);
            let tmp = bb.wrapping_add(co >> 1);
            let cg = g.wrapping_sub(tmp);
            o1[i] = co;
            o2[i] = cg;
            o0[i] = tmp.wrapping_add(cg >> 1);
        } else {
            let f = a[i];
            let mut s = b[i];
            let mut t = c[i];
            if second == 1 {
                s = s.wrapping_sub(f);
            } else if second == 2 {
                s = s.wrapping_sub(f.wrapping_add(t) >> 1);
            }
            if third != 0 {
                t = t.wrapping_sub(f);
            }
            o0[i] = f;
            o1[i] = s;
            o2[i] = t;
        }
    }
    img.ch[begin].d = o0;
    img.ch[begin + 1].d = o1;
    img.ch[begin + 2].d = o2;
    img.tr.push(Mt { id: MTR_RCT, begin: begin as u32, rct: typ, ..Default::default() });
    true
}

fn minmax(ch: &Mc) -> (i32, i32) {
    let mut mn = i32::MAX;
    let mut mx = i32::MIN;
    for &v in &ch.d {
        mn = mn.min(v);
        mx = mx.max(v);
    }
    (mn, mx)
}

/// implicit palette colour k (palette size 0), channel c
fn implicit_value(k: i32, c: usize, bd: u32) -> i32 {
    if c >= 3 {
        return 0;
    }
    let scale = |v: i64| -> i32 { ((v * ((1i64 << bd) - 1)) >> 2) as i32 };
    if k < 64 {
        let idx = (k >> (c * 2)) % 4;
        scale(idx as i64) + (1i32 << (bd as i32 - 3).max(0))
    } else {
        let mut idx = k - 64;
        if c == 1 {
            idx /= 5;
        } else if c == 2 {
            idx /= 25;
        }
        scale((idx % 5) as i64)
    }
}

/// a deterministic open addressing map: colour (up to 4 channels) -> u32
struct ColorMap {
    keys: Vec<u128>,
    vals: Vec<u32>,
    used: Vec<bool>,
    n: usize,
}

impl ColorMap {
    fn new() -> ColorMap {
        ColorMap { keys: vec![0; 1024], vals: vec![0; 1024], used: vec![false; 1024], n: 0 }
    }
    #[inline]
    fn slot(&self, k: u128) -> usize {
        let h = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((k >> 64) as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        (h >> 20) as usize & (self.keys.len() - 1)
    }
    fn grow(&mut self) {
        let old_k = core::mem::take(&mut self.keys);
        let old_v = core::mem::take(&mut self.vals);
        let old_u = core::mem::take(&mut self.used);
        let n = old_k.len() * 2;
        self.keys = vec![0; n];
        self.vals = vec![0; n];
        self.used = vec![false; n];
        self.n = 0;
        for i in 0..old_k.len() {
            if old_u[i] {
                *self.entry(old_k[i]).0 = old_v[i];
            }
        }
    }
    /// (value slot, newly inserted)
    fn entry(&mut self, k: u128) -> (&mut u32, bool) {
        if (self.n + 1) * 2 > self.keys.len() {
            self.grow();
        }
        let mut s = self.slot(k);
        loop {
            if !self.used[s] {
                self.used[s] = true;
                self.keys[s] = k;
                self.vals[s] = 0;
                self.n += 1;
                return (&mut self.vals[s], true);
            }
            if self.keys[s] == k {
                return (&mut self.vals[s], false);
            }
            s = (s + 1) & (self.keys.len() - 1);
        }
    }
    fn get(&self, k: u128) -> Option<u32> {
        let mut s = self.slot(k);
        loop {
            if !self.used[s] {
                return None;
            }
            if self.keys[s] == k {
                return Some(self.vals[s]);
            }
            s = (s + 1) & (self.keys.len() - 1);
        }
    }
}

#[inline(always)]
fn ckey(img: &Mi, b: usize, nb: usize, i: usize) -> u128 {
    let mut k = 0u128;
    for c in 0..nb {
        k |= (img.ch[b + c].d[i] as u32 as u128) << (32 * c);
    }
    k
}

/// FwdPalette (lossless): channels begin ..= end into indices; nb_colors = the limit in,
/// the palette size out
pub fn fwd_palette(img: &mut Mi, begin: usize, end: usize, nb_colors: u32, ordered: bool) -> bool {
    if !equal_channels(img, begin, end) || begin < img.nbmeta as usize {
        return false;
    }
    let nb = end - begin + 1;
    let bd = img.bd;
    if bd >= 32 || nb_colors < 2 {
        return false;
    }
    let n = img.ch[begin].d.len();
    if nb == 1 {
        let (mn, mx) = minmax(&img.ch[begin]);
        let size = (mx as i64 - mn as i64 + 1) as usize;
        let mut pal: Vec<i32>;
        if size > 1 << 16 {
            let mut vals = img.ch[begin].d.clone();
            vals.sort_unstable();
            vals.dedup();
            if vals.len() > nb_colors as usize {
                return false;
            }
            pal = vals;
            let d = &mut img.ch[begin].d;
            for v in d.iter_mut() {
                *v = pal.binary_search(v).unwrap_or(0) as i32;
            }
        } else {
            let mut lookup = vec![0i32; size];
            let mut idx = 0u32;
            for &v in &img.ch[begin].d {
                let k = (v - mn) as usize;
                if lookup[k] == 0 {
                    lookup[k] = 1;
                    idx += 1;
                    if idx > nb_colors {
                        return false;
                    }
                }
            }
            pal = Vec::with_capacity(idx as usize);
            for (i, l) in lookup.iter_mut().enumerate() {
                if *l != 0 {
                    *l = pal.len() as i32;
                    pal.push(i as i32 + mn);
                }
            }
            for v in img.ch[begin].d.iter_mut() {
                *v = lookup[(*v - mn) as usize];
            }
        }
        let pw = pal.len() as u32;
        img.nbmeta += 1;
        img.ch.insert(0, Mc { w: pw, h: 1, hs: -1, vs: -1, d: pal });
        img.tr.push(Mt { id: MTR_PALETTE, begin: begin as u32, numc: 1, nbcol: pw, nbdelta: 0, pred: 0, ..Default::default() });
        return true;
    }
    if nb > 4 {
        return false;
    }
    // implicit colours
    let mut implicit = ColorMap::new();
    let mut implicit_list: Vec<u128> = Vec::with_capacity(189);
    for k in 0..189 {
        let mut key = 0u128;
        for c in 0..nb {
            key |= (implicit_value(k, c, bd) as u32 as u128) << (32 * c);
        }
        *implicit.entry(key).0 = k as u32 + 1;
        implicit_list.push(key);
    }
    let mut cand = ColorMap::new(); // colour -> frequency
    let mut order: Vec<u128> = Vec::new();
    let mut implicit_used = 0u32;
    for i in 0..n {
        let key = ckey(img, begin, nb, i);
        let (f, new) = cand.entry(key);
        *f += 1;
        if new {
            if implicit.get(key).is_some() {
                implicit_used += 1;
            } else {
                order.push(key);
                if order.len() > nb_colors as usize {
                    return false;
                }
            }
        }
    }
    let mut ncol = order.len() as u32;
    if ncol + implicit_used == 1 {
        return false;
    }
    for &k in &implicit_list {
        if cand.get(k).unwrap_or(0) > 10 {
            ncol += 1;
            order.push(k);
        }
    }
    let mut inv = ColorMap::new();
    for (k, &key) in implicit_list.iter().enumerate() {
        *inv.entry(key).0 = ncol + k as u32;
    }
    if ordered && nb >= 3 {
        let luma = |k: u128| -> f32 {
            let c = |i: usize| (k >> (32 * i)) as u32 as i32 as f32;
            let mut y = 0.299f32 * c(0) + 0.587f32 * c(1) + 0.114f32 * c(2) + 0.1f32;
            if nb > 3 {
                y *= 1.0 + c(3);
            }
            if cand.get(k).unwrap_or(0) > 4 { -y } else { y }
        };
        order.sort_by(|&a, &b| luma(a).total_cmp(&luma(b)));
    }
    let pw = order.len();
    let mut pal = vec![0i32; pw * nb];
    for (x, &key) in order.iter().enumerate() {
        for c in 0..nb {
            pal[c * pw + x] = (key >> (32 * c)) as u32 as i32;
        }
        *inv.entry(key).0 = x as u32;
    }
    let mut idx = vec![0i32; n];
    for (i, v) in idx.iter_mut().enumerate() {
        *v = inv.get(ckey(img, begin, nb, i)).unwrap_or(0) as i32;
    }
    img.ch[begin].d = idx;
    img.ch.drain(begin + 1..=end);
    img.nbmeta += 1;
    img.ch.insert(0, Mc { w: pw as u32, h: nb as u32, hs: -1, vs: -1, d: pal });
    img.tr.push(Mt { id: MTR_PALETTE, begin: begin as u32, numc: nb as u32, nbcol: ncol, nbdelta: 0, pred: 0, ..Default::default() });
    true
}

/// SmoothTendency of the squeeze (B = before / above, a = average, n = next)
#[inline(always)]
fn tendency(b: i64, a: i64, n: i64) -> i64 {
    let mut diff = 0i64;
    if b >= a && a >= n {
        diff = (4 * b - 3 * n - a + 6) / 12;
        if diff - (diff & 1) > 2 * (b - a) {
            diff = 2 * (b - a) + 1;
        }
        if diff + (diff & 1) > 2 * (a - n) {
            diff = 2 * (a - n);
        }
    } else if b <= a && a <= n {
        diff = (4 * b - 3 * n - a - 6) / 12;
        if diff + (diff & 1) < 2 * (b - a) {
            diff = 2 * (b - a) - 1;
        }
        if diff - (diff & 1) < 2 * (a - n) {
            diff = 2 * (a - n);
        }
    }
    diff
}

/// FwdHSqueeze / FwdVSqueeze of channel c into (averages in c, residuals as a new channel at
/// position rpos)
fn squeeze_one(img: &mut Mi, c: usize, rpos: usize, horizontal: bool) {
    let ch = img.ch[c].clone();
    let (w, h) = (ch.w as usize, ch.h as usize);
    let (aw, ah, rw, rh) = if horizontal { ((w + 1) / 2, h, w / 2, h) } else { (w, (h + 1) / 2, w, h / 2) };
    let mut avg = vec![0i32; aw * ah];
    let mut res = vec![0i32; rw * rh];
    let at = |x: usize, y: usize| ch.d[y * w + x] as i64;
    if horizontal {
        for y in 0..h {
            for x in 0..rw {
                let a = at(2 * x, y);
                let b = at(2 * x + 1, y);
                let av = (a + b + (a > b) as i64) >> 1;
                avg[y * aw + x] = av as i32;
            }
            if aw > rw {
                avg[y * aw + rw] = at(w - 1, y) as i32;
            }
            for x in 0..rw {
                let diff = at(2 * x, y) - at(2 * x + 1, y);
                let a = avg[y * aw + x] as i64;
                let next = if x + 1 < aw { avg[y * aw + x + 1] as i64 } else { a };
                let left = if x > 0 { at(2 * x - 1, y) } else { a };
                let t = tendency(left, a, next);
                res[y * rw + x] = (diff - t) as i32;
            }
        }
    } else {
        for x in 0..w {
            for y in 0..rh {
                let a = at(x, 2 * y);
                let b = at(x, 2 * y + 1);
                let av = (a + b + (a > b) as i64) >> 1;
                avg[y * aw + x] = av as i32;
            }
            if ah > rh {
                avg[rh * aw + x] = at(x, h - 1) as i32;
            }
            for y in 0..rh {
                let diff = at(x, 2 * y) - at(x, 2 * y + 1);
                let a = avg[y * aw + x] as i64;
                let next = if y + 1 < ah { avg[(y + 1) * aw + x] as i64 } else { a };
                let top = if y > 0 { at(x, 2 * y - 1) } else { a };
                let t = tendency(top, a, next);
                res[y * rw + x] = (diff - t) as i32;
            }
        }
    }
    let (mut hs, mut vs) = (ch.hs, ch.vs);
    if horizontal {
        if hs >= 0 {
            hs += 1;
        }
    } else if vs >= 0 {
        vs += 1;
    }
    img.ch[c] = Mc { w: aw as u32, h: ah as u32, hs, vs, d: avg };
    img.ch.insert(rpos, Mc { w: rw as u32, h: rh as u32, hs, vs, d: res });
}

/// FwdSqueeze with the default parameters (DefaultSqueezeParameters)
pub fn fwd_squeeze(img: &mut Mi) -> bool {
    let nm = img.nbmeta as usize;
    let nch = img.ch.len();
    if nch <= nm {
        return false;
    }
    let nbc = nch - nm;
    let mut steps: Vec<[u32; 4]> = Vec::new();
    let (mut w, mut h) = (img.ch[nm].w, img.ch[nm].h);
    if nbc > 2 && img.ch[nm + 1].w == w && img.ch[nm + 1].h == h {
        steps.push([1, 0, nm as u32 + 1, 2]);
        steps.push([0, 0, nm as u32 + 1, 2]);
    }
    if w <= h && h > 8 {
        steps.push([0, 1, nm as u32, nbc as u32]);
        h = (h + 1) >> 1;
    }
    while w > 8 || h > 8 {
        if w > 8 {
            steps.push([1, 1, nm as u32, nbc as u32]);
            w = (w + 1) >> 1;
        }
        if h > 8 {
            steps.push([0, 1, nm as u32, nbc as u32]);
            h = (h + 1) >> 1;
        }
    }
    for st in &steps {
        let bc = st[2] as usize;
        let nc = st[3] as usize;
        let ec = bc + nc - 1;
        let offset = if st[1] != 0 { ec + 1 } else { img.ch.len() };
        for c in bc..=ec {
            squeeze_one(img, c, c - bc + offset, st[0] != 0);
        }
    }
    img.tr.push(Mt { id: MTR_SQUEEZE, ..Default::default() });
    true
}

/// try_palettes (lossless): full palette (all colour channels), then channel palettes
pub fn try_palettes(img: &mut Mi, tier: i32, chan_percent: f32, palette_colors: u32) {
    let nbc = img.ch.len() - img.nbmeta as usize;
    if nbc == 0 {
        return;
    }
    let npix = (img.ch[img.nbmeta as usize].w * img.ch[img.nbmeta as usize].h) as f32;
    let nb_chans = nbc as f32;
    let est = 0.2f32 * img.bd as f32 * nb_chans;
    let mut did = 0usize;
    let mut cost_before = 0.0f32;
    if palette_colors != 0 {
        cost_before = if tier <= 3 { est_cost(img) } else { npix * est };
        if nbc > 1 {
            let ncol = ((cost_before * 0.0005 + npix / 128.0 + 128.0) as i32).min(palette_colors as i32);
            let b = img.nbmeta as usize;
            if maybe(img, tier, cost_before, |m| fwd_palette(m, b, b + nbc - 1, ncol.max(0) as u32, true)) {
                did = 1;
            }
        }
        if did == 0 && nbc > 3 {
            let ncol = ((cost_before * 0.0005 + npix / 128.0 + 128.0) as i32).min(palette_colors as i32);
            let b = img.nbmeta as usize;
            if maybe(img, tier, cost_before, |m| fwd_palette(m, b, b + nbc - 2, ncol.max(0) as u32, true)) {
                did = 1;
            }
        }
    }
    if chan_percent > 0.0 {
        let nch = img.ch.len() - img.nbmeta as usize - did;
        if nch > 0 && (did != 0 || cost_before == 0.0) {
            cost_before = if tier < 3 { est_cost(img) } else { 0.0 };
        }
        for i in did..nch + did {
            let c = i + img.nbmeta as usize;
            if c >= img.ch.len() {
                break;
            }
            let (mn, mx) = minmax(&img.ch[c]);
            let colors = mx as i64 - mn as i64 + 1;
            let lim = ((npix / 16.0) as i64).min((chan_percent as f64 / 100.0 * colors as f64) as i64);
            let begin = c;
            let _ = maybe(img, tier, cost_before, |m| fwd_palette(m, begin, begin, lim.max(0) as u32, true));
        }
    }
}

/// maybe_do_transform: at efforts 8+ keep the transform only if the estimated cost drops
fn maybe(img: &mut Mi, tier: i32, cost_before: f32, f: impl FnOnce(&mut Mi) -> bool) -> bool {
    if tier >= 3 {
        return f(img);
    }
    let backup_ch = img.ch.clone();
    let backup_tr = img.tr.len();
    let backup_nm = img.nbmeta;
    if !f(img) {
        return false;
    }
    if est_cost(img) > cost_before {
        img.ch = backup_ch;
        img.tr.truncate(backup_tr);
        img.nbmeta = backup_nm;
        return false;
    }
    true
}

/// the RCT search of PrepareStreamParams: the best of n candidates by est_cost
pub fn rct_search(img: &mut Mi, n: usize, threads: bool) {
    static RCTS: [u32; 19] = [0, 6, 5, 10, 26, 40, 12, 19, 8, 4, 9, 15, 16, 17, 32, 33, 2, 1, 3];
    let b = img.nbmeta as usize;
    // jxl-speed: the candidates on copies at the same time, chosen in their order
    let src = &*img;
    let costs: Vec<Option<f32>> = par_map(n.min(RCTS.len()), threads, |k| {
        let mut c = Mi { w: src.w, h: src.h, bd: src.bd, nbmeta: src.nbmeta, ch: src.ch.clone(), tr: src.tr.clone() };
        if fwd_rct(&mut c, b, RCTS[k]) { Some(est_cost(&c)) } else { None }
    });
    let mut best_cost = f32::MAX;
    let mut best = 0u32;
    for (k, c) in costs.iter().enumerate() {
        if let Some(c) = *c
            && c < best_cost
        {
            best = RCTS[k];
            best_cost = c;
        }
    }
    fwd_rct(img, b, best);
}

/// a cost estimate of squeezed channels: zero predictor, one histogram per channel
pub fn est_zero_cost(img: &Mi) -> f32 {
    let cfg = Huc::default();
    let mut extra = 0usize;
    let mut cost = 0.0f32;
    let mut h = Histo::default();
    for ch in &img.ch[img.nbmeta as usize..] {
        for &v in &ch.d {
            let (tok, nb, _) = cfg.encode(pack_signed(v));
            h.add(tok as usize);
            extra += nb as usize;
        }
        cost += h.shannon();
        h.clear();
    }
    cost + extra as f32
}

/// the RCT search for a squeezed image (jxl-plus): each candidate judged after the squeeze
/// (candidates on the pool); the best one applied, the squeeze not
pub fn rct_search_squeeze(img: &mut Mi, n: usize, threads: bool) {
    static RCTS: [u32; 19] = [0, 6, 5, 10, 26, 40, 12, 19, 8, 4, 9, 15, 16, 17, 32, 33, 2, 1, 3];
    let b = img.nbmeta as usize;
    let src = &*img;
    let costs: Vec<f32> = par_map(n.min(RCTS.len()), threads, |k| {
        let mut c = Mi { w: src.w, h: src.h, bd: src.bd, nbmeta: src.nbmeta, ch: src.ch.clone(), tr: src.tr.clone() };
        if !fwd_rct(&mut c, b, RCTS[k]) {
            return f32::MAX;
        }
        fwd_squeeze(&mut c);
        est_zero_cost(&c)
    });
    let mut best = 0usize;
    for (k, &c) in costs.iter().enumerate() {
        if c < costs[best] {
            best = k;
        }
    }
    fwd_rct(img, b, RCTS[best]);
}

// ------------------------------------------------------------------------- headers

static D_BEGIN: [u32; 8] = [0, 3, 8, 6, 72, 10, 1096, 13];
static D_RCT: [u32; 8] = [6, 0, 0, 2, 2, 4, 10, 6];
static D_NUMC: [u32; 8] = [1, 0, 3, 0, 4, 0, 1, 13];
static D_NBCOL: [u32; 8] = [0, 8, 256, 10, 1280, 12, 5376, 16];
static D_NBDELTA: [u32; 8] = [0, 0, 1, 8, 257, 10, 1281, 16];
static D_NT: [u32; 8] = [0, 0, 1, 0, 2, 4, 18, 8];
static D_NSQ: [u32; 8] = [0, 0, 1, 4, 9, 6, 41, 8];

/// GroupHeader: use_global_tree, WP header, transforms
pub fn write_group_header(jw: &mut Jw, global: bool, wp: &[u32; 11], tr: &[Mt]) {
    jw.bits(global as u64, 1);
    if *wp == WH_DEF {
        jw.bits(1, 1);
    } else {
        jw.bits(0, 1);
        for &v in wp.iter().take(7) {
            jw.bits(v as u64, 5);
        }
        for &v in wp.iter().skip(7) {
            jw.bits(v as u64, 4);
        }
    }
    jw.u32(tr.len() as u32, &D_NT);
    for t in tr {
        jw.bits(t.id as u64, 2); // U32 Val(0), Val(1), Val(2), Val(3): selector = id
        match t.id {
            MTR_SQUEEZE => jw.u32(0, &D_NSQ), // default squeeze parameters
            MTR_RCT => {
                jw.u32(t.begin, &D_BEGIN);
                jw.u32(t.rct, &D_RCT);
            }
            _ => {
                jw.u32(t.begin, &D_BEGIN);
                jw.u32(t.numc, &D_NUMC);
                jw.u32(t.nbcol, &D_NBCOL);
                jw.u32(t.nbdelta, &D_NBDELTA);
                jw.bits(t.pred as u64, 4);
            }
        }
    }
}

// ------------------------------------------------------------------------- the stream set

/// one modular stream
pub struct Stream {
    pub img: Mi,
    pub wp: [u32; 11],
    pub opts: MOpts,
    pub tokens: Vec<Token>,
    pub width: usize,
    pub coded: usize, // channels coded in this stream (meta + those within max_chan)
}

impl Stream {
    pub fn new(img: Mi, opts: MOpts) -> Stream {
        Stream { img, wp: WH_DEF, opts, tokens: Vec::new(), width: 0, coded: 0 }
    }
    /// the channels coded here: every meta channel and the others up to the first larger than
    /// max_chan
    fn count_coded(&mut self) {
        let mut n = 0usize;
        let mut w = 0usize;
        for (i, c) in self.img.ch.iter().enumerate() {
            if i >= self.img.nbmeta as usize && (c.w > self.opts.max_chan || c.h > self.opts.max_chan) {
                break;
            }
            n = i + 1;
            if c.w as usize > w {
                w = c.w as usize;
            }
        }
        self.coded = n;
        self.width = w;
    }
}

/// the modular streams of a frame and their shared tree / histograms
pub struct Streams {
    pub st: Vec<Option<Stream>>,
    pub tree: Tree,
    pub tree_tokens: Vec<Token>,
    pub code: Code,
    pub splits: Vec<usize>,
    pub tier: i32,
    pub mul_info: Vec<MulInfo>,
}

impl Streams {
    pub fn new(n: usize, tier: i32) -> Streams {
        let mut st = Vec::with_capacity(n);
        st.resize_with(n, || None);
        Streams { st, tree: Vec::new(), tree_tokens: Vec::new(), code: Code::default(), splits: vec![0, n], tier, mul_info: Vec::new() }
    }

    /// ComputeTree: learn (or pick) the tree over all streams; threads = gather in parallel
    pub fn learn_tree(&mut self, threads: bool) {
        for s in self.st.iter_mut().flatten() {
            s.count_coded();
        }
        let n = self.st.len();
        let splits = self.splits.clone();
        let mut useful: Vec<usize> = Vec::new();
        for k in 0..splits.len() - 1 {
            if (splits[k]..splits[k + 1]).any(|i| self.st[i].as_ref().is_some_and(|s| s.coded > 0)) {
                useful.push(splits[k]);
            }
        }
        if useful.is_empty() {
            return;
        }
        useful.push(*splits.last().unwrap_or(&n));
        let mut trees: Vec<Tree> = Vec::with_capacity(useful.len() - 1);
        for k in 0..useful.len() - 1 {
            let (mut start, mut stop) = (useful[k], useful[k + 1]);
            while start < stop && self.st[start].as_ref().is_none_or(|s| s.coded == 0) {
                start += 1;
            }
            while start < stop && self.st[stop - 1].as_ref().is_none_or(|s| s.coded == 0) {
                stop -= 1;
            }
            let o = self.st[start].as_ref().map(|s| s.opts.clone()).unwrap_or_default();
            if o.kind != Kind::Learn {
                let mut total = 0usize;
                for i in start..stop {
                    if let Some(s) = &self.st[i] {
                        for c in &s.img.ch {
                            total += (c.w * c.h) as usize;
                        }
                    }
                }
                trees.push(ema::fixed_tree(o.kind, total, 8, 0));
                continue;
            }
            let mut samples = Samples::new(o.predictor, &o.props, o.wp_tree);
            // property quantization from pixel samples of every stream
            let mut pix: Vec<i32> = Vec::new();
            let mut diff: Vec<i32> = Vec::new();
            let mut gcount: Vec<u32> = Vec::new();
            let mut ccount: Vec<u32> = Vec::new();
            let mut max_c = 0u32;
            for i in start..stop {
                if let Some(s) = &self.st[i] {
                    max_c = max_c.max(s.img.ch.len() as u32);
                    collect_pixel_samples(s, i as u32, &mut gcount, &mut ccount, &mut pix, &mut diff);
                }
            }
            let range = [[0, max_c], [start as u32, stop as u32]];
            samples.prequantize(&range, &self.mul_info, &gcount, &ccount, &mut pix, &mut diff, o.max_vals);
            // gather per stream in parallel into private sample sets with the same property
            // quantization, then merge them in stream order (deterministic for any thread count)
            let ids: Vec<usize> = (start..stop).filter(|&i| self.st[i].as_ref().is_some_and(|s| s.coded > 0)).collect();
            let st = &self.st;
            let proto = &samples;
            let parts: Vec<(Samples, usize)> = par_map(ids.len(), threads, |k| {
                let s = st[ids[k]].as_ref().expect("stream");
                let mut local = proto.empty_like();
                let mut t = 0usize;
                gather_stream(s, ids[k] as u32, &mut local, &mut t);
                (local, t)
            });
            let total: usize = parts.iter().map(|p| p.1).sum();
            let parts: Vec<Samples> = parts.into_iter().map(|p| p.0).collect();
            samples.merge_all(&parts, threads);
            drop(parts);
            let tree = if !samples.has_samples() {
                vec![Node::leaf(samples.preds[0])]
            } else {
                let frac = samples.num_samples as f32 / total.max(1) as f32;
                let required = frac * 0.9 + 0.1;
                samples.done();
                ema::best_tree(&mut samples, o.threshold * required, &self.mul_info, range, o.fast_mult, threads)
            };
            trees.push(tree);
        }
        let mut t: Tree = Vec::new();
        ema::merge(&trees, &useful, 0, useful.len() - 1, &mut t);
        let (toks, dt) = ema::tokenize(&t);
        self.tree = dt;
        self.tree_tokens = toks;
    }

    /// ComputeTokens (in parallel over the streams)
    pub fn tokens(&mut self, threads: bool) {
        let tree = &self.tree;
        let st = &self.st;
        let toks: Vec<Vec<Token>> = par_map(st.len(), threads, |i| {
            let mut v = Vec::new();
            if let Some(s) = &st[i] {
                for c in 0..s.coded {
                    chan_tokens(&s.img, c, i as u32, &s.wp, tree, &mut v);
                }
            }
            v
        });
        for (s, t) in self.st.iter_mut().zip(toks) {
            if let Some(s) = s {
                s.tokens = t;
            }
        }
    }

    /// EncodeGlobalInfo: tree flag, tree, histograms of every stream's tokens
    pub fn write_global(&mut self, jw: &mut Jw, hp: &HParams) {
        if self.tree_tokens.is_empty() {
            jw.bits(0, 1);
            return;
        }
        jw.bits(1, 1);
        let mut tt = vec![core::mem::take(&mut self.tree_tokens)];
        let tc = eans::build(hp, 6, &mut tt, Some(jw));
        eans::write_tokens(&tt[0], &tc, jw);
        self.tree_tokens = core::mem::take(&mut tt[0]);
        let mut hp2 = hp.clone();
        hp2.widths = self.st.iter().map(|s| s.as_ref().map(|s| s.width).unwrap_or(0)).collect();
        let mut streams: Vec<Vec<Token>> = self.st.iter_mut().map(|s| s.as_mut().map(|s| core::mem::take(&mut s.tokens)).unwrap_or_default()).collect();
        let nctx = (self.tree.len() + 1) / 2;
        self.code = eans::build(&hp2, nctx, &mut streams, Some(jw));
        for (s, t) in self.st.iter_mut().zip(streams) {
            if let Some(s) = s {
                s.tokens = t;
            }
        }
    }

    /// EncodeStream: group header + tokens (nothing for a stream without channels)
    pub fn write_stream(&self, id: usize, jw: &mut Jw) {
        let Some(s) = self.st.get(id).and_then(|s| s.as_ref()) else { return };
        if s.img.ch.is_empty() {
            return;
        }
        write_group_header(jw, true, &s.wp, &s.img.tr);
        if s.coded > 0 && s.img.ch[..s.coded].iter().any(|c| c.w != 0 && c.h != 0) {
            eans::write_tokens(&s.tokens, &self.code, jw);
        }
    }
}

/// ModularGenericCompress with a local tree: group header (no global tree), the tree with its
/// histograms, the histograms of the residuals and the residual tokens; nothing for a stream
/// without channels.  `id` is the stream id (property 1).
pub fn encode_local(s: &mut Stream, id: u32, threads: bool) -> Jw {
    let mut jw = Jw::new();
    if s.img.ch.is_empty() {
        return jw;
    }
    s.count_coded();
    write_group_header(&mut jw, false, &s.wp, &s.img.tr);
    if !s.img.ch[..s.coded].iter().any(|c| c.w != 0 && c.h != 0) {
        return jw;
    }
    let mut total = 0usize;
    for c in &s.img.ch[..s.coded] {
        total += (c.w * c.h) as usize;
    }
    let total = total.max(1);
    let o = &s.opts;
    let tree: Tree = if o.kind != Kind::Learn {
        ema::fixed_tree(o.kind, total, s.img.bd, 0)
    } else {
        let mut samples = Samples::new(o.predictor, &o.props, o.wp_tree);
        let mut pix: Vec<i32> = Vec::new();
        let mut diff: Vec<i32> = Vec::new();
        let mut gcount: Vec<u32> = Vec::new();
        let mut ccount: Vec<u32> = Vec::new();
        collect_pixel_samples(s, 0, &mut gcount, &mut ccount, &mut pix, &mut diff);
        let range = [[0u32, 0u32], [0u32, 0u32]];
        samples.prequantize(&range, &[], &gcount, &ccount, &mut pix, &mut diff, o.max_vals);
        let mut t = 0usize;
        gather_stream(s, id, &mut samples, &mut t);
        if !samples.has_samples() {
            vec![Node::leaf(samples.preds[0])]
        } else {
            let frac = samples.num_samples as f32 / total as f32;
            let required = frac * 0.9 + 0.1;
            samples.done();
            ema::best_tree(&mut samples, o.threshold * required, &[], range, o.fast_mult, threads)
        }
    };
    let (tt, dt) = ema::tokenize(&tree);
    let mut tts = vec![tt];
    let tc = eans::build(&o.hp, 6, &mut tts, Some(&mut jw));
    eans::write_tokens(&tts[0], &tc, &mut jw);
    let mut toks = Vec::new();
    for c in 0..s.coded {
        chan_tokens(&s.img, c, id, &s.wp, &dt, &mut toks);
    }
    let mut hp = o.hp.clone();
    hp.widths = vec![s.width];
    let mut streams = vec![toks];
    let code = eans::build(&hp, (dt.len() + 1) / 2, &mut streams, Some(&mut jw));
    eans::write_tokens(&streams[0], &code, &mut jw);
    jw
}

/// CollectPixelSamples of a stream
fn collect_pixel_samples(s: &Stream, gid: u32, gcount: &mut Vec<u32>, ccount: &mut Vec<u32>, pix: &mut Vec<i32>, diff: &mut Vec<i32>) {
    if s.opts.nb_repeats == 0.0 {
        return;
    }
    let img = &s.img;
    if gcount.len() <= gid as usize {
        gcount.resize(gid as usize + 1, 0);
    }
    if ccount.len() < img.ch.len() {
        ccount.resize(img.ch.len(), 0);
    }
    let mut rng = Rng::new(gid as u64);
    let fraction = (s.opts.nb_repeats as f64 * 0.1).min(0.99) as f32;
    let dist = 1.0f32 / crate::emath::ln(1.0 - fraction);
    let mut ids: Vec<usize> = Vec::new();
    for (i, c) in img.ch.iter().enumerate() {
        if c.w <= 1 || c.h == 0 {
            continue;
        }
        if i >= img.nbmeta as usize && (c.w > s.opts.max_chan || c.h > s.opts.max_chan) {
            break;
        }
        ids.push(i);
        gcount[gid as usize] += c.w * c.h;
        ccount[i] += c.w * c.h;
    }
    if ids.is_empty() {
        return;
    }
    let (mut i, mut y, mut x) = (0usize, 0usize, 0usize);
    let advance = |amount: usize, i: &mut usize, y: &mut usize, x: &mut usize| -> bool {
        *x += amount;
        while *x >= img.ch[ids[*i]].w as usize {
            *x -= img.ch[ids[*i]].w as usize;
            *y += 1;
            if *y == img.ch[ids[*i]].h as usize {
                *i += 1;
                *y = 0;
                if *i >= ids.len() {
                    return false;
                }
            }
        }
        true
    };
    let g = rng.geometric(dist) as usize;
    if !advance(g, &mut i, &mut y, &mut x) {
        return;
    }
    while i < ids.len() {
        let ch = &img.ch[ids[i]];
        let row = &ch.d[y * ch.w as usize..];
        pix.push(row[x]);
        let xp = if x == 0 { 1 } else { x - 1 };
        diff.push((row[x] as i64 - row[xp] as i64) as i32);
        let g = rng.geometric(dist) as usize + 1;
        if !advance(g, &mut i, &mut y, &mut x) {
            break;
        }
    }
}

/// keep `gather` reachable for single-stream use (the parallel path uses gather_into)
pub fn gather_stream(s: &Stream, gid: u32, samples: &mut Samples, total: &mut usize) {
    for c in 0..s.coded {
        gather(&s.img, c, gid, &s.wp, &s.opts, samples, total);
    }
}
