// evdct -- the new lossy writer: one VarDCT frame in XYB, as libjxl 0.11.1 at the same effort
// (enc_frame.cc ComputeVarDCTEncodingData / EncodeGroups, enc_heuristics.cc
// LossyFrameHeuristics, enc_cache.cc InitializePassesEncoder, enc_group.cc
// ComputeCoefficients, enc_coeff_order.cc, enc_entropy_coder.cc)          [jxl-plus]
//
//   encode_lossy(px, opts) -> codestream
//
// Effort -> tools (libjxl speed tiers): 3 and below: DCT8 everywhere, one quant field value;
// 5+: adaptive quantization field, AC strategy search (DCT2..DCT64, AFV, identity), CfL per
// 64x64 tile, gaborish; 4+: near-lossless DC (WP quantized, one extra bit); 5+ (Hare): quant
// field per block adjustment; 6+: full strategy search; 7: initial CfL, block context map,
// custom coefficient orders, pixel based chroma scaling; EPF strength by distance.  Effort 8 / 9
// add learned MA trees for the DC / metadata images, LZ77 and (jxl-tools) the Butteraugli
// quantization loop (ebq.rs: 2 / 4 round trips of the frame).
// Groups run on the worker pool.

use alloc::vec;
use alloc::vec::Vec;

use rt::fx::maxss;
use rt::sync::SpinLock;

use crate::eaq::{self, Img, Quantizer};
use crate::eacs::{self, Cfg, Scratch};
use crate::eans::{self, pack_signed, HParams, Token};
use crate::ecfl;
use crate::efrm::{self, Colour, Dims, FrameHdr, ImgHdr, Passes};
use crate::ema::{Kind, PRED_GRADIENT, PRED_WEIGHTED};
use crate::emath::roundf;
use crate::emod::{self, par_map, MOpts, Stream};
use crate::enew::{lossless_opts, tier, Opts, Px, IOF_JXL_PROGRESSIVE};
use crate::etx::{self, Qm, Tx};
use crate::vdc::Qe;
use crate::ewr::Jw;
use crate::modular::{jm_predict, Mc, Mi, Wp, WH_DEF};
use crate::tab;
use crate::venc::{cbrt, distance, v};

static D_GS: [u32; 8] = [1, 11, 2049, 11, 4097, 12, 8193, 16];
static D_QDC: [u32; 8] = [16, 0, 1, 5, 1, 8, 1, 16];
static D_ORDER: [u32; 8] = [0x5F, 0, 0x13, 0, 0, 0, 0, 13];
static D_DCTHR: [u32; 8] = [0, 4, 16, 8, 272, 16, 65808, 32];
static D_QFTHR: [u32; 8] = [0, 2, 4, 3, 12, 5, 44, 8];
static DEFCTX: [u8; 39] = [
    0, 1, 2, 2, 3, 3, 4, 5, 6, 6, 6, 6, 6, 7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14, 7, 8, 9, 9, 10, 11, 12, 13, 14,
    14, 14, 14, 14,
];
const NUM_ORDERS: usize = 13;
const COLOR_SCALE: f32 = 1.0 / 84.0;
const Y_TO_B: f32 = 1.0;

/// the block context map (BlockCtxMap)
#[derive(Clone)]
struct Bcm {
    qft: Vec<u32>,
    map: Vec<u8>,
    nctx: u32,
}

impl Bcm {
    fn default_map() -> Bcm {
        Bcm { qft: Vec::new(), map: DEFCTX.to_vec(), nctx: 15 }
    }
    #[inline]
    fn ctx(&self, qf: u32, ord: usize, c: usize) -> u32 {
        let mut qi = 0usize;
        for &t in &self.qft {
            if qf > t {
                qi += 1;
            }
        }
        let ci = if c < 2 { c ^ 1 } else { 2 };
        let idx = (ci * NUM_ORDERS + ord) * (self.qft.len() + 1) + qi;
        self.map[idx] as u32
    }
}

/// AdjustQuantBias with the default biases
#[inline]
fn adj(q: i32, c: usize) -> f32 {
    let qf = q as f32;
    let a = qf.abs();
    if a < 1.125 {
        if a > 0.0 {
            let b = tab::OPSIN_DEF[12 + c];
            if qf > 0.0 { b } else { -b }
        } else {
            0.0
        }
    } else {
        qf - tab::OPSIN_DEF[15] / qf
    }
}

/// linear sRGB -> XYB of the 8-bit image, padded to whole blocks by repetition (bands of rows
/// straight into the planes on the pool)
fn to_xyb(px: &Px, threads: bool) -> Img {
    let (w, h) = (px.w as usize, px.h as usize);
    let pw = w.div_ceil(8) * 8;
    let ph = h.div_ceil(8) * 8;
    let mut img = Img::new(pw, ph);
    let [p0, p1, p2] = &mut img.p;
    emod::par_rows(vec![(&mut p0[..], pw), (&mut p1[..], pw), (&mut p2[..], pw)], ph, 16, threads, |y0, n, b| {
        let cb = cbrt(v(9));
        let [o0, o1, o2] = b else { return };
        for yy in 0..n {
            // rows below the image repeat the last one
            let y = (y0 + yy).min(h - 1);
            let r = yy * pw;
            for x in 0..w {
                let p = &px.rgba[(y * w + x) * 4..];
                let rr = tab::SRGBLIN[p[0] as usize];
                let gg = tab::SRGBLIN[p[1] as usize];
                let bb = tab::SRGBLIN[p[2] as usize];
                let mut l = [0f32; 3];
                for (i, li) in l.iter_mut().enumerate() {
                    let mut m = v(3 * i) * rr + v(3 * i + 1) * gg;
                    m = m + v(3 * i + 2) * bb;
                    m = m + v(9);
                    m = maxss(m, 0.0);
                    *li = cbrt(m) - cb;
                }
                o0[r + x] = (l[0] - l[1]) * v(11);
                o1[r + x] = (l[0] + l[1]) * v(11);
                o2[r + x] = l[2];
            }
            for o in [&mut *o0, &mut *o1, &mut *o2] {
                let last = o[r + w - 1];
                for x in w..pw {
                    o[r + x] = last;
                }
            }
        }
    });
    img
}

/// the 8-bit image as YCbCr planes (Cb, Y - 128 / 255, Cr: the X, Y, B slots), padded to whole
/// blocks by repetition  [jxl-plus]
fn to_ycc(px: &Px) -> Img {
    let (w, h) = (px.w as usize, px.h as usize);
    let pw = w.div_ceil(8) * 8;
    let ph = h.div_ceil(8) * 8;
    let mut img = Img::new(pw, ph);
    let k = 1.0f32 / 255.0;
    for y in 0..ph {
        for x in 0..pw {
            let p = &px.rgba[((y.min(h - 1)) * w + x.min(w - 1)) * 4..];
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let i = y * pw + x;
            img.p[1][i] = (0.299 * r + 0.587 * g + 0.114 * b) * k - 128.0 / 255.0;
            img.p[0][i] = (-0.168_736 * r - 0.331_264 * g + 0.5 * b) * k;
            img.p[2][i] = (0.5 * r - 0.418_688 * g - 0.081_312 * b) * k;
        }
    }
    img
}

/// the quantization weights of the YCbCr path: the library DCT8 table with the Y weights for
/// all channels, chroma coarser by YCC_CHROMA; DC weights alike
const YCC_CHROMA: f32 = 0.5;
/// the distance of the YCbCr path relative to the XYB one
const YCC_DIST: f32 = 2.5;

fn ycc_tables() -> (Vec<Qe>, [f32; 3]) {
    let mut qe: Vec<Qe> = (0..17).map(Qe::library).collect();
    let q0 = &mut qe[0];
    let nb = q0.nb as usize;
    for i in 0..nb {
        let yv = q0.b1[16 + i];
        q0.b1[i] = yv;
        q0.b1[32 + i] = yv;
    }
    q0.b1[0] *= YCC_CHROMA;
    q0.b1[32] *= YCC_CHROMA;
    (qe, [512.0 * YCC_CHROMA, 512.0, 512.0 * YCC_CHROMA])
}

/// SimplifyInvisible (lossy): invisible pixels become a smear of their visible neighbours
fn simplify_invisible(img: &mut Img, px: &Px) {
    let (w, h) = (px.w as usize, px.h as usize);
    let s = img.w;
    let a = |x: usize, y: usize| px.rgba[(y * w + x) * 4 + 3] as f32;
    for c in 0..3 {
        let p = &mut img.p[c];
        for y in 0..h {
            for x in 0..w {
                if a(x, y) != 0.0 {
                    continue;
                }
                let mut d = 0.0f32;
                let mut v = 0.0f32;
                if x > 0 {
                    v += p[y * s + x - 1];
                    d += 1.0;
                    if a(x - 1, y) > 0.0 {
                        v += p[y * s + x - 1];
                        d += 1.0;
                    }
                }
                if x + 1 < w {
                    if y > 0 {
                        v += p[(y - 1) * s + x + 1];
                        d += 1.0;
                    }
                    if a(x + 1, y) > 0.0 {
                        v += 2.0 * p[y * s + x + 1];
                        d += 2.0;
                    }
                    if y > 0 && a(x + 1, y - 1) > 0.0 {
                        v += 2.0 * p[(y - 1) * s + x + 1];
                        d += 2.0;
                    }
                    if y + 1 < h && a(x + 1, y + 1) > 0.0 {
                        v += 2.0 * p[(y + 1) * s + x + 1];
                        d += 2.0;
                    }
                }
                if y > 0 && a(x, y - 1) > 0.0 {
                    v += 2.0 * p[(y - 1) * s + x];
                    d += 2.0;
                }
                if y + 1 < h && a(x, y + 1) > 0.0 {
                    v += 2.0 * p[(y + 1) * s + x];
                    d += 2.0;
                }
                if d > 1.0 {
                    v /= d;
                }
                p[y * s + x] = v;
            }
        }
    }
}

/// ComputeChromacityAdjustments -> (x_qm_scale, b_qm_scale)
fn chroma_scales(img: &Img, w: usize, h: usize, d: f32, t: i32) -> (u32, u32) {
    let mut xqm = 3u32;
    for s in [2.5f32, 5.5, 9.5] {
        if d > s {
            xqm += 1;
        }
    }
    let (mut dx, mut db, mut eb) = (0f32, 0f32, 0f32);
    if t <= 3 {
        let s = img.w;
        let (p0, p1, p2) = (&img.p[0], &img.p[1], &img.p[2]);
        let (mut xm, mut ym) = (0f32, 0f32);
        for ty in 1..h {
            for tx in 1..w {
                let cur = p0[ty * s + tx];
                xm = xm.max((cur - p0[ty * s + tx - 1]).abs());
                ym = ym.max((cur - p0[(ty - 1) * s + tx]).abs());
            }
        }
        dx = xm.max(ym);
        let (mut xm, mut ym) = (0f32, 0f32);
        for ty in 1..h {
            for tx in 1..w {
                let cy = p1[ty * s + tx];
                let cb = p2[ty * s + tx];
                let mut exposed = cb - cy * 1.2;
                let diff_b = cb - cy;
                let prev_row = p2[(ty - 1) * s + tx];
                let prev = p2[ty * s + tx - 1];
                let dpr = prev_row - p1[(ty - 1) * s + tx];
                let dp = prev - p1[ty * s + tx - 1];
                xm = xm.max((diff_b - dp).abs());
                ym = ym.max((diff_b - dpr).abs());
                if exposed >= 0.0 {
                    exposed *= (cb - prev).abs() + (cb - prev_row).abs();
                    eb = eb.max(exposed);
                }
            }
        }
        db = xm.max(ym);
    }
    let xpix = if dx >= 0.026 {
        3
    } else if dx >= 0.022 {
        2
    } else if dx >= 0.015 {
        1
    } else {
        0
    };
    let add = (eb >= 0.13) as u32;
    let bpix = if db > 0.38 {
        2 + add
    } else if db > 0.33 {
        1 + add
    } else if db > 0.28 {
        add
    } else {
        0
    };
    (xqm.max(2 + xpix), 2 + bpix)
}

/// FindBestBlockEntropyModel (efforts 4+)
fn block_model(d: f32, rqf: &[i32], acs: &[u8]) -> Bcm {
    let tot = rqf.len();
    let size_ctx = ((1u32 << 10) as f32 * d) as usize;
    if tot < size_ctx {
        return Bcm::default_map();
    }
    let mut qf_counts = vec![0usize; 256];
    let mut qf_ord = vec![[0usize; 256]; NUM_ORDERS];
    for i in 0..tot {
        let ord = tab::STRATEGYORDER[(acs[i] >> 1) as usize] as usize;
        let q = (rqf[i] - 1).clamp(0, 255) as usize;
        qf_counts[q] += 1;
        qf_ord[ord][q] += 1;
    }
    let size_qf = ((1u32 << 13) as f32 * d) as usize;
    let nseg = if tot < size_qf { 1 } else { 2 };
    let mut qft: Vec<u32> = Vec::new();
    let mut cum = 0usize;
    let mut next = 1usize;
    let mut last_cut = 256usize;
    let mut cut = tot * next / nseg;
    for j in 0..256u32 {
        cum += qf_counts[j as usize];
        if cum > cut {
            if j != 0 {
                qft.push(j);
            }
            last_cut = j as usize;
            while cum > cut {
                next += 1;
                cut = tot * next / nseg;
            }
        } else if next > qft.len() + 1 && j as usize == last_cut.wrapping_add(1) && j != 0 {
            qft.push(j);
        }
    }
    let nq = qft.len() + 1;
    let mut counts = vec![0usize; NUM_ORDERS * nq];
    let mut qp = 0usize;
    for j in 0..256usize {
        if qp < qft.len() && j as u32 == qft[qp] {
            qp += 1;
        }
        for (i, row) in qf_ord.iter().enumerate() {
            counts[qp + i * nq] += row[j];
        }
    }
    let n = nq * NUM_ORDERS;
    let mut remap: Vec<usize> = (0..n).collect();
    let mut clusters: Vec<usize> = (0..n).collect();
    let nb = ((tot / size_ctx.max(1) / 2) as i64).clamp(2, 9) as usize;
    let nbc = ((tot / size_ctx.max(1) / 3) as i64).clamp(1, 5) as usize;
    while clusters.len() > nb {
        clusters.sort_by(|&a, &b| counts[b].cmp(&counts[a]));
        let l = clusters.len();
        counts[clusters[l - 2]] += counts[clusters[l - 1]];
        counts[clusters[l - 1]] = 0;
        remap[clusters[l - 1]] = clusters[l - 2];
        clusters.pop();
    }
    for i in 0..n {
        while remap[remap[i]] != remap[i] {
            remap[i] = remap[remap[i]];
        }
    }
    let mut rr = vec![n; n];
    let mut num = 0usize;
    for i in 0..n {
        if rr[remap[i]] == n {
            rr[remap[i]] = num;
            num += 1;
        }
        remap[i] = rr[remap[i]];
    }
    let mut map: Vec<u8> = remap.iter().map(|&v| v as u8).collect();
    map.resize(n * 3, 0);
    for i in n..n * 3 {
        map[i] = (num + (remap[i % n]).min(nbc - 1)) as u8;
    }
    let nctx = *map.iter().max().unwrap_or(&0) as u32 + 1;
    Bcm { qft, map, nctx }
}

/// the encoder state shared by the group workers
struct St<'a> {
    img: &'a Img,
    qm: &'a Qm,
    q: Quantizer,
    xb: usize,
    yb: usize,
    acs: Vec<u8>,
    rqf: Vec<i32>,
    ytox: Vec<i8>,
    ytob: Vec<i8>,
    cw: usize,
    tier: i32,
    xmul: f32,
    bmul: f32,
    base_b: f32, // Y-to-B base correlation (1 for XYB, 0 for YCbCr)
}

/// QuantizeBlockAC
#[allow(clippy::too_many_arguments)]
fn quantize_block(st: &St, c: usize, mult: f32, acs: usize, xs: usize, ys: usize, thr: &mut [f32; 4], inp: &[f32], quant: i32, out: &mut [i32]) {
    let (_, qm) = st.qm.of(acs, c);
    let qac = st.q.scale * quant as f32;
    if c != 1 && xs * ys >= 4 {
        for t in thr.iter_mut() {
            *t -= 0.007_44 * (xs * ys) as f32;
            if *t < 0.5 {
                *t = 0.5;
            }
        }
    }
    let qv = qac * mult;
    for y in 0..ys * 8 {
        let yfix = (y >= ys * 4) as usize * 2;
        let off = y * 8 * xs;
        for x in 0..xs * 8 {
            let th = if xs == 1 { thr[yfix + (x >= 4) as usize] } else { thr[yfix + (x >= xs * 4) as usize] };
            let qq = qm[off + x] * qv;
            let val = qq * inp[off + x];
            out[off + x] = if val.abs() >= th { crate::emath::rintf(val) as i32 } else { 0 };
        }
    }
}

/// AdjustQuantBlockAC
#[allow(clippy::too_many_arguments)]
fn adjust_quant_block(st: &St, c: usize, mult: f32, acs: usize, xs: usize, ys: usize, thr: &mut [f32; 4], inp: &[f32], quant: &mut i32) {
    if matches!(acs, 1 | 2 | 3 | 12 | 13 | 14 | 15 | 16 | 17) {
        return;
    }
    let (_, qm) = st.qm.of(acs, c);
    let qac = st.q.scale * *quant as f32;
    if xs > 1 || ys > 1 {
        for t in thr.iter_mut() {
            *t -= (0.003 * (xs * ys) as f32).clamp(0.0, 0.08);
            if *t < 0.54 {
                *t = 0.54;
            }
        }
    }
    let mut hf_sum = 0f32;
    let mut sum_err = 0f32;
    let mut sum_vals = 0f32;
    let mut hfnz = [0f32; 4];
    let mut hfmax = [0f32; 4];
    for y in 0..ys * 8 {
        for x in 0..xs * 8 {
            let pos = y * 8 * xs + x;
            if x < xs && y < ys {
                continue;
            }
            let hfix = (y >= ys * 4) as usize * 2 + (x >= xs * 4) as usize;
            let val = inp[pos] * (qm[pos] * qac * mult);
            let v = if val.abs() < thr[hfix] { 0.0 } else { crate::emath::rintf(val) };
            let err = (val - v).abs();
            sum_err += err;
            sum_vals += v.abs();
            if c == 1 && v == 0.0 && hfmax[hfix] < err {
                hfmax[hfix] = err;
            }
            if v != 0.0 {
                hfnz[hfix] += v.abs();
                let corner = y >= 7 * ys && x >= 7 * xs;
                let border = y == ys * 8 - 1 || x == xs * 8 - 1;
                let larger = x >= 4 * xs && y >= 4 * ys;
                if corner || (border && larger) {
                    hf_sum += val.abs();
                }
            }
        }
    }
    if c == 1 && sum_vals * 8.0 < (xs * ys) as f32 {
        let orig = *quant;
        let mut nq = *quant;
        let big = |i: usize| hfnz[i] == 0.0 && hfmax[i] as f64 > 0.46;
        for i in 1..4 {
            if big(i) {
                nq = orig + 1;
                break;
            }
        }
        *quant = nq;
        let sc = |e: f32| (0.9999 * e as f64 * nq as f64 / orig as f64) as f32;
        if big(3) {
            thr[3] = sc(hfmax[3]);
        } else if big(1) || big(2) {
            thr[1] = sc(hfmax[1].max(hfmax[2]));
            thr[2] = thr[1];
        } else if big(0) {
            thr[0] = sc(hfmax[0]);
        }
    }
    {
        let all = hfnz[0] + hfnz[1] + hfnz[2] + hfnz[3] + 1.0;
        let mul = [70.0f32, 30.0, 60.0];
        if mul[c] * hf_sum >= all {
            *quant = (*quant as f32 + mul[c] * hf_sum / all) as i32;
            if *quant >= 256 {
                *quant = 255;
            }
        }
    }
    if acs == etx::DCT && hfnz[0] + hfnz[1] + hfnz[2] + hfnz[3] < 11.0 {
        *quant += 1;
        if *quant >= 256 {
            *quant = 255;
        }
    }
    {
        static K1: [[f64; 3]; 4] = [
            [0.220_806_157_538_484_04, 0.457_974_798_242_620_1, 0.298_592_350_959_779_65],
            [0.701_094_865_102_868_3, 0.161_852_813_055_126_4, 0.143_876_917_300_354_73],
            [0.114_985_964_456_218_64, 0.446_568_404_410_276_95, 0.105_876_582_151_490_48],
            [0.468_496_652_644_093_96, 0.412_390_779_377_819_54, 0.088_667_407_767_185_44],
        ];
        static K2: [[f64; 3]; 4] = [
            [0.274_502_819_418_221_97, 1.125_576_654_998_499_6, 0.989_504_591_341_283_9],
            [0.465_216_867_559_828_5, 0.409_458_079_834_558_2, 0.365_818_998_117_513_67],
            [0.280_349_724_247_157_15, 0.918_265_320_192_973_8, 1.558_153_154_305_741_6],
            [0.268_731_181_140_337_3, 0.688_637_123_903_924_8, 1.208_218_540_866_678_6],
        ];
        let qn = 2.294_270_834_328_472_f64;
        let se = (sum_err as f64 * qn) as f32 as f64;
        let sv = (sum_vals as f64 * qn) as f32 as f64;
        if acs >= etx::DCT16X16 {
            let ix = if acs == etx::DCT32X16 || acs == etx::DCT16X32 {
                1
            } else if acs == etx::DCT16X16 {
                0
            } else if acs == etx::DCT32X32 {
                2
            } else {
                3
            };
            let lim = K1[ix][c] * (xs * ys * 64) as f64 + K2[ix][c] * sv;
            let mut step = (se / lim) as i32;
            if step >= 2 {
                step = 2;
            }
            if step < 0 {
                step = 0;
            }
            if se > lim {
                *quant += step;
                if *quant >= 256 {
                    *quant = 255;
                }
            }
        }
    }
    {
        let div = (xs * ys) as i32;
        let mut activity = (hfnz[0] as i32 + div / 2) / div;
        let lim = 4.max(*quant / 2);
        for v in hfnz.iter().skip(1) {
            activity = activity.min((*v as i32 + div / 2) / div);
        }
        if activity >= 15 {
            activity = 15;
        }
        let mut qp = *quant - activity;
        if c == 1 {
            for t in thr.iter_mut().skip(1) {
                *t = (*t as f64 + 0.01 * activity as f64) as f32;
            }
        }
        if qp < lim {
            qp = lim;
        }
        *quant = qp;
    }
}

/// the coefficients of one group: quantized coefficients per channel, DC values of its
/// blocks, the final quant field values of its first blocks
struct GroupOut {
    coef: Vec<[Vec<i32>; 3]>, // per pass
    dc: [Vec<f32>; 3], // gw x gh blocks of the group
    rqf: Vec<i32>,
}

/// the buffers of coefficients (reused from group to group)
struct CoefScratch {
    tx: Tx,
    cin: Vec<f32>,
    qz: Vec<i32>,
}

impl CoefScratch {
    fn new() -> CoefScratch {
        CoefScratch { tx: Tx::new(), cin: vec![0f32; 3 * 4096], qz: vec![0i32; 3 * 4096] }
    }
}

/// ComputeCoefficients of group g
fn coefficients(st: &St, dims: &Dims, g: usize, cs: &mut CoefScratch) -> GroupOut {
    let gx = g % dims.xg as usize;
    let gy = g / dims.xg as usize;
    let bx0 = gx * 32;
    let by0 = gy * 32;
    let gw = (st.xb - bx0).min(32);
    let gh = (st.yb - by0).min(32);
    let n = gw * gh * 64;
    let mut out = GroupOut {
        coef: vec![[Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)]],
        dc: [vec![0.0; gw * gh], vec![0.0; gw * gh], vec![0.0; gw * gh]],
        rqf: vec![0; gw * gh],
    };
    let CoefScratch { tx, cin, qz } = cs;
    let stride = st.img.w;
    let mut dcb = [0f32; 64];
    for by in 0..gh {
        for bx in 0..gw {
            let ax = bx0 + bx;
            let ay = by0 + by;
            out.rqf[by * gw + bx] = st.rqf[ay * st.xb + ax];
            let a8 = st.acs[ay * st.xb + ax];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            let (cx, cy) = etx::cover(a);
            let (ys, xs) = (cx.min(cy), cx.max(cy));
            let size = 64 * cx * cy;
            let ti = (ay / 8) * st.cw + ax / 8;
            let xf = st.ytox[ti] as f32 * COLOR_SCALE;
            let bf = st.base_b + st.ytob[ti] as f32 * COLOR_SCALE;
            let mut quant = st.rqf[ay * st.xb + ax];
            let o = ay * 8 * stride + ax * 8;
            for c in 0..3 {
                etx::from_pixels(tx, a, &st.img.p[c][o..], stride, &mut cin[c * size..(c + 1) * size]);
            }
            etx::dc_from_llf(tx, a, &cin[size..2 * size], &mut dcb, cx);
            for iy in 0..cy {
                for ix in 0..cx {
                    out.dc[1][(by + iy) * gw + bx + ix] = dcb[iy * cx + ix];
                }
            }
            // QuantizeRoundtripYBlockAC
            let mut thr_y = [0.58f32, 0.64, 0.64, 0.64];
            if st.tier <= 5 {
                let orig = quant;
                let mut mxq = 0;
                let mul = [st.xmul, 1.0, st.bmul];
                for c in [1usize, 0, 2] {
                    let mut th = [0.58f32, 0.64, 0.64, 0.64];
                    quant = orig;
                    adjust_quant_block(st, c, mul[c], a, xs, ys, &mut th, &cin[c * size..(c + 1) * size], &mut quant);
                    if c == 1 {
                        thr_y = th;
                    }
                    mxq = mxq.max(quant);
                }
                quant = mxq;
            } else {
                thr_y = [0.56, 0.62, 0.62, 0.62];
            }
            {
                let (inp, rest) = cin.split_at_mut(size);
                let _ = inp;
                let ych = &mut rest[..size];
                let yq = &mut qz[size..2 * size];
                quantize_block(st, 1, 1.0, a, xs, ys, &mut thr_y, ych, quant, yq);
                let (m, _) = st.qm.of(a, 1);
                let iq = st.q.inv_quant_ac(quant);
                for k in 0..size {
                    ych[k] = adj(yq[k], 1) * m[k] * iq;
                }
            }
            // unapply the colour correlation
            for k in 0..size {
                let y = cin[size + k];
                cin[k] -= xf * y;
                cin[2 * size + k] -= bf * y;
            }
            for c in [0usize, 2] {
                let mut th = [0.58f32, 0.62, 0.62, 0.62];
                let mult = if c == 0 { st.xmul } else { st.bmul };
                let (lo, hi) = qz.split_at_mut(c * size);
                let _ = lo;
                quantize_block(st, c, mult, a, xs, ys, &mut th, &cin[c * size..(c + 1) * size], quant, &mut hi[..size]);
                etx::dc_from_llf(tx, a, &cin[c * size..(c + 1) * size], &mut dcb, cx);
                for iy in 0..cy {
                    for ix in 0..cx {
                        out.dc[c][(by + iy) * gw + bx + ix] = dcb[iy * cx + ix];
                    }
                }
            }
            out.rqf[by * gw + bx] = quant;
            for c in 0..3 {
                out.coef[0][c].extend_from_slice(&qz[c * size..(c + 1) * size]);
            }
        }
    }
    out
}

/// QuantizeWP of one DC value
#[inline]
fn quantize_wp(d: &[i32], w: usize, x: usize, y: usize, wp: &mut Wp, value: f32, inv: f32) -> i32 {
    let nb = nb7(d, w, x, y);
    let g = wp.predict(x as u32, y as u32, &[nb[1], nb[0], nb[3], nb[2], nb[5]], None);
    let mut sv = value * inv;
    sv -= g as f32;
    if sv > -0.62 && sv < 0.62 {
        sv = 0.0;
    }
    let mut r = roundf(sv) as i32;
    if !(-2..=2).contains(&r) {
        r = roundf(sv * 0.5) as i32 * 2;
    }
    r + g as i32
}

/// neighbours (left, top, topleft, topright, leftleft, toptop, toprightright)
#[inline]
fn nb7(d: &[i32], w: usize, x: usize, y: usize) -> [i32; 7] {
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

/// AddVarDCTDC: the quantized DC image of DC group rect (x0, y0, w, h in blocks)
fn dc_image(q: &Quantizer, dc: &[Vec<f32>; 3], xb: usize, r: [usize; 4], nl: bool, base_b: f32) -> Mi {
    let (x0, y0, w, h) = (r[0], r[1], r[2], r[3]);
    let mul = if nl { 2.0f32 } else { 1.0 };
    let mut ch: [Vec<i32>; 3] = [vec![0; w * h], vec![0; w * h], vec![0; w * h]];
    for c in [1usize, 0, 2] {
        let inv = q.inv_dc_step(c) * mul;
        let yf = q.dc_step(1) / mul;
        let cfl = if c == 0 { 0.0 } else { base_b };
        let oc = if c < 2 { c ^ 1 } else { c };
        let mut wp = Wp::new(&WH_DEF, w as u32).ok();
        for y in 0..h {
            for x in 0..w {
                let v = dc[c][(y0 + y) * xb + x0 + x];
                let value = if c == 1 { v } else { v - ch[0][y * w + x] as f32 * (yf * cfl) };
                let qv = if nl {
                    let d = &ch[oc];
                    let wst = wp.as_mut().expect("wp");
                    let r = quantize_wp(d, w, x, y, wst, value, inv);
                    wst.update(r, x as u32, y as u32);
                    r
                } else {
                    roundf(value * inv) as i32
                };
                ch[oc][y * w + x] = qv;
            }
        }
    }
    let mut mi = Mi::new(w as u32, h as u32, 8);
    let [a, b, c] = ch;
    for d in [a, b, c] {
        mi.ch.push(Mc { w: w as u32, h: h as u32, hs: 0, vs: 0, d });
    }
    mi
}

/// natural coefficient orders and Lehmer coding
fn natural(acs: usize) -> Vec<u16> {
    let mut o = vec![0u16; etx::size_of(acs)];
    crate::vac::jv_natorder(acs, &mut o);
    o
}

/// the coefficient orders: per order (13) and channel the scan, the used and customized masks
struct Orders {
    ord: Vec<Vec<u16>>, // [13 * 3]
    used: u32,          // orders coded
}

/// ComputeUsedOrders + ComputeCoeffOrder
fn compute_orders(t: i32, st: &St, dims: &Dims, groups: &[GroupOut], pass: usize, threads: bool) -> Orders {
    let mut used_acs = 0u32;
    let mut ret = 0u32;
    let mut cust = 0u32;
    for &a8 in &st.acs {
        let a = (a8 >> 1) as usize;
        used_acs |= 1 << a;
        let o = tab::STRATEGYORDER[a] as u32;
        ret |= 1 << o;
        if o <= 6 {
            cust |= 1 << o;
        }
    }
    if t >= 7 || (st.xb < 5 && st.yb < 5) {
        cust = 0;
    }
    let mut ord: Vec<Vec<u16>> = vec![Vec::new(); NUM_ORDERS * 3];
    // zero counts per order, channel and coefficient
    let mut nz: Vec<Vec<i32>> = vec![Vec::new(); NUM_ORDERS * 3];
    if cust != 0 {
        let frac = if t >= 3 && ret == 1 { 0.5f64 } else { 1.0 };
        let thr = ((u64::MAX >> 32) as f64 * frac) as u64;
        // the sampling generator advances once per varblock in group order: its state at the
        // start of every group first (serial, cheap), then the zero counts of each group on
        // the pool and their sums in group order (jxl-speed; integer sums, the same counts).
        // The lowest frequencies end as -1 once a block of the order was sampled.
        let first = |g: usize, f: &mut dyn FnMut(usize, usize)| {
            let gx = g % dims.xg as usize;
            let gy = g / dims.xg as usize;
            let (bx0, by0) = (gx * 32, gy * 32);
            let gw = (st.xb - bx0).min(32);
            let gh = (st.yb - by0).min(32);
            let mut off = 0usize;
            for by in 0..gh {
                for bx in 0..gw {
                    let a8 = st.acs[(by0 + by) * st.xb + bx0 + bx];
                    if a8 & 1 == 0 {
                        continue;
                    }
                    let a = (a8 >> 1) as usize;
                    f(a, off);
                    off += etx::size_of(a);
                }
            }
        };
        #[inline(always)]
        fn step(s: &mut [u64; 2]) -> u64 {
            let mut s1 = s[0];
            let s0 = s[1];
            let bits = s1.wrapping_add(s0);
            s[0] = s0;
            s1 ^= s1 << 23;
            s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5);
            s[1] = s1;
            bits
        }
        let mut s: [u64; 2] = [0x94D0_49BB_1331_11EB, 0xBF58_476D_1CE4_E5B9];
        let mut starts = Vec::with_capacity(groups.len());
        for g in 0..groups.len() {
            starts.push(s);
            first(g, &mut |_, _| {
                step(&mut s);
            });
        }
        let counts: Vec<Vec<Vec<i32>>> = par_map(groups.len(), threads, |g| {
            let go = &groups[g];
            let mut s = starts[g];
            let mut nzg: Vec<Vec<i32>> = vec![Vec::new(); NUM_ORDERS * 3];
            first(g, &mut |a, off| {
                let bits = step(&mut s);
                if (bits >> 32) <= thr {
                    let o = tab::STRATEGYORDER[a] as usize;
                    let size = etx::size_of(a);
                    for c in 0..3 {
                        let v = &mut nzg[o * 3 + c];
                        if v.is_empty() {
                            v.resize(size, 0);
                        }
                        let blk = &go.coef[pass][c][off..off + size];
                        for (vk, &q) in v.iter_mut().zip(blk) {
                            *vk += (q == 0) as i32;
                        }
                    }
                }
            });
            nzg
        });
        for nzg in counts {
            for (v, part) in nz.iter_mut().zip(nzg) {
                if part.is_empty() {
                    continue;
                }
                if v.is_empty() {
                    *v = part;
                } else {
                    for (a, b) in v.iter_mut().zip(&part) {
                        *a += b;
                    }
                }
            }
        }
        for a in 0..27usize {
            let o = tab::STRATEGYORDER[a] as usize;
            let (mn, mx) = etx::llf_dims(a);
            for c in 0..3 {
                let v = &mut nz[o * 3 + c];
                if v.is_empty() || v.len() != etx::size_of(a) {
                    continue;
                }
                for iy in 0..mn {
                    for ix in 0..mx {
                        v[iy * 8 * mx + ix] = -1;
                    }
                }
            }
        }
    }
    let mut done = 0u32;
    let mut used = 0u32;
    for a in 0..27usize {
        let o = tab::STRATEGYORDER[a] as usize;
        if done & (1 << o) != 0 {
            continue;
        }
        done |= 1 << o;
        if used_acs & (1 << a) == 0 && (ret & (1 << o)) == 0 {
            continue;
        }
        let nat = natural(a);
        let sz = nat.len();
        if cust & (1 << o) == 0 || nz[o * 3].is_empty() {
            for c in 0..3 {
                ord[o * 3 + c] = nat.clone();
            }
            continue;
        }
        let inv = 1.0f32 / crate::emath::sqrtf(sz as f32);
        let mut nondef = false;
        for c in 0..3 {
            let v = &nz[o * 3 + c];
            let mut pc: Vec<(u16, u32)> = nat.iter().map(|&p| (p, (v[p as usize] as f32 * inv + 0.1) as i32 as u32)).collect();
            pc.sort_by_key(|e| e.1);
            let o2: Vec<u16> = pc.iter().map(|e| e.0).collect();
            if o2 != nat {
                nondef = true;
            }
            ord[o * 3 + c] = o2;
        }
        if nondef {
            used |= 1 << o;
        }
    }
    Orders { ord, used }
}

/// Lehmer code of a permutation
fn lehmer(p: &[u32]) -> Vec<u32> {
    let n = p.len();
    let mut tmp = vec![0u32; n + 1];
    let mut code = vec![0u32; n];
    for (idx, &s) in p.iter().enumerate() {
        let mut pen = 0u32;
        let mut i = s + 1;
        while i != 0 {
            pen += tmp[i as usize];
            i &= i - 1;
        }
        code[idx] = s - pen;
        let mut i = s + 1;
        while (i as usize) < n + 1 {
            tmp[i as usize] += 1;
            i += i & i.wrapping_neg();
        }
    }
    code
}

/// EncodeCoeffOrders
fn write_orders(jw: &mut Jw, o: &Orders) {
    jw.u32(o.used, &D_ORDER);
    if o.used == 0 {
        return;
    }
    let mut toks: Vec<Token> = Vec::new();
    let mut done = 0u32;
    for a in 0..27usize {
        let ord = tab::STRATEGYORDER[a] as usize;
        if done & (1 << ord) != 0 {
            continue;
        }
        done |= 1 << ord;
        if o.used & (1 << ord) == 0 {
            continue;
        }
        let nat = natural(a);
        let size = nat.len();
        let llf = size / 64;
        let mut lut = vec![0u32; size];
        for (i, &p) in nat.iter().enumerate() {
            lut[p as usize] = i as u32;
        }
        for c in 0..3 {
            let zz: Vec<u32> = o.ord[ord * 3 + c].iter().map(|&p| lut[p as usize]).collect();
            let code = lehmer(&zz);
            let mut end = size;
            while end > llf && code[end - 1] == 0 {
                end -= 1;
            }
            toks.push(Token::new(crate::frm::tc_ctx(size as u32), (end - llf) as u32));
            let mut last = 0u32;
            for &v in &code[llf..end] {
                toks.push(Token::new(crate::frm::tc_ctx(last), v));
                last = v;
            }
        }
    }
    let mut st = vec![toks];
    let code = eans::build(&HParams::default(), 8, &mut st, Some(jw));
    eans::write_tokens(&st[0], &code, jw);
}

/// the progressive passes (libjxl progressive_passes_dc_vlf_lf_full_ac): coefficients per
/// block side kept in each pass, no shifts
const PASS_COEFS: [usize; 3] = [2, 3, 8];

/// SplitACCoefficients of group g: the coefficients of pass 0 split into the passes
fn split_passes(st: &St, dims: &Dims, g: usize, go: &mut GroupOut) {
    let gx = g % dims.xg as usize;
    let gy = g / dims.xg as usize;
    let (bx0, by0) = (gx * 32, gy * 32);
    let gw = (st.xb - bx0).min(32);
    let gh = (st.yb - by0).min(32);
    let full = core::mem::take(&mut go.coef[0]);
    let n = full[0].len();
    go.coef = (0..PASS_COEFS.len()).map(|_| [vec![0i32; n], vec![0i32; n], vec![0i32; n]]).collect();
    let mut off = 0usize;
    for by in 0..gh {
        for bx in 0..gw {
            let a8 = st.acs[(by0 + by) * st.xb + bx0 + bx];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            let (cx, cy) = etx::cover(a);
            let (ys, xs) = (cx.min(cy), cx.max(cy));
            let mut done = 1usize;
            for (p, &nc) in PASS_COEFS.iter().enumerate() {
                for y in 0..ys * nc {
                    for x in 0..xs * nc {
                        if x < xs * done && y < ys * done {
                            continue;
                        }
                        let pos = off + y * xs * 8 + x;
                        for c in 0..3 {
                            go.coef[p][c][pos] = full[c][pos];
                        }
                    }
                }
                done = nc;
            }
            off += 64 * cx * cy;
        }
    }
}

/// TokenizeCoefficients of group g
fn tokenize(st: &St, dims: &Dims, g: usize, go: &GroupOut, pass: usize, orders: &Orders, bcm: &Bcm) -> Vec<Token> {
    let gx = g % dims.xg as usize;
    let gy = g / dims.xg as usize;
    let (bx0, by0) = (gx * 32, gy * 32);
    let gw = (st.xb - bx0).min(32);
    let gh = (st.yb - by0).min(32);
    let mut nzs = vec![0i32; 3 * 1024];
    let mut out: Vec<Token> = Vec::with_capacity(gw * gh * 64);
    let nctx = bcm.nctx;
    let mut off = 0usize;
    for by in 0..gh {
        for bx in 0..gw {
            let bi = (by0 + by) * st.xb + bx0 + bx;
            let a8 = st.acs[bi];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            let log2 = tab::ACSLOG2[a] as u32;
            let size = 64usize << log2;
            let cov = 1u32 << log2;
            let (cx, cy) = etx::cover(a);
            let ob = tab::STRATEGYORDER[a] as usize;
            for c in [1usize, 0, 2] {
                let nzp = c * 1024;
                let idx = by * 32 + bx;
                let pred = if bx != 0 {
                    let l = nzs[nzp + idx - 1] as u32;
                    if by != 0 { (l + nzs[nzp + idx - 32] as u32 + 1) >> 1 } else { l }
                } else if by != 0 {
                    nzs[nzp + idx - 32] as u32
                } else {
                    32
                };
                let qf = go.rqf[by * gw + bx] as u32;
                let bctx = bcm.ctx(qf, ob, c);
                let order = &orders.ord[ob * 3 + c];
                let blk = &go.coef[pass][c][off..off + size];
                let mut nzeros = 0u32;
                for k in cov as usize..size {
                    if blk[order[k] as usize] != 0 {
                        nzeros += 1;
                    }
                }
                let v = (nzeros + cov - 1) >> log2;
                for yy in 0..cy {
                    for xx in 0..cx {
                        let p = (by + yy) * 32 + bx + xx;
                        if p < 1024 {
                            nzs[nzp + p] = v as i32;
                        }
                    }
                }
                let mut e = pred.min(64);
                if e >= 8 {
                    e = (e >> 1) + 4;
                }
                out.push(Token::new(e * nctx + bctx, nzeros));
                let hbase = bctx * 458 + nctx * 37;
                let mut nzr = nzeros;
                let mut prev = (nzr <= (size as u32) >> 4) as u32;
                let mut k = cov as usize;
                while k < size && nzr != 0 {
                    let a1 = tab::NZCTX[((nzr + cov - 1) >> log2) as usize & 63] as u32;
                    let a2 = tab::FREQCTX[(k >> log2) & 63] as u32;
                    let ctx = (a1 + a2) * 2 + prev + hbase;
                    let co = blk[order[k] as usize];
                    out.push(Token::new(ctx, pack_signed(co)));
                    prev = (co != 0) as u32;
                    nzr -= prev;
                    k += 1;
                }
            }
            off += size;
        }
    }
    out
}

/// the VarDCT codestream
pub fn encode_lossy(px: &Px, o: &Opts) -> Result<Vec<u8>, i32> {
    let e = o.effort();
    let t = tier(e);
    let threads = o.threads();
    let quality = if o.quality != 0 { o.quality } else { 90 };
    let d = distance(quality);
    let (w, h) = (px.w as usize, px.h as usize);
    let dims = Dims::new(px.w, px.h, 1);
    let xb = dims.xb as usize;
    let yb = dims.yb as usize;
    // an ICC profile: sRGB-like ones coded as sRGB (XYB, no profile: decoders without colour
    // management output XYB images with a profile in linear sRGB), others as YCbCr of the
    // profile's values with the profile embedded
    let (colour, gray_icc, icc, _) = crate::enew::colour_of(px, o.icc);
    let ycc = icc.is_some_and(|p| !crate::eicc::icc_srgb_like(p));
    let (colour, gray_icc, icc) = if ycc { (colour, gray_icc, icc) } else { (Colour::Srgb, false, None) };
    // heuristics tier: the YCbCr path uses the plain ones (constant quant field, DCT8)
    let t_entropy = t;
    // YCbCr: about the size of the XYB path at the same quality (it spends its bits on PSNR)
    let d = if ycc { d * YCC_DIST } else { d };
    let t = if ycc { t.max(7) } else { t };
    // ---- XYB / YCbCr
    let mut img = if ycc { to_ycc(px) } else { to_xyb(px, threads) };
    if px.alpha {
        simplify_invisible(&mut img, px);
    }
    // jxl-tools: patches out of the XYB image into a reference frame (libjxl: effort 7+)
    let patches = if !ycc && o.patches(false) { lossy_patches(px, &mut img, d, t, threads, o)? } else { None };
    let gab = (t <= 5 || ycc) && d > 0.5;
    let mut epf = 0u32;
    for th in [0.7f32, 1.5, 4.0] {
        if d >= th {
            epf += 1;
        }
    }
    if ycc {
        epf = 0;
    }
    let (xqm, bqm) = if ycc { (2, 2) } else { chroma_scales(&img, w, h, d, t) };
    // ---- quant field, inverse gaborish
    let quant_dc = eaq::initial_quant_dc(d);
    let mut q = Quantizer::new();
    let (qes, dcw) = if ycc { ycc_tables() } else { ((0..17).map(Qe::library).collect(), eaq::DC_WEIGHTS) };
    q.dcw = dcw;
    let (mut qf, m1): (Vec<f32>, Vec<f32>);
    if t > 5 {
        let qv = 0.79 / d;
        qf = vec![qv; xb * yb];
        m1 = Vec::new();
        q.compute(quant_dc, qv, 0.0);
    } else {
        let d_iqf = if gab { d } else { d * 0.62 };
        let (a, _mask, b) = eaq::aq_map(d_iqf, &img, eaq::AC_QUANT / d_iqf, threads);
        qf = a;
        m1 = b;
        q.compute(quant_dc, 0.39 / d, 0.0);
    }
    // EPF sharpness per block from reconstructions (ComputeARHeuristics, efforts 6+)
    let ar = !ycc && t <= 4 && d >= 0.5 && epf > 0 && !m1.is_empty();
    let orig = if ar { Some(img.clone()) } else { None };
    if gab {
        eaq::gaborish_inverse(&mut img, threads);
    }
    if o.cancelled() {
        return Err(img_core::ERR_CANCEL);
    }
    let qm = Qm::with(&qes);
    // ---- CfL and AC strategies per 64x64 tile
    let tw = xb.div_ceil(8);
    let th = yb.div_ceil(8);
    let mut acs = vec![1u8; xb * yb]; // DCT8, first
    let mut ytox = vec![0i8; tw * th];
    let mut ytob = vec![0i8; tw * th];
    if t <= 5 {
        let cfg = Cfg::new(&img, &qm, &qf, &m1, d);
        let tiles: Vec<(i8, i8, [u8; 64])> = emod::par_map_with(tw * th, threads, || (Scratch::new(), ecfl::Buf::new()), |(sc, cb_), k| {
            let (ttx, tty) = (k % tw, k / tw);
            let (cx, cb) = if t <= 3 {
                ecfl::tile(&mut sc.tx, cb_, &img, &qm, None, None, &q, xb, yb, ttx, tty, false)
            } else {
                (0, 0)
            };
            let fx = cx as f32 * COLOR_SCALE;
            let fb = Y_TO_B + cb as f32 * COLOR_SCALE;
            let a = eacs::tile(&cfg, sc, ttx, tty, fx, fb, t, d);
            (cx, cb, a)
        });
        for (k, (cx, cb, a)) in tiles.iter().enumerate() {
            let (ttx, tty) = (k % tw, k / tw);
            ytox[k] = *cx;
            ytob[k] = *cb;
            for iy in 0..8 {
                for ix in 0..8 {
                    let (x, y) = (ttx * 8 + ix, tty * 8 + iy);
                    if x < xb && y < yb {
                        acs[y * xb + x] = a[iy * 8 + ix];
                    }
                }
            }
        }
    }
    eaq::adjust_quant_field(&acs, xb, yb, d, &mut qf);
    let rqf: Vec<i32> = qf.iter().map(|&v| q.raw(v)).collect();
    if t <= 5 {
        let fast = t >= 4;
        let acsr = &acs;
        let rr = &rqf;
        let imgr = &img;
        let res: Vec<(i8, i8)> = emod::par_map_with(tw * th, threads, || (Tx::new(), ecfl::Buf::new()), |(tx, cb), k| {
            ecfl::tile(tx, cb, imgr, &qm, Some(acsr), Some(rr), &q, xb, yb, k % tw, k / tw, fast)
        });
        for (k, (a, b)) in res.into_iter().enumerate() {
            ytox[k] = a;
            ytob[k] = b;
        }
    }
    // jxl-tools: everything from the block context model on: once per Butteraugli round trip at
    // efforts 8 / 9 (last = false: EPF sharpness 4, no photon noise) and once for the file
    let tail = |q: Quantizer, mut rqf: Vec<i32>, last: bool| -> Result<Vec<u8>, i32> {
    let bcm = if t < 7 { block_model(d, &rqf, &acs) } else { Bcm::default_map() };
    if o.cancelled() {
        return Err(img_core::ERR_CANCEL);
    }
    // ---- coefficients per group
    let st = St {
        img: &img,
        qm: &qm,
        q,
        xb,
        yb,
        acs: acs.clone(),
        rqf: rqf.clone(),
        ytox: ytox.clone(),
        ytob: ytob.clone(),
        cw: tw,
        tier: t,
        xmul: crate::emath::fast_powf(1.25, xqm as f32 - 2.0),
        bmul: crate::emath::fast_powf(1.25, bqm as f32 - 2.0),
        base_b: if ycc { 0.0 } else { Y_TO_B },
    };
    let ng = dims.ng as usize;
    let prog = o.flags & IOF_JXL_PROGRESSIVE != 0;
    let np = if prog { PASS_COEFS.len() } else { 1 };
    let dcfr = prog && !ycc; // progressive DC: a DC frame (XYB only)
    let mut groups: Vec<GroupOut> = emod::par_map_with(ng, threads, CoefScratch::new, |cs, g| coefficients(&st, &dims, g, cs));
    if np > 1 {
        for (g, go) in groups.iter_mut().enumerate() {
            split_passes(&st, &dims, g, go);
        }
    }
    // the full DC image and the final quant field
    let mut dc: [Vec<f32>; 3] = [vec![0.0; xb * yb], vec![0.0; xb * yb], vec![0.0; xb * yb]];
    for (g, go) in groups.iter().enumerate() {
        let gx = g % dims.xg as usize;
        let gy = g / dims.xg as usize;
        let (bx0, by0) = (gx * 32, gy * 32);
        let gw = (xb - bx0).min(32);
        let gh = (yb - by0).min(32);
        for by in 0..gh {
            for bx in 0..gw {
                let i = (by0 + by) * xb + bx0 + bx;
                for c in 0..3 {
                    dc[c][i] = go.dc[c][by * gw + bx];
                }
                rqf[i] = go.rqf[by * gw + bx];
            }
        }
    }
    if o.cancelled() {
        return Err(img_core::ERR_CANCEL);
    }
    // ---- two independent parts at the same time (jxl-speed): (a) the AC entropy coding:
    // orders, tokens, histograms, the AC global section and the AC group sections; (b) the
    // modular streams: DC, alpha, the AC metadata with the default sharpness, the image header
    // and the DC frame
    let nstreams = dims.num_streams(np as u32);
    let nl = t < 7;
    let ndg = dims.ndg as usize;
    let base = lossless_opts(t, nstreams, dims.gdim);
    let hp_mod = HParams::for_modular(t, false, false);
    let meta_opts = MOpts {
        max_chan: 0xFF_FFFF,
        predictor: PRED_GRADIENT,
        wp_tree: 2,
        kind: if t >= 7 {
            Kind::FalconAcMeta
        } else if t > 2 {
            Kind::AcMeta
        } else {
            Kind::Learn
        },
        hp: hp_mod.clone(),
        ..base.clone()
    };
    // AC metadata of DC group g: CfL tiles, strategies + quant field, EPF sharpness
    let meta_stream = |g: usize, sharp: &[u8]| -> Jw {
        let gx = g % dims.xdg as usize;
        let gy = g / dims.xdg as usize;
        let (x0, y0) = (gx * 256, gy * 256);
        let rw = (xb - x0).min(256);
        let rh = (yb - y0).min(256);
        let cw = rw.div_ceil(8);
        let chh = rh.div_ceil(8);
        let mut tx_ = vec![0i32; cw * chh];
        let mut tb_ = vec![0i32; cw * chh];
        for y in 0..chh {
            for x in 0..cw {
                tx_[y * cw + x] = st.ytox[(y0 / 8 + y) * tw + x0 / 8 + x] as i32;
                tb_[y * cw + x] = st.ytob[(y0 / 8 + y) * tw + x0 / 8 + x] as i32;
            }
        }
        let mut av: Vec<i32> = Vec::new();
        let mut qv: Vec<i32> = Vec::new();
        let mut ep = vec![0i32; rw * rh];
        for y in 0..rh {
            for x in 0..rw {
                let i = (y0 + y) * xb + x0 + x;
                ep[y * rw + x] = sharp[i] as i32;
                let a8 = st.acs[i];
                if a8 & 1 == 0 {
                    continue;
                }
                av.push((a8 >> 1) as i32);
                qv.push(rqf[i] - 1);
            }
        }
        let num = av.len();
        let mut both = av;
        both.extend_from_slice(&qv);
        let mut mi = Mi::new(rw as u32, rh as u32, 8);
        mi.ch.push(Mc { w: cw as u32, h: chh as u32, hs: 3, vs: 3, d: tx_ });
        mi.ch.push(Mc { w: cw as u32, h: chh as u32, hs: 3, vs: 3, d: tb_ });
        mi.ch.push(Mc { w: num as u32, h: 2, hs: 0, vs: 0, d: both });
        mi.ch.push(Mc { w: rw as u32, h: rh as u32, hs: 0, vs: 0, d: ep });
        let mut sm = Stream::new(mi, meta_opts.clone());
        emod::encode_local(&mut sm, dims.sid_ac_meta(g as u32) as u32, threads)
    };
    let sharp4 = vec![4u8; xb * yb];
    let (side_a, side_b) = emod::join(
        threads,
        || {
            // ---- orders, tokens
            let orders: Vec<Orders> = (0..np).map(|p| compute_orders(t, &st, &dims, &groups, p, threads)).collect();
            let toks: Vec<Vec<Token>> =
                par_map(ng * np, threads, |k| tokenize(&st, &dims, k % ng, &groups[k % ng], k / ng, &orders[k / ng], &bcm));
            drop(groups);
            let mut acg = Jw::new();
            if ycc {
                // own DCT8 table (mode DCT: bands per channel), the rest from the library
                acg.bits(0, 1);
                let q0 = &qes[0];
                acg.bits(6, 3);
                acg.bits((q0.nb - 1) as u64, 4);
                for c in 0..3 {
                    for i in 0..q0.nb as usize {
                        let v = q0.b1[c * 16 + i];
                        acg.f16(if i == 0 { v / 64.0 } else { v });
                    }
                }
                for _ in 1..17 {
                    acg.bits(0, 3);
                }
            } else {
                acg.bits(1, 1); // library quantization tables
            }
            let hb = crate::eans::ceil_log2(dims.ng);
            if hb != 0 {
                acg.bits(0, hb); // one histogram set
            }
            let te = t_entropy;
            let mut hp = HParams::for_tier(te);
            if te > 1 {
                hp.lz = eans::Lz::None;
            }
            hp.par = threads;
            let nacctx = bcm.nctx as usize * (37 + 458);
            let mut tokv = toks;
            let mut codes = Vec::with_capacity(np);
            for p in 0..np {
                write_orders(&mut acg, &orders[p]);
                let mut tp: Vec<Vec<Token>> = tokv[p * ng..(p + 1) * ng].iter_mut().map(core::mem::take).collect();
                codes.push(eans::build(&hp, nacctx, &mut tp, Some(&mut acg)));
                for (g, tk) in tp.into_iter().enumerate() {
                    tokv[p * ng + g] = tk;
                }
            }
            let acs_parts: Vec<Jw> = par_map(ng * np, threads, |k| {
                let mut s = Jw::new();
                eans::write_tokens(&tokv[k], &codes[k / ng], &mut s);
                s
            });
            (acg, acs_parts)
        },
        || -> Result<_, i32> {
            // ---- modular streams: DC, AC metadata, alpha
            let dc_opts = MOpts {
                max_chan: 0xFF_FFFF,
                predictor: PRED_WEIGHTED,
                wp_tree: 1,
                kind: if t >= 3 { Kind::WpFixedDc } else { Kind::Learn },
                hp: hp_mod.clone(),
                ..base.clone()
            };
            let mut streams: Vec<Option<Stream>> = (0..nstreams).map(|_| None).collect();
            let mut meta_count = vec![0u32; ndg];
            for g in 0..ndg {
                let gx = g % dims.xdg as usize;
                let gy = g / dims.xdg as usize;
                let (x0, y0) = (gx * 256, gy * 256);
                let rw = (xb - x0).min(256);
                let rh = (yb - y0).min(256);
                if !dcfr {
                    let mi = dc_image(&st.q, &dc, xb, [x0, y0, rw, rh], nl, st.base_b);
                    streams[dims.sid_vardct_dc(g as u32)] = Some(Stream::new(mi, dc_opts.clone()));
                }
                meta_count[g] = (0..rh)
                    .map(|y| (0..rw).filter(|&x| st.acs[(y0 + y) * xb + x0 + x] & 1 != 0).count() as u32)
                    .sum();
            }
            if px.alpha {
                let n = w * h;
                let mut full = Mi::new(px.w, px.h, 8);
                full.ch.push(Mc { w: px.w, h: px.h, hs: 0, vs: 0, d: (0..n).map(|i| px.rgba[i * 4 + 3] as i32).collect() });
                let mut ao = lossless_opts(t, nstreams, dims.gdim);
                if t < 7 {
                    ao.predictor = PRED_GRADIENT;
                }
                ao.hp = hp_mod.clone();
                emod::try_palettes(&mut full, t, if t <= 8 { 95.0 } else { 0.0 }, 1024);
                if ng == 1 {
                    streams[0] = Some(Stream::new(full, ao));
                } else {
                    let gd = dims.gdim;
                    for g in 0..ng as u32 {
                        let gx = g % dims.xg;
                        let gy = g / dims.xg;
                        let gi = crop_alpha(&full, [gx * gd, gy * gd, gd, gd], gd);
                        streams[dims.sid_modular_ac(g, np as u32 - 1)] = Some(Stream::new(gi, ao.clone()));
                    }
                    let mut s0 = Stream::new(full, ao);
                    s0.opts.max_chan = gd;
                    streams[0] = Some(s0);
                }
            }
            let ids: Vec<usize> = (0..nstreams).filter(|&i| streams[i].is_some()).collect();
            // the streams and the AC metadata with the default sharpness, together
            let cells: Vec<SpinLock<Option<Stream>>> = ids.iter().map(|&i| SpinLock::new(streams[i].take())).collect();
            let mut coded: Vec<Jw> = par_map(ids.len() + ndg, threads, |k| {
                if k >= ids.len() {
                    return meta_stream(k - ids.len(), &sharp4);
                }
                let mut gd = cells[k].lock();
                match gd.as_mut() {
                    Some(s) => emod::encode_local(s, ids[k] as u32, threads),
                    None => Jw::new(),
                }
            });
            let metas4 = coded.split_off(ids.len());
            let mut parts: Vec<Option<Jw>> = (0..nstreams).map(|_| None).collect();
            for (k, jw) in coded.into_iter().enumerate() {
                parts[ids[k]] = Some(jw);
            }
            // the image header (and the DC frame)
            let mut head = Jw::new();
            let ih = ImgHdr { w: px.w, h: px.h, bits: 8, alpha: px.alpha, xyb: !ycc, colour, gray_icc, icc };
            efrm::write_image(&mut head, &ih);
            if let Some((rf, _)) = patches.as_ref() {
                head.append(rf); // jxl-tools: the patch reference frame
            }
            if dcfr {
                dc_frame(&mut head, px, &st.q, &dc, xb, yb, t_entropy, threads, nl, o)?;
            }
            Ok((parts, meta_count, metas4, head))
        },
    );
    let (acg, mut acs_parts) = side_a;
    let (mut parts, meta_count, metas4, head) = side_b?;
    if o.cancelled() {
        return Err(img_core::ERR_CANCEL);
    }
    // ---- sections
    // jxl-tools: features (splines: test-only; photon noise: XYB only)
    let iso = o.noise_iso();
    let noise = if iso > 0 && !ycc && last { Some(crate::enoise::lut(px.w, px.h, iso as f32)) } else { None };
    let feat = crate::efeat::Feat { patches: patches.as_ref().map(|p| &p.1), splines: o.splines, noise };
    let mut lf = Jw::new();
    feat.write(&mut lf);
    if ycc {
        lf.bits(0, 1); // DC quantization: 1 / weights * 128 as f16
        for c in 0..3 {
            lf.f16(128.0 / dcw[c]);
        }
    } else {
        lf.bits(1, 1); // default DC quantization weights
    }
    lf.u32(st.q.global_scale as u32, &D_GS);
    lf.u32(st.q.quant_dc as u32, &D_QDC);
    if bcm.qft.is_empty() && bcm.map[..] == DEFCTX[..] {
        lf.bits(1, 1);
    } else {
        lf.bits(0, 1);
        for _ in 0..3 {
            lf.bits(0, 4); // no DC thresholds
        }
        lf.bits(bcm.qft.len() as u64, 4);
        for &v in &bcm.qft {
            lf.u32(v - 1, &D_QFTHR);
        }
        eans::encode_ctxmap(&bcm.map, bcm.nctx as usize, &mut lf);
    }
    let _ = D_DCTHR;
    if ycc {
        lf.bits(0, 1); // colour correlation: factor 84, base X 0, base B 0, DC factors 0
        lf.bits(0, 2);
        lf.f16(0.0);
        lf.f16(0.0);
        lf.bits(128, 8);
        lf.bits(128, 8);
    } else {
        lf.bits(1, 1); // default colour correlation
    }
    lf.bits(0, 1); // no global tree
    if let Some(p) = parts[0].take() {
        lf.append(&p);
    }
    // the DC group sections up to their AC metadata
    let mut dcs: Vec<Jw> = Vec::with_capacity(ndg);
    for g in 0..ndg {
        let mut s = Jw::new();
        if !dcfr {
            s.bits(nl as u64, 2); // extra DC precision
            if let Some(p) = parts[dims.sid_vardct_dc(g as u32)].take() {
                s.append(&p);
            }
        }
        if let Some(p) = parts[dims.sid_modular_dc(g as u32)].take() {
            s.append(&p);
        }
        let gx = g % dims.xdg as usize;
        let gy = g / dims.xdg as usize;
        let rw = (xb - gx * 256).min(256);
        let rh = (yb - gy * 256).min(256);
        let nb = crate::eans::ceil_log2((rw * rh) as u32);
        if nb != 0 {
            s.bits((meta_count[g] - 1) as u64, nb);
        }
        dcs.push(s);
    }
    for (k, part) in acs_parts.iter_mut().enumerate() {
        if let Some(p) = parts[dims.sid_modular_ac((k % ng) as u32, (k / ng) as u32)].take() {
            part.append(&p);
        }
    }
    // ---- codestream: the headers (and the DC frame) once, the frame per sharpness map
    let passes = if np > 1 {
        Passes { n: np as u32, shift: vec![0; np - 1], ds: vec![4, 2], last_pass: vec![0, 1] }
    } else {
        Passes { n: 1, ..Passes::default() }
    };
    let fh = FrameHdr {
        modular: false,
        xyb: !ycc,
        ycc,
        flags: (if dcfr { 32 } else { 0 }) | feat.flags(), // kUseDcFrame
        nec: px.alpha as u32,
        xqm,
        bqm,
        passes,
        gab,
        epf,
        ..FrameHdr::default()
    };
    let build = |sharp: &[u8]| -> Vec<u8> {
        // the AC metadata streams: made already for the default sharpness
        let metas_new: Vec<Jw>;
        let metas: &[Jw] = if sharp == &sharp4[..] {
            &metas4
        } else {
            metas_new = par_map(ndg, threads, |g| meta_stream(g, sharp));
            &metas_new
        };
        let sections: Vec<Jw> = if ng == 1 && np == 1 {
            let mut one = Jw::new();
            one.append(&lf);
            one.append(&dcs[0]);
            one.append(&metas[0]);
            one.append(&acg);
            one.append(&acs_parts[0]);
            vec![one]
        } else {
            let mut v = Vec::with_capacity(dims.ntoc(np as u32));
            let mut l = Jw::new();
            l.append(&lf);
            v.push(l);
            for (g, m) in metas.iter().enumerate() {
                let mut s = Jw::new();
                s.append(&dcs[g]);
                s.append(m);
                v.push(s);
            }
            let mut a = Jw::new();
            a.append(&acg);
            v.push(a);
            for p in &acs_parts {
                let mut s = Jw::new();
                s.append(p);
                v.push(s);
            }
            v
        };
        let mut out = Jw::new();
        out.append(&head);
        efrm::assemble(&mut out, &fh, &sections);
        out.pad();
        out.buf
    };
    let mut sharp = sharp4.clone();
    if let Some(orig) = orig.as_ref().filter(|_| last) {
        if o.cancelled() {
            return Err(img_core::ERR_CANCEL);
        }
        if let Some(m) = ar_sharpness(orig, &m1, px.w as usize, px.h as usize, xb, yb, d, threads, &build) {
            sharp = m;
        }
    }
    let r = build(&sharp);
    let _ = jm_predict;
    Ok(r)
    };
    // jxl-tools: efforts 8 and 9 refine the quant field with Butteraugli round trips (libjxl
    // FindBestQuantization; the CfL map stays the one made with the initial field, as libjxl's)
    // (not below 8 x 8 pixels: Butteraugli measures nothing there; a failed round trip keeps
    // the initial field)
    let rqf = if t <= 2 && !ycc && w >= 8 && h >= 8 {
        let lin = linear_of(px);
        let iters = if t <= 1 { 4 } else { 2 };
        let q0 = q;
        let r = crate::ebq::find_best(&lin, w, h, &acs, xb, yb, d, iters, threads, &mut qf, &mut q, &mut |qq, r| {
            let cs = tail(*qq, r, false)?;
            crate::decode_linear(&cs, w, h)
        });
        match r {
            Ok(v) => v,
            Err(img_core::ERR_CANCEL) => return Err(img_core::ERR_CANCEL),
            Err(_) => {
                q = q0;
                rqf
            }
        }
    } else {
        rqf
    };
    tail(q, rqf, true)
}

/// the linear sRGB planes of the 8-bit image, invisible pixels simplified as in the XYB image
/// (the reference of the Butteraugli round trips)  [jxl-tools]
fn linear_of(px: &Px) -> [Vec<f32>; 3] {
    let (w, h) = (px.w as usize, px.h as usize);
    let mut li = Img::new(w, h);
    for (c, p) in li.p.iter_mut().enumerate() {
        for (i, v) in p.iter_mut().enumerate() {
            *v = tab::SRGBLIN[px.rgba[i * 4 + c] as usize];
        }
    }
    if px.alpha {
        simplify_invisible(&mut li, px);
    }
    li.p
}

/// ComputeARHeuristics: the EPF sharpness of every block from reconstructions of the frame with
/// one sharpness everywhere (our decoder), against the pre-gaborish XYB image with the 1x1
/// masking; None when a reconstruction fails
#[allow(clippy::too_many_arguments)]
fn ar_sharpness(
    orig: &Img, mask1: &[f32], w: usize, h: usize, xb: usize, yb: usize, d: f32, threads: bool,
    build: &dyn Fn(&[u8]) -> Vec<u8>,
) -> Option<Vec<u8>> {
    let cd = d.min(5.0);
    let steps: &[u8] = if d > 4.5 { &[0, 4] } else { &[0, 2, 7] };
    let mut lut = [0usize; 8];
    for (i, &v) in steps.iter().enumerate() {
        lut[v as usize] = i;
    }
    let mut err: Vec<Vec<f32>> = vec![Vec::new(); 8];
    let cs0 = build(&vec![4u8; xb * yb]);
    let mut rec = crate::Recon::new(&cs0).ok()?;
    for &val in steps {
        let (dw, dh, ds, pl) = rec.planes(&vec![val; xb * yb]).ok()?;
        if (dw as usize) < w || (dh as usize) < h {
            return None;
        }
        let ow = orig.w;
        let rows: Vec<Vec<f32>> = par_map(yb, threads, |by| {
            let mut row = vec![0f32; xb];
            for (bx, r) in row.iter_mut().enumerate() {
                let mut e2 = [0f32; 3];
                for y in by * 8..(by * 8 + 8).min(h) {
                    for x in bx * 8..(bx * 8 + 8).min(w) {
                        let mk = mask1[y * ow + x];
                        let m2 = mk * mk;
                        for c in 0..3 {
                            let df = orig.p[c][y * ow + x] - pl[c][y * ds + x];
                            e2[c] += m2 * df * df;
                        }
                    }
                }
                *r = (12.339_445_295_782_363f64 * e2[0] as f64 + e2[1] as f64 + 0.2 * e2[2] as f64) as f32;
            }
            row
        });
        err[val as usize] = rows.concat();
    }
    let (c5, c6, c7) = (0.007_620_386_6f32, 0.008_322_480_6f32, 0.996_639_396_856_867_53f64);
    let mut out = vec![0u8; xb * yb];
    let mut histo = [[0usize; 8]; 9];
    let mut totals = [1usize; 9];
    for by in 0..yb {
        for bx in 0..xb {
            let i = by * xb + bx;
            let top = if by > 0 { out[i - xb] } else { 0 };
            let left = if bx > 0 { out[i - 1] } else { 0 };
            let te = err[top as usize][i];
            let le = err[left as usize][i];
            let (mut bv, mut be) = (0u8, f32::MAX);
            for &v in steps {
                let mut e = err[v as usize][i];
                if v == 0 {
                    e = (e as f64 * (c7 - (c5 * cd) as f64)) as f32;
                }
                if e < be {
                    bv = v;
                    be = e;
                }
            }
            out[i] = if be < (1.0 - c6 * cd) * te.min(le) {
                bv
            } else if te < le {
                top
            } else {
                left
            };
            let ctx = lut[top as usize] * 3 + lut[left as usize];
            histo[ctx][out[i] as usize] += 1;
            totals[ctx] += 1;
        }
    }
    let (c1, c2, c4) = (0.059_588_212f32, 0.105_994_97f32, 1.247_544_7f32);
    let c3 = crate::emath::powf(0.97, cd);
    let cw = c1 + c2 * cd;
    for by in 0..yb {
        for bx in 0..xb {
            let i = by * xb + bx;
            let top = if by > 0 { out[i - xb] } else { 0 };
            let left = if bx > 0 { out[i - 1] } else { 0 };
            let ctx = lut[top as usize] * 3 + lut[left as usize];
            let (mut bv, mut be) = (0u8, f32::MAX);
            for &v in steps {
                let mut e = err[v as usize][i] / (c4 + crate::emath::log1p(histo[ctx][v as usize] as f32 * cw / totals[ctx] as f32));
                if v == 0 {
                    e *= c3;
                }
                if e < be {
                    bv = v;
                    be = e;
                }
            }
            out[i] = bv;
        }
    }
    Some(out)
}

/// the F16 value the writer stores for v (ewr.rs: the mantissa truncated), normal range
fn f16_trunc(v: f32) -> f32 {
    f32::from_bits(v.to_bits() & !0x1FFF)
}

/// patches of the XYB image (jxl-tools): FindTextLikePatches on the opsin planes (none found:
/// the dots of edots.rs from distance 3 on), the
/// reference frame as a modular XYB frame quantized like libjxl's lossy modular XYB (steps
/// (1 + 23 d) / 32768 for X, (1 + 14 d) / 2048 for Y and B; channels Y, X, B - Y), its decoded
/// values subtracted from img
fn lossy_patches(px: &Px, img: &mut Img, d: f32, t: i32, threads: bool, o: &Opts) -> Result<Option<(Jw, crate::epatch::Dict)>, i32> {
    // the image padded to whole blocks, as libjxl 0.7 searches its opsin image
    let mut found = crate::epatch::find([&img.p[0], &img.p[1], &img.p[2]], img.w, img.w, img.h, true);
    // no text-like patches: dots (libjxl: from distance 3 on)
    if found.is_empty() && d >= crate::edots::MIN_DISTANCE {
        found = crate::edots::find([&img.p[0], &img.p[1], &img.p[2]], img.w, img.h, px.w as usize, px.h as usize, threads);
    }
    if found.is_empty() {
        return Ok(None);
    }
    let pk = crate::epatch::pack(found, px.alpha as u32);
    let (rw, rh) = (pk.w, pk.h);
    let n = rw * rh;
    let f16v = [f16_trunc(128.0 * (1.0 + 23.0 * d) / 32768.0), f16_trunc(128.0 * (1.0 + 14.0 * d) / 2048.0), f16_trunc(128.0 * (1.0 + 14.0 * d) / 2048.0)];
    let dq: [f32; 3] = core::array::from_fn(|c| f16v[c] * (1.0 / 128.0));
    let qx: Vec<i32> = pk.planes[0].iter().map(|&v| roundf(v / dq[0]) as i32).collect();
    let qy: Vec<i32> = pk.planes[1].iter().map(|&v| roundf(v / dq[1]) as i32).collect();
    let qb: Vec<i32> = pk.planes[2].iter().map(|&v| roundf(v / dq[2]) as i32).collect();
    // the decoder's values: int * step (B: (B - Y + Y) * step)
    let rv: [Vec<f32>; 3] = [
        qx.iter().map(|&v| v as f32 * dq[0]).collect(),
        qy.iter().map(|&v| v as f32 * dq[1]).collect(),
        qb.iter().map(|&v| v as f32 * dq[2]).collect(),
    ];
    let sw = img.w;
    pk.subtract(&rv, &mut img.p, sw);
    let mut mi = Mi::new(rw as u32, rh as u32, 8);
    let bmy: Vec<i32> = qb.iter().zip(&qy).map(|(&b, &y)| b - y).collect();
    for dd in [qy, qx, bmy] {
        mi.ch.push(Mc { w: rw as u32, h: rh as u32, hs: 0, vs: 0, d: dd });
    }
    if px.alpha {
        mi.ch.push(Mc { w: rw as u32, h: rh as u32, hs: 0, vs: 0, d: vec![0; n] });
    }
    let gshift = if rw <= 400 && rh <= 400 { 2 } else { 1 };
    let dims = Dims::new(rw as u32, rh as u32, gshift);
    let mut opts = lossless_opts(t, dims.num_streams(1), dims.gdim);
    opts.predictor = PRED_GRADIENT;
    opts.kind = Kind::Learn;
    opts.hp = HParams::for_modular(t, false, false);
    let mut lf = Jw::new();
    lf.bits(0, 1); // own DC quantization: the X, Y, B steps * 128 as f16
    for v in f16v {
        lf.f16(v);
    }
    let sections = crate::enew::modular_sections(mi, &dims, t, threads, &opts, 0, lf, o)?;
    let fh = FrameHdr {
        typ: 2,
        size: Some((rw as u32, rh as u32)),
        modular: true,
        xyb: true,
        nec: px.alpha as u32,
        gshift,
        gab: false,
        epf: 0,
        islast: false,
        saveref: crate::epatch::PATCH_REF,
        ..FrameHdr::default()
    };
    let mut rf = Jw::new();
    efrm::assemble(&mut rf, &fh, &sections);
    Ok(Some((rf, pk.dict)))
}

/// the nearest power of two
fn pow2_near(v: f32) -> f32 {
    let mut p = 1.0f32;
    while p * 1.414_213_5 < v {
        p *= 2.0;
    }
    while p > v * 1.414_213_5 {
        p *= 0.5;
    }
    p
}

/// the progressive DC: a modular DC frame (level 1) with the DC image, quantized in power of
/// two steps near the VarDCT DC steps; X, Y, B - Y as in modular XYB  [jxl-plus]
#[allow(clippy::too_many_arguments)]
fn dc_frame(
    out: &mut Jw, px: &Px, q: &Quantizer, dc: &[Vec<f32>; 3], xb: usize, yb: usize, t: i32, threads: bool, nl: bool,
    o: &Opts,
) -> Result<(), i32> {
    let mul = if nl { 2.0 } else { 1.0 };
    let sx = pow2_near(q.dc_step(0) / mul);
    let sy = pow2_near(q.dc_step(1) / mul);
    let n = xb * yb;
    let mut cy = vec![0i32; n];
    let mut cx = vec![0i32; n];
    let mut cb = vec![0i32; n];
    for i in 0..n {
        cy[i] = roundf(dc[1][i] / sy) as i32;
        cx[i] = roundf(dc[0][i] / sx) as i32;
        cb[i] = roundf(dc[2][i] / sy) as i32 - cy[i];
    }
    let (w, h) = (xb as u32, yb as u32);
    let mut mi = Mi::new(w, h, 8);
    mi.ch.push(Mc { w, h, hs: 0, vs: 0, d: cy });
    mi.ch.push(Mc { w, h, hs: 0, vs: 0, d: cx });
    mi.ch.push(Mc { w, h, hs: 0, vs: 0, d: cb });
    if px.alpha {
        mi.ch.push(Mc { w, h, hs: 0, vs: 0, d: vec![0; n] });
    }
    let dims = Dims::new(w, h, 1);
    let mut opts = lossless_opts(t, dims.num_streams(1), dims.gdim);
    opts.hp = HParams::for_modular(t, false, false);
    let mut lf = Jw::new();
    lf.bits(0, 1); // own DC quantization: X, Y, B factors * 128 as f16
    for s in [sx, sy, sy] {
        lf.f16(s * 128.0);
    }
    let sections = crate::enew::modular_sections(mi, &dims, t, threads, &opts, 0, lf, o)?;
    let fh = FrameHdr {
        typ: 1,
        modular: true,
        xyb: true,
        nec: px.alpha as u32,
        gshift: 1,
        dclevel: 1,
        gab: false,
        epf: 0,
        ..FrameHdr::default()
    };
    efrm::assemble(out, &fh, &sections);
    Ok(())
}

/// a group's crop of the alpha image
fn crop_alpha(full: &Mi, rect: [u32; 4], gdim: u32) -> Mi {
    let mut gi = Mi::new(rect[2], rect[3], full.bd);
    let mut first = full.nbmeta as usize;
    while first < full.ch.len() {
        let c = &full.ch[first];
        if c.w > gdim || c.h > gdim {
            break;
        }
        first += 1;
    }
    for c in &full.ch[first..] {
        let x0 = rect[0];
        let y0 = rect[1];
        let w = if c.w > x0 { (c.w - x0).min(rect[2]) } else { 0 };
        let hh = if c.h > y0 { (c.h - y0).min(rect[3]) } else { 0 };
        if w == 0 || hh == 0 {
            continue;
        }
        let mut d = Vec::with_capacity((w * hh) as usize);
        for y in 0..hh {
            let o = ((y0 + y) * c.w + x0) as usize;
            d.extend_from_slice(&c.d[o..o + w as usize]);
        }
        gi.ch.push(Mc { w, h: hh, hs: c.hs, vs: c.vs, d });
    }
    gi
}
