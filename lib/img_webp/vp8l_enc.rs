// vp8l_enc -- WebP lossless (VP8L) encoder                            [src/img/webp_vp8l_enc.s]
//
//   encode(sf, opt) -> Ok(VP8L chunk payload) / Err(ERR_*)          vp8l_encode
//   enc_argb(argb, w, h, cfg) -> Ok(stream) / Err(ERR_*)            vp8l_enc_argb
//   enc_argb_ex(argb, w, h, cfg) -> Ok((result, bytes))             the same with the test
//                                  outputs of WEF_T_REPORT / WEF_T_DUMPM (codectest op 126)
//
// Pipeline (RFC 9649, modelled on libwebp 1.3 vp8l_enc.c): analysis (alpha, palette <= 256
// colours, libwebp AnalyzeEntropy estimate of direct / spatial / subtract-green / palette
// coding), transforms (colour indexing with pixel bundling, subtract green, predictor with
// the per-tile mode chosen by libwebp's accumulated-histogram cost, cross colour with
// libwebp's multiplier search), then the entropy-coded image: hash chains (2-pixel hash, run
// hash, window / iterations from the effort), greedy parse, cost-model shortest-path parse
// (effort >= 4), colour cache size chosen by entropy, meta prefix codes (tile histograms
// clustered by entropy bins + greedy merging, effort >= 4), length-limited Huffman codes (15
// / 7 bits, libwebp's count_min method), code-length RLE and trimming like libwebp.  Effort
// 7..9 tries more configurations and keeps the smallest stream.  No threads.  Every float
// step keeps the operation order of the assembly (SSE single precision, no FMA).

use alloc::vec;
use alloc::vec::Vec;

use gfx::Surface;
use img_core::*;

use crate::container::{cfg_from_opt, load_argb};
use crate::vp8l::{CODE_TO_PLANE, HASH_MUL, avg2};
use crate::{
    Cfg, WEF_NOEXACT, WEF_NOHDR, WEF_T_CACHE_SHIFT, WEF_T_CFG_SHIFT, WEF_T_DUMPM, WEF_T_KEEPDP, WEF_T_MODE_SHIFT,
    WEF_T_NOCACHE, WEF_T_NOCROSS, WEF_T_NODP, WEF_T_NOGREEDY, WEF_T_NOMETA, WEF_T_NORLE, WEF_T_REPORT,
};

// ---------------------------------------------------------------- constants
const HG_G: usize = 0; // histogram / code table: green + 24 lengths + cache
const HG_R: usize = 1304; // red 256
const HG_B: usize = 1560; // blue 256
const HG_A: usize = 1816; // alpha 256
const HG_D: usize = 2072; // distance 40
const HG_N: usize = 2112; // u32 entries per histogram
const NLEN: usize = 24;
const NDIST: usize = 40;
const CACHE_MAXB: u32 = 10;
const LZ_MAXLEN: usize = 4096;
const LZ_WINDOW: u32 = (1 << 20) - 120;
const HASH_BITS: u32 = 18;
const HASH_SIZE: usize = 1 << HASH_BITS;
const DP_LCAP: usize = 48; // shortest path: every length up to this, then the longest

// configurations
#[allow(dead_code)]
const LC_DIRECT: u32 = 0;
const LC_SPATIAL: u32 = 1; // predictor + cross colour
const LC_SUBGREEN: u32 = 2;
const LC_SPATIALSG: u32 = 3; // subtract green + predictor + cross colour
const LC_PALETTE: u32 = 4;

// float constants (bit patterns of the assembly)
const F099: u32 = 0x3F7D_70A4;
const F001: u32 = 0x3C23_D70A;
const F095: u32 = 0x3F73_3333;
const F07: u32 = 0x3F33_3333;
const F0627: u32 = 0x3F20_8312;
const F15625: u32 = 0x3FC8_0000;
const F0234: u32 = 0x3E70_0000;
const F2578: u32 = 0x4025_0000;
const F0703: u32 = 0x3F34_0000;
const F1796: u32 = 0x3FE6_0000;
const F3281: u32 = 0x4052_0000;
const F479: u32 = 0x423F_999A;
const F15: u32 = 0x4170_0000;
const F3: u32 = 0x4040_0000;
const FLOG14: u32 = 0x4073_ABB4;
const FLOG24: u32 = 0x4092_B803;
const F1: u32 = 0x3F80_0000;
const FBIG: u32 = 0x7149_F2CA;
const F016: u32 = 0x3E23_D70A;
const F05: u32 = 0x3F00_0000;
const F4M: u32 = 0x407F_FFEF; // 4 - 1e-6
const F2: u32 = 0x4000_0000;
const F2M24: u32 = 0x3380_0000; // 2^-24

#[inline(always)]
fn f(b: u32) -> f32 {
    f32::from_bits(b)
}

// PredictionCostSpatial coefficients (-0.1 * weight_0, -0.1 * exp * 0.6^(i-1))
static PS94: [u32; 16] = [
    0xBDCCCCCD, 0xBDC08312, 0xBD6703B0, 0xBD0A9BD0, 0xBCA65493, 0xBC4798B0, 0xBBEF8407, 0xBB8FB59E, 0xBB2C738A,
    0xBACEF10C, 0xBA785475, 0xBA14FF79, 0xB9B2CC2B, 0xB9568E9A, 0xB900BBF6, 0xB89A7B27,
];
static PS24: [u32; 16] = [
    0xBE99999A, 0xBE75C28F, 0xBE1374BC, 0xBDB0F27C, 0xBD54562E, 0xBCFECDD1, 0xBC98E1E4, 0xBC377578, 0xBBDC2690,
    0xBB841723, 0xBB1E822A, 0xBABE35CC, 0xBA64408E, 0xBA08F389, 0xB9A45771, 0xB94535BA,
];
static CL_ORDER: [u8; 19] = [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
// libwebp-like quality for effort 1..9
static QUAL: [u8; 9] = [0, 20, 30, 50, 75, 80, 90, 95, 100];
// red / blue analysis histograms of the modes (libwebp kHistoPairs)
const LA_A: usize = 0;
const LA_AP: usize = 1;
const LA_G: usize = 2;
const LA_GP: usize = 3;
const LA_R: usize = 4;
const LA_RP: usize = 5;
const LA_B: usize = 6;
const LA_BP: usize = 7;
const LA_RSG: usize = 8;
const LA_RPSG: usize = 9;
const LA_BSG: usize = 10;
const LA_BPSG: usize = 11;
const LA_PAL: usize = 12;
static RBPAIRS: [(usize, usize); 5] = [(LA_R, LA_B), (LA_RP, LA_BP), (LA_RSG, LA_BSG), (LA_RPSG, LA_BPSG), (LA_R, LA_B)];
static CC_DELTA: [u8; 8] = [16, 16, 8, 4, 2, 2, 2, 0];
static CC_AXIS: [(i32, i32); 8] = [(0, -1), (0, 1), (-1, 0), (1, 0), (-1, -1), (-1, 1), (1, -1), (1, 1)];
// alphabet offsets and base sizes: green (+ cache), red, blue, alpha, distance
static ALPHA_OFF: [(usize, usize); 5] = [(HG_G, 280), (HG_R, 256), (HG_B, 256), (HG_A, 256), (HG_D, NDIST)];

// ------------------------------------------------------------------ bit writer

/// LSB-first bit writer (BW_*)
#[derive(Default)]
struct Bw {
    buf: Vec<u8>,
    acc: u64,
    n: u32, // pending bits (< 32 between calls)
}

impl Bw {
    fn new(cap: usize) -> Bw {
        Bw { buf: Vec::with_capacity(cap.max(4096)), acc: 0, n: 0 }
    }

    // bw_put: append n (0..32) bits (bits < 2^n), LSB first
    #[inline(always)]
    fn put(&mut self, bits: u32, n: u32) {
        self.acc |= (bits as u64) << self.n;
        self.n += n;
        if self.n >= 32 {
            self.buf.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.n -= 32;
        }
    }

    // bw_finish: flush the pending bits (zero padded to bytes)
    fn finish(&mut self) {
        if self.n != 0 {
            let k = ((self.n + 7) >> 3) as usize;
            self.buf.extend_from_slice(&self.acc.to_le_bytes()[..k]);
            self.n = 0;
            self.acc = 0;
        }
    }
}

// ------------------------------------------------------------------ Huffman codes

/// the Huffman work area (HW_*; the tree scratch of hf_lengths is HwTree)
struct Hw {
    tok: Vec<u16>,
    len: Vec<u8>,
    tmp: Vec<u32>,
}

impl Hw {
    fn new() -> Hw {
        Hw { tok: vec![0; 2400], len: vec![0; 2400], tmp: vec![0; 2400] }
    }
}

// hf_lengths: length-limited Huffman code lengths like libwebp GenerateOptimalTree: counts
// below count_min are raised to count_min (doubled until the tree depth fits maxlen); a
// single used symbol gets length 1 -> used symbols
fn hf_lengths(counts: &[u32], maxlen: u32, lengths: &mut [u8], hw: &mut HwTree) -> u32 {
    let n = counts.len();
    let lengths = &mut lengths[..n];
    lengths.fill(0);
    let mut used = 0usize;
    let mut last = 0usize;
    for (i, &c) in counts.iter().enumerate() {
        if c != 0 {
            used += 1;
            last = i;
        }
    }
    if used <= 1 {
        if used == 1 {
            lengths[last] = 1;
        }
        return used as u32;
    }
    let mut cmin = 1u64;
    loop {
        let mut k = 0;
        for (i, &c) in counts.iter().enumerate() {
            if c != 0 {
                hw.key[k] = (c as u64).max(cmin) << 16 | i as u64;
                k += 1;
            }
        }
        hw.key[..used].sort_unstable(); // distinct keys: any sort gives the shell sort's order
        for i in 0..used {
            hw.w[i] = hw.key[i] >> 16;
        }
        let (mut leaf, mut inner, mut node) = (0usize, used, used);
        while node < 2 * used - 1 {
            let mut pick = || {
                let take_inner = if leaf >= used {
                    true
                } else if inner >= node {
                    false
                } else {
                    hw.w[leaf] > hw.w[inner]
                };
                if take_inner {
                    inner += 1;
                    inner - 1
                } else {
                    leaf += 1;
                    leaf - 1
                }
            };
            let a = pick();
            let b = pick();
            hw.w[node] = hw.w[a] + hw.w[b];
            hw.par[a] = node as u32;
            hw.par[b] = node as u32;
            node += 1;
        }
        let root = 2 * used - 2;
        hw.dep[root] = 0;
        let mut maxd = 0u32;
        for i in (0..root).rev() {
            let d = hw.dep[hw.par[i] as usize] + 1;
            hw.dep[i] = d;
            if i < used && d > maxd {
                maxd = d;
            }
        }
        if maxd <= maxlen {
            break;
        }
        cmin <<= 1;
    }
    for i in 0..used {
        lengths[(hw.key[i] & 0xFFFF) as usize] = hw.dep[i] as u8;
    }
    used as u32
}

/// the scratch of hf_lengths
struct HwTree {
    key: Vec<u64>,
    w: Vec<u64>,
    par: Vec<u32>,
    dep: Vec<u32>,
}

impl HwTree {
    fn new() -> HwTree {
        HwTree { key: vec![0; 2400], w: vec![0; 4800], par: vec![0; 4800], dep: vec![0; 4800] }
    }
}

// hf_codes: canonical codes (deflate order), bit-reversed for the LSB-first writer:
// table[s] = code | length << 16 (0 for unused symbols)
fn hf_codes(lengths: &[u8], table: &mut [u32]) {
    let mut cnt = [0u32; 16];
    for &l in lengths {
        cnt[(l & 15) as usize] += 1;
    }
    cnt[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for l in 1..16 {
        code = (code + cnt[l - 1]) * 2;
        next[l] = code;
    }
    for (s, &l) in lengths.iter().enumerate() {
        let l = (l & 15) as u32;
        let mut r = 0;
        if l != 0 {
            let mut c = next[l as usize];
            next[l as usize] += 1;
            for _ in 0..l {
                r = r * 2 | (c & 1);
                c >>= 1;
            }
            r |= l << 16;
        }
        table[s] = r;
    }
}

// hf_make: build the length-limited code of counts, write its description and fill table
// (a code with <= 1 used symbol is written with 0 bits: entries 0) -> used symbols
fn hf_make(bw: &mut Bw, counts: &[u32], table: &mut [u32], hw: &mut Hw, ht: &mut HwTree) -> u32 {
    let n = counts.len();
    let mut len = core::mem::take(&mut hw.len);
    let used = hf_lengths(counts, 15, &mut len, ht);
    hf_write(bw, &len[..n], used, hw, ht);
    hf_codes(&len[..n], &mut table[..n]);
    if used <= 1 {
        table[..n].fill(0); // 0-bit code
    }
    hw.len = len;
    used
}

// hf_write: write a prefix-code description
fn hf_write(bw: &mut Bw, lengths: &[u8], used: u32, hw: &mut Hw, ht: &mut HwTree) {
    let n = lengths.len();
    if used == 0 {
        bw.put(1, 4); // simple, 1 symbol, 1-bit symbol 0
        return;
    }
    if used <= 2 {
        // first two used symbols
        let mut first = u32::MAX;
        let mut second = u32::MAX;
        for (i, &l) in lengths.iter().enumerate() {
            if l != 0 {
                if first == u32::MAX {
                    first = i as u32;
                } else {
                    second = i as u32;
                    break;
                }
            }
        }
        if first < 256 && (used < 2 || second < 256) {
            bw.put(1, 1); // simple code
            bw.put(used - 1, 1);
            if first <= 1 {
                bw.put(first * 2, 2); // is_first_8bits 0, symbol (1 bit)
            } else {
                bw.put(first << 1 | 1, 9);
            }
            if used >= 2 {
                bw.put(second, 8);
            }
            return;
        }
    }
    bw.put(0, 1); // normal code
    // RLE tokens (libwebp VP8LCreateCompressedHuffmanTree)
    let tok = &mut hw.tok;
    let mut nt = 0usize;
    let mut prev = 8u32;
    let mut i = 0usize;
    while i < n {
        let v = lengths[i] as u32;
        let mut k = i + 1;
        while k < n && lengths[k] as u32 == v {
            k += 1;
        }
        let mut runs = (k - i) as u32;
        i = k;
        if v == 0 {
            while runs != 0 {
                if runs < 3 {
                    tok[nt] = 0;
                    nt += 1;
                    runs -= 1;
                } else if runs < 11 {
                    tok[nt] = (17 | (runs - 3) << 8) as u16;
                    nt += 1;
                    break;
                } else if runs < 139 {
                    tok[nt] = (18 | (runs - 11) << 8) as u16;
                    nt += 1;
                    break;
                } else {
                    tok[nt] = 18 | (127 << 8);
                    nt += 1;
                    runs -= 138;
                }
            }
            continue;
        }
        if v != prev {
            tok[nt] = v as u16;
            nt += 1;
            runs -= 1;
        }
        prev = v;
        while runs != 0 {
            if runs < 3 {
                tok[nt] = v as u16;
                nt += 1;
                runs -= 1;
            } else if runs < 7 {
                tok[nt] = (16 | (runs - 3) << 8) as u16;
                nt += 1;
                break;
            } else {
                tok[nt] = 16 | (3 << 8);
                nt += 1;
                runs -= 6;
            }
        }
    }
    // histogram of the 19 code-length symbols
    let mut clh = [0u32; 19];
    for &t in &tok[..nt] {
        clh[(t & 0xFF) as usize] += 1;
    }
    let mut cll = [0u8; 19];
    let ucl = hf_lengths(&clh, 7, &mut cll, ht);
    // codes_to_store
    let mut cs = 19usize;
    while cs > 4 && cll[CL_ORDER[cs - 1] as usize] == 0 {
        cs -= 1;
    }
    bw.put((cs - 4) as u32, 4);
    for &o in &CL_ORDER[..cs] {
        bw.put(cll[o as usize] as u32, 3);
    }
    let mut clc = [0u32; 19];
    hf_codes(&cll, &mut clc);
    if ucl <= 1 {
        clc = [0; 19];
        cll = [0; 19];
    }
    // trailing zeros (libwebp: trim when > 12 bits of trailing zero tokens)
    let mut tl = nt;
    let mut tz = 0u32;
    while tl != 0 {
        let c = (tok[tl - 1] & 0xFF) as usize;
        match c {
            0 => {}
            17 => tz += 3,
            18 => tz += 7,
            _ => break,
        }
        tz += cll[c] as u32;
        tl -= 1;
    }
    if tl > 1 && tz > 12 {
        nt = tl;
        bw.put(1, 1);
        if tl == 2 {
            bw.put(0, 5);
        } else {
            let nbp1 = (31 - ((tl - 2) as u32).leading_zeros()) >> 1; // nbitpairs - 1
            bw.put(nbp1, 3);
            bw.put((tl - 2) as u32, nbp1 * 2 + 2);
        }
    } else {
        bw.put(0, 1);
    }
    for &t in &tok[..nt] {
        let c = (t & 0xFF) as usize;
        bw.put(clc[c] & 0xFFFF, clc[c] >> 16);
        if c >= 16 {
            let nb = match c {
                16 => 2,
                17 => 3,
                _ => 7,
            };
            bw.put((t >> 8) as u32, nb);
        }
    }
}

// ------------------------------------------------------------------ entropy estimates

/// the log2 table and the cost helpers (LE_L2)
struct L2 {
    t: [f32; 4096],
}

impl L2 {
    // log2(i) for i = 1..4095: integer squaring, 24 fraction bits (deterministic)
    fn new() -> L2 {
        let mut t = [0f32; 4096];
        for i in 1..4096u32 {
            let k = 31 - i.leading_zeros();
            let mut fv = (i as u64) << (30 - k); // f in [2^30, 2^31)
            let mut fr = 0u32;
            for _ in 0..24 {
                fv = (fv * fv) >> 30;
                fr += fr;
                if fv & (1 << 31) != 0 {
                    fv >>= 1;
                    fr |= 1;
                }
            }
            t[i as usize] = ((k << 24) + fr) as i32 as f32 * f(F2M24);
        }
        L2 { t }
    }

    // le_slog2: v * log2(v)
    #[inline(always)]
    fn slog2(&self, v: u32) -> f32 {
        if v < 4096 {
            return v as f32 * self.t[v as usize];
        }
        let k = (31 - v.leading_zeros()) - 11;
        let l = self.t[(v >> k) as usize] + k as i32 as f32;
        v as f32 * l
    }

    // le_log2f: log2(v) (v >= 1)
    #[inline(always)]
    fn log2f(&self, v: u32) -> f32 {
        if v < 4096 {
            return self.t[v as usize];
        }
        let k = (31 - v.leading_zeros()) - 11;
        self.t[(v >> k) as usize] + k as i32 as f32
    }

    // le_entropy: libwebp VP8LBitsEntropy (mode 0), VP8LPopulationCost (mode 1: +
    // FinalHuffmanCost) or the code description only (mode 2) of a[n] (+ b[n])
    fn entropy(&self, a: &[u32], b: Option<&[u32]>, mode: u32) -> f32 {
        let n = a.len();
        let mut ent = 0f32;
        let mut sum = 0u64;
        let mut nonzeros = 0u32;
        let mut maxv = 0u32;
        let mut counts = [0u32; 2];
        let mut streaks = [[0u32; 2]; 2];
        let val = |i: usize| match b {
            Some(b) => a[i].wrapping_add(b[i]),
            None => a[i],
        };
        let mut i_prev = 0usize;
        let mut v_prev = val(0);
        // EN_STEP: close the streak of v_prev at i
        macro_rules! en_step {
            ($i:expr, $newv:expr) => {{
                let streak = ($i - i_prev) as u32;
                let vp = v_prev;
                let mut nz = 0;
                if vp != 0 {
                    sum += vp as u64 * streak as u64;
                    nonzeros += streak;
                    if vp > maxv {
                        maxv = vp;
                    }
                    let x = self.slog2(vp) * streak as f32;
                    ent -= x;
                    nz = 1;
                }
                let long = (streak > 3) as usize;
                counts[nz] += long as u32;
                streaks[nz][long] += streak;
                v_prev = $newv;
                i_prev = $i;
            }};
        }
        for i in 1..n {
            let v = val(i);
            if v != v_prev {
                en_step!(i, v);
            }
        }
        en_step!(n, 0);
        let _ = (v_prev, i_prev);
        ent += self.slog2(sum as u32);
        // BitsEntropyRefine
        let refine = |ent: f32, mix: f32| {
            let ml = (sum as f32 + sum as f32) - maxv as f32; // min_limit = 2 sum - max
            let x = ml * mix + (f(F1) - mix) * ent;
            rt::fx::maxss(ent, x)
        };
        if nonzeros >= 5 {
            ent = refine(ent, f(F0627));
        } else if nonzeros <= 1 {
            ent = 0.0;
        } else if nonzeros == 2 {
            let x = sum as f32 * f(F099);
            ent = x + ent * f(F001);
        } else if nonzeros == 3 {
            ent = refine(ent, f(F095));
        } else {
            ent = refine(ent, f(F07));
        }
        if mode == 0 {
            return ent;
        }
        if mode == 2 {
            ent = 0.0; // the code description only
        }
        // FinalHuffmanCost
        let mut c = f(F479);
        c += counts[0] as f32 * f(F15625);
        c += streaks[0][1] as f32 * f(F0234);
        c += counts[1] as f32 * f(F2578);
        c += streaks[1][1] as f32 * f(F0703);
        c += streaks[0][0] as f32 * f(F1796);
        c += streaks[1][0] as f32 * f(F3281);
        ent + c
    }

    // le_combent: libwebp CombinedShannonEntropy of X[256] and the accumulated Y[256]
    fn combent(&self, x: &[u32; 256], y: &[u32; 256]) -> f32 {
        let mut e = 0f32;
        let mut sx = 0u32;
        let mut sxy = 0u32;
        for i in 0..256 {
            let xv = x[i];
            if xv != 0 {
                sx = sx.wrapping_add(xv);
                e -= self.slog2(xv);
                let xy = xv.wrapping_add(y[i]);
                sxy = sxy.wrapping_add(xy);
                e -= self.slog2(xy);
            } else {
                let yv = y[i];
                if yv != 0 {
                    sxy = sxy.wrapping_add(yv);
                    e -= self.slog2(yv);
                }
            }
        }
        e += self.slog2(sx);
        self.slog2(sxy) + e
    }
}

// le_pcspatial: libwebp PredictionCostSpatial with the pre-multiplied coefficients
fn pcspatial(c: &[u32; 256], coef: &[u32; 16]) -> f32 {
    let mut x = c[0] as f32 * f(coef[0]);
    for i in 1..16 {
        let v = c[i].wrapping_add(c[256 - i]);
        x += v as f32 * f(coef[i]);
    }
    x
}

// ------------------------------------------------------------------ the encoder context

/// the encoder context (LE_*)
struct Le<'a> {
    w: u32,
    h: u32,
    effort: u32,
    flags: u32,
    cfg: &'a Cfg<'a>,
    err: i32, // sticky error
    alpha: u32,
    npal: u32,
    qual: u32,
    method: u32,
    iters: u32,
    window: u32,
    cachemax: u32,
    orig: Vec<u32>,
    work: Vec<u32>,
    hw: Hw,
    ht: HwTree,
    tbits: u32,
    hbits: u32,
    rbzero: bool,
    cfgs: Vec<u32>,
    xs: u32,
    dbg: u32,
    dbgm: Option<Vec<u8>>,
    dbgc: [f32; 16],
    pal: [u32; 256],
    plut: [u8; 128],
    l2: L2,
}

/// encode sf (any get_row format) as a VP8L bitstream (payload of a "VP8L" chunk, 5-byte
/// header included); honours effort, IOF_WEBP_NOEXACT, the cancel flag  [vp8l_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt) -> Result<Vec<u8>, i32> {
    let (px, w, h) = load_argb(sf)?;
    let cfg = cfg_from_opt(opt);
    enc_argb(&px, w, h, &cfg)
}

/// VP8L stream of the w x h pixels (1..16384 each); cfg effort 0 = 5, WEF_NOHDR: no 5-byte
/// header (ALPH)  [vp8l_enc_argb]
pub fn enc_argb(px: &[u32], w: u32, h: u32, cfg: &Cfg) -> Result<Vec<u8>, i32> {
    let (r, v) = enc_argb_ex(px, w, h, cfg)?;
    if r != 0 { Err(r) } else { Ok(v) }
}

/// enc_argb with the test outputs: WEF_T_DUMPM -> (0, the main image's match array),
/// WEF_T_REPORT -> (0x1000000 + (cache bits | groups << 8), the 16 cache-size estimates)
pub fn enc_argb_ex(px: &[u32], w: u32, h: u32, cfg: &Cfg) -> Result<(i32, Vec<u8>), i32> {
    if w.wrapping_sub(1) >= 16384 || h.wrapping_sub(1) >= 16384 {
        return Err(ERR_DIMS);
    }
    let effort = match cfg.effort {
        0 => 5,
        e => e.min(9),
    };
    let n = w as usize * h as usize;
    let mut le = Le {
        w,
        h,
        effort,
        flags: cfg.flags,
        cfg,
        err: 0,
        alpha: 0,
        npal: 0,
        qual: 0,
        method: 0,
        iters: 0,
        window: 0,
        cachemax: 0,
        orig: px[..n].to_vec(),
        work: vec![0; n + 64],
        hw: Hw::new(),
        ht: HwTree::new(),
        tbits: 0,
        hbits: 0,
        rbzero: false,
        cfgs: Vec::new(),
        xs: 0,
        dbg: 0,
        dbgm: None,
        dbgc: [0.0; 16],
        pal: [0; 256],
        plut: [0; 128],
        l2: L2::new(),
    };
    le.setup();
    le.analyze();
    // trials: keep the smaller stream
    let mut best: Option<Vec<u8>> = None;
    for k in 0..le.cfgs.len() {
        let c = le.cfgs[k];
        let s = le.trial(c)?;
        match &best {
            Some(b) if s.len() >= b.len() => {}
            _ => best = Some(s),
        }
    }
    if le.flags & WEF_T_DUMPM != 0 {
        return Ok((0, le.dbgm.take().unwrap_or_default()));
    }
    if le.flags & WEF_T_REPORT != 0 {
        let mut o = Vec::with_capacity(64);
        for v in le.dbgc {
            o.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        return Ok((0x100_0000 + le.dbg as i32, o));
    }
    Ok((0, best.unwrap_or_default()))
}

impl Le<'_> {
    // le_setup: plane-code table, effort parameters (libwebp's method / quality mapping:
    // effort e -> method min(6, e - 1), quality 25 .. 100); the log2 table is built in L2
    fn setup(&mut self) {
        for (c, &v) in CODE_TO_PLANE.iter().enumerate() {
            self.plut[(v & 127) as usize] = c as u8;
        }
        self.method = (self.effort - 1).min(6);
        let q = QUAL[(self.effort - 1) as usize] as u32;
        self.qual = q;
        // hash chain candidates 8 + q^2 / 128, window by quality
        self.iters = ((q * q) >> 7) + 8;
        let w = self.w;
        let win = if q > 75 {
            LZ_WINDOW
        } else if q > 50 {
            w << 8
        } else if q > 25 {
            w << 6
        } else {
            w << 4
        };
        self.window = win.min(LZ_WINDOW);
        // colour cache: none for quality <= 25, else up to 10 bits
        self.cachemax = if q > 25 { CACHE_MAXB } else { 0 };
    }

    // le_poll: the cancel flag
    fn poll(&mut self) -> Result<(), i32> {
        if self.cfg.cancelled() {
            self.err = ERR_CANCEL;
            return Err(ERR_CANCEL);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ analysis

    // le_analyze: alpha use, palette (<= 256 colours, sorted and delta-minimised like
    // libwebp), libwebp AnalyzeEntropy, tile bits, configurations
    fn analyze(&mut self) {
        let n = (self.w * self.h) as usize;
        // NOEXACT: RGB of transparent pixels -> 0 (own copy of the pixels)
        if self.flags & WEF_NOEXACT != 0 {
            for p in self.orig.iter_mut() {
                if *p < 0x0100_0000 {
                    *p = 0;
                }
            }
        }
        let px = &self.orig;
        if px.iter().any(|&p| p >> 24 != 255) {
            self.alpha = 1;
        }
        // colour set
        let mut set = vec![0u64; 1024];
        let mut nc = 0usize;
        let mut prev = !px[0];
        let mut nopal = false;
        for &c in px[..n].iter() {
            if c == prev {
                continue;
            }
            prev = c;
            let mut slot = (c.wrapping_mul(0x1E35_A7BD) >> 22) as usize;
            let key = c as u64 | 1 << 32;
            loop {
                let e = set[slot];
                if e == 0 {
                    if nc >= 256 {
                        nopal = true;
                    } else {
                        set[slot] = key;
                        self.pal[nc] = c;
                        nc += 1;
                    }
                    break;
                }
                if e == key {
                    break;
                }
                slot = (slot + 1) & 1023;
            }
            if nopal {
                break;
            }
        }
        if nopal {
            nc = 0;
        } else {
            self.pal[..nc].sort_unstable(); // distinct colours
            pal_mindelta(&mut self.pal[..nc]);
        }
        self.npal = nc as u32;
        // tile bits (libwebp GetHistoBits / GetTransformBits)
        let mut b = (if nc != 0 { 9 } else { 7 }) - self.method as i32;
        loop {
            let m = (1u32 << b) - 1;
            let t = ((self.w + m) >> b).wrapping_mul((self.h + m) >> b);
            if t <= 2600 {
                break;
            }
            b += 1;
        }
        let hb = b.clamp(2, 9) as u32;
        self.hbits = hb;
        let tmax = if self.method < 4 {
            6
        } else if self.method == 4 {
            5
        } else {
            4
        };
        self.tbits = hb.min(tmax);
        // entropy analysis
        let mut hist = vec![[0u32; 256]; 13];
        let w = self.w as usize;
        let mut pp = px[0];
        for y in 0..self.h as usize {
            for x in 0..w {
                let p = px[y * w + x];
                let d = sub_px(p, pp);
                pp = p;
                if d == 0 {
                    continue;
                }
                if y != 0 && p == px[(y - 1) * w + x] {
                    continue;
                }
                hist[LA_A][(p >> 24) as usize] += 1;
                hist[LA_B][(p & 0xFF) as usize] += 1;
                hist[LA_G][((p >> 8) & 0xFF) as usize] += 1;
                hist[LA_R][((p >> 16) & 0xFF) as usize] += 1;
                hist[LA_AP][(d >> 24) as usize] += 1;
                hist[LA_BP][(d & 0xFF) as usize] += 1;
                hist[LA_GP][((d >> 8) & 0xFF) as usize] += 1;
                hist[LA_RP][((d >> 16) & 0xFF) as usize] += 1;
                let g = (p >> 8) as u8;
                hist[LA_RSG][((p >> 16) as u8).wrapping_sub(g) as usize] += 1;
                hist[LA_BSG][(p as u8).wrapping_sub(g) as usize] += 1;
                let dg = (d >> 8) as u8;
                hist[LA_RPSG][((d >> 16) as u8).wrapping_sub(dg) as usize] += 1;
                hist[LA_BPSG][(d as u8).wrapping_sub(dg) as usize] += 1;
                let hp = (p.wrapping_add(p >> 19).wrapping_mul(0x39c5_fba7)) >> 24;
                hist[LA_PAL][hp as usize] += 1;
            }
        }
        for k in [LA_RPSG, LA_BPSG, LA_RP, LA_GP, LA_BP, LA_AP] {
            hist[k][0] += 1;
        }
        let mut e = [0f32; 13];
        for k in 0..13 {
            e[k] = self.l2.entropy(&hist[k], None, 0);
        }
        let mut m = [0f32; 5];
        m[0] = e[LA_A] + e[LA_R] + e[LA_G] + e[LA_B];
        m[1] = e[LA_AP] + e[LA_RP] + e[LA_GP] + e[LA_BP];
        m[2] = e[LA_A] + e[LA_RSG] + e[LA_G] + e[LA_BSG];
        m[3] = e[LA_AP] + e[LA_RPSG] + e[LA_GP] + e[LA_BPSG];
        m[4] = e[LA_PAL] + ((nc as u32) << 3) as i32 as f32;
        // transform overheads: tiles * log2(14) (spatial), tiles * log2(24) (spatial + sg)
        let tb = self.tbits;
        let mm = (1u32 << tb) - 1;
        let tiles = (((self.w + mm) >> tb).wrapping_mul((self.h + mm) >> tb)) as i32 as f32;
        m[1] = tiles * f(FLOG14) + m[1];
        m[3] = tiles * f(FLOG24) + m[3];
        // minimum over the modes (palette only when possible)
        let last = if nc != 0 { LC_PALETTE } else { LC_SPATIALSG } as usize;
        let mut best = 0usize;
        for c in 1..=last {
            if m[best] > m[c] {
                best = c;
            }
        }
        // small palettes (<= 16 colours, bundled) always win (libwebp AnalyzeEntropy)
        if (nc as u32).wrapping_sub(1) < 16 {
            best = LC_PALETTE as usize;
        }
        // red and blue always zero for the chosen mode
        let (hr, hb2) = RBPAIRS[best];
        self.rbzero = (1..256).all(|i| hist[hr][i] | hist[hb2][i] == 0);
        // configurations: the estimate; effort >= 7 adds the runner-up, 9 all modes
        self.cfgs = vec![best as u32];
        if self.effort >= 9 {
            self.cfgs = (0..=last as u32).collect();
        } else if self.effort >= 7 {
            let mut ru = usize::MAX;
            for c in 0..=last {
                if c == best {
                    continue;
                }
                if ru == usize::MAX || m[ru] > m[c] {
                    ru = c;
                }
            }
            self.cfgs.push(ru as u32);
        }
        // test: forced configuration
        let t = (self.flags >> WEF_T_CFG_SHIFT) & 15;
        if t != 0 {
            let c = t - 1;
            if c < LC_PALETTE || (c == LC_PALETTE && self.npal != 0) {
                self.cfgs = vec![c];
            }
        }
    }

    // ------------------------------------------------------------------ one configuration

    // le_trial: encode the image with configuration LC_* (header, transforms, entropy-coded
    // image) -> the stream
    fn trial(&mut self, c: u32) -> Result<Vec<u8>, i32> {
        let n = (self.w * self.h) as usize;
        let mut bw = Bw::new(n + 1024);
        if self.flags & WEF_NOHDR == 0 {
            bw.put((self.w - 1) << 8 | 0x2F, 22); // signature, width - 1
            bw.put(self.alpha << 14 | (self.h - 1), 18); // height - 1, alpha hint, version 0
        }
        self.work[..n].copy_from_slice(&self.orig[..n]);
        self.xs = self.w;
        if c == LC_PALETTE {
            self.tr_palette(&mut bw)?;
        } else {
            if c == LC_SUBGREEN || c == LC_SPATIALSG {
                bw.put(1 | 2 << 1, 3);
                for p in self.work[..n].iter_mut() {
                    let g = (*p >> 8) & 0xFF;
                    *p = sub_px(*p, g | g << 16);
                }
            }
            if c == LC_SPATIAL || c == LC_SPATIALSG {
                self.tr_predictor(&mut bw)?;
                if !self.rbzero && self.flags & WEF_T_NOCROSS == 0 {
                    self.tr_cross(&mut bw)?;
                }
            }
        }
        bw.put(0, 1); // no more transforms
        let work = core::mem::take(&mut self.work);
        let r = self.encode_image(&mut bw, &work[..(self.xs * self.h) as usize], self.xs, self.h, 0);
        self.work = work;
        r?;
        bw.finish();
        if self.err != 0 {
            return Err(self.err);
        }
        Ok(bw.buf)
    }

    // le_tr_palette: colour-indexing transform: palette (delta coded sub-image) and the
    // bundled index image in work (xs = its width)
    fn tr_palette(&mut self, bw: &mut Bw) -> Result<(), i32> {
        let np = self.npal as usize;
        bw.put(((np as u32) - 1) << 3 | 1 | 3 << 1, 11);
        let mut d = vec![0u32; np];
        let mut prev = 0u32;
        for i in 0..np {
            d[i] = sub_px(self.pal[i], prev);
            prev = self.pal[i];
        }
        self.encode_image(bw, &d, np as u32, 1, 1)?;
        // colour -> index set: u64 (colour | 1 << 32 | index << 40)
        let mut set = vec![0u64; 1024];
        for i in 0..np {
            let c = self.pal[i];
            let mut slot = (c.wrapping_mul(0x1E35_A7BD) >> 22) as usize;
            while set[slot] != 0 {
                slot = (slot + 1) & 1023;
            }
            set[slot] = (i as u64) << 40 | 1 << 32 | c as u64;
        }
        // bundling
        let xb = if np > 16 {
            0
        } else if np > 4 {
            1
        } else if np > 2 {
            2
        } else {
            3
        };
        let w = self.w as usize;
        self.xs = (self.w + (1 << xb) - 1) >> xb;
        let mut last = self.work[0] ^ 1; // differs from the first pixel
        let mut lidx = 0u64;
        let mut dst = 0usize;
        for y in 0..self.h as usize {
            let row = y * w;
            let mut x = 0usize;
            while x < w {
                let mut code = 0u32;
                let mut k = 0u32;
                while k < 1 << xb && x < w {
                    let c = self.work[row + x];
                    if c != last {
                        last = c;
                        let mut slot = (c.wrapping_mul(0x1E35_A7BD) >> 22) as usize;
                        loop {
                            let e = set[slot];
                            if e as u32 == c && e & (1 << 32) != 0 {
                                lidx = e >> 40;
                                break;
                            }
                            slot = (slot + 1) & 1023;
                        }
                    }
                    code |= (lidx as u32) << ((8 >> xb) * k);
                    k += 1;
                    x += 1;
                }
                self.work[dst] = code << 8 | 0xFF00_0000;
                dst += 1;
            }
        }
        Ok(())
    }

    // le_tr_predictor: predictor transform of work: per tile the mode with the lowest
    // libwebp PredictionCostSpatialHistogram (accumulated histograms, bias 15 for the left /
    // above tile's mode), residuals in place, modes sub-image
    fn tr_predictor(&mut self, bw: &mut Bw) -> Result<(), i32> {
        let w = self.w as usize;
        let h = self.h as usize;
        let bits = self.tbits;
        let tw = (w + (1 << bits) - 1) >> bits;
        let th = (h + (1 << bits) - 1) >> bits;
        let mut modes = vec![0u32; tw * th];
        let mut acc = [0u32; 1024];
        let mut hcur = [0u32; 1024];
        let mut hbest = [0u32; 1024];
        let forced = (self.flags >> WEF_T_MODE_SHIFT) & 15;
        for ty in 0..th {
            let y0 = ty << bits;
            let y1 = (y0 + (1 << bits)).min(h);
            for tx in 0..tw {
                let x0 = tx << bits;
                let x1 = (x0 + (1 << bits)).min(w);
                let t = ty * tw + tx;
                let lm = if tx != 0 { modes[t - 1] } else { 0xFF };
                let am = if ty != 0 { modes[t - tw] } else { 0xFF };
                if forced != 0 {
                    modes[t] = forced - 1;
                    continue;
                }
                let mut bestc = f(FBIG);
                let mut bm = 0u32;
                for m in 0..14u32 {
                    hcur.fill(0);
                    tile_hist(m, &self.work, w, x0, x1, y0, y1, &mut hcur);
                    let mut cost = 0f32;
                    for ch in 0..4 {
                        let hc: &[u32; 256] = hcur[ch * 256..ch * 256 + 256].try_into().unwrap();
                        cost += pcspatial(hc, &PS94);
                        cost += self.l2.combent(hc, acc[ch * 256..ch * 256 + 256].try_into().unwrap());
                    }
                    if m == lm {
                        cost -= f(F15);
                    }
                    if m == am {
                        cost -= f(F15);
                    }
                    if cost < bestc {
                        bestc = cost;
                        bm = m;
                        core::mem::swap(&mut hcur, &mut hbest);
                    }
                }
                // accumulate the best histogram, store the mode
                for (a, &b) in acc.iter_mut().zip(hbest.iter()) {
                    *a = a.wrapping_add(b);
                }
                modes[t] = bm;
                self.poll()?;
            }
        }
        // residuals: up row (w + 1) and current row (w + 1)
        let mut up = vec![0u32; w + 1];
        let mut cur = vec![0u32; w + 1];
        for y in 0..h {
            cur[..w].copy_from_slice(&self.work[y * w..y * w + w]);
            up[w] = cur[0]; // TR of the last column
            let out = &mut self.work[y * w..y * w + w];
            if y == 0 {
                out[0] = sub_px(cur[0], 0xFF00_0000);
                for x in 1..w {
                    out[x] = sub_px(cur[x], cur[x - 1]);
                }
            } else {
                out[0] = sub_px(cur[0], up[0]);
                let mrow = &modes[(y >> bits) * tw..(y >> bits) * tw + tw];
                for (tx, &m) in mrow.iter().enumerate() {
                    let x0 = (tx << bits).max(1);
                    let x1 = ((tx + 1) << bits).min(w);
                    res_row(m, &cur, &up, out, x0, x1);
                }
            }
            up[..w].copy_from_slice(&cur[..w]);
        }
        bw.put(1 | 0 << 1 | (bits - 2) << 3, 6);
        for m in modes.iter_mut() {
            *m = *m << 8 | 0xFF00_0000;
        }
        self.encode_image(bw, &modes, tw as u32, th as u32, 1)
    }

    // le_tr_cross: cross-colour transform of work (libwebp VP8LColorSpaceTransform: greedy
    // green-to-red and green/red-to-blue search per tile with accumulated histograms),
    // sub-image of the multipliers
    fn tr_cross(&mut self, bw: &mut Bw) -> Result<(), i32> {
        let w = self.w as usize;
        let h = self.h as usize;
        let bits = self.tbits;
        let tw = (w + (1 << bits) - 1) >> bits;
        let th = (h + (1 << bits) - 1) >> bits;
        let mut mult = vec![0u32; tw * th];
        let mut acc_r = [0u32; 256];
        let mut acc_b = [0u32; 256];
        let mut hist = [0u32; 256];
        let mut prev_x = 0u32;
        let mut prev_y = 0u32;
        let q = self.qual;
        for ty in 0..th {
            let y0 = ty << bits;
            let thei = (y0 + (1 << bits)).min(h) - y0;
            for tx in 0..tw {
                let x0 = tx << bits;
                let twid = (x0 + (1 << bits)).min(w) - x0;
                let tile = y0 * w + x0;
                if ty != 0 {
                    prev_y = mult[(ty - 1) * tw + tx] & 0xFF_FFFF;
                }
                let cc = Cc { tile, w, twid, thei, prev_x, prev_y };
                // green to red: iterations 4 + (7 q >> 8), delta 32 >> it
                let mut g2r = 0i32;
                let mut best = self.cc_eval(&cc, &mut hist, &acc_r, 0, 0, 0);
                for it in 0..(((q * 7) >> 8) + 4) {
                    let d = 32i32 >> it;
                    for off in [-d, d] {
                        let cand = g2r + off;
                        let c = self.cc_eval(&cc, &mut hist, &acc_r, 0, cand, 0);
                        if c < best {
                            best = c;
                            g2r = cand;
                        }
                    }
                }
                // green / red to blue
                let mut g2b = 0i32;
                let mut r2b = 0i32;
                let mut best = self.cc_eval(&cc, &mut hist, &acc_b, 1, 0, 0);
                let iters = if q < 25 {
                    1
                } else if q <= 50 {
                    4
                } else {
                    7
                };
                for it in 0..iters {
                    let d = CC_DELTA[it] as i32;
                    for &(ag, ar) in &CC_AXIS {
                        let cg = ag * d + g2b;
                        let cr = ar * d + r2b;
                        let c = self.cc_eval(&cc, &mut hist, &acc_b, 1, cg, cr);
                        if c < best {
                            best = c;
                            g2b = cg;
                            r2b = cr;
                        }
                    }
                    if CC_DELTA[it] == 2 && g2b | r2b == 0 {
                        break;
                    }
                }
                // multipliers -> prev_x and the colour code
                let code = (g2r as u32 & 0xFF) | (g2b as u32 & 0xFF) << 8 | (r2b as u32 & 0xFF) << 16;
                prev_x = code;
                mult[ty * tw + tx] = code | 0xFF00_0000;
                // transform the tile in place
                let (mg2r, mg2b, mr2b) = (g2r as u8 as i8 as i32, g2b as u8 as i8 as i32, r2b as u8 as i8 as i32);
                for y in 0..thei {
                    let row = tile + y * w;
                    for p in self.work[row..row + twid].iter_mut() {
                        let v = *p;
                        let g = (v >> 8) as u8 as i8 as i32;
                        let r = (v >> 16) as u8 as i8 as i32;
                        let nr = ((v >> 16) as i32).wrapping_sub((mg2r * g) >> 5) as u32 & 0xFF;
                        let nb = (v as i32).wrapping_sub((mg2b * g) >> 5).wrapping_sub((mr2b * r) >> 5) as u32 & 0xFF;
                        *p = (v & 0xFF00_FF00) | nr << 16 | nb;
                    }
                }
                // accumulate red / blue (repeated pixels are skipped like libwebp)
                let px = &self.work;
                for y in 0..thei {
                    let mut ix = (y0 + y) * w + x0;
                    for _ in 0..twid {
                        let p = px[ix];
                        let skip = if ix < 2 {
                            false
                        } else if p == px[ix - 2] && p == px[ix - 1] {
                            true
                        } else if ix < w + 2 {
                            false
                        } else {
                            px[ix - 2] == px[ix - w - 2] && px[ix - 1] == px[ix - w - 1] && p == px[ix - w]
                        };
                        if !skip {
                            acc_r[((p >> 16) & 0xFF) as usize] += 1;
                            acc_b[(p & 0xFF) as usize] += 1;
                        }
                        ix += 1;
                    }
                }
                self.poll()?;
            }
        }
        bw.put(1 | 1 << 1 | (bits - 2) << 3, 6);
        self.encode_image(bw, &mult, tw as u32, th as u32, 1)
    }

    // le_cc_eval: libwebp GetPredictionCostCrossColorRed (mode 0: green-to-red g) /
    // ...Blue (mode 1: green-to-blue g, red-to-blue r) of the tile, including the -3
    // bonuses for prev_x / prev_y / zero
    fn cc_eval(&self, cc: &Cc, hist: &mut [u32; 256], acc: &[u32; 256], mode: u32, g: i32, r: i32) -> f32 {
        let g = g as u8 as i8 as i32;
        let r = r as u8 as i8 as i32;
        hist.fill(0);
        let px = &self.work;
        for y in 0..cc.thei {
            let row = cc.tile + y * cc.w;
            let tp = &px[row..row + cc.twid];
            if mode == 0 {
                for &p in tp {
                    let d = ((p >> 8) as u8 as i8 as i32 * g) >> 5;
                    hist[((p >> 16) as i32).wrapping_sub(d) as usize & 0xFF] += 1;
                }
            } else {
                for &p in tp {
                    let d = ((p >> 8) as u8 as i8 as i32 * g) >> 5;
                    let rd = ((p >> 16) as u8 as i8 as i32 * r) >> 5;
                    hist[(p as i32).wrapping_sub(d).wrapping_sub(rd) as usize & 0xFF] += 1;
                }
            }
        }
        let mut cost = pcspatial(hist, &PS24);
        cost += self.l2.combent(hist, acc);
        let k = mode * 8; // byte of the g multiplier in prev_x / prev_y
        let f3 = f(F3);
        if (cc.prev_x >> k) as u8 == g as u8 {
            cost -= f3;
        }
        if (cc.prev_y >> k) as u8 == g as u8 {
            cost -= f3;
        }
        if g == 0 {
            cost -= f3;
        }
        if mode != 0 {
            if (cc.prev_x >> 16) as u8 == r as u8 {
                cost -= f3;
            }
            if (cc.prev_y >> 16) as u8 == r as u8 {
                cost -= f3;
            }
            if r == 0 {
                cost -= f3;
            }
        }
        cost
    }
}

/// the tile of a cross-colour evaluation (CC_*)
struct Cc {
    tile: usize, // index of the tile's first pixel
    w: usize,    // row stride in pixels
    twid: usize,
    thei: usize,
    prev_x: u32, // g2r | g2b << 8 | r2b << 16
    prev_y: u32,
}

// per-byte wrapping subtract (psubb)
#[inline(always)]
fn sub_px(a: u32, b: u32) -> u32 {
    let ag = (a | 0x00FF_00FF).wrapping_sub(b & 0xFF00_FF00);
    let rb = (a | 0xFF00_FF00).wrapping_sub(b & 0x00FF_00FF);
    (ag & 0xFF00_FF00) | (rb & 0x00FF_00FF)
}

// le_pal_mindelta: libwebp PaletteSortMinimizeDeltas on the sorted palette
fn pal_mindelta(pal: &mut [u32]) {
    let n = pal.len();
    if n < 2 {
        return;
    }
    // non-monotonous deltas?
    let mut predict = 0u32;
    let mut signs = 0u32;
    for &c in pal.iter() {
        let d = sub_px(c, predict);
        predict = c;
        for (k, (lo, hi)) in [(16u32, (1u32, 2u32)), (8, (8, 16)), (0, (64, 128))] {
            let v = (d >> k) & 0xFF;
            if v != 0 {
                signs |= if v < 0x80 { lo } else { hi };
            }
        }
    }
    if (signs * 2) & signs == 0 {
        return;
    }
    // greedy: always take the closest remaining colour to the previous one
    let mut predict = 0u32;
    for i in 0..n {
        let mut bi = i;
        let mut bs = u32::MAX;
        for k in i..n {
            let d = sub_px(pal[k], predict);
            let mut s = 0u32;
            for c in 0..3 {
                let v = (d >> (8 * c)) & 0xFF;
                s += if v > 128 { 256 - v } else { v };
            }
            s *= 9;
            let a = d >> 24;
            s += if a > 128 { 256 - a } else { a };
            if s < bs {
                bs = s;
                bi = k;
            }
        }
        pal.swap(i, bi);
        predict = pal[i];
    }
}

// le_pred: VP8L predictor m (0..13) of a pixel from its neighbours
#[inline(always)]
fn le_pred(m: u32, l: u32, t: u32, tr: u32, tl: u32) -> u32 {
    match m {
        1 => l,
        2 => t,
        3 => tr,
        4 => tl,
        5 => avg2(avg2(l, tr), t),
        6 => avg2(l, tl),
        7 => avg2(l, t),
        8 => avg2(tl, t),
        9 => avg2(t, tr),
        10 => avg2(avg2(l, tl), avg2(t, tr)),
        11 => {
            // sum |L - TL| <= sum |T - TL| ? T : L
            let mut pa = 0u32;
            let mut pb = 0u32;
            for k in 0..4 {
                let c = (tl >> (8 * k)) as u8;
                pa += ((l >> (8 * k)) as u8).abs_diff(c) as u32;
                pb += ((t >> (8 * k)) as u8).abs_diff(c) as u32;
            }
            if pa <= pb { t } else { l }
        }
        12 => {
            let mut o = 0u32;
            for k in 0..4 {
                let v = (l >> (8 * k)) as u8 as i32 + (t >> (8 * k)) as u8 as i32 - (tl >> (8 * k)) as u8 as i32;
                o |= (v.clamp(0, 255) as u32) << (8 * k);
            }
            o
        }
        13 => {
            let a = avg2(l, t);
            let mut o = 0u32;
            for k in 0..4 {
                let av = (a >> (8 * k)) as u8 as i32;
                let d = av - (tl >> (8 * k)) as u8 as i32;
                o |= ((av + d / 2).clamp(0, 255) as u32) << (8 * k);
            }
            o
        }
        _ => 0xFF00_0000,
    }
}

// ------------------------------------------------------------------ entropy-coded image

/// one entropy-coded image (EI_*)
struct Ei<'a> {
    argb: &'a [u32],
    w: u32,
    h: u32,
    n: usize,
    level: u32, // 0 = main image, 1 = sub-image
    cbits: u32, // colour cache bits (0 = none)
    m: Vec<u32>, // best match: dist | (len - 1) << 20
    step: Vec<u16>, // parse: step length at step starts
    pd: Vec<u32>, // parse: distance of the copy at a step start
    step2: Vec<u16>, // alternative parse
    pd2: Vec<u32>,
    cost: Vec<f32>, // shortest-path costs
    hbits: u32,
    ngrp: u32,
    gmap: Vec<u32>, // tile -> group
    tw: u32,
    th: u32,
    hist: Vec<u32>, // histograms (HG_N each)
    model: Vec<f32>, // symbol costs (HG_N) + length costs (4097)
    cache: Vec<u32>, // colour cache of the walks (1024)
}

/// the histograms of one colour cache size in le_ei_cache: green 256 + lengths 24 + cache
/// codes (up to 1024), red | blue | alpha, the cache
#[derive(Clone)]
struct CacheHist {
    g: [u32; 280 + 1024],
    rba: [u32; 768],
    cache: [u32; 1024],
}

impl CacheHist {
    const ZERO: CacheHist = CacheHist { g: [0; 1304], rba: [0; 768], cache: [0; 1024] };
}

// le_mlen: number of equal leading pixels of a and b (<= max)
#[inline(always)]
fn mlen(a: &[u32], b: &[u32], max: usize) -> usize {
    let (a, b) = (&a[..max], &b[..max]);
    let mut i = 0;
    // 4 pixels at a time, then the rest
    while i + 4 <= max {
        if a[i..i + 4] != b[i..i + 4] {
            break;
        }
        i += 4;
    }
    while i < max && a[i] == b[i] {
        i += 1;
    }
    i
}

// the residual histogram (4 x 256: alpha, red, green, blue) of predictor m over a tile
#[allow(clippy::too_many_arguments)]
fn tile_hist(m: u32, px: &[u32], w: usize, x0: usize, x1: usize, y0: usize, y1: usize, h: &mut [u32; 1024]) {
    macro_rules! go {
        ($($k:literal)*) => {
            match m {
                $($k => tile_hist_m::<$k>(px, w, x0, x1, y0, y1, h),)*
                _ => tile_hist_m::<0>(px, w, x0, x1, y0, y1, h),
            }
        };
    }
    go!(1 2 3 4 5 6 7 8 9 10 11 12 13);
}

#[inline(always)]
fn hadd4(h: &mut [u32; 1024], r: u32) {
    h[(r >> 24) as usize] += 1;
    h[256 + ((r >> 16) & 0xFF) as usize] += 1;
    h[512 + ((r >> 8) & 0xFF) as usize] += 1;
    h[768 + (r & 0xFF) as usize] += 1;
}

fn tile_hist_m<const M: u32>(px: &[u32], w: usize, x0: usize, x1: usize, y0: usize, y1: usize, h: &mut [u32; 1024]) {
    for y in y0..y1 {
        let cur = &px[y * w..y * w + w];
        if y == 0 {
            for x in x0..x1 {
                let pred = if x == 0 { 0xFF00_0000 } else { cur[x - 1] };
                hadd4(h, sub_px(cur[x], pred));
            }
            continue;
        }
        let up = &px[(y - 1) * w..y * w + 1]; // up[w] = TR of the last column
        let mut xs = x0;
        if xs == 0 {
            hadd4(h, sub_px(cur[0], up[0]));
            xs = 1;
        }
        for x in xs..x1 {
            let pred = le_pred(M, cur[x - 1], up[x], up[x + 1], up[x - 1]);
            hadd4(h, sub_px(cur[x], pred));
        }
    }
}

// residuals x0..x1 (x0 >= 1) of one row with predictor m (cur / up: w + 1 entries)
fn res_row(m: u32, cur: &[u32], up: &[u32], out: &mut [u32], x0: usize, x1: usize) {
    macro_rules! go {
        ($($k:literal)*) => {
            match m {
                $($k => res_row_m::<$k>(cur, up, out, x0, x1),)*
                _ => res_row_m::<0>(cur, up, out, x0, x1),
            }
        };
    }
    go!(1 2 3 4 5 6 7 8 9 10 11 12 13);
}

fn res_row_m<const M: u32>(cur: &[u32], up: &[u32], out: &mut [u32], x0: usize, x1: usize) {
    for x in x0..x1 {
        out[x] = sub_px(cur[x], le_pred(M, cur[x - 1], up[x], up[x + 1], up[x - 1]));
    }
}

// le_prefix: VP8L prefix code of v >= 1 -> (code, extra bit count, extra bits)
#[inline(always)]
fn prefix(v: u32) -> (u32, u32, u32) {
    let x = v - 1;
    if x < 4 {
        return (x, 0, 0);
    }
    let hb = 31 - x.leading_zeros();
    let nb = hb - 1;
    let second = (x >> nb) & 1;
    (2 * hb + second, nb, x & ((1 << nb) - 1))
}

impl Le<'_> {
    // le_plane: the VP8L distance code (plane code) of dist for the width w
    #[inline(always)]
    fn plane(&self, dist: u32, w: u32) -> u32 {
        let dy = dist / w;
        let dx = dist % w;
        if dx <= 8 {
            if dy >= 8 {
                return dist + 120;
            }
            return self.plut[(dy * 16 + 8 - dx) as usize] as u32 + 1;
        }
        if dx + 8 <= w || dy >= 7 {
            return dist + 120;
        }
        self.plut[((dy + 1) * 16 + 8 + w - dx) as usize] as u32 + 1
    }

    // le_encode_image: one entropy-coded image (RFC 9649 5.2): level 0 = the main image
    // (colour cache, meta prefix codes), 1 = a sub-image (transform data, palette, entropy
    // image)
    fn encode_image(&mut self, bw: &mut Bw, argb: &[u32], w: u32, h: u32, level: u32) -> Result<(), i32> {
        let r = self.encode_image_body(bw, argb, w, h, level);
        if let Err(e) = r
            && self.err == 0
        {
            self.err = e;
        }
        r
    }

    fn encode_image_body(&mut self, bw: &mut Bw, argb: &[u32], w: u32, h: u32, level: u32) -> Result<(), i32> {
        let n = w as usize * h as usize;
        let mut ei = Ei {
            argb,
            w,
            h,
            n,
            level,
            cbits: 0,
            m: vec![0; n + 16],
            step: vec![0; n + 32],
            pd: vec![0; n + 16],
            step2: vec![0; n + 32],
            pd2: vec![0; n + 16],
            cost: Vec::new(),
            hbits: 0,
            ngrp: 1,
            gmap: Vec::new(),
            tw: 0,
            th: 0,
            hist: Vec::new(),
            model: vec![0.0; HG_N + 4097 + 16],
            cache: vec![0; 1024],
        };
        self.ei_matches(&mut ei)?;
        if self.flags & WEF_T_DUMPM != 0 && level == 0 {
            let mut d = Vec::with_capacity(n * 4);
            for &v in &ei.m[..n] {
                d.extend_from_slice(&v.to_le_bytes());
            }
            self.dbgm = Some(d);
            return Ok(()); // test: nothing more to do
        }
        // libwebp GetBackwardReferences: greedy LZ77 and RLE parses, each with its own
        // colour cache size, the cheaper estimate wins; a winning LZ77 parse is refined by
        // the shortest path (effort >= 4, again from 7), kept when its estimate is lower
        ei_greedy(&mut ei);
        self.ei_cachesel(&mut ei);
        let mut best = self.ei_estimate(&mut ei);
        let bcb = ei.cbits;
        let mut rle_won = false;
        if self.flags & WEF_T_NORLE == 0 {
            ei_swap(&mut ei);
            ei_rle(&mut ei);
            self.ei_cachesel(&mut ei);
            let e = self.ei_estimate(&mut ei);
            if e < best {
                best = e;
                rle_won = true;
            } else {
                ei_swap(&mut ei); // back to the greedy parse
                ei.cbits = bcb;
            }
        }
        if !rle_won && self.effort >= 4 && n >= 16 && self.flags & WEF_T_NODP == 0 {
            let mut rounds = 0;
            loop {
                self.ei_model(&mut ei);
                ei_swap(&mut ei);
                self.ei_dp(&mut ei)?;
                let e = self.ei_estimate(&mut ei);
                if self.flags & WEF_T_KEEPDP == 0 && !(e < best) {
                    ei_swap(&mut ei); // the shortest path was not better
                    break;
                }
                best = e;
                rounds += 1;
                if self.effort < 7 || rounds >= 2 {
                    break;
                }
            }
        }
        self.ei_meta(&mut ei);
        if level == 0 {
            self.dbg = ei.ngrp << 8 | ei.cbits;
        }
        self.ei_write(bw, &mut ei)
    }

    // le_ei_matches: best (longest) match per position like libwebp VP8LHashChainFill:
    // pixel above, previous pixel, then the hash chain (2-pixel hash; runs hashed with their
    // remaining length), iters candidates inside the window, positions from the end, every
    // match extended to the left at the same distance
    fn ei_matches(&mut self, ei: &mut Ei) -> Result<(), i32> {
        let px = ei.argb;
        let n = ei.n;
        let m = &mut ei.m;
        m[..n].fill(0);
        if n < 3 {
            return Ok(());
        }
        let mut head = vec![u32::MAX; HASH_SIZE];
        let mut prev = vec![0u32; n];
        // chains
        prev[n - 1] = u32::MAX;
        let mut pos = 0usize;
        while pos + 1 < n {
            let a = px[pos];
            if a == px[pos + 1] {
                // run: remaining equal pixels after pos
                let mut c = 1usize;
                while pos + c + 1 < n && px[pos + c + 1] == a {
                    c += 1;
                }
                if c > LZ_MAXLEN {
                    // skip the head of a very long run (found through distance 1 anyway)
                    for _ in 0..c - LZ_MAXLEN {
                        prev[pos] = u32::MAX;
                        pos += 1;
                    }
                    c = LZ_MAXLEN;
                }
                while c != 0 {
                    let hv = ((c as u32).wrapping_mul(0xc6a4_a793).wrapping_add(a.wrapping_mul(0x5bd1_e996)) >> (32 - HASH_BITS)) as usize;
                    prev[pos] = head[hv];
                    head[hv] = pos as u32;
                    pos += 1;
                    c -= 1;
                }
            } else {
                let hv = (px[pos + 1].wrapping_mul(0xc6a4_a793).wrapping_add(a.wrapping_mul(0x5bd1_e996)) >> (32 - HASH_BITS)) as usize;
                prev[pos] = head[hv];
                head[hv] = pos as u32;
                pos += 1;
            }
        }
        drop(head);
        // libwebp VP8LHashChainFill: positions from the end; the best match of a position
        // is extended to the left while the pixels keep matching
        prev[n - 1] = u32::MAX;
        let w = ei.w as usize;
        let mut base = n - 2;
        let mut polls = 0u32;
        'pos: while base != 0 {
            polls = polls.wrapping_add(1);
            if polls & 0xFFFF == 0 {
                self.poll()?;
            }
            let maxl = (n - base - 1).min(LZ_MAXLEN);
            let good = maxl.min(256);
            let mut iters = self.iters as i32;
            let minpos = base.saturating_sub(self.window as usize);
            let mut bl = 0usize;
            let mut bd = 0usize;
            if base >= w {
                bl = mlen(&px[base - w..], &px[base..], maxl);
                bd = if bl != 0 { w } else { 0 };
                iters -= 1;
            }
            let l = mlen(&px[base - 1..], &px[base..], maxl);
            if l > bl {
                bl = l;
                bd = 1;
            }
            iters -= 1;
            if bl < LZ_MAXLEN {
                let mut cand = prev[base];
                while cand != u32::MAX && cand as usize >= minpos {
                    iters -= 1;
                    if iters <= 0 {
                        break;
                    }
                    let c = cand as usize;
                    if px[c + bl] == px[base + bl] {
                        let l = mlen(&px[c..], &px[base..], maxl);
                        if l > bl {
                            bl = l;
                            bd = base - c;
                            if bl >= good {
                                break;
                            }
                        }
                    }
                    cand = prev[c];
                }
            }
            // store, then extend to the left while the pixels at the same distance match
            let mut maxb = base;
            loop {
                m[base] = if bl != 0 { ((bl - 1) << 20 | bd) as u32 } else { 0 };
                base -= 1;
                if bd == 0 {
                    continue 'pos;
                }
                if base == 0 {
                    break 'pos;
                }
                if base < bd || px[base - bd] != px[base] {
                    continue 'pos;
                }
                if bl >= LZ_MAXLEN {
                    if bd == 1 {
                        continue;
                    }
                    if base + LZ_MAXLEN < maxb {
                        continue 'pos;
                    }
                    continue;
                }
                bl += 1;
                maxb = base;
            }
        }
        Ok(())
    }

    // le_ei_cachesel: colour cache bits for the current parse (main image only; 0
    // otherwise)
    fn ei_cachesel(&mut self, ei: &mut Ei) {
        ei.cbits = 0;
        if ei.level == 0 && self.cachemax != 0 && self.flags & WEF_T_NOCACHE == 0 {
            self.ei_cache(ei);
        }
    }

    // le_ei_estimate: estimated bits of the current parse (one group, cache cbits)
    fn ei_estimate(&mut self, ei: &mut Ei) -> f32 {
        let mut hist = vec![0u32; HG_N];
        self.ei_walk(ei, &mut hist, 0);
        let mut cost = self.hcost(ei, &hist, None, true);
        // + the extra bits of the length and distance codes (libwebp VP8LExtraCost)
        let mut bits = 0u64;
        for c in 4..NLEN {
            let eb = ((c - 2) >> 1) as u64;
            bits += hist[HG_G + 256 + c] as u64 * eb;
            bits += hist[HG_D + c] as u64 * eb;
        }
        for c in NLEN..NDIST {
            bits += hist[HG_D + c] as u64 * ((c - 2) >> 1) as u64;
        }
        cost += bits as i64 as f32;
        cost
    }

    // le_ei_walk: add every symbol of the parse (with the colour cache cbits) to a
    // histogram: mode 0 = hists[0], 1 = the tile's histogram, 2 = the tile's group
    fn ei_walk(&self, ei: &mut Ei, hists: &mut [u32], mode: u32) {
        let px = ei.argb;
        let cb = ei.cbits;
        let ksh = 32u32.wrapping_sub(cb);
        ei.cache.fill(0);
        let (mut x, mut y) = (0u32, 0u32);
        let mut pos = 0usize;
        while pos < ei.n {
            let hi = if mode == 0 {
                0
            } else {
                let t = ((y >> ei.hbits) * ei.tw + (x >> ei.hbits)) as usize;
                if mode == 1 { t } else { ei.gmap[t] as usize }
            };
            let hist = &mut hists[hi * HG_N..hi * HG_N + HG_N];
            let s = ei.step[pos] as usize;
            let cnt;
            if s <= 1 {
                let p = px[pos];
                let mut hit = false;
                if cb != 0 {
                    let k = (p.wrapping_mul(HASH_MUL) >> ksh) as usize;
                    if ei.cache[k] == p {
                        hist[HG_G + 280 + k] += 1;
                        hit = true;
                    }
                }
                if !hit {
                    hist[HG_G + ((p >> 8) & 0xFF) as usize] += 1;
                    hist[HG_R + ((p >> 16) & 0xFF) as usize] += 1;
                    hist[HG_B + (p & 0xFF) as usize] += 1;
                    hist[HG_A + (p >> 24) as usize] += 1;
                }
                cnt = 1;
            } else {
                let (c, _, _) = prefix(s as u32);
                hist[HG_G + 256 + c as usize] += 1;
                let d = self.plane(ei.pd[pos], ei.w);
                let (dc, _, _) = prefix(d);
                hist[HG_D + dc as usize] += 1;
                cnt = s;
            }
            if cb != 0 {
                for &q in &px[pos..pos + cnt] {
                    ei.cache[(q.wrapping_mul(HASH_MUL) >> ksh) as usize] = q;
                }
            }
            pos += cnt;
            x += cnt as u32;
            while x >= ei.w {
                x -= ei.w;
                y += 1;
            }
        }
    }

    // le_ei_model: bit-cost model of the current parse (libwebp
    // ConvertPopulationCountTableToBitEstimates) and the length costs
    fn ei_model(&mut self, ei: &mut Ei) {
        let mut hist = vec![0u32; HG_N];
        self.ei_walk(ei, &mut hist, 0);
        let model = &mut ei.model;
        for (a, &(off, size0)) in ALPHA_OFF.iter().enumerate() {
            let mut size = size0;
            if a == 0 && ei.cbits != 0 {
                size += 1 << ei.cbits;
            }
            let hs = &hist[off..off + size];
            let mut sum = 0u64;
            let mut nz = 0u32;
            for &c in hs {
                sum += c as u64;
                if c != 0 {
                    nz += 1;
                }
            }
            let l2sum = if nz > 1 { self.l2.log2f(sum as u32) } else { 0.0 };
            for (i, &c) in hs.iter().enumerate() {
                model[off + i] = if nz > 1 {
                    if c != 0 { l2sum - self.l2.log2f(c) } else { l2sum }
                } else {
                    0.0
                };
            }
        }
        // length costs
        for l in 1..=LZ_MAXLEN as u32 {
            let (c, nb, _) = prefix(l);
            model[HG_N + l as usize] = model[HG_G + 256 + c as usize] + nb as i32 as f32;
        }
    }

    // le_ei_dp: shortest-path parse with the model costs (every length up to DP_LCAP of the
    // best match, and its full length), colour cache aware
    fn ei_dp(&mut self, ei: &mut Ei) -> Result<(), i32> {
        let n = ei.n;
        if ei.cost.is_empty() {
            ei.cost = vec![0.0; n + 16];
        }
        let px = ei.argb;
        let w = ei.w;
        let ksh = 32u32.wrapping_sub(ei.cbits);
        ei.cache.fill(0);
        ei.cost[0] = 0.0;
        for c in ei.cost[1..=n].iter_mut() {
            *c = f(FBIG);
        }
        for i in 0..n {
            if i & 0xFFFF == 0 {
                self.poll()?;
            }
            let c0 = ei.cost[i];
            let p = px[i];
            let mut lit = None;
            if ei.cbits != 0 {
                if i != 0 {
                    let q = px[i - 1];
                    ei.cache[(q.wrapping_mul(HASH_MUL) >> ksh) as usize] = q;
                }
                let k = (p.wrapping_mul(HASH_MUL) >> ksh) as usize;
                if ei.cache[k] == p {
                    lit = Some(ei.model[HG_G + 280 + k]);
                }
            }
            let lc = match lit {
                Some(v) => v,
                None => {
                    let md = &ei.model;
                    let mut x = md[HG_G + ((p >> 8) & 0xFF) as usize];
                    x += md[HG_R + ((p >> 16) & 0xFF) as usize];
                    x += md[HG_B + (p & 0xFF) as usize];
                    x += md[HG_A + (p >> 24) as usize];
                    x
                }
            };
            let lc = lc + c0;
            if lc < ei.cost[i + 1] {
                ei.cost[i + 1] = lc;
                ei.step[i + 1] = 1;
            }
            let v = ei.m[i];
            if v == 0 {
                continue;
            }
            let ll = ((v >> 20) + 1) as usize;
            let d = self.plane(v & 0xF_FFFF, w);
            let (dc, nb, _) = prefix(d);
            let base = (ei.model[HG_D + dc as usize] + nb as i32 as f32) + c0;
            let cap = ll.min(DP_LCAP);
            for l in 3..=cap {
                let x = ei.model[HG_N + l] + base;
                if x < ei.cost[i + l] {
                    ei.cost[i + l] = x;
                    ei.step[i + l] = l as u16;
                }
            }
            if ll > cap {
                let x = ei.model[HG_N + ll] + base;
                if x < ei.cost[i + ll] {
                    ei.cost[i + ll] = x;
                    ei.step[i + ll] = ll as u16;
                }
            }
        }
        // arrival lengths -> forward steps (in place)
        let mut j = n;
        let mut l = ei.step[n] as usize;
        loop {
            let s = j - l;
            if s == 0 {
                ei.step[0] = l as u16;
                break;
            }
            let t = ei.step[s] as usize;
            ei.step[s] = l as u16;
            j = s;
            l = t;
        }
        // distances of the copies
        let mut pos = 0usize;
        while pos < n {
            ei.pd[pos] = ei.m[pos] & 0xF_FFFF;
            pos += ei.step[pos] as usize;
        }
        Ok(())
    }

    // le_ei_cache: colour cache size (0..cachemax bits) with the smallest estimated cost
    // of green / red / blue / alpha over the current parse
    fn ei_cache(&mut self, ei: &mut Ei) {
        let maxb = self.cachemax as usize;
        // per b: green 256 + lengths 24 + cache codes 2^b | red | blue | alpha | the cache
        let mut ch = vec![CacheHist::ZERO; maxb + 1];
        let px = ei.argb;
        let mut pos = 0usize;
        while pos < ei.n {
            let s = ei.step[pos] as usize;
            let cnt;
            if s <= 1 {
                let p = px[pos];
                let key = p.wrapping_mul(HASH_MUL);
                for (b, c) in ch.iter_mut().enumerate() {
                    if b != 0 {
                        let k = (key >> (32 - b)) as usize & 1023;
                        if c.cache[k] == p {
                            c.g[280 + k] += 1;
                            continue;
                        }
                    }
                    c.g[((p >> 8) & 0xFF) as usize] += 1;
                    c.rba[((p >> 16) & 0xFF) as usize] += 1;
                    c.rba[256 + (p & 0xFF) as usize] += 1;
                    c.rba[512 + (p >> 24) as usize] += 1;
                }
                cnt = 1;
            } else {
                let (c, _, _) = prefix(s as u32);
                for h in ch.iter_mut() {
                    h.g[256 + (c as usize & 31)] += 1;
                }
                cnt = s;
            }
            // insert the pixels into every cache
            for &q in &px[pos..pos + cnt] {
                let key = q.wrapping_mul(HASH_MUL);
                for (b, c) in ch.iter_mut().enumerate().skip(1) {
                    c.cache[(key >> (32 - b)) as usize & 1023] = q;
                }
            }
            pos += cnt;
        }
        let mut best = 0u32;
        let mut bestc = f(FBIG);
        for b in 0..=maxb {
            let gs = if b == 0 { 280 } else { 280 + (1 << b) };
            let mut c = self.hufcost(&ch[b].g[..gs], None);
            for k in 0..3 {
                c += self.hufcost(&ch[b].rba[k * 256..k * 256 + 256], None);
            }
            if ei.level == 0 {
                self.dbgc[b] = c;
            }
            if c < bestc {
                bestc = c;
                best = b as u32;
            }
        }
        let t = (self.flags >> WEF_T_CACHE_SHIFT) & 15; // test: forced cache bits
        ei.cbits = if t != 0 { t - 1 } else { best };
    }

    // le_hufcost: bits of a (+ b) coded with its own length-limited Huffman code (exact)
    // plus libwebp's FinalHuffmanCost estimate of the code description
    fn hufcost(&mut self, a: &[u32], b: Option<&[u32]>) -> f32 {
        let n = a.len();
        let mut tmp = core::mem::take(&mut self.hw.tmp);
        let src: &[u32] = match b {
            Some(b) => {
                for i in 0..n {
                    tmp[i] = a[i].wrapping_add(b[i]);
                }
                &tmp[..n]
            }
            None => a,
        };
        let mut len = core::mem::take(&mut self.hw.len);
        let used = hf_lengths(src, 15, &mut len, &mut self.ht);
        let mut bits = 0u64;
        if used > 1 {
            for i in 0..n {
                bits += src[i] as u64 * len[i] as u64;
            }
        }
        let e = self.l2.entropy(src, None, 2);
        self.hw.len = len;
        self.hw.tmp = tmp;
        e + bits as i64 as f32
    }

    // le_hcost: libwebp-style population cost of the five alphabets of the histogram a (+
    // b); huff: le_hufcost of a alone
    fn hcost(&mut self, ei: &Ei, a: &[u32], b: Option<&[u32]>, huff: bool) -> f32 {
        let mut c = 0f32;
        for (k, &(off, size0)) in ALPHA_OFF.iter().enumerate() {
            let mut size = size0;
            if k == 0 && ei.cbits != 0 {
                size += 1 << ei.cbits;
            }
            let aa = &a[off..off + size];
            let x = if huff {
                self.hufcost(aa, None)
            } else {
                self.l2.entropy(aa, b.map(|b| &b[off..off + size]), 1)
            };
            c += x;
        }
        c
    }

    // le_ei_meta: prefix-code groups: one for sub-images and low effort, else tile
    // histograms (hbits) clustered like libwebp's VP8LGetHistoImageSymbols (entropy bins
    // with the combine cost factor, then greedy pair merging while it saves bits)
    fn ei_meta(&mut self, ei: &mut Ei) {
        ei.ngrp = 1;
        if ei.level == 0 && self.effort >= 4 && self.flags & WEF_T_NOMETA == 0 {
            let hb = self.hbits;
            ei.hbits = hb;
            let m = (1u32 << hb) - 1;
            ei.tw = (ei.w + m) >> hb;
            ei.th = (ei.h + m) >> hb;
            let t = (ei.tw * ei.th) as usize;
            if t >= 2 && self.meta_cluster(ei, t) {
                return;
            }
        }
        // single group
        ei.ngrp = 1;
        let mut hist = vec![0u32; HG_N];
        self.ei_walk(ei, &mut hist, 0);
        ei.hist = hist;
    }

    // the clustering of le_ei_meta (true: the groups are set up; false: a single group)
    fn meta_cluster(&mut self, ei: &mut Ei, t: usize) -> bool {
        let mut th = vec![0u32; t * HG_N];
        self.ei_walk(ei, &mut th, 1);
        let mut cost = vec![0f32; t];
        let mut clus = vec![0u32; t];
        let mut bin = vec![0u32; t];
        let mut comp = vec![0f32; 3 * t];
        let mut reps = [0u32; 64];
        let mut firsts = [u32::MAX; 64];
        let mut mat = vec![0f32; 4096];
        // combine cost factor 0.16 / 2^(size > 256, 512, 1024) / 2 (quality <= 50) below 90
        let mut fac = f(F016);
        let q = self.qual;
        if q < 90 {
            let h = f(F05);
            if t > 256 {
                fac *= h;
                if t > 512 {
                    fac *= h;
                    if t > 1024 {
                        fac *= h;
                    }
                }
            }
            if q <= 50 {
                fac *= h;
            }
        }
        // per tile: cost, green / red / blue costs, empty -> clus -1
        let mut mn = [f(FBIG); 3];
        let mut mx = [0f32; 3];
        for i in 0..t {
            let h = &th[i * HG_N..(i + 1) * HG_N];
            if h.iter().all(|&c| c == 0) {
                clus[i] = u32::MAX;
                cost[i] = 0.0;
                continue;
            }
            clus[i] = i as u32;
            cost[i] = self.hcost(ei, h, None, false);
            for k in 0..3 {
                let (off, size0) = ALPHA_OFF[k];
                let mut size = size0;
                if k == 0 && ei.cbits != 0 {
                    size += 1 << ei.cbits;
                }
                let c = self.l2.entropy(&h[off..off + size], None, 1);
                comp[3 * i + k] = c;
                mn[k] = rt::fx::minss(mn[k], c);
                mx[k] = rt::fx::maxss(mx[k], c);
            }
        }
        // bin = (g * 4 + r) * 4 + b with each index (4 - 1e-6) * (v - min) / (max - min)
        for i in 0..t {
            let mut b = 0u32;
            for k in 0..3 {
                let v = comp[3 * i + k] - mn[k];
                let r = mx[k] - mn[k];
                let mut idx = 0u32;
                if r > 0.0 {
                    idx = rt::fx::cvtt((v * f(F4M)) / r) as u32;
                    if idx > 3 {
                        idx = 3;
                    }
                }
                b = b * 4 + idx;
            }
            bin[i] = b;
        }
        // entropy-bin merging into the first tile of each bin
        for i in 0..t {
            if clus[i] == u32::MAX {
                continue;
            }
            let b = bin[i] as usize;
            let fi = firsts[b];
            if fi == u32::MAX {
                firsts[b] = i as u32;
                continue;
            }
            let fi = fi as usize;
            let comb = {
                let (lo, hi) = th.split_at(i * HG_N);
                self.hcost(ei, &lo[fi * HG_N..(fi + 1) * HG_N], Some(&hi[..HG_N]), false)
            };
            let diff = (comb - cost[fi]) - cost[i];
            let thr = 0.0 - cost[i] * fac;
            if diff < thr {
                cost[fi] = comb;
                hadd(&mut th, fi, i);
                clus[i] = fi as u32;
            }
        }
        // representatives (clus[t] == t)
        let k = (0..t).filter(|&i| clus[i] == i as u32).count();
        if k > 64 {
            // too many: merge every representative into the first of its bin
            for i in 0..t {
                if clus[i] != i as u32 {
                    continue;
                }
                let fi = firsts[bin[i] as usize] as usize;
                if fi == i {
                    continue;
                }
                hadd(&mut th, fi, i);
                clus[i] = fi as u32;
                cost[fi] = self.hcost(ei, &th[fi * HG_N..(fi + 1) * HG_N], None, false);
            }
        }
        let mut kk = 0usize;
        for i in 0..t {
            if clus[i] == i as u32 && kk < 64 {
                reps[kk] = i as u32;
                kk += 1;
            }
        }
        if kk == 0 {
            return false;
        }
        if self.flags & WEF_T_NOGREEDY == 0 {
            // greedy merging: merge deltas of all pairs
            for i in 0..kk {
                for j in i + 1..kk {
                    mat[i * 64 + j] = self.mt_pair(ei, &th, &cost, &reps, i, j);
                }
            }
            loop {
                let mut bestv = f(FBIG);
                let mut bi = usize::MAX;
                let mut bj = 0usize;
                for i in 0..kk {
                    for j in i + 1..kk {
                        let v = mat[i * 64 + j];
                        if v < bestv {
                            bestv = v;
                            bi = i;
                            bj = j;
                        }
                    }
                }
                if bi == usize::MAX || !(bestv < 0.0) {
                    break;
                }
                // merge rep j into rep i
                let (ri, rj) = (reps[bi] as usize, reps[bj] as usize);
                hadd(&mut th, ri, rj);
                cost[ri] = (cost[ri] + cost[rj]) + bestv;
                clus[rj] = ri as u32;
                // remove j: move the last cluster into slot j (with its matrix entries)
                kk -= 1;
                if bj != kk {
                    reps[bj] = reps[kk];
                    for i in 0..kk {
                        if i == bj {
                            continue;
                        }
                        let (a, b) = if i < bj { (i, bj) } else { (bj, i) };
                        mat[a * 64 + b] = self.mt_pair(ei, &th, &cost, &reps, a, b);
                    }
                }
                // recompute the pairs of the merged cluster i
                for i in 0..kk {
                    if i == bi {
                        continue;
                    }
                    let (a, b) = if i < bi { (i, bi) } else { (bi, i) };
                    mat[a * 64 + b] = self.mt_pair(ei, &th, &cost, &reps, a, b);
                }
            }
        }
        // group ids: reps in order; tile -> representative (follow the chain)
        let mut gid = bin; // the bin area is reused
        for i in 0..kk {
            gid[reps[i] as usize] = i as u32;
        }
        let mut gmap = vec![0u32; t];
        for i in 0..t {
            let mut g = 0;
            if clus[i] != u32::MAX {
                let mut r = i;
                while clus[r] as usize != r {
                    r = clus[r] as usize;
                }
                g = gid[r];
            }
            gmap[i] = g;
        }
        ei.gmap = gmap;
        ei.ngrp = kk as u32;
        drop(th);
        if kk <= 1 {
            return false;
        }
        let mut hist = vec![0u32; kk * HG_N];
        self.ei_walk(ei, &mut hist, 2);
        ei.hist = hist;
        if self.effort >= 5 && self.flags & WEF_T_NOGREEDY == 0 {
            self.ei_remap(ei);
        }
        true
    }

    // .mt_pair: cost(rep_i + rep_j) - cost_i - cost_j
    fn mt_pair(&mut self, ei: &Ei, th: &[u32], cost: &[f32], reps: &[u32; 64], i: usize, j: usize) -> f32 {
        let (ri, rj) = (reps[i] as usize, reps[j] as usize);
        let c = self.hcost(ei, &th[ri * HG_N..(ri + 1) * HG_N], Some(&th[rj * HG_N..(rj + 1) * HG_N]), false);
        (c - cost[ri]) - cost[rj]
    }

    // le_ei_remap: libwebp HistogramRemap-like refinement: every tile moves to the group
    // whose code (bit costs of the group histograms) codes it cheapest; empty groups are
    // dropped and the group histograms rebuilt
    fn ei_remap(&mut self, ei: &mut Ei) {
        let t = (ei.tw * ei.th) as usize;
        let k = ei.ngrp as usize;
        if k > 64 {
            return;
        }
        let mut th = vec![0u32; t * HG_N];
        self.ei_walk(ei, &mut th, 1);
        // cost tables of the groups: log2(total / count), unused log2(total) + 2, 0 for a
        // single-symbol alphabet
        let mut co = vec![0f32; k * HG_N];
        for g in 0..k {
            let hist = &ei.hist[g * HG_N..(g + 1) * HG_N];
            let c = &mut co[g * HG_N..(g + 1) * HG_N];
            for (a, &(off, size0)) in ALPHA_OFF.iter().enumerate() {
                let mut size = size0;
                if a == 0 && ei.cbits != 0 {
                    size += 1 << ei.cbits;
                }
                let mut tot = 0u64;
                let mut used = 0u32;
                for &v in &hist[off..off + size] {
                    tot += v as u64;
                    if v != 0 {
                        used += 1;
                    }
                }
                let l2t = if tot != 0 { self.l2.log2f(tot as u32) } else { 0.0 };
                for i in 0..size {
                    let v = hist[off + i];
                    c[off + i] = if v == 0 {
                        l2t + f(F2)
                    } else if used > 1 {
                        l2t - self.l2.log2f(v)
                    } else {
                        0.0
                    };
                }
            }
        }
        let mut tf = vec![0f32; HG_N];
        for i in 0..t {
            let h = &th[i * HG_N..(i + 1) * HG_N];
            let mut any = 0u32;
            for (d, &v) in tf.iter_mut().zip(h.iter()) {
                any |= v;
                *d = v as f32;
            }
            if any == 0 {
                continue; // empty tiles keep their group
            }
            let mut best = f(FBIG);
            let mut bg = 0u32;
            for g in 0..k {
                let c = &co[g * HG_N..(g + 1) * HG_N];
                // 4 lanes (mulps / addps), then (l0 + l2) + (l1 + l3)
                let mut acc = [0f32; 4];
                for j in (0..HG_N).step_by(4) {
                    for l in 0..4 {
                        acc[l] += tf[j + l] * c[j + l];
                    }
                }
                let s = (acc[0] + acc[2]) + (acc[1] + acc[3]);
                if s < best {
                    best = s;
                    bg = g as u32;
                }
            }
            ei.gmap[i] = bg;
        }
        // drop groups without tiles: new ids in order of first use
        let mut new = [u32::MAX; 64];
        let mut next = 0u32;
        for i in 0..t {
            let g = ei.gmap[i] as usize;
            if g <= 63 && new[g] == u32::MAX {
                new[g] = next;
                next += 1;
            }
            ei.gmap[i] = new[g & 63];
        }
        ei.ngrp = next;
        let mut hist = core::mem::take(&mut ei.hist);
        hist[..next as usize * HG_N].fill(0);
        self.ei_walk(ei, &mut hist, if next > 1 { 2 } else { 0 });
        ei.hist = hist;
    }

    // le_ei_write: colour cache / meta headers, the prefix codes of every group and the
    // symbols of the parse
    fn ei_write(&mut self, bw: &mut Bw, ei: &mut Ei) -> Result<(), i32> {
        let gs = 280 + if ei.cbits != 0 { 1usize << ei.cbits } else { 0 };
        if ei.level == 0 {
            // colour cache
            if ei.cbits != 0 {
                bw.put(ei.cbits << 1 | 1, 5);
            } else {
                bw.put(0, 1);
            }
            // meta prefix codes
            if ei.ngrp > 1 {
                bw.put((ei.hbits - 2) << 1 | 1, 4);
                let img: Vec<u32> = ei.gmap.iter().map(|&g| g << 8).collect();
                self.encode_image(bw, &img, ei.tw, ei.th, 1)?;
            } else {
                bw.put(0, 1);
            }
        } else {
            bw.put(0, 1); // (sub-image) no colour cache
        }
        let ng = ei.ngrp as usize;
        let mut codes = vec![0u32; ng * HG_N];
        for g in 0..ng {
            for (a, &(off, size0)) in ALPHA_OFF.iter().enumerate() {
                let size = if a == 0 { gs } else { size0 };
                let h = &ei.hist[g * HG_N + off..g * HG_N + off + size];
                let t = &mut codes[g * HG_N + off..g * HG_N + off + size];
                hf_make(bw, h, t, &mut self.hw, &mut self.ht);
            }
        }
        let px = ei.argb;
        let cb = ei.cbits;
        let ksh = 32u32.wrapping_sub(cb);
        ei.cache.fill(0);
        let (mut x, mut y) = (0u32, 0u32);
        let mut pos = 0usize;
        while pos < ei.n {
            if pos & 0xFFFF == 0 {
                self.poll()?;
            }
            let ct = if ng > 1 {
                let t = ((y >> ei.hbits) * ei.tw + (x >> ei.hbits)) as usize;
                ei.gmap[t] as usize * HG_N
            } else {
                0
            };
            let ct = &codes[ct..ct + HG_N];
            let s = ei.step[pos] as usize;
            let cnt;
            if s <= 1 {
                let p = px[pos];
                let mut hit = false;
                if cb != 0 {
                    let k = (p.wrapping_mul(HASH_MUL) >> ksh) as usize;
                    if ei.cache[k] == p {
                        let e = ct[HG_G + 280 + k];
                        bw.put(e & 0xFFFF, e >> 16);
                        hit = true;
                    }
                }
                if !hit {
                    let g = ct[HG_G + ((p >> 8) & 0xFF) as usize];
                    let r = ct[HG_R + ((p >> 16) & 0xFF) as usize];
                    bw.put((g & 0xFFFF) | (r & 0xFFFF) << (g >> 16), (g >> 16) + (r >> 16));
                    let b = ct[HG_B + (p & 0xFF) as usize];
                    let a = ct[HG_A + (p >> 24) as usize];
                    bw.put((b & 0xFFFF) | (a & 0xFFFF) << (b >> 16), (b >> 16) + (a >> 16));
                }
                cnt = 1;
            } else {
                let (c, nb, ex) = prefix(s as u32);
                let e = ct[HG_G + 256 + c as usize];
                let l = e >> 16;
                bw.put((e & 0xFFFF) | ex << l, l + nb);
                let d = self.plane(ei.pd[pos], ei.w);
                let (dc, dnb, dex) = prefix(d);
                let e = ct[HG_D + dc as usize];
                bw.put(e & 0xFFFF, e >> 16);
                bw.put(dex, dnb);
                cnt = s;
            }
            if cb != 0 {
                for &q in &px[pos..pos + cnt] {
                    ei.cache[(q.wrapping_mul(HASH_MUL) >> ksh) as usize] = q;
                }
            }
            pos += cnt;
            x += cnt as u32;
            while x >= ei.w {
                x -= ei.w;
                y += 1;
            }
        }
        Ok(())
    }
}

// le_hadd: hist dst += src (HG_N counts each, inside th)
fn hadd(th: &mut [u32], dst: usize, src: usize) {
    for k in 0..HG_N {
        th[dst * HG_N + k] = th[dst * HG_N + k].wrapping_add(th[src * HG_N + k]);
    }
}

// le_ei_greedy: libwebp BackwardReferencesLz77: take the match at i unless a later start
// inside it reaches further (then cut it there); matches below 4 pixels are literals
fn ei_greedy(ei: &mut Ei) {
    let n = ei.n;
    let m = &ei.m;
    let mut i = 0usize;
    while i < n {
        let v = m[i];
        let mut step = 1usize;
        if v != 0 {
            let len = ((v >> 20) + 1) as usize;
            if len >= 4 {
                let jmax = (i + len).min(n - 1);
                let mut reach = 0usize;
                for j in i + 1..=jmax {
                    let vj = m[j];
                    let mut lj = 1usize;
                    if vj != 0 {
                        let l = ((vj >> 20) + 1) as usize;
                        if l >= 4 {
                            lj = l;
                        }
                    }
                    let r = j + lj;
                    if r > reach {
                        reach = r;
                        step = j - i;
                        if reach >= n {
                            break;
                        }
                    }
                }
            }
        }
        ei.step[i] = step as u16;
        ei.pd[i] = v & 0xF_FFFF;
        i += step;
    }
}

// le_ei_rle: libwebp BackwardReferencesRle: copies only from the previous pixel or the
// pixel above (the longer, >= 4), else literals
fn ei_rle(ei: &mut Ei) {
    let px = ei.argb;
    let n = ei.n;
    let w = ei.w as usize;
    ei.step[0] = 1;
    let mut i = 1usize;
    while i < n {
        let maxl = (n - i).min(LZ_MAXLEN);
        let run = mlen(&px[i..], &px[i - 1..], maxl);
        let above = if i >= w { mlen(&px[i..], &px[i - w..], maxl) } else { 0 };
        if run >= above && run >= 4 {
            ei.step[i] = run as u16;
            ei.pd[i] = 1;
            i += run;
        } else if above >= 4 {
            ei.step[i] = above as u16;
            ei.pd[i] = w as u32;
            i += above;
        } else {
            ei.step[i] = 1;
            i += 1;
        }
    }
}

// le_ei_swap: exchange the current and the alternative parse
fn ei_swap(ei: &mut Ei) {
    core::mem::swap(&mut ei.step, &mut ei.step2);
    core::mem::swap(&mut ei.pd, &mut ei.pd2);
}

