// color -- JPEG XR output formatting (ITU-T T.832 9.10) into 8-bit BGRA surfaces
//                                                                    [src/img/jxr_color.s]
//
//   output(jx, ajx, dst, stride, flags) -> 0 / ERR_*   convert the reconstructed planes of
//       the codestream jx (alpha: interleaved plane 1 of jx, else plane 0 of the separate alpha
//       codestream ajx) row by row:
//         1. chroma upsampling like WIC (4-tap (-2, 9, 28, -3) / 32, centred, edges repeated)
//         2. InvColorFmtConvert1/2/3 (YONLY / YUV -> RGB, YUVK -> CMYK)
//         3. AddBias, ComputeScaling, PostscalingProcess, clipping: the samples of the pixel
//            format the codestream was made for ("native" samples, BD1 .. BD32F)
//         4. conversion of the native samples to 8-bit BGRA like WIC's FormatConvertedBitmap
//       flags: bit 0 straight output wanted, bit 1 premultiplied source (kept premultiplied
//       with colour <= alpha, or unpremultiplied like WIC), bit 2 = dst receives the native
//       samples as i32[h][w][5] (channels 0..3, alpha; test hook) instead of pixels; internal:
//       bit 8 R / B swapped (packed formats), bit 9 BD8 fast path (co_fast8)
//
// OUTPUT_BITDEPTH: 0 BD1WHITE1, 1 BD8, 2 BD16, 3 BD16S, 4 BD16F, 6 BD32S, 7 BD32F, 8 BD5, 9 BD10,
// 10 BD565, 15 BD1BLACK1.  OUTPUT_CLR_FMT: 0 YONLY, 1..3 YUV, 4 CMYK, 5 CMYKDIRECT, 6 NCOMPONENT, 7 RGB.
// The assembly's SSE2 loops (upsampling, BD8 fast path) are exact integer lane operations;
// the scalar loops here give the same values.

use alloc::vec;
use alloc::vec::Vec;

use img_core::ERR_UNSUPPORTED;
use rt::fx;
use simd::{Kernel, Simd};

use crate::ent::{Cs, JXF_RBNS};
use crate::tab::{RECIP, SRGB};

const BD1W: u32 = 0;
const BD8: u32 = 1;
const BD16: u32 = 2;
const BD16S: u32 = 3;
const BD16F: u32 = 4;
const BD32S: u32 = 6;
const BD32F: u32 = 7;
const BD5: u32 = 8;
const BD10: u32 = 9;
const BD565: u32 = 10;
const BD1B: u32 = 15;

static BGR: [usize; 4] = [2, 1, 0, 3]; // byte offset of R, G, B in a BGRA pixel
static BIAS: [u32; 16] = [0, 128, 32768, 0, 0, 0, 0, 0, 16, 512, 32, 0, 0, 0, 0, 0]; // AddBias iBias

/// the output record (CO)
struct Co<'a> {
    spl: [&'a [i32]; 4], // component planes
    sst: [usize; 4],     // plane strides
    sh: [usize; 4],      // plane heights
    a: Option<&'a [i32]>, // alpha plane
    as_: usize,          // alpha stride
    asc: u32,            // alpha scaled flag
    amt: usize,          // alpha margins
    aml: usize,
    abd: u32, // alpha OUTPUT_BITDEPTH
    ash: u32, // alpha SHIFT_BITS
    amb: u32, // alpha LEN_MANTISSA | EXP_BIAS << 16
    cf: u32,
    ocf: u32,
    obd: u32,
    sc: u32,
    shift: u32,
    mant: u32,
    ebias: i32,
    nch: usize,
    nout: usize, // output channels (1 gray, 3 RGB / YUV, 4 CMYK)
    w: usize,
    mt: usize,
    ml: usize,
    fl: u32,
    pm: u32, // 1 keep premultiplied, 2 unpremultiply, 0 straight source
    urow: Vec<i32>, // upsampled U / V rows (luma resolution), vertical scratch
    vrow: Vec<i32>,
    tmp: Vec<i32>,
    out: [Vec<i32>; 5], // channels 0..3, alpha
}

/// -2 a + 9 b + 28 c - 3 d (wrapping)
#[inline(always)]
fn tap(a: i32, b: i32, c: i32, d: i32) -> i32 {
    a.wrapping_mul(-2).wrapping_add(b.wrapping_mul(9)).wrapping_add(c.wrapping_mul(28)).wrapping_add(d.wrapping_mul(-3))
}

/// TAPX: -2 a + 9 b + 28 c - 3 d on i32 lanes (the assembly's sequence; equal mod 2^32)
#[inline(always)]
fn tapx<S: Simd>(x: S, a: S::V, b: S::V, c: S::V, d: S::V) -> S::V {
    let d3 = x.add32(x.add32(d, d), d);
    let a2d3 = x.add32(x.add32(a, a), d3);
    let b9 = x.add32(x.sll32::<3>(b), b);
    let t = x.sub32(b9, a2d3);
    x.sub32(x.add32(t, x.sll32::<5>(c)), x.sll32::<2>(c))
}

/// one source sample k of upfilt with repeated edges  [.uf_one]
#[inline(always)]
fn up_one(src: &[i32], dst: &mut [i32], k: usize, n: usize, rnd: i32, sh: u32) {
    let last = n as isize - 1;
    let ki = k as isize;
    let a = src[(ki - 2).clamp(0, last) as usize];
    let b = src[(ki - 1).clamp(0, last) as usize];
    let c = src[k];
    let d = src[(ki + 1).clamp(0, last) as usize];
    dst[2 * k] = tap(a, b, c, d).wrapping_add(rnd) >> sh;
    let f = src[(ki + 2).min(last) as usize];
    dst[2 * k + 1] = tap(f, d, c, b).wrapping_add(rnd) >> sh;
}

/// horizontal 2x upsampling with (-2, 9, 28, -3) (even) and (-3, 28, 9, -2) (odd), edges
/// repeated, result (sum + rounding) >> shift (10 or 5); 4 source samples per step inside
/// the row (SSE2 like the assembly: the even / odd interleave is per 128-bit register)
/// [upfilt]
struct UpK<'a> {
    src: &'a [i32],
    dst: &'a mut [i32],
    n: usize,
    rnd: i32,
    sh: u32,
}

impl Kernel for UpK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (src, dst, n, rnd, sh) = (self.src, self.dst, self.n, self.rnd, self.sh);
        let mut k = 0;
        if n >= 8 {
            up_one(src, dst, 0, n, rnd, sh); // edges
            up_one(src, dst, 1, n, rnd, sh);
            k = 2;
            let rv = x.splat32(rnd as u32);
            while k + 6 <= n {
                let a = x.load_i32(&src[k - 2..k + 2]);
                let b = x.load_i32(&src[k - 1..k + 3]);
                let c = x.load_i32(&src[k..k + 4]);
                let d = x.load_i32(&src[k + 1..k + 5]);
                let e = x.load_i32(&src[k + 2..k + 6]);
                let mut ev = x.add32(tapx(x, a, b, c, d), rv);
                let mut od = x.add32(tapx(x, e, d, c, b), rv);
                if sh == 10 {
                    ev = x.sra32::<10>(ev);
                    od = x.sra32::<10>(od);
                } else {
                    ev = x.sra32::<5>(ev);
                    od = x.sra32::<5>(od);
                }
                // e0 o0 e1 o1 / e2 o2 e3 o3
                let lo = x.shuf32::<0xD8>(x.shufps::<0x44>(ev, od));
                let hi = x.shuf32::<0xD8>(x.shufps::<0xEE>(ev, od));
                x.store_i32(&mut dst[2 * k..2 * k + 4], lo);
                x.store_i32(&mut dst[2 * k + 4..2 * k + 8], hi);
                k += 4;
            }
        }
        while k < n {
            up_one(src, dst, k, n, rnd, sh);
            k += 1;
        }
    }
}

/// vertical 2x upsampling of one output row of a 4:2:0 chroma plane (gain 32, no rounding):
/// even rows k-2 .. k+1 with (-2, 9, 28, -3), odd rows k-1 .. k+2 with (-3, 28, 9, -2) (= the
/// even taps on the reversed rows)  [vfilt]
struct VK<'a> {
    rows: [&'a [i32]; 4],
    dst: &'a mut [i32],
}

impl Kernel for VK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let g = S::BYTES / 4;
        let w = self.dst.len();
        let full = w - w % g;
        let [a, b, c, d] = self.rows;
        let mut i = 0;
        while i < full {
            let v = tapx(x, x.load_i32(&a[i..]), x.load_i32(&b[i..]), x.load_i32(&c[i..]), x.load_i32(&d[i..]));
            x.store_i32(&mut self.dst[i..], v);
            i += g;
        }
        for i in full..w {
            self.dst[i] = tap(a[i], b[i], c[i], d[i]);
        }
    }
}

fn vfilt(p: &[i32], w: usize, h: usize, orow: usize, dst: &mut [i32]) {
    let last = h as isize - 1;
    let mut first = (orow >> 1) as isize - 2;
    let rev = orow & 1 != 0;
    if rev {
        first += 1;
    }
    let mut rows = [0usize; 4];
    for i in 0..4 {
        let r = (first + i as isize).clamp(0, last) as usize;
        rows[if rev { 3 - i } else { i }] = r * w;
    }
    let rs = [&p[rows[0]..rows[0] + w], &p[rows[1]..rows[1] + w], &p[rows[2]..rows[2] + w], &p[rows[3]..rows[3] + w]];
    simd::dispatch(VK { rows: rs, dst: &mut dst[..w] });
}

/// BD8 colour / alpha rows -> BGRA pixels: (v + addend) >> shift saturated like packssdw +
/// packuswb  [co_fast8]
struct Fast8K<'a> {
    r: &'a [i32],
    g: &'a [i32],
    b: &'a [i32],
    a: Option<&'a [i32]>,
    dst: &'a mut [u8],
    add: i32,
    sh: u32,
    aadd: i32,
    ash: u32,
}

#[inline(always)]
fn sra3<S: Simd>(x: S, v: S::V, sh: u32) -> S::V {
    if sh != 0 { x.sra32::<3>(v) } else { v }
}

#[inline(always)]
fn cv8(v: i32, ad: i32, sh: u32) -> u8 {
    (v.wrapping_add(ad) >> sh).clamp(0, 255) as u8
}

impl Kernel for Fast8K<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let g = S::BYTES / 4;
        let w = self.r.len();
        let full = w - w % g;
        let addv = x.splat32(self.add as u32);
        let aaddv = x.splat32(self.aadd as u32);
        let a255 = x.splat32(255);
        let dst = self.dst;
        let mut i = 0;
        while i < full {
            let bv = sra3(x, x.add32(x.load_i32(&self.b[i..]), addv), self.sh);
            let gv = sra3(x, x.add32(x.load_i32(&self.g[i..]), addv), self.sh);
            let rv = sra3(x, x.add32(x.load_i32(&self.r[i..]), addv), self.sh);
            let av = match self.a {
                Some(a) => sra3(x, x.add32(x.load_i32(&a[i..]), aaddv), self.ash),
                None => a255,
            };
            let bg = x.packs32(bv, gv); // B0..B3 G0..G3
            let ra = x.packs32(rv, av); // R0..R3 A0..A3
            let lo = x.unpacklo16(bg, ra);
            let hi = x.unpackhi16(bg, ra);
            let p01 = x.unpacklo16(lo, hi); // B0 G0 R0 A0 B1 G1 R1 A1
            let p23 = x.unpackhi16(lo, hi);
            x.store(&mut dst[i * 4..], x.packus16(p01, p23));
            i += g;
        }
        for i in full..w {
            let px = &mut dst[i * 4..i * 4 + 4];
            px[0] = cv8(self.b[i], self.add, self.sh);
            px[1] = cv8(self.g[i], self.add, self.sh);
            px[2] = cv8(self.r[i], self.add, self.sh);
            px[3] = match self.a {
                Some(a) => cv8(a[i], self.aadd, self.ash),
                None => 255,
            };
        }
    }
}

/// AddBias, ComputeScaling, PostscalingProcess and clipping of one channel row in place;
/// p = OUTPUT_BITDEPTH | scaled << 8 | channel << 16 | shift << 24, mb = LEN_MANTISSA |
/// EXP_BIAS << 16  [natfmt]
fn natfmt(row: &mut [i32], bias: u32, p: u32, mb: u32) {
    let bd = p & 255;
    let shift = p >> 24;
    let (mut lo, mut hi) = (0i32, 255i32);
    if bd == BD16 {
        hi = 65535;
    }
    if bd == BD16S {
        lo = -32768;
        hi = 32767;
    }
    if bd == BD5 {
        hi = 31;
    }
    if bd == BD10 {
        hi = 1023;
    }
    if bd == BD1W || bd == BD1B {
        hi = 1;
    }
    let mut rnd = 0i32;
    let mut isc = 0u32;
    if p & 0x100 != 0 {
        isc = 3;
        rnd = 3;
        if bd == BD1W || bd == BD1B || bd == BD16 {
            rnd = 4;
        }
    }
    let bias = bias.wrapping_shl(isc) as i32;
    if bd == BD565 {
        hi = 63; // G: 6 bits
        if (p >> 16) & 255 != 1 {
            hi = 31; // R / B: 5 bits, jScale + 1
            isc += 1;
        }
    }
    let js = isc;
    for v in row.iter_mut() {
        let x = v.wrapping_add(bias).wrapping_add(rnd) >> js;
        *v = match bd {
            BD16F => {
                // sign << 15 | min(|x|, 32767)
                let s = x >> 31;
                let mut a = (x ^ s).wrapping_sub(s);
                if a > 32767 {
                    a = 32767;
                }
                a | (s & 0x8000)
            }
            BD32F => postfl(x, mb) as i32,
            BD16 | BD16S => x.wrapping_shl(shift).clamp(lo, hi),
            BD32S => x.wrapping_shl(shift),
            _ => x.max(lo).min(hi),
        };
    }
}

/// PostScalingFl for BD32F: integer sample -> IEEE single bits  [postfl]
fn postfl(x: i32, mb: u32) -> u32 {
    let sign = (x as u32) & 0x8000_0000;
    let s = x >> 31;
    let a = ((x ^ s).wrapping_sub(s)) as u32;
    let mant = mb & 31;
    let one = 1u32.wrapping_shl(mant);
    let mut e = (a >> mant) as i32;
    let mut m = (a & one.wrapping_sub(1)) | one;
    if e == 0 {
        m ^= one;
        e = 1;
    }
    let ebias = (mb as i32) >> 16;
    e = e.wrapping_sub(ebias).wrapping_add(127);
    while m < one && e > 1 && m != 0 {
        e -= 1;
        m = m.wrapping_add(m);
    }
    if m < one {
        e = 0;
    } else {
        m ^= one;
    }
    let sh = 23u32.wrapping_sub(mant) & 31;
    m = m.wrapping_shl(sh);
    let e = (e.clamp(0, 255) as u32) << 23;
    m | e | sign
}

/// AddBias iBias  [biasof]
fn biasof(bd: u32, shift: u32) -> u32 {
    let bd = bd & 15;
    let b = BIAS[bd as usize];
    if bd == BD16 || bd == BD16S || bd == BD32S { b.wrapping_shr(shift) } else { b }
}

/// round(x / 255) for 0..65025  [div255]
#[inline]
fn div255(x: u32) -> u32 {
    let x = x.wrapping_add(128);
    x.wrapping_add(x >> 8) >> 8
}

/// half bits -> float  [half2f]
fn half2f(h: u32) -> f32 {
    let a = h & 0x7FFF;
    let s = (h & 0x8000) << 16;
    if a >= 0x7C00 {
        return f32::from_bits((a << 13) | 0x7F80_0000 | s);
    }
    if a >= 0x0400 {
        return f32::from_bits(((a << 13) + ((127 - 15) << 23)) | s);
    }
    let f = a as i32 as f32 * f32::from_bits(0x3380_0000); // m * 2^-24
    f32::from_bits(f.to_bits() | s)
}

/// native sample -> linear float value (BD16S / BD16F / BD32S / BD32F)  [natf]
fn natf(v: i32, bd: u32) -> f32 {
    if bd == BD32F {
        return f32::from_bits(v as u32);
    }
    if bd == BD16F {
        return half2f(v as u32);
    }
    let k = if bd == BD16S { f32::from_bits(0x3900_0000) } else { f32::from_bits(0x3380_0000) };
    v as f32 * k
}

/// round(clamp(x) * 255) (NaN -> 0)  [unit8]
fn unit8(x: f32) -> u32 {
    let x = fx::minss(fx::maxss(x, 0.0), 1.0);
    fx::cvtt(x * 255.0 + 0.5) as u32
}

/// linear value -> 8-bit sRGB (clamped; 4096-step table)  [lin2srgb8]
fn lin2srgb8(x: f32) -> u32 {
    let x = fx::minss(fx::maxss(x, 0.0), 1.0);
    SRGB[(fx::cvtt(x * 4095.0 + 0.5) & 4095) as usize] as u32
}

/// native sample -> 8-bit value like WIC (ch 8 = gray)  [to8ch]
fn to8ch(v: i32, ch: u32, bd: u32) -> u32 {
    let u = v as u32;
    match bd {
        BD8 => u,
        BD16 => {
            if ch == 8 {
                u.wrapping_mul(255).wrapping_add(32767) / 65535 // gray: round(v * 255 / 65535)
            } else {
                u >> 8 // RGB / alpha: v >> 8 (WIC)
            }
        }
        BD1W | BD1B => u.wrapping_neg() & 255, // 1 = white
        BD565 if ch == 1 => (u << 2) | (u >> 4),
        BD5 | BD565 => (u << 3) | (u >> 2),
        BD10 => u.wrapping_mul(255).wrapping_add(511) / 1023,
        BD32F | BD16F | BD16S | BD32S => lin2srgb8(natf(v, bd)),
        _ => 0,
    }
}

/// alpha -> 8 bit (float formats: linear)  [to8a]
fn to8a(v: i32, bd: u32) -> u32 {
    match bd {
        BD32F | BD16F | BD16S | BD32S => unit8(natf(v, bd)),
        _ => to8ch(v, 0, bd),
    }
}

impl Co<'_> {
    /// the source rows of the components; U / V of 4:2:0 / 4:2:2 upsampled to luma resolution
    /// (urow / vrow); returns the row start of each component (usize::MAX = the upsampled
    /// row)  [co_rowsrc]
    fn rowsrc(&mut self, row: usize) -> [usize; 4] {
        let mut src = [0usize; 4];
        for c in 0..self.nch {
            if self.cf == 1 && c != 0 {
                continue; // 4:2:0 chroma rows: below
            }
            src[c] = row * self.sst[c];
        }
        if self.cf == 1 || self.cf == 2 {
            let (rnd, sh) = if self.cf == 1 { (512, 10) } else { (16, 5) }; // gain 32 * 32 / 32
            let n = self.sst[1];
            for k in 0..2 {
                let c = k + 1;
                let dst = if k == 0 { &mut self.urow } else { &mut self.vrow };
                let inp: &[i32] = if self.cf == 1 {
                    vfilt(self.spl[c], self.sst[1], self.sh[1], row, &mut self.tmp);
                    &self.tmp
                } else {
                    &self.spl[c][src[c]..]
                };
                simd::dispatch_sse2(UpK { src: &inp[..n], dst: &mut dst[..2 * n], n, rnd, sh });
                src[c] = usize::MAX;
            }
        }
        src
    }

    /// InvColorFmtConvert1 / 2 / 3 / 4 of the output row (window applied) into out[0..3]:
    /// gray; R, G, B; C, M, Y, K  [co_colour]
    fn colour(&mut self, src: &[usize; 4]) {
        let w = self.w;
        let ml = self.ml;
        let (urow, vrow): (&[i32], &[i32]) = (&self.urow, &self.vrow);
        let mut s: [&[i32]; 4] = [&[], &[], &[], &[]];
        for c in 0..self.nch {
            s[c] = if src[c] == usize::MAX {
                if c == 1 { &urow[ml..] } else { &vrow[ml..] }
            } else {
                &self.spl[c][src[c] + ml..]
            };
        }
        let [s0, s1, s2, s3] = s;
        let [o0, o1, o2, o3, _] = &mut self.out;
        let (o0, o2) = if self.fl & 0x100 != 0 { (o2, o0) } else { (o0, o2) }; // R <-> B (packed formats)
        let (o0, o1, o2, o3) = (&mut o0[..w], &mut o1[..w], &mut o2[..w], &mut o3[..w]);
        let s0 = &s0[..w];
        let (s1, s2) = if self.nch >= 3 { (&s1[..w], &s2[..w]) } else { (s1, s2) };
        let s3 = if self.nch >= 4 { &s3[..w] } else { s3 };
        if self.cf == 0 {
            // YONLY (RGB output: R = G = B = Y)
            o0.copy_from_slice(s0);
            o1.copy_from_slice(s0);
            o2.copy_from_slice(s0);
        } else if self.cf == 4 {
            if self.ocf == 4 {
                // k = K + (Y >> 1), m = k - Y - (U >> 1), c = U + m + (V >> 1), y = c - V
                for x in 0..w {
                    let (y, u, v, k) = (s0[x], s1[x], s2[x], s3[x]);
                    let kk = (y >> 1).wrapping_add(k);
                    o3[x] = kk;
                    let m = kk.wrapping_sub(y).wrapping_sub(u >> 1);
                    o1[x] = m;
                    let c = m.wrapping_add(u).wrapping_add(v >> 1);
                    o0[x] = c;
                    o2[x] = c.wrapping_sub(v);
                }
            } else {
                // CMYKDIRECT: c = U, m = V, y = K, k = Y
                for x in 0..w {
                    o0[x] = s1[x];
                    o1[x] = s2[x];
                    o2[x] = s3[x];
                    o3[x] = s0[x];
                }
            }
        } else {
            // t = -U, G = Y - (t >> 1), R = t + G - ((V + 1) >> 1), B = V + R
            let ins = s0.iter().zip(s1).zip(s2);
            let outs = o0.iter_mut().zip(o1.iter_mut()).zip(o2.iter_mut());
            for (((y, u), v), ((r0, g1), b2)) in ins.zip(outs) {
                let t = u.wrapping_neg();
                let g = y.wrapping_sub(t >> 1);
                *g1 = g;
                let r = t.wrapping_add(g).wrapping_sub(v.wrapping_add(1) >> 1);
                *r0 = r;
                *b2 = r.wrapping_add(*v);
            }
        }
    }

    fn arow(&self, y0: usize) -> Option<&[i32]> {
        self.a.map(|a| &a[(y0 + self.amt) * self.as_ + self.aml..])
    }

    /// natfmt on the output channels (CMYK: c, m, y + bias / 2, k - bias / 2) and on the
    /// alpha row (its own plane parameters)  [co_native]
    fn native(&mut self, y0: usize) {
        let bias = biasof(self.obd, self.shift);
        let mb = ((self.ebias as u32) << 16) | self.mant;
        for ch in 0..self.nout {
            let mut b = bias;
            if self.ocf == 4 {
                b >>= 1;
                if ch == 3 {
                    b = b.wrapping_neg();
                }
            }
            let p = self.obd | (self.sc << 8) | ((ch as u32) << 16) | (self.shift << 24);
            natfmt(&mut self.out[ch][..self.w], b, p, mb);
        }
        if let Some(a) = self.a {
            let o = (y0 + self.amt) * self.as_ + self.aml;
            let w = self.w;
            self.out[4][..w].copy_from_slice(&a[o..o + w]);
            let b = biasof(self.abd, self.ash);
            let p = self.abd | (self.asc << 8) | (self.ash << 24);
            natfmt(&mut self.out[4][..w], b, p, self.amb);
        }
    }

    /// BD8 output without premultiplied alpha: AddBias / ComputeScaling / clipping of the colour
    /// rows (gray: row 0 only) and the alpha row (or 255) straight into BGRA pixels  [co_fast8]
    fn fast8(&self, y0: usize, dst: &mut [u8]) {
        let s = self.sc.wrapping_neg() & 3; // 3 when scaled (rounding 3), else 0
        let add = (128i32 << s) + if s != 0 { 3 } else { 0 };
        let sa = self.asc.wrapping_neg() & 3;
        let aadd = (128i32 << sa) + if sa != 0 { 3 } else { 0 };
        let w = self.w;
        let r = &self.out[0][..w];
        let (g, b) = if self.nout == 1 { (r, r) } else { (&self.out[1][..w], &self.out[2][..w]) };
        let a = self.arow(y0).map(|a| &a[..w]);
        simd::dispatch(Fast8K { r, g, b, a, dst: &mut dst[..w * 4], add, sh: s, aadd, ash: sa });
    }

    /// test hook: 5 i32 per pixel (channels 0..3, alpha) into the dump row  [co_dumprow]
    fn dumprow(&self, dst: &mut [u8]) {
        for x in 0..self.w {
            for ch in 0..5 {
                let o = (x * 5 + ch) * 4;
                dst[o..o + 4].copy_from_slice(&self.out[ch][x].to_le_bytes());
            }
        }
    }

    /// native samples of the row -> 8-bit BGRA like WIC  [co_bgra]
    fn bgra(&self, dst: &mut [u8]) {
        let obd = self.obd;
        let has_a = self.a.is_some();
        let out = &self.out;
        for x in 0..self.w {
            let px = &mut dst[x * 4..x * 4 + 4];
            let alpha: u32;
            if obd == BD16S || obd == BD16F || obd == BD32S || obd == BD32F {
                // float / half / fixed point: linear -> sRGB, unpremultiplied in float
                let fa = if has_a { natf(out[4][x], self.abd) } else { 1.0 };
                for ch in 0..self.nout {
                    let mut f = natf(out[ch][x], obd);
                    if self.pm != 0 {
                        // like WIC: c / a, a == 0 -> 0
                        f = if fa == 0.0 || fa.is_nan() { 0.0 } else { f / fa };
                    }
                    let v = lin2srgb8(f) as u8;
                    px[BGR[ch]] = v;
                    if self.nout == 1 {
                        px[0] = v;
                        px[1] = v;
                    }
                }
                alpha = unit8(fa);
            } else if obd == BD16 && self.pm != 0 {
                // 16-bit premultiplied source: unpremultiplied at 16 bits (rounded), then >> 8
                let a16 = if has_a { out[4][x] as u32 } else { 65535 };
                for ch in 0..self.nout {
                    let mut v = out[ch][x] as u32;
                    if a16 == 0 {
                        v = 0;
                    } else {
                        v = v.wrapping_mul(65535).wrapping_add(a16 >> 1) / a16;
                        if v > 65535 {
                            v = 65535;
                        }
                    }
                    let v = (v >> 8) as u8;
                    px[BGR[ch]] = v;
                    if self.nout == 1 {
                        px[0] = v;
                        px[1] = v;
                    }
                }
                alpha = a16 >> 8;
            } else {
                // integer formats
                let c0 = to8ch(out[0][x], if self.nout == 1 { 8 } else { 0 }, obd);
                let (mut r, mut g, mut b) = (c0, c0, c0);
                if self.nout != 1 {
                    g = to8ch(out[1][x], 1, obd);
                    b = to8ch(out[2][x], 2, obd);
                    if self.nout == 4 {
                        // CMYK -> RGB: (255 - c) * (255 - k) / 255
                        let nk = 255u32.wrapping_sub(to8ch(out[3][x], 3, obd));
                        r = div255(255u32.wrapping_sub(r).wrapping_mul(nk));
                        g = div255(255u32.wrapping_sub(g).wrapping_mul(nk));
                        b = div255(255u32.wrapping_sub(b).wrapping_mul(nk));
                    }
                }
                px[0] = b as u8;
                px[1] = g as u8;
                px[2] = r as u8;
                let mut a = 255;
                if has_a {
                    a = to8a(out[4][x], self.abd);
                    if self.pm == 1 {
                        // keep premultiplied: colour <= alpha
                        for c in px[..3].iter_mut() {
                            if *c as u32 > a {
                                *c = a as u8;
                            }
                        }
                    } else if self.pm == 2 {
                        // straight like WIC: min(255, c * floor(255 * 2^16 / a) >> 16), alpha 0 -> 0
                        let rc = RECIP[(a & 255) as usize];
                        for c in px[..3].iter_mut() {
                            *c = ((*c as u32).wrapping_mul(rc) >> 16).min(255) as u8;
                        }
                    }
                }
                px[3] = a as u8;
                continue;
            }
            // alpha (8 bit), colour straight; PARGB output wanted: premultiply
            px[3] = alpha as u8;
            if self.pm == 1 {
                for c in px[..3].iter_mut() {
                    *c = div255((*c as u32).wrapping_mul(alpha)) as u8;
                }
            }
        }
    }
}

/// see the module header; dst = surface pixels with row stride dstr, or the dump buffer
/// (flags bit 2, dstr = w * 20)  [jxc_output]
pub fn output(jx: &Cs, ja: Option<&Cs>, dst: &mut [u8], dstr: usize, flags: u32) -> i32 {
    let mut fl = flags;
    let pm = if fl & 2 != 0 {
        if fl & 1 != 0 { 2 } else { 1 }
    } else {
        0
    };
    let p0 = &jx.pl[0];
    let (obd, ocf, cf) = (jx.obd, jx.ocf, p0.cf);
    // packed BD5 / BD565 / BD10 RGB output: R and B swapped unless RED_BLUE_NOT_SWAPPED
    if (BD5..=BD565).contains(&obd) && ocf == 7 && jx.flags & JXF_RBNS == 0 {
        fl |= 0x100;
    }
    // supported: YONLY -> YONLY / RGB, YUV420 / 422 / 444 -> RGB, YUVK -> CMYK / CMYKDIRECT
    let nout = match (cf, ocf) {
        (0, 0) => 1,
        (0, 7) => 3,
        (1..=3, 7) => 3,
        (4, 4) | (4, 5) => 4,
        _ => return ERR_UNSUPPORTED,
    };
    let nch = p0.nch as usize;
    if nch > 4 {
        return ERR_UNSUPPORTED;
    }
    let mut spl: [&[i32]; 4] = [&[], &[], &[], &[]];
    let mut sst = [0usize; 4];
    let mut sh = [0usize; 4];
    for c in 0..nch {
        spl[c] = &p0.buf[c];
        sst[c] = p0.pw[c] as usize;
        sh[c] = p0.ph[c] as usize;
    }
    // alpha plane: interleaved (plane 1) or the separate codestream
    let arec = if jx.npl >= 2 { Some((jx, &jx.pl[1])) } else { ja.map(|a| (a, &a.pl[0])) };
    let w = jx.w as usize;
    let pw = p0.pw[0] as usize;
    let mut co = Co {
        spl,
        sst,
        sh,
        a: None,
        as_: 0,
        asc: 0,
        amt: 0,
        aml: 0,
        abd: 0,
        ash: 0,
        amb: 0,
        cf,
        ocf,
        obd,
        sc: p0.scaled,
        shift: p0.shift,
        mant: p0.mant,
        ebias: p0.ebias,
        nch,
        nout,
        w,
        mt: jx.mt as usize,
        ml: jx.ml as usize,
        fl,
        pm,
        urow: vec![0; pw + 4],
        vrow: vec![0; pw + 4],
        tmp: vec![0; pw + 4],
        out: [vec![0; w + 4], vec![0; w + 4], vec![0; w + 4], vec![0; w + 4], vec![0; w + 4]],
    };
    if let Some((rec, pl)) = arec {
        co.a = Some(&pl.buf[0]);
        co.as_ = pl.pw[0] as usize;
        co.asc = pl.scaled;
        co.ash = pl.shift;
        co.amb = ((pl.ebias as u32) << 16) | pl.mant;
        co.amt = rec.mt as usize;
        co.aml = rec.ml as usize;
        co.abd = rec.obd;
    }
    // fast path: BD8 gray / RGB, no premultiplied source, BD8 alpha, no dump
    let fast = obd == BD8 && nout != 4 && fl & 4 == 0 && pm == 0 && (co.a.is_none() || co.abd == BD8);
    let h = jx.h as usize;
    for y0 in 0..h {
        let row = y0 + co.mt;
        let src = co.rowsrc(row);
        co.colour(&src);
        let d = &mut dst[y0 * dstr..];
        if fast {
            co.fast8(y0, d);
        } else {
            co.native(y0);
            if fl & 4 != 0 {
                co.dumprow(d);
            } else {
                co.bgra(d);
            }
        }
    }
    0
}
