// bc7 -- BC7 block encoder                                             [src/img/bc7_enc.s]
//
//   enc(scr, px, dst, cfg)      one block: 16 straight BGRA pixels -> 16 bytes at dst
// Also used by bc6h (the B7 scratch: aos, moments, partition estimates): moments, est, top,
// submask, Bw (the 128-bit bit writer).
//
// Levels (DirectXTex's BC7 speeds): fast = mode 6 only; normal = modes 1, 3 and 6 for opaque
// blocks, 4 / 5 (rotation 0, both index selections), 6 and 7 for blocks with alpha; slow = all
// 8 modes, every rotation / index selection, all p-bit combinations, more partition
// candidates and refinement passes.
//
// Per subset (enc's pixel set helpers): principal axis endpoints -> quantization to the
// mode's endpoint precision (tables B7_QT: nearest code per 8-bit value, with / without
// p-bit; unique p-bits chosen per endpoint, shared ones per subset, slow: every combination)
// -> exact palette ((e0 (64 - w) + e1 w + 32) >> 6) -> nearest entry per pixel (squared RGBA
// error) -> least-squares endpoints for the chosen weights, repeated (fast 1, normal 2, slow 3
// passes); the best is kept.  Partitioned modes: every partition is estimated from per-pixel
// moments: n (trace - lambda + lambda / (levels - 1)^2) per subset, lambda = largest
// eigenvalue of the covariance (power iteration, Rayleigh quotient); the K best partitions
// (normal 4, slow 16) are fitted exactly.  Anchor indices are fixed by swapping a subset's
// endpoints and mirroring its indices.  One-colour blocks use mode 5 with table endpoints
// (index 1, exact for every colour) and exact 8-bit alpha.

use rt::fx::minss;

use crate::dec::{W2, W3, W4};
use simd::{Simd, Sse2};

use crate::enc::{assign, bput, fbits, lane, load, lsq, matv, pca, vmax, vsum, PalB, Pxs, Scratch, V4, INF, PALB0, TINY};
use crate::tables::{ANC2, ANC3A, ANC3B, B7_M2, B7_M3, B7_QT, B7_SINGLE};
use crate::{BCQ_FAST, BCQ_LEVEL, BCQ_NORMAL, BCQ_SLOW};

// fit config word (mq): ib | ec << 4 | ea << 8 | pm << 12 | iterations << 16 | all p-bits << 20
const MQ_ALLP: u32 = 1 << 20;

// mode descriptors: ns, partition bits, colour bits, alpha bits, p-bit mode (0 none, 1 per
// endpoint, 2 per subset), index bits, effective colour bits, effective alpha bits, second
// index bits
const MD_NS: usize = 0;
const MD_PB: usize = 1;
const MD_CB: usize = 2;
const MD_AB: usize = 3;
const MD_PM: usize = 4;
const MD_IB: usize = 5;
const MD_EC: usize = 6;
const MD_EA: usize = 7;
const MD_IB2: usize = 8;
static MODES: [[u8; 9]; 8] = [
    [3, 4, 4, 0, 1, 3, 5, 0, 0],
    [2, 6, 6, 0, 2, 3, 7, 0, 0],
    [3, 6, 5, 0, 0, 2, 5, 0, 0],
    [2, 6, 7, 0, 1, 2, 8, 0, 0],
    [1, 0, 5, 6, 0, 2, 5, 6, 3],
    [1, 0, 7, 8, 0, 2, 7, 8, 2],
    [1, 0, 7, 7, 1, 4, 8, 8, 0],
    [2, 6, 5, 5, 1, 2, 6, 6, 0],
];
// q = 1 / (levels - 1)^2 for 2, 3, 4 index bits: 1/9, 1/49, 1/225
static QLEV: [u32; 3] = [0x3DE3_8E39, 0x3CA7_2F05, 0x3B91_A2B4];
// least-squares weights w / 64 per index: 2 bits, 3 bits, 4 bits
static LW2: [f32; 4] = fbits([0x0000_0000, 0x3EA8_0000, 0x3F2C_0000, 0x3F80_0000]);
static LW3: [f32; 8] = fbits([0x0000_0000, 0x3E10_0000, 0x3E90_0000, 0x3ED8_0000, 0x3F14_0000, 0x3F38_0000, 0x3F5C_0000, 0x3F80_0000]);
static LW4: [f32; 16] = fbits([
    0x0000_0000, 0x3D80_0000, 0x3E10_0000, 0x3E50_0000, 0x3E88_0000, 0x3EA8_0000, 0x3ED0_0000, 0x3EF0_0000,
    0x3F08_0000, 0x3F18_0000, 0x3F2C_0000, 0x3F3C_0000, 0x3F4C_0000, 0x3F5C_0000, 0x3F70_0000, 0x3F80_0000,
]);

/// a fit result (FR_*)
#[derive(Clone, Copy)]
pub(crate) struct Fr {
    err: f32,
    c0: [u8; 4], // codes of endpoint 0 (effective bits, p-bit = bit 0)
    c1: [u8; 4],
    idx: [u8; 16], // index per lane
}

impl Fr {
    const ZERO: Fr = Fr { err: 0.0, c0: [0; 4], c1: [0; 4], idx: [0; 16] };
}

/// the fitting state (B7_E .. B7_IDX)
#[derive(Clone, Copy)]
pub(crate) struct Work {
    palb: PalB,
    e: [[f32; 4]; 2],  // fit endpoints
    qx: [[i32; 4]; 2], // endpoints as clamped integers
    qe: [[i32; 4]; 2], // expanded (8-bit) endpoint values
    qc: [[u8; 4]; 2],  // codes
    idx: [u8; 16],     // indices of the last assignment
}

/// the BC7 scratch (B7_*)
#[derive(Clone, Copy)]
pub(crate) struct B7 {
    pub aos: [[f32; 4]; 16],   // block (R, G, B, A)
    mom: [[[f32; 4]; 5]; 16],  // per pixel: x, x * r, x * g, x * b, x * a
    tot: [[f32; 4]; 5],        // sums of the 16 pixels
    res: [f32; 64],            // residual term per partition
    lam: [f32; 64],            // lambda term per partition
    pub cand: [(f32, u32); 16], // {score, partition}
    pub ncand: u32,
    pxs: [Pxs; 4], // 3 subsets; [3] = the scalar part of modes 4 / 5
    fr: [Fr; 4],
    w: Work,
    best: [u8; 16], // best block
    berr: f32,
    lvl: u32,
    pix: [u8; 16], // per-pixel indices (packing)
    anc: [u8; 4],  // anchor pixels
}

impl B7 {
    pub const ZERO: B7 = B7 {
        aos: [[0.0; 4]; 16],
        mom: [[[0.0; 4]; 5]; 16],
        tot: [[0.0; 4]; 5],
        res: [0.0; 64],
        lam: [0.0; 64],
        cand: [(0.0, 0); 16],
        ncand: 0,
        pxs: [Pxs::ZERO; 4],
        fr: [Fr::ZERO; 4],
        w: Work {
            palb: PALB0,
            e: [[0.0; 4]; 2],
            qx: [[0; 4]; 2],
            qe: [[0; 4]; 2],
            qc: [[0; 4]; 2],
            idx: [0; 16],
        },
        best: [0; 16],
        berr: 0.0,
        lvl: 0,
        pix: [0; 16],
        anc: [0; 4],
    };
}

// ------------------------------------------------------------------ encoder

/// one BC7 block  [bc7_enc]
pub(crate) fn enc(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], cfg: u32) {
    let b = &mut s.b7;
    b.lvl = cfg & BCQ_LEVEL;
    if b.lvl == 0 {
        b.lvl = BCQ_NORMAL;
    }
    // opaque?  one colour?
    let first = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
    let mut opq = true;
    let mut multi = false;
    for i in 0..16 {
        let v = u32::from_le_bytes([px[i * 4], px[i * 4 + 1], px[i * 4 + 2], px[i * 4 + 3]]);
        if v != first {
            multi = true;
        }
        if v >> 24 != 255 {
            opq = false;
        }
    }
    if !multi {
        single_block(dst, first);
        return;
    }
    load(px, &mut b.aos);
    b.berr = INF;
    one(b, 6);
    if b.lvl != BCQ_FAST {
        moments(b);
        if opq {
            // opaque: 2-subset RGB modes 1 and 3
            est(b, 2);
            parts(b, 1);
            parts(b, 3);
            if b.lvl == BCQ_SLOW {
                // slow: 3-subset modes 0 and 2, modes 4 / 5 (all rotations), 7
                est(b, 3);
                parts(b, 0);
                parts(b, 2);
                all45(b);
                est(b, 2);
                parts(b, 7);
            }
        } else {
            if b.lvl != BCQ_SLOW {
                // normal: modes 5 and 4 with rotation 0
                try45(b, 5, 0);
                try45(b, 4, 0);
                try45(b, 4, 4); // isb 1
            } else {
                all45(b);
            }
            est(b, 2);
            parts(b, 7);
        }
    }
    dst[..16].copy_from_slice(&b.best);
}

// a one-colour block: mode 5, colour index 1 with table endpoints, alpha endpoints = alpha
// (index 0)  [b7_single_block]
fn single_block(dst: &mut [u8], bgra: u32) {
    // bits: mode 5 (6 bits 100000), rotation 0 (2), R0 R1 G0 G1 B0 B1 (7 each), A0 A1 (8
    // each), colour indices (31 bits: pixel 0 1 bit, others 2), alpha indices (31)
    let mut lo: u64 = 1 << 5;
    let mut pos = 8;
    for ch in (0..3).rev() {
        let v = (bgra >> (8 * ch) & 255) as usize;
        lo |= (B7_SINGLE[2 * v] as u64) << pos;
        pos += 7;
        lo |= (B7_SINGLE[2 * v + 1] as u64) << pos;
        pos += 7;
    }
    // alpha endpoints: bits 50..57, 58..65
    let a = (bgra >> 24) as u64;
    lo |= a << 50;
    lo |= a << 58;
    let mut hi = a >> 6;
    // colour indices: bits 66..96: pixel 0 (1 bit) = 1, then 15 x 2 bits = 01
    let ci: u64 = ((0x5555_5555u64 << 1 | 1) & 0x7FFF_FFFF) << 2;
    hi |= ci;
    dst[0..8].copy_from_slice(&lo.to_le_bytes());
    dst[8..16].copy_from_slice(&hi.to_le_bytes());
}

// ------------------------------------------------------------------ modes

// a 1-subset mode (6) on the whole block  [b7_one]
fn one(b: &mut B7, mode: u32) {
    b.pxs[0].gather(&b.aos, 0xFFFF, 0);
    let mq = mq_of(b, mode);
    fit(&mut b.w, &b.pxs[0], mq, &mut b.fr[0]);
    if !(b.fr[0].err >= b.berr) {
        b.berr = b.fr[0].err;
        pack(b, mode, 0, 0);
    }
}

// fit config (colour / combined part) for the level  [b7_mq]
fn mq_of(b: &B7, mode: u32) -> u32 {
    let md = &MODES[(mode & 7) as usize];
    let mut q = md[MD_IB] as u32 | (md[MD_EC] as u32) << 4 | (md[MD_EA] as u32) << 8 | (md[MD_PM] as u32) << 12;
    q |= match b.lvl {
        BCQ_SLOW => MQ_ALLP | 3 << 16,
        BCQ_FAST => 1 << 16,
        _ => 2 << 16,
    };
    q
}

// modes 5 and 4, every rotation and index selection  [b7_all45]
fn all45(b: &mut B7) {
    for r in 0..4 {
        try45(b, 5, r);
        try45(b, 4, r);
        try45(b, 4, r + 4);
    }
}

// index bits of modes 4 / 5: (colour, alpha); mode 5 both 2; mode 4 colour 2 / alpha 3,
// with index selection colour 3 / alpha 2
#[inline]
fn ib45(mode: u32, r: u32) -> (u32, u32) {
    if mode != 4 {
        (2, 2)
    } else if r & 4 != 0 {
        (3, 2)
    } else {
        (2, 3)
    }
}

// mode 4 / 5: colour fit (3 channels after the rotation) + scalar fit (the channel in the
// alpha slot), separate index sets; r = rotation | isb << 2  [b7_try45]
fn try45(b: &mut B7, mode: u32, r: u32) {
    let rot = r & 3;
    let (cib, aib) = ib45(mode, r);
    // colour: alpha channel zeroed after the rotation
    b.pxs[0].gather(&b.aos, 0xFFFF, rot + (8 << 4));
    let mq = mq_of(b, mode) & !0xF0F | cib;
    fit(&mut b.w, &b.pxs[0], mq, &mut b.fr[0]);
    if b.fr[0].err >= b.berr {
        return;
    }
    // scalar: the alpha slot only
    b.pxs[3].gather(&b.aos, 0xFFFF, rot + (7 << 4));
    let mq = mq_of(b, mode) & !0x0FF | aib;
    fit(&mut b.w, &b.pxs[3], mq, &mut b.fr[3]);
    let t = b.fr[0].err + b.fr[3].err;
    if t >= b.berr {
        return;
    }
    b.berr = t;
    pack(b, mode, 0, r);
}

// a partitioned mode: the best candidates of est's terms fitted exactly (est must have run
// for the mode's subset count)  [b7_parts]
fn parts(b: &mut B7, mode: u32) {
    let md = &MODES[(mode & 7) as usize];
    let qi = (md[MD_IB] as u32).wrapping_sub(2).min(2) as usize;
    let k = if b.lvl == BCQ_SLOW { 16 } else { 4 };
    top(b, f32::from_bits(QLEV[qi]), 1 << md[MD_PB], k);
    let mq = mq_of(b, mode);
    let ns = md[MD_NS] as u32;
    let perm = if md[MD_AB] != 0 { 0 } else { 8 << 4 }; // no alpha: zero channel 3
    'cand: for ci in 0..b.ncand as usize {
        let part = b.cand[ci & 15].1 & 63;
        let mut sum = 0f32;
        for s in 0..ns as usize {
            let m = submask(part, s as u32, ns);
            b.pxs[s].gather(&b.aos, m, perm);
            fit(&mut b.w, &b.pxs[s], mq, &mut b.fr[s]);
            sum += b.fr[s].err;
            if sum >= b.berr {
                continue 'cand; // already worse
            }
        }
        if sum >= b.berr {
            continue;
        }
        b.berr = sum;
        pack(b, mode, part, 0);
    }
}

/// pixels of the subset (16-bit mask)  [b7_submask]
pub(crate) fn submask(part: u32, s: u32, ns: u32) -> u32 {
    let p = (part & 63) as usize;
    if ns != 3 {
        let m = B7_M2[p] as u32;
        return if s != 0 { m } else { m ^ 0xFFFF };
    }
    let (m1, m2) = (B7_M3[2 * p] as u32, B7_M3[2 * p + 1] as u32);
    match s {
        1 => m1,
        2 => m2,
        _ => (m1 | m2) ^ 0xFFFF,
    }
}

// ------------------------------------------------------------------ fitting

// best endpoints / indices for the pixel set  [b7_fit]
fn fit(w: &mut Work, p: &Pxs, mq: u32, fr: &mut Fr) {
    w.e = pca(p);
    fr.err = INF;
    let iters = mq >> 16 & 15;
    let pm = mq >> 12 & 3;
    let lw: &[f32] = match mq & 15 {
        0..=2 => &LW2,
        3 => &LW3,
        _ => &LW4,
    };
    let mut it = 0;
    loop {
        if mq & MQ_ALLP != 0 && pm != 0 {
            let n = if pm == 1 { 4 } else { 2 }; // unique: 4 combinations, shared: 2
            for f in (0..n).rev() {
                try_q(w, p, mq, fr, f);
            }
        } else {
            try_q(w, p, mq, fr, -1);
        }
        if it >= iters {
            break;
        }
        let idx = fr.idx;
        if !lsq(p, &idx, lw, &mut w.e) {
            break;
        }
        it += 1;
    }
}

// quantize e (force: p-bit combination or -1 = choose), palette, nearest entries; keep in
// fr when better  [b7_try]
fn try_q(w: &mut Work, p: &Pxs, mq: u32, fr: &mut Fr, force: i32) {
    quant(w, mq, force);
    let ib = mq & 15;
    pal(w, ib);
    let n = 1usize << ib;
    let err = assign(p, &w.palb, n, &mut w.idx);
    if !(err >= fr.err) {
        fr.err = err;
        fr.c0 = w.qc[0];
        fr.c1 = w.qc[1];
        fr.idx = w.idx;
    }
}

// e -> codes qc and expanded values qe; force = -1: choose the p-bits, else unique: p0 =
// bit 0, p1 = bit 1, shared: p = bit 0  [b7_quant]
fn quant(w: &mut Work, mq: u32, force: i32) {
    let s = Sse2;
    let (z, c255) = (s.zero(), s.splatf(255.0));
    for k in 0..2 {
        let x = s.cvtr_f32_i32(s.minf(s.maxf(s.load_f32(&w.e[k]), z), c255));
        s.store_i32(&mut w.qx[k], x);
    }
    let pm = mq >> 12 & 3;
    if pm == 0 {
        qend(w, 0, mq, 0);
        qend(w, 1, mq, 0);
    } else if force == -1 {
        if pm == 1 {
            for k in 0..2 {
                let e0 = qend(w, k, mq, 1);
                let e1 = qend(w, k, mq, 2);
                if e1 >= e0 {
                    qend(w, k, mq, 1);
                }
            }
        } else {
            // shared: p minimizing both endpoints' error
            let a = qend(w, 0, mq, 1);
            let s0 = qend(w, 1, mq, 1) + a;
            let b = qend(w, 0, mq, 2);
            let s1 = qend(w, 1, mq, 2) + b;
            if s1 >= s0 {
                qend(w, 0, mq, 1);
                qend(w, 1, mq, 1);
            }
        }
    } else {
        qend(w, 0, mq, (force & 1) as u32 + 1);
        let f1 = if pm == 1 { force >> 1 } else { force };
        qend(w, 1, mq, (f1 & 1) as u32 + 1);
    }
}

// endpoint k of qx through the code table tab (0 any, 1 p = 0, 2 p = 1) of each channel's
// effective bits (0 bits: code 0, value 0); codes -> qc, values -> qe; squared error against
// the float endpoint  [b7_qend]
fn qend(w: &mut Work, k: usize, mq: u32, tab: u32) -> f32 {
    let mut err = 0f32;
    for c in 0..4 {
        let e = if c < 3 { mq >> 4 & 15 } else { mq >> 8 & 15 };
        if !(5..=8).contains(&e) {
            w.qc[k][c] = 0;
            w.qe[k][c] = 0;
            continue;
        }
        let t = (((e - 5) * 3 + tab) & 15).min(11) as usize;
        let code = B7_QT[t * 256 + (w.qx[k][c] & 255) as usize] as u32;
        w.qc[k][c] = code as u8;
        let v = ((code << (8 - e)) | (code >> (2 * e - 8))) & 255;
        w.qe[k][c] = v as i32;
        let d = v as i32 as f32 - w.e[k][c];
        err += d * d;
    }
    err
}

// weight pairs: w_k in i16 lanes 0..3, w_k+1 in lanes 4..7
const fn wpairs<const N: usize>(w: [u8; N]) -> [[i16; 8]; 8] {
    let mut t = [[0i16; 8]; 8];
    let mut k = 0;
    while k < N / 2 {
        let mut l = 0;
        while l < 8 {
            t[k][l] = w[2 * k + l / 4] as i16;
            l += 1;
        }
        k += 1;
    }
    t
}
static WP2: [[i16; 8]; 8] = wpairs(W2);
static WP3: [[i16; 8]; 8] = wpairs(W3);
static WP4: [[i16; 8]; 8] = wpairs(W4);

// qe endpoints -> 2^ib palette entries (e0 (64 - w) + e1 w + 32) >> 6 per channel, as
// broadcast floats for assign (exact: the assembly computes the same values in single
// precision)  [b7_pal + bce_bcast]
fn pal(w: &mut Work, ib: u32) {
    let s = Sse2;
    let z = s.zero();
    let x = s.packs32(s.load_i32(&w.qe[0]), s.load_i32(&w.qe[1])); // e0 lanes 0..3, e1 4..7
    let a = s.shuf32::<0x44>(x); // e0 | e0
    let d = s.sub16(s.shuf32::<0xEE>(x), a); // e1 - e0
    let base = s.add16(s.sll16::<6>(a), s.splat16(32)); // 64 e0 + 32
    let wp = match ib {
        0..=2 => &WP2,
        3 => &WP3,
        _ => &WP4,
    };
    for k in 0..(1usize << ((ib & 7) - 1)).min(8) {
        let t = s.sra16::<6>(s.add16(s.mullo16(s.load_i16(&wp[k]), d), base));
        bput(&mut w.palb[(2 * k) & 15], s.cvt_i32_f32(s.unpacklo16(t, z)));
        bput(&mut w.palb[(2 * k + 1) & 15], s.cvt_i32_f32(s.unpackhi16(t, z)));
    }
}

// ------------------------------------------------------------------ partition estimates

/// per pixel x, x * r, x * g, x * b, x * a and the sums of all 16  [b7_moments]
pub(crate) fn moments(b: &mut B7) {
    let s = Sse2;
    let mut tot = [s.zero(); 5];
    for p in 0..16 {
        let x = s.load_f32(&b.aos[p]);
        let m = [
            x,
            s.mulf(s.shuf32::<0x00>(x), x),
            s.mulf(s.shuf32::<0x55>(x), x),
            s.mulf(s.shuf32::<0xAA>(x), x),
            s.mulf(s.shuf32::<0xFF>(x), x),
        ];
        for k in 0..5 {
            s.store_f32(&mut b.mom[p][k], m[k]);
            tot[k] = s.addf(tot[k], m[k]);
        }
    }
    for k in 0..5 {
        s.store_f32(&mut b.tot[k], tot[k]);
    }
}

/// per partition (64) the residual and lambda terms of the 2- or 3-subset partitions
/// [b7_est]
pub(crate) fn est(b: &mut B7, ns: u32) {
    let s = Sse2;
    for part in 0..64u32 {
        b.res[part as usize] = 0.0;
        b.lam[part as usize] = 0.0;
        let mut ss = [[s.zero(); 5]; 3];
        let mut cnt = [0u32; 3];
        // subset sums: 1 (and 2) by iteration, 0 = total - the others
        for sb in 1..(ns as usize).min(3) {
            let mut m = submask(part, sb as u32, ns);
            let mut acc = [s.zero(); 5];
            let mut n = 0;
            while m != 0 {
                let p = m.trailing_zeros() as usize & 15;
                m &= m - 1;
                for k in 0..5 {
                    acc[k] = s.addf(acc[k], s.load_f32(&b.mom[p][k]));
                }
                n += 1;
            }
            ss[sb] = acc;
            cnt[sb] = n;
            subest(b, &acc, n, part);
        }
        let mut s0 = [s.zero(); 5];
        for k in 0..5 {
            s0[k] = s.load_f32(&b.tot[k]);
        }
        let mut n0 = 16;
        for sb in 1..(ns as usize).min(3) {
            for k in 0..5 {
                s0[k] = s.subf(s0[k], ss[sb][k]);
            }
            n0 -= cnt[sb];
        }
        subest(b, &s0, n0, part);
    }
}

// add n (trace - lambda) to res[part] and n lambda to lam[part] for the subset with moment
// sums and n pixels  [b7_subest]
fn subest(b: &mut B7, sums: &[V4; 5], n: u32, part: u32) {
    let s = Sse2;
    if n < 2 {
        return; // 0 / 1 pixel: no error
    }
    let part = (part & 63) as usize;
    let nf = n as f32;
    let inv = s.splatf(1.0 / nf);
    let mean = s.mulf(sums[0], inv);
    let col = [
        s.subf(s.mulf(sums[1], inv), s.mulf(s.shuf32::<0x00>(mean), mean)),
        s.subf(s.mulf(sums[2], inv), s.mulf(s.shuf32::<0x55>(mean), mean)),
        s.subf(s.mulf(sums[3], inv), s.mulf(s.shuf32::<0xAA>(mean), mean)),
        s.subf(s.mulf(sums[4], inv), s.mulf(s.shuf32::<0xFF>(mean), mean)),
    ];
    // trace and the column with the largest diagonal element
    let diag = [lane::<0x00>(col[0]), lane::<0x55>(col[1]), lane::<0xAA>(col[2]), lane::<0xFF>(col[3])];
    let mut dmax = diag[0];
    let mut trace = dmax;
    let mut ci = 0;
    for (k, &d) in diag.iter().enumerate().skip(1) {
        trace += d;
        if d > dmax {
            dmax = d;
            ci = k;
        }
    }
    if !(dmax > TINY) {
        return; // no variance
    }
    let mut v = col[ci];
    let mut w;
    let mut it = 4;
    loop {
        w = matv(&col, v);
        it -= 1;
        if it == 0 {
            break;
        }
        let m = vmax(s.and(w, s.splat32(0x7FFF_FFFF)));
        if !(lane::<0>(m) > TINY) {
            return;
        }
        v = s.divf(w, m);
    }
    // lambda = (C v . v) / (v . v)
    let pv = lane::<0>(vsum(s.mulf(w, v)));
    let vv = lane::<0>(vsum(s.mulf(v, v)));
    if !(vv > TINY) {
        return;
    }
    let lam = minss(pv / vv, trace); // <= trace
    let tr = trace - lam;
    b.res[part] = tr * nf + b.res[part];
    b.lam[part] = lam * nf + b.lam[part];
}

/// the k partitions (of the first `parts`) with the smallest res + q lam, ascending, into
/// cand / ncand  [b7_top]
pub(crate) fn top(b: &mut B7, q: f32, parts: u32, k: u32) {
    let k = (k & 31).min(16) as usize;
    let mut nc = 0usize;
    for p in 0..parts.min(64) as usize {
        let score = b.lam[p] * q + b.res[p];
        // insertion position: after every candidate with score <= this one
        let mut pos = nc;
        while pos > 0 && !(score >= b.cand[(pos - 1) & 15].0) {
            pos -= 1;
        }
        if pos >= k {
            continue; // beyond the k best
        }
        let mut j = if nc < k { nc } else { k - 1 };
        while j > pos {
            b.cand[j & 15] = b.cand[(j - 1) & 15];
            j -= 1;
        }
        b.cand[pos & 15] = (score, p as u32);
        if nc < k {
            nc += 1;
        }
    }
    b.ncand = nc as u32;
}

// ------------------------------------------------------------------ packing

/// the 128-bit bit writer {lo, hi, pos}
pub(crate) struct Bw {
    lo: u64,
    hi: u64,
    pos: u32,
}

impl Bw {
    pub fn new() -> Bw {
        Bw { lo: 0, hi: 0, pos: 0 }
    }

    /// append the low n (1..16) bits of v  [b7_put]
    #[inline]
    pub fn put(&mut self, v: u32, n: u32) {
        let v = (v & ((1u32 << n) - 1)) as u64;
        let p = self.pos & 127;
        if p < 64 {
            self.lo |= v << p;
            if p + n > 64 {
                self.hi |= v >> (64 - p);
            }
        } else {
            self.hi |= v << (p - 64);
        }
        self.pos += n;
    }

    pub fn bytes(&self) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[..8].copy_from_slice(&self.lo.to_le_bytes());
        o[8..].copy_from_slice(&self.hi.to_le_bytes());
        o
    }
}

// the fit results (subsets / colour + scalar) of the mode as a block into best (anchor
// fix-up included)  [b7_pack]
fn pack(b: &mut B7, mode: u32, part: u32, r: u32) {
    let mode = mode & 7;
    let part = (part & 63) as usize;
    let md = MODES[mode as usize];
    let mut bw = Bw::new();
    if mode == 4 || mode == 5 {
        // colour fr[0] (ib c), alpha fr[3] (ib a); anchor = pixel 0 for both
        let (cib, aib) = ib45(mode, r);
        fix0(&mut b.fr[0], cib);
        fix0(&mut b.fr[3], aib);
        bw.put(1 << mode, mode + 1);
        bw.put(r & 3, 2);
        if mode == 4 {
            bw.put(r >> 2 & 1, 1);
        }
        let cb = md[MD_CB] as u32;
        for ch in 0..3 {
            bw.put(b.fr[0].c0[ch] as u32, cb);
            bw.put(b.fr[0].c1[ch] as u32, cb);
        }
        let ab = md[MD_AB] as u32;
        bw.put(b.fr[3].c0[3] as u32, ab);
        bw.put(b.fr[3].c1[3] as u32, ab);
        // index set 1: the 2-bit set (mode 4: colour unless isb; mode 5: colour), then set 2
        let (s1, s2) = if cib == 2 { (b.fr[0].idx, b.fr[3].idx) } else { (b.fr[3].idx, b.fr[0].idx) };
        idxset(&mut bw, &s1, 2);
        idxset(&mut bw, &s2, md[MD_IB2] as u32);
        b.best = bw.bytes();
        return;
    }
    // anchors
    b.anc = [0, 255, 255, 255];
    let ns = md[MD_NS] as usize;
    if ns == 2 {
        b.anc[1] = ANC2[part];
    } else if ns == 3 {
        b.anc[1] = ANC3A[part];
        b.anc[2] = ANC3B[part];
    }
    // per subset: anchor index MSB set -> swap endpoints, mirror indices; scatter
    let ib = md[MD_IB] as u32;
    let maxi = ((1u32 << ib) - 1) as u8;
    for s in 0..ns {
        let p = &b.pxs[s];
        let fr = &mut b.fr[s];
        let n = ((p.n & 31) as usize).min(16);
        if let Some(l) = (0..n).find(|&l| p.pos[l] == b.anc[s])
            && fr.idx[l] > maxi >> 1
        {
            core::mem::swap(&mut fr.c0, &mut fr.c1);
            for x in fr.idx.iter_mut() {
                *x = maxi.wrapping_sub(*x) & maxi;
            }
        }
        for l in 0..n {
            b.pix[(p.pos[l] & 15) as usize] = fr.idx[l];
        }
    }
    bw.put(1 << mode, mode + 1);
    let pb = md[MD_PB] as u32;
    if pb != 0 {
        bw.put(part as u32, pb);
    }
    // endpoints: channel-major, subsets, endpoints
    let pm = md[MD_PM];
    for ch in 0..4 {
        let bits = if ch < 3 { md[MD_CB] } else { md[MD_AB] } as u32;
        if ch == 3 && bits == 0 {
            break;
        }
        for s in 0..ns {
            let fr = &b.fr[s];
            let (mut c0, mut c1) = (fr.c0[ch] as u32, fr.c1[ch] as u32);
            if pm != 0 {
                c0 >>= 1;
                c1 >>= 1;
            }
            bw.put(c0, bits);
            bw.put(c1, bits);
        }
    }
    // p-bits
    if pm != 0 {
        for s in 0..ns {
            bw.put(b.fr[s].c0[0] as u32 & 1, 1);
            if pm == 1 {
                bw.put(b.fr[s].c1[0] as u32 & 1, 1);
            }
        }
    }
    for i in 0..16u8 {
        let n = if b.anc[..3].contains(&i) { ib - 1 } else { ib };
        bw.put(b.pix[i as usize] as u32, n);
    }
    b.best = bw.bytes();
}

// 16 indices, `bits` bits each (pixel 0: bits - 1)  [IDXSET]
fn idxset(bw: &mut Bw, idx: &[u8; 16], bits: u32) {
    for (i, &x) in idx.iter().enumerate() {
        bw.put(x as u32, if i == 0 { bits - 1 } else { bits });
    }
}

// modes 4 / 5: pixel 0's index MSB set -> swap the endpoints, mirror the indices (lane i =
// pixel i)  [b7_fix0]
fn fix0(fr: &mut Fr, ib: u32) {
    let maxi = ((1u32 << ib) - 1) as u8;
    if fr.idx[0] > maxi >> 1 {
        core::mem::swap(&mut fr.c0, &mut fr.c1);
        for x in fr.idx.iter_mut() {
            *x = maxi.wrapping_sub(*x) & maxi;
        }
    }
}
