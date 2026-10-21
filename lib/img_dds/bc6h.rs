// bc6h -- BC6H block encoder                                          [src/img/bc6h_enc.s]
//
//   enc(scr, px, dst, cfg)      16 straight BGRA pixels -> 16 bytes at dst; cfg bit 31 =
//                               SF16 (bc6hs_enc), else UF16 (bc6h_enc)
//
// The 8-bit channels become halves h = half(v / 255) (round to nearest even, table H6_HALF)
// and are fitted in the decoder's interpolation domain X = h * 64 / 31 (UF16) or h * 32 / 31
// (SF16), where finish_unquantize maps interpolated values linearly to half bits.  Every one
// of the 14 modes can be written (header bits through BC6_LAYOUT / BC6_MODES): the 1-region
// modes 11..14 at every level, the 2-region modes 1..10 from normal on for the best
// partition candidates of bc7's moment estimate (normal 2, slow 8).  Per region: principal
// axis endpoints, per mode quantization to the endpoint precision (nearest unquantized value
// among 3 codes), exact palette (unquantize, interpolation, finish_unquantize, then the byte
// the decoder produces), nearest entry per pixel in bytes, least-squares endpoints for the
// chosen weights (fast 1, normal 2, slow 3 passes).  Anchor indices are fixed by swapping a
// region's endpoints and mirroring its indices; transformed modes are used only when every
// delta fits (with the decoder's wrap-around).  The block with the smallest byte error wins.

use rt::fx::{cvtt, maxss, minss};

use crate::bc7::{self, Bw};
use crate::dec::{bc6_unq, finish, h2u8, W3, W4};
use simd::{Simd, Sse2};

use crate::enc::{assign, fbits, lsq, pca, PalB, Pxs, Scratch, INF, PALB0};
use crate::tables::{ANC2, BC6_LAYOUT, BC6_MODES, H6_HALF};
use crate::{BCQ_FAST, BCQ_LEVEL, BCQ_NORMAL, BCQ_SLOW};

const K64: u32 = 0x4004_2108; // 64 / 31
const K32: u32 = 0x3F84_2108; // 32 / 31
const Q49: u32 = 0x3CA7_2F05; // 1 / 49
const C32767: u32 = 0x46FF_FE00;
const C32768: u32 = 0x4700_0000;
const C65535: u32 = 0x477F_FF00;
const C65536: u32 = 0x4780_0000;
// least-squares weights w / 64: 3-bit, 4-bit indices
static LW3: [f32; 8] = fbits([0x0000_0000, 0x3E10_0000, 0x3E90_0000, 0x3ED8_0000, 0x3F14_0000, 0x3F38_0000, 0x3F5C_0000, 0x3F80_0000]);
static LW4: [f32; 16] = fbits([
    0x0000_0000, 0x3D80_0000, 0x3E10_0000, 0x3E50_0000, 0x3E88_0000, 0x3EA8_0000, 0x3ED0_0000, 0x3EF0_0000,
    0x3F08_0000, 0x3F18_0000, 0x3F2C_0000, 0x3F3C_0000, 0x3F4C_0000, 0x3F5C_0000, 0x3F70_0000, 0x3F80_0000,
]);
// mode bits per mode index (indices 0 / 1: 2 bits, others 5 bits)
static MVAL: [u8; 14] = [0x00, 0x01, 0x02, 0x06, 0x0A, 0x0E, 0x12, 0x16, 0x1A, 0x1E, 0x03, 0x07, 0x0B, 0x0F];

/// the BC6H scratch (H6_*, after the part of the B7 scratch it shares)
#[derive(Clone, Copy)]
pub(crate) struct H6 {
    y: [[f32; 4]; 16], // pixels as bytes (R, G, B, 0)
    px: [Pxs; 2],      // X domain
    py: [Pxs; 2],      // byte domain
    palb: PalB,
    pe: [[[f32; 4]; 2]; 2], // principal axis endpoints per region
    e: [[[f32; 4]; 2]; 2],  // fit endpoints
    q: [[[i32; 4]; 2]; 2],  // quantized endpoints
    bq: [[[i32; 4]; 2]; 2], // best
    idx: [[u8; 16]; 2],     // lane indices per region
    bidx: [u8; 16],         // best per-pixel indices
    berr: f32,
    bmode: u32, // mode index 0..13
    bpart: u32,
    sgn: bool,
    lvl: u32,
    part: u32, // current partition
}

impl H6 {
    pub const ZERO: H6 = H6 {
        y: [[0.0; 4]; 16],
        px: [Pxs::ZERO; 2],
        py: [Pxs::ZERO; 2],
        palb: PALB0,
        pe: [[[0.0; 4]; 2]; 2],
        e: [[[0.0; 4]; 2]; 2],
        q: [[[0; 4]; 2]; 2],
        bq: [[[0; 4]; 2]; 2],
        idx: [[0; 16]; 2],
        bidx: [0; 16],
        berr: 0.0,
        bmode: 0,
        bpart: 0,
        sgn: false,
        lvl: 0,
        part: 0,
    };
}

/// one BC6H block (cfg bit 31: SF16)  [bc6h_enc / bc6hs_enc]
pub(crate) fn enc(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], cfg: u32) {
    let b7 = &mut s.b7;
    let h = &mut s.h6;
    h.sgn = cfg >> 31 != 0;
    h.lvl = cfg & BCQ_LEVEL;
    if h.lvl == 0 {
        h.lvl = BCQ_NORMAL;
    }
    // pixels: X = half(v / 255) * k, Y = v (R, G, B, 0)
    let k = f32::from_bits(if h.sgn { K32 } else { K64 });
    for p in 0..16 {
        for c in 0..3 {
            let v = px[p * 4 + 2 - c];
            h.y[p][c] = v as f32;
            b7.aos[p][c] = H6_HALF[v as usize] as f32 * k;
        }
        h.y[p][3] = 0.0;
        b7.aos[p][3] = 0.0;
    }
    h.berr = INF;
    h.bmode = 10;
    h.bpart = 0;
    // one region: modes 11..14
    h.part = 0;
    region(&b7.aos, h, 0, 0xFFFF);
    for m in 10..14 {
        try_mode(h, m);
    }
    if h.lvl != BCQ_FAST && h.berr.to_bits() != 0 {
        // two regions: partition candidates
        bc7::moments(b7);
        bc7::est(b7, 2);
        bc7::top(b7, f32::from_bits(Q49), 32, if h.lvl == BCQ_SLOW { 8 } else { 2 });
        for ci in 0..b7.ncand as usize {
            let part = b7.cand[ci & 15].1 & 31;
            h.part = part;
            for r in 0..2 {
                region(&b7.aos, h, r, bc7::submask(part, r, 2));
            }
            for m in 0..10 {
                try_mode(h, m);
            }
        }
    }
    pack(h, dst);
}

// region r's pixel sets (X and byte domain) and principal axis  [h6_region]
fn region(aos: &[[f32; 4]; 16], h: &mut H6, r: u32, mask: u32) {
    let r = (r & 1) as usize;
    h.px[r].gather(aos, mask, 0);
    h.py[r].gather(&h.y, mask, 0);
    h.pe[r] = pca(&h.px[r]);
}

// mode index 0..13 on the current region(s): quantize / palette / nearest bytes / least
// squares; keeps the best block data  [h6_try_mode]
fn try_mode(h: &mut H6, mode: u32) {
    if mode > 13 {
        return;
    }
    let desc = &BC6_MODES[mode as usize];
    let (ns, ib) = if mode < 10 { (2usize, 3u32) } else { (1, 4) };
    // start from the principal axis endpoints
    h.e = h.pe;
    let mut it = 0;
    loop {
        quant(h, mode, ns);
        let mut tot = 0f32;
        for r in 0..ns {
            pal(h, r, ib, mode);
            let n = 1usize << ib;
            let err = assign(&h.py[r], &h.palb, n, &mut h.idx[r]);
            tot = err + tot;
            anchor(h, r, ib);
        }
        // transformed modes: every delta must fit
        let valid = desc[0] == 0 || deltas_ok(h, desc, ns);
        if valid && !(tot >= h.berr) {
            h.berr = tot;
            h.bmode = mode;
            h.bpart = h.part;
            h.bq = h.q;
            // per-pixel indices from the lanes of each region
            for r in 0..ns {
                let p = &h.py[r];
                for l in 0..(p.n as usize).min(16) {
                    h.bidx[(p.pos[l] & 15) as usize] = h.idx[r][l];
                }
            }
        }
        // least squares for the next pass
        if it >= h.lvl {
            break;
        }
        let lw: &[f32] = if ib == 3 { &LW3 } else { &LW4 };
        for r in 0..ns {
            let idx = h.idx[r];
            lsq(&h.px[r], &idx, lw, &mut h.e[r]);
        }
        it += 1;
    }
}

// region r: its anchor pixel's index MSB set -> swap the quantized endpoints, mirror the
// indices  [h6_anchor]
fn anchor(h: &mut H6, r: usize, ib: u32) {
    let maxi = ((1u32 << ib) - 1) as u8;
    let a = if r == 0 { 0 } else { ANC2[(h.part & 63) as usize] };
    let p = &h.py[r];
    let n = (p.n as usize).min(16);
    if let Some(l) = (0..n).find(|&l| p.pos[l] == a)
        && h.idx[r][l] > maxi >> 1
    {
        h.q[r].swap(0, 1);
        for x in h.idx[r].iter_mut() {
            *x = maxi.wrapping_sub(*x) & maxi;
        }
    }
}

// every endpoint other than the base (q[0][0]) differs from it by an amount that fits the
// mode's delta bits (modulo 2^epb)  [h6_deltas_ok]
fn deltas_ok(h: &H6, desc: &[u16; 7], ns: usize) -> bool {
    let epb = desc[1] as u32;
    let m = ((1u32 << epb) - 1) as i32;
    let half = (m >> 1) + 1;
    for c in 0..3 {
        let db = desc[2 + c] as u32;
        let hi = (1i32 << (db - 1)) - 1;
        let lo = -(1i32 << (db - 1));
        let base = h.q[0][0][c];
        for slot in 1..4 {
            if slot >= 2 && ns < 2 {
                break; // one region: slot 1 only
            }
            let v = h.q[slot >> 1][slot & 1][c];
            let d = (v.wrapping_sub(base).wrapping_add(half) & m).wrapping_sub(half);
            if d < lo || d > hi {
                return false;
            }
        }
    }
    true
}

// e (X domain) -> q codes of the mode's endpoint bits: of the codes around the linear
// estimate the one whose unquantized value is nearest  [h6_quant]
fn quant(h: &mut H6, mode: u32, ns: usize) {
    let epb = BC6_MODES[mode.min(13) as usize][1] as u32;
    let (bits, umax, d) = if h.sgn {
        (epb - 1, f32::from_bits(C32767), f32::from_bits(C32768))
    } else {
        (epb, f32::from_bits(C65535), f32::from_bits(C65536))
    };
    let hc = (1i32 << bits) - 1; // highest code
    let step = (hc as f32 + 1.0) / d;
    for i in 0..(ns * 8).min(16) {
        let (r, k, c) = (i >> 3, (i >> 2) & 1, i & 3);
        if c == 3 {
            h.q[r][k][c] = 0; // alpha slot
            continue;
        }
        // target U, clamped; code range
        let u = minss(maxss(h.e[r][k][c], 0.0), umax);
        let est = cvtt(step * u);
        let ui = cvtt(u); // the integer part is enough for the comparison
        let (mut bd, mut best) = (0x7FFF_FFFFu32, 0i32);
        for j in 0..3 {
            let cand = est.wrapping_sub(1).wrapping_add(j);
            if cand < 0 || cand > hc {
                continue;
            }
            let v = bc6_unq(cand, epb, h.sgn);
            let dist = v.wrapping_sub(ui).wrapping_abs() as u32;
            if dist < bd {
                bd = dist;
                best = cand;
            }
        }
        h.q[r][k][c] = best;
    }
}

// region r's palette as the bytes the decoder produces: 2^ib entries i32 (R, G, B, 0)
// [h6_pal]
fn pal(h: &mut H6, r: usize, ib: u32, mode: u32) {
    let sv = Sse2;
    let epb = BC6_MODES[mode.min(13) as usize][1] as u32;
    let wt: &[u8] = if ib == 3 { &W3 } else { &W4 };
    let n = 1usize << ib;
    for c in 0..3 {
        let a = bc6_unq(h.q[r][0][c], epb, h.sgn);
        let b = bc6_unq(h.q[r][1][c], epb, h.sgn);
        for (e, &w) in wt[..n].iter().enumerate() {
            let w = w as i32;
            let v = ((64 - w).wrapping_mul(a).wrapping_add(w.wrapping_mul(b)).wrapping_add(32)) >> 6;
            let x = h2u8(finish(v, h.sgn) as u32);
            // the byte, broadcast as a float for assign (bce_bcast of the integer palette)
            sv.store_f32(&mut h.palb[e & 15][c], sv.splatf(x as i32 as f32));
        }
    }
    for e in h.palb[..n].iter_mut() {
        e[3] = [0.0; 4]; // alpha slot 0
    }
}

// the best mode / partition / endpoints / indices as a BC6H block  [h6_pack]
fn pack(h: &H6, dst: &mut [u8]) {
    let mode = if h.bmode > 13 { 10 } else { h.bmode } as usize;
    let desc = &BC6_MODES[mode];
    // fields rw rx ry rz gw ... bz d
    let mut f = [0i32; 16];
    for c in 0..3 {
        let w = h.bq[0][0][c];
        let (mut x, mut y, mut z) = (h.bq[0][1][c], h.bq[1][0][c], h.bq[1][1][c]);
        if desc[0] != 0 {
            x = x.wrapping_sub(w);
            y = y.wrapping_sub(w);
            z = z.wrapping_sub(w);
        }
        f[4 * c] = w;
        f[4 * c + 1] = x;
        if mode < 10 {
            f[4 * c + 2] = y;
            f[4 * c + 3] = z;
        }
    }
    f[12] = (h.bpart & 31) as i32;
    // mode bits, header bits
    let mut bw = Bw::new();
    bw.put(MVAL[mode] as u32, if mode < 2 { 2 } else { 5 });
    let (hb, lay) = (desc[5] as usize, desc[6] as usize);
    for k in 0..hb {
        if lay + k > 1015 {
            break;
        }
        let x = BC6_LAYOUT[lay + k] as u32;
        let (bit, fi) = (x & 15, (x >> 4) & 15);
        bw.put((f[fi as usize] as u32) >> bit & 1, 1);
    }
    // indices: 2 regions 3 bits (anchors 0, ANC2[part]), 1 region 4 bits (anchor 0)
    let (ib, a2) = if mode < 10 { (3u32, ANC2[(h.bpart & 63) as usize] as usize) } else { (4, 255) };
    for i in 0..16 {
        let n = if i == 0 || i == a2 { ib - 1 } else { ib };
        bw.put(h.bidx[i] as u32, n);
    }
    dst[..16].copy_from_slice(&bw.bytes());
}
