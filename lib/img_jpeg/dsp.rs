// dsp -- JPEG signal processing kernels and tables (shared by dec / enc)  [src/img/jpeg_dsp.s]
//
//   idct(coef, quant, dst, stride)        8x8 inverse DCT, libjpeg "islow" (the SSE2 sequence)
//   idct2(a, qa, b, qb, out)              the same for two blocks (AVX2: one per lane) into a
//                                         128-byte buffer; put_block copies one into a plane
//   ac_zero(coef)                         coefficients 1..63 all zero
//   dc_fill(dst, stride, dcq)             the IDCT of a DC-only block (jd_dcfill, jpeg_dec.s)
//   fdct_quant(src, stride, out, div)     8 rows of 8 samples -> 64 quantised i16
//                                         coefficients: libjpeg "islow" forward DCT (x8),
//                                         then the divisors (reciprocal tables)
//   ycc_rgb(dst, y, cb, cr, n)            YCbCr rows -> PARGB (alpha 255), libjpeg exact
//   gray_rgb(dst, y, n)                   grey row -> PARGB
//   rgb_rgb(dst, r, g, b, n)              R, G, B rows -> PARGB
//   cmyk_rgb(dst, rows, n, mode)          C, M, Y, K rows -> PARGB
//   up_h2v1(dst, src, n)                  "fancy" (triangle) 2:1 horizontal upsampling
//   up_h2v2(dst, this, nbr, n)            "fancy" 2:1 horizontal + vertical upsampling
//   up_h1v2(dst, this, nbr, n, bias)      "fancy" 2:1 vertical upsampling
//   rgb_ycc(src, y, cb, cr, n, mode)      BGRA pixels -> Y, Cb, Cr rows (libjpeg exact)
//   down_h2v2(dst, r0, r1, n)             2x2 box downsampling (libjpeg bias 1, 2, 1, 2)
//   down_h2v1(dst, src, n)                2:1 horizontal downsampling (bias 0, 1, 0, 1)
//
// The assembly is SSE2 only.  The block kernels run its SSE2 sequences (simd::Sse2; on an
// AVX2 machine VEX-encoded through simd::dispatch, idct2 with one block per 128-bit lane);
// the row kernels are simd::Kernels whose
// lanes each do the SSE2 operations on a group of 8 samples (AVX2: two groups per
// register), with the same bytes for any width (--simd sse2).  The 16-bit steps wrap and
// saturate exactly like the instructions (pmullw, paddw, packssdw, ...), so coefficients
// outside libjpeg's range give the assembly's results too.  Row kernels read / write up to
// the next multiple of 8 samples (16 input bytes per 8 outputs for the downsamplers):
// callers keep that much slack.  The upsamplers read one sample past round_up(n, 8); the
// sample before the row (src[-1] in the assembly) only reaches outputs that are replaced
// by the edge values, here a shifted copy stands in for it.
//
// Tables: ZZ (zigzag index -> natural index, 64 entries + 16 x 63 so that a run past the end
// of a block stays inside it), the Annex K Huffman tables {DC,AC}_{L,C}_{BITS,VALS} and the
// Annex K quantisation tables STD_QL / STD_QC (natural order).

use simd::{Kernel, Simd, Sse2};

// islow constants (CONST_BITS 13)
const F_0_298: i32 = 2446;
const F_0_390: i32 = 3196;
const F_0_541: i32 = 4433;
const F_0_765: i32 = 6270;
const F_0_899: i32 = 7373;
const F_1_175: i32 = 9633;
const F_1_501: i32 = 12299;
const F_1_847: i32 = 15137;
const F_1_961: i32 = 16069;
const F_2_053: i32 = 16819;
const F_2_562: i32 = 20995;
const F_3_072: i32 = 25172;

/// a register of 4 repeated (lo, hi) i16 pairs (the pmaddwd constants)
#[inline(always)]
fn pair<S: Simd>(x: S, lo: i32, hi: i32) -> S::V {
    x.splat32((lo as u16 as u32) | ((hi as u16 as u32) << 16))
}

/// TRANSPOSE8W: the 8x8 i16 matrix in v (row k in v[k]); three rounds of interleaving rows
/// i and i + 4 (each round rotates the 6-bit element index by one bit)
#[inline(always)]
fn transpose8<S: Simd>(x: S, v: [S::V; 8]) -> [S::V; 8] {
    let [a0, a1, a2, a3, a4, a5, a6, a7] = v;
    let (b0, b1) = (x.unpacklo16(a0, a4), x.unpackhi16(a0, a4));
    let (b2, b3) = (x.unpacklo16(a1, a5), x.unpackhi16(a1, a5));
    let (b4, b5) = (x.unpacklo16(a2, a6), x.unpackhi16(a2, a6));
    let (b6, b7) = (x.unpacklo16(a3, a7), x.unpackhi16(a3, a7));
    let (c0, c1) = (x.unpacklo16(b0, b4), x.unpackhi16(b0, b4));
    let (c2, c3) = (x.unpacklo16(b1, b5), x.unpackhi16(b1, b5));
    let (c4, c5) = (x.unpacklo16(b2, b6), x.unpackhi16(b2, b6));
    let (c6, c7) = (x.unpacklo16(b3, b7), x.unpackhi16(b3, b7));
    [
        x.unpacklo16(c0, c4),
        x.unpackhi16(c0, c4),
        x.unpacklo16(c1, c5),
        x.unpackhi16(c1, c5),
        x.unpacklo16(c2, c6),
        x.unpackhi16(c2, c6),
        x.unpacklo16(c3, c7),
        x.unpackhi16(c3, c7),
    ]
}

/// DESCALE of a lo / hi pair of i32 registers, packed to saturated i16 (packssdw)
#[inline(always)]
fn desc<S: Simd, const SH: i32>(x: S, lo: S::V, hi: S::V, rnd: S::V) -> S::V {
    x.packs32(x.sra32::<SH>(x.add32(lo, rnd)), x.sra32::<SH>(x.add32(hi, rnd)))
}

/// IDCT_PASS: one 1-D islow IDCT on v (input k in v[k], 8 lanes in parallel), outputs
/// DESCALE(x, SH) with the rounding constant rnd as saturated i16
#[inline(always)]
fn idct_pass<S: Simd, const SH: i32>(x: S, v: [S::V; 8], rnd: S::V) -> [S::V; 8] {
    let z = x.zero();
    // even part
    let l26 = x.unpacklo16(v[2], v[6]);
    let h26 = x.unpackhi16(v[2], v[6]);
    let c130 = pair(x, F_0_541 + F_0_765, F_0_541);
    let c054 = pair(x, F_0_541, F_0_541 - F_1_847);
    let t3l = x.madd16(l26, c130);
    let t3h = x.madd16(h26, c130);
    let t2l = x.madd16(l26, c054);
    let t2h = x.madd16(h26, c054);
    let s04 = x.add16(v[0], v[4]);
    let d04 = x.sub16(v[0], v[4]);
    let t0l = x.sra32::<3>(x.unpacklo16(z, s04)); // (in0 + in4) << 13
    let t0h = x.sra32::<3>(x.unpackhi16(z, s04));
    let t1l = x.sra32::<3>(x.unpacklo16(z, d04)); // (in0 - in4) << 13
    let t1h = x.sra32::<3>(x.unpackhi16(z, d04));
    let t10l = x.add32(t0l, t3l);
    let t13l = x.sub32(t0l, t3l);
    let t10h = x.add32(t0h, t3h);
    let t13h = x.sub32(t0h, t3h);
    let t11l = x.add32(t1l, t2l);
    let t12l = x.sub32(t1l, t2l);
    let t11h = x.add32(t1h, t2h);
    let t12h = x.sub32(t1h, t2h);
    // odd part
    let z3 = x.add16(v[7], v[3]);
    let z4 = x.add16(v[5], v[1]);
    let zl = x.unpacklo16(z3, z4);
    let zh = x.unpackhi16(z3, z4);
    let c078 = pair(x, F_1_175 - F_1_961, F_1_175);
    let c117 = pair(x, F_1_175, F_1_175 - F_0_390);
    let z3l = x.madd16(zl, c078);
    let z3h = x.madd16(zh, c078);
    let z4l = x.madd16(zl, c117);
    let z4h = x.madd16(zh, c117);
    let al = x.unpacklo16(v[7], v[1]);
    let ah = x.unpackhi16(v[7], v[1]);
    let c060 = pair(x, F_0_298 - F_0_899, -F_0_899);
    let c089 = pair(x, -F_0_899, F_1_501 - F_0_899);
    let tmp0l = x.add32(x.madd16(al, c060), z3l);
    let tmp0h = x.add32(x.madd16(ah, c060), z3h);
    let tmp3l = x.add32(x.madd16(al, c089), z4l);
    let tmp3h = x.add32(x.madd16(ah, c089), z4h);
    let bl = x.unpacklo16(v[5], v[3]);
    let bh = x.unpackhi16(v[5], v[3]);
    let c050 = pair(x, F_2_053 - F_2_562, -F_2_562);
    let c256 = pair(x, -F_2_562, F_3_072 - F_2_562);
    let tmp1l = x.add32(x.madd16(bl, c050), z4l);
    let tmp1h = x.add32(x.madd16(bh, c050), z4h);
    let tmp2l = x.add32(x.madd16(bl, c256), z3l);
    let tmp2h = x.add32(x.madd16(bh, c256), z3h);
    [
        desc::<S, SH>(x, x.add32(t10l, tmp3l), x.add32(t10h, tmp3h), rnd),
        desc::<S, SH>(x, x.add32(t11l, tmp2l), x.add32(t11h, tmp2h), rnd),
        desc::<S, SH>(x, x.add32(t12l, tmp1l), x.add32(t12h, tmp1h), rnd),
        desc::<S, SH>(x, x.add32(t13l, tmp0l), x.add32(t13h, tmp0h), rnd),
        desc::<S, SH>(x, x.sub32(t13l, tmp0l), x.sub32(t13h, tmp0h), rnd),
        desc::<S, SH>(x, x.sub32(t12l, tmp1l), x.sub32(t12h, tmp1h), rnd),
        desc::<S, SH>(x, x.sub32(t11l, tmp2l), x.sub32(t11h, tmp2h), rnd),
        desc::<S, SH>(x, x.sub32(t10l, tmp3l), x.sub32(t10h, tmp3h), rnd),
    ]
}

// ------------------------------------------------------------------ block kernels
// The assembly's SSE2 sequences; dispatched like every kernel so that an AVX2 machine runs
// them VEX-encoded (three-operand forms, fewer register copies) -- the same operations.

/// one IDCT per 128-bit lane (lane 0 block a, lane 1 block b; SSE2: a only), 8 rows of 8
/// samples each to da / db, sa / sb bytes apart
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn idct_lanes<S: Simd>(
    x: S,
    a: &[i16; 64],
    qa: &[i16; 64],
    da: &mut [u8],
    sa: usize,
    b: &[i16; 64],
    qb: &[i16; 64],
    db: &mut [u8],
    sb: usize,
) {
    let mut v = [x.zero(); 8];
    for (k, r) in v.iter_mut().enumerate() {
        *r = x.mullo16(x.load_i16_lanes(&a[k * 8..], &b[k * 8..]), x.load_i16_lanes(&qa[k * 8..], &qb[k * 8..]));
    }
    let v = idct_pass::<S, 11>(x, v, x.splat32(1 << 10)); // columns
    let v = transpose8(x, v);
    let v = idct_pass::<S, 18>(x, v, x.splat32((1 << 17) + (128 << 18))); // rows, +128 folded in
    let [r0, r1, r2, r3, r4, r5, r6, r7] = transpose8(x, v);
    let da = &mut da[..7 * sa + 8];
    let db = &mut db[..7 * sb + 8];
    let (p0, p1, p2, p3) = (x.packus16(r0, r1), x.packus16(r2, r3), x.packus16(r4, r5), x.packus16(r6, r7));
    x.store_raw_lanes(&mut da[0..], &mut db[0..], p0);
    x.store_raw_lanes(&mut da[sa..], &mut db[sb..], x.bsrl::<8>(p0));
    x.store_raw_lanes(&mut da[2 * sa..], &mut db[2 * sb..], p1);
    x.store_raw_lanes(&mut da[3 * sa..], &mut db[3 * sb..], x.bsrl::<8>(p1));
    x.store_raw_lanes(&mut da[4 * sa..], &mut db[4 * sb..], p2);
    x.store_raw_lanes(&mut da[5 * sa..], &mut db[5 * sb..], x.bsrl::<8>(p2));
    x.store_raw_lanes(&mut da[6 * sa..], &mut db[6 * sb..], p3);
    x.store_raw_lanes(&mut da[7 * sa..], &mut db[7 * sb..], x.bsrl::<8>(p3));
}

struct IdctK<'a> {
    a: &'a [i16; 64],
    qa: &'a [i16; 64],
    da: &'a mut [u8],
    sa: usize,
    b: Option<(&'a [i16; 64], &'a [i16; 64])>, // with b: da = 128 bytes, both blocks at stride 8
}

impl Kernel for IdctK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let mut none = [0u8; 8];
        match self.b {
            // two blocks: one per lane (AVX2), else one after the other
            Some((b, qb)) => {
                let (da, db) = self.da.split_at_mut(64);
                if S::BYTES == 32 {
                    idct_lanes(x, self.a, self.qa, da, 8, b, qb, db, 8);
                } else {
                    idct_lanes(Sse2, self.a, self.qa, da, 8, self.a, self.qa, &mut none, 0);
                    idct_lanes(Sse2, b, qb, db, 8, b, qb, &mut none, 0);
                }
            }
            None => idct_lanes(Sse2, self.a, self.qa, self.da, self.sa, self.a, self.qa, &mut none, 0),
        }
    }
}

/// jpeg_idct: inverse DCT of one block (coef = 64 quantised i16, quant = 64 u16 as i16, both
/// natural order); writes 8 rows of 8 samples to dst, `stride` bytes apart.  Exactly
/// libjpeg's jpeg_idct_islow for coefficients that fit 16 bits.
pub fn idct(coef: &[i16; 64], quant: &[i16; 64], dst: &mut [u8], stride: usize) {
    simd::dispatch(IdctK { a: coef, qa: quant, da: dst, sa: stride, b: None });
}

/// jpeg_idct of two blocks at once (AVX2: one per 128-bit lane, the same operations): block
/// a to out[0..64], block b to out[64..128], 8 bytes per row
pub fn idct2(a: &[i16; 64], qa: &[i16; 64], b: &[i16; 64], qb: &[i16; 64], out: &mut [u8; 128]) {
    simd::dispatch(IdctK { a, qa, da: out, sa: 8, b: Some((b, qb)) });
}

/// 8 rows of 8 samples from a block buffer (stride 8) into dst, `stride` bytes apart
#[inline(always)]
pub fn put_block(dst: &mut [u8], stride: usize, src: &[u8]) {
    for r in 0..8 {
        dst[r * stride..r * stride + 8].copy_from_slice(&src[r * 8..r * 8 + 8]);
    }
}

/// coefficients 1..63 of a block are all zero (jd_nodc mask + pcmpeqw / pmovmskb)
#[inline(always)]
pub fn ac_zero(b: &[i16; 64]) -> bool {
    let x = Sse2;
    let mut v = x.and(x.load_i16(&b[0..]), x.bsll::<2>(x.splat16(0xFFFF)));
    for k in 1..8 {
        v = x.or(v, x.load_i16(&b[k * 8..]));
    }
    x.movemask8(x.cmpeq16(v, x.zero())) == 0xFFFF
}

/// jd_dcfill: the IDCT of a block with only a DC term (dequantised value dcq): 8x8 samples
/// of clamp(((dcq + 4) >> 3) + 128)
pub fn dc_fill(dst: &mut [u8], stride: usize, dcq: i32) {
    let mut v = (dcq.wrapping_add(4) >> 3).wrapping_add(128);
    if v as u32 > 255 {
        v = !(v >> 31) & 255;
    }
    let row = [v as u8; 8];
    for k in 0..8 {
        dst[k * stride..k * stride + 8].copy_from_slice(&row);
    }
}

/// FDCT_PASS: one 1-D islow forward DCT on v (element k in v[k]); pass 1 scales the output
/// up by 4 (PASS1_BITS), pass 2 removes that again
#[inline(always)]
fn fdct_pass<S: Simd, const P: i32>(x: S, v: [S::V; 8]) -> [S::V; 8] {
    let tmp0 = x.add16(v[0], v[7]);
    let tmp7 = x.sub16(v[0], v[7]);
    let tmp1 = x.add16(v[1], v[6]);
    let tmp6 = x.sub16(v[1], v[6]);
    let tmp2 = x.add16(v[2], v[5]);
    let tmp5 = x.sub16(v[2], v[5]);
    let tmp3 = x.add16(v[3], v[4]);
    let tmp4 = x.sub16(v[3], v[4]);
    let tmp10 = x.add16(tmp0, tmp3);
    let tmp13 = x.sub16(tmp0, tmp3);
    let tmp11 = x.add16(tmp1, tmp2);
    let tmp12 = x.sub16(tmp1, tmp2);
    let mut out0 = x.add16(tmp10, tmp11);
    let mut out4 = x.sub16(tmp10, tmp11);
    let rnd;
    if P == 1 {
        out0 = x.sll16::<2>(out0);
        out4 = x.sll16::<2>(out4);
        rnd = x.splat32(1 << 10);
    } else {
        let two = x.splat16(2);
        out0 = x.sra16::<2>(x.add16(out0, two));
        out4 = x.sra16::<2>(x.add16(out4, two));
        rnd = x.splat32(1 << 14);
    }
    let l = x.unpacklo16(tmp13, tmp12);
    let h = x.unpackhi16(tmp13, tmp12);
    let c130 = pair(x, F_0_541 + F_0_765, F_0_541);
    let c054 = pair(x, F_0_541, F_0_541 - F_1_847);
    let (o2l, o2h) = (x.madd16(l, c130), x.madd16(h, c130));
    let (o6l, o6h) = (x.madd16(l, c054), x.madd16(h, c054));
    // odd part
    let z3 = x.add16(tmp4, tmp6);
    let z4 = x.add16(tmp5, tmp7);
    let zl = x.unpacklo16(z3, z4);
    let zh = x.unpackhi16(z3, z4);
    let c078 = pair(x, F_1_175 - F_1_961, F_1_175);
    let c117 = pair(x, F_1_175, F_1_175 - F_0_390);
    let z3l = x.madd16(zl, c078);
    let z3h = x.madd16(zh, c078);
    let z4l = x.madd16(zl, c117);
    let z4h = x.madd16(zh, c117);
    let al = x.unpacklo16(tmp4, tmp7);
    let ah = x.unpackhi16(tmp4, tmp7);
    let c060 = pair(x, F_0_298 - F_0_899, -F_0_899);
    let c089 = pair(x, -F_0_899, F_1_501 - F_0_899);
    let o7l = x.add32(x.madd16(al, c060), z3l);
    let o7h = x.add32(x.madd16(ah, c060), z3h);
    let o1l = x.add32(x.madd16(al, c089), z4l);
    let o1h = x.add32(x.madd16(ah, c089), z4h);
    let bl = x.unpacklo16(tmp5, tmp6);
    let bh = x.unpackhi16(tmp5, tmp6);
    let c050 = pair(x, F_2_053 - F_2_562, -F_2_562);
    let c256 = pair(x, -F_2_562, F_3_072 - F_2_562);
    let o5l = x.add32(x.madd16(bl, c050), z4l);
    let o5h = x.add32(x.madd16(bh, c050), z4h);
    let o3l = x.add32(x.madd16(bl, c256), z3l);
    let o3h = x.add32(x.madd16(bh, c256), z3h);
    if P == 1 {
        [
            out0,
            desc::<S, 11>(x, o1l, o1h, rnd),
            desc::<S, 11>(x, o2l, o2h, rnd),
            desc::<S, 11>(x, o3l, o3h, rnd),
            out4,
            desc::<S, 11>(x, o5l, o5h, rnd),
            desc::<S, 11>(x, o6l, o6h, rnd),
            desc::<S, 11>(x, o7l, o7h, rnd),
        ]
    } else {
        [
            out0,
            desc::<S, 15>(x, o1l, o1h, rnd),
            desc::<S, 15>(x, o2l, o2h, rnd),
            desc::<S, 15>(x, o3l, o3h, rnd),
            out4,
            desc::<S, 15>(x, o5l, o5h, rnd),
            desc::<S, 15>(x, o6l, o6h, rnd),
            desc::<S, 15>(x, o7l, o7h, rnd),
        ]
    }
}

struct FdctK<'a> {
    src: &'a [u8],
    stride: usize,
    out: &'a mut [i16; 64],
    div: &'a [i16; 192],
}

impl Kernel for FdctK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, _s: S) {
        let x = Sse2;
        let w128 = x.splat16(128);
        let mut v = [x.zero(); 8];
        for (k, r) in v.iter_mut().enumerate() {
            *r = x.sub16(x.load_px(&self.src[k * self.stride..]), w128);
        }
        let v = transpose8(x, v);
        let v = fdct_pass::<Sse2, 1>(x, v); // rows
        let v = transpose8(x, v);
        let v = fdct_pass::<Sse2, 2>(x, v); // columns
        // jpeg_quantize: round(coef / d) (half away from zero) with the reciprocal tables
        let div = self.div;
        for (k, &c) in v.iter().enumerate() {
            let s = x.sra16::<15>(c); // sign
            let a = x.sub16(x.xor(c, s), s); // |x|
            let a = x.add16(a, x.load_i16(&div[64 + k * 8..])); // + correction
            let a = x.mulhiu16(a, x.load_i16(&div[k * 8..])); // * reciprocal
            let a = x.mulhiu16(a, x.load_i16(&div[128 + k * 8..])); // * scale
            x.store_i16(&mut self.out[k * 8..], x.sub16(x.xor(a, s), s));
        }
    }
}

/// jpeg_fdct + jpeg_quantize: forward DCT of 8 rows of 8 samples at src (`stride` bytes
/// apart, level shifted by -128 here; libjpeg's jpeg_fdct_islow, scaled up by 8), then every
/// coefficient divided with the 3 x 64 u16 tables (reciprocal, correction, scale) of
/// libjpeg-turbo's compute_reciprocal: out = round(coef / d) in natural order
pub fn fdct_quant(src: &[u8], stride: usize, out: &mut [i16; 64], div: &[i16; 192]) {
    simd::dispatch(FdctK { src, stride, out, div });
}

// ------------------------------------------------------------------ row kernels
// Written for groups of G = S::BYTES / 2 samples (one group of 8 per 128-bit lane, so every
// sample sees the SSE2 operations of the assembly): the main loop takes whole groups of the
// dispatched width, the rest (and the first group of the upsamplers, whose left neighbour
// is shifted in within a lane) runs as SSE2 groups of 8 -- the bytes written are those of
// the SSE2 kernel for any width.

/// (G = samples per group of the width, round_up(n, 8), samples in whole groups)
#[inline(always)]
fn groups<S: Simd>(n: usize) -> (usize, usize, usize) {
    let g = S::BYTES / 2;
    (g, (n + 7) & !7, n / g * g)
}

/// 2 x 4 pixels per lane from B, G, R byte registers (8 bytes per lane each) with alpha 255
#[inline(always)]
fn put_px<S: Simd>(x: S, dst: &mut [u8], b: S::V, g: S::V, r: S::V) {
    let bg = x.unpacklo8(b, g);
    let ra = x.unpacklo8(r, x.splat8(255));
    x.store_lanes2(dst, x.unpacklo16(bg, ra), x.unpackhi16(bg, ra));
}

/// one group of jpeg_ycc_rgb
#[inline(always)]
fn ycc_group<S: Simd>(x: S, dst: &mut [u8], y: &[u8], cb: &[u8], cr: &[u8]) {
    let w128 = x.splat16(128);
    let w1 = x.splat16(1);
    let b = x.sub16(x.load_px(cb), w128); // Cb - 128
    let r = x.sub16(x.load_px(cr), w128); // Cr - 128
    let mut bb = x.mulhis16(x.add16(b, b), x.splat16(-14942i16 as u16));
    bb = x.sra16::<1>(x.add16(bb, w1));
    bb = x.add16(x.add16(bb, b), b); // B - Y = 1.772 Cb
    let mut rr = x.mulhis16(x.add16(r, r), x.splat16(26345));
    rr = x.sra16::<1>(x.add16(rr, w1));
    rr = x.add16(rr, r); // R - Y = 1.402 Cr
    let cg = pair(x, -22554, 18734);
    let d32768 = x.splat32(32768);
    let lo = x.sra32::<16>(x.add32(x.madd16(x.unpacklo16(b, r), cg), d32768));
    let hi = x.sra32::<16>(x.add32(x.madd16(x.unpackhi16(b, r), cg), d32768));
    let gg = x.sub16(x.packs32(lo, hi), r); // G - Y
    let yy = x.load_px(y);
    let (bo, go, ro) = (x.add16(bb, yy), x.add16(gg, yy), x.add16(rr, yy));
    put_px(x, dst, x.packus16(bo, bo), x.packus16(go, go), x.packus16(ro, ro));
}

struct YccK<'a> {
    dst: &'a mut [u8],
    y: &'a [u8],
    cb: &'a [u8],
    cr: &'a [u8],
    n: usize,
}

impl Kernel for YccK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (g, n8, full) = groups::<S>(self.n);
        let (y, cb, cr) = (&self.y[..n8], &self.cb[..n8], &self.cr[..n8]);
        let (dm, dt) = self.dst[..n8 * 4].split_at_mut(full * 4);
        let ins = y[..full].chunks_exact(g).zip(cb[..full].chunks_exact(g)).zip(cr[..full].chunks_exact(g));
        for (d, ((y, cb), cr)) in dm.chunks_exact_mut(4 * g).zip(ins) {
            ycc_group(x, d, y, cb, cr);
        }
        let ins = y[full..].chunks_exact(8).zip(cb[full..].chunks_exact(8)).zip(cr[full..].chunks_exact(8));
        for (d, ((y, cb), cr)) in dt.chunks_exact_mut(32).zip(ins) {
            ycc_group(Sse2, d, y, cb, cr);
        }
    }
}

/// jpeg_ycc_rgb: n (rounded up to 8) YCbCr samples -> opaque PARGB pixels, exactly like
/// libjpeg's ycc_rgb_convert
pub fn ycc_rgb(dst: &mut [u8], y: &[u8], cb: &[u8], cr: &[u8], n: usize) {
    simd::dispatch(YccK { dst, y, cb, cr, n });
}

/// grey / R, G, B rows -> opaque PARGB (jpeg_gray_rgb / jpeg_rgb_rgb)
struct PlainK<'a> {
    dst: &'a mut [u8],
    r: &'a [u8],
    g: &'a [u8],
    b: &'a [u8],
    n: usize,
}

#[inline(always)]
fn plain_group<S: Simd>(x: S, dst: &mut [u8], r: &[u8], g: &[u8], b: &[u8]) {
    let (r, g, b) = (x.load_px(r), x.load_px(g), x.load_px(b));
    put_px(x, dst, x.packus16(b, b), x.packus16(g, g), x.packus16(r, r));
}

impl Kernel for PlainK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (g, n8, full) = groups::<S>(self.n);
        let (r, gr, b) = (&self.r[..n8], &self.g[..n8], &self.b[..n8]);
        let (dm, dt) = self.dst[..n8 * 4].split_at_mut(full * 4);
        let ins = r[..full].chunks_exact(g).zip(gr[..full].chunks_exact(g)).zip(b[..full].chunks_exact(g));
        for (d, ((r, gr), b)) in dm.chunks_exact_mut(4 * g).zip(ins) {
            plain_group(x, d, r, gr, b);
        }
        let ins = r[full..].chunks_exact(8).zip(gr[full..].chunks_exact(8)).zip(b[full..].chunks_exact(8));
        for (d, ((r, gr), b)) in dt.chunks_exact_mut(32).zip(ins) {
            plain_group(Sse2, d, r, gr, b);
        }
    }
}

/// jpeg_gray_rgb: grey samples -> opaque PARGB (n rounded up to 8)
pub fn gray_rgb(dst: &mut [u8], y: &[u8], n: usize) {
    simd::dispatch(PlainK { dst, r: y, g: y, b: y, n });
}

/// jpeg_rgb_rgb: separate R, G, B rows -> opaque PARGB (n rounded up to 8)
pub fn rgb_rgb(dst: &mut [u8], r: &[u8], g: &[u8], b: &[u8], n: usize) {
    simd::dispatch(PlainK { dst, r, g, b, n });
}

/// exact round(x / 255) = (x + 128 + ((x + 128) >> 8)) >> 8
#[inline(always)]
fn div255(v: u32) -> u32 {
    let v = v + 128;
    (v + (v >> 8)) >> 8
}

/// jpeg_cmyk_rgb: C, M, Y, K rows -> PARGB.  mode 0: Adobe (inverted) CMYK, R = C * K / 255;
/// mode 1: plain CMYK, R = (255 - C) * (255 - K) / 255; mode 2: YCCK, dst already holds the
/// converted YCC part (R', G', B') and R = (255 - R') * K / 255 (Adobe inverted K)
pub fn cmyk_rgb(dst: &mut [u8], c: &[u8], m: &[u8], y: &[u8], k: &[u8], n: usize, mode: u32) {
    for i in 0..n {
        let mut kk = k[i] as u32;
        if mode == 1 {
            kk ^= 255;
        }
        let (r, g, b) = if mode == 2 {
            (dst[i * 4 + 2] as u32 ^ 255, dst[i * 4 + 1] as u32 ^ 255, dst[i * 4] as u32 ^ 255)
        } else if mode == 1 {
            (c[i] as u32 ^ 255, m[i] as u32 ^ 255, y[i] as u32 ^ 255)
        } else {
            (c[i] as u32, m[i] as u32, y[i] as u32)
        };
        let p = 0xFF00_0000 | div255(r * kk) << 16 | div255(g * kk) << 8 | div255(b * kk);
        dst[i * 4..i * 4 + 4].copy_from_slice(&p.to_le_bytes());
    }
}

/// the 2 output samples per input of the h2 upsamplers: even = (prev + 3 cur + re) >> sh,
/// odd = (next + 3 cur + ro) >> sh, interleaved and packed to bytes (2 G bytes)
#[inline(always)]
fn h2_out<S: Simd, const SH: i32>(x: S, dst: &mut [u8], prev: S::V, cur: S::V, next: S::V, re: u16, ro: u16) {
    let c3 = x.add16(x.add16(cur, cur), cur);
    let ev = x.srl16::<SH>(x.add16(x.add16(prev, c3), x.splat16(re)));
    let od = x.srl16::<SH>(x.add16(x.add16(next, c3), x.splat16(ro)));
    x.store(dst, x.packus16(x.unpacklo16(ev, od), x.unpackhi16(ev, od)));
}

/// one group of jpeg_up_h2v1 at sample i >= 1
#[inline(always)]
fn h2v1_group<S: Simd>(x: S, dst: &mut [u8], src: &[u8], i: usize) {
    let (prev, cur, next) = (x.load_px(&src[i - 1..]), x.load_px(&src[i..]), x.load_px(&src[i + 1..]));
    h2_out::<S, 2>(x, &mut dst[2 * i..], prev, cur, next, 1, 2);
}

/// COLSUM3: 3 * this + nbr (i16 lanes)
#[inline(always)]
fn colsum3<S: Simd>(x: S, this: &[u8], nbr: &[u8]) -> S::V {
    let a = x.load_px(this);
    x.add16(x.add16(x.add16(a, a), a), x.load_px(nbr))
}

/// one group of jpeg_up_h2v2 at sample i >= 1
#[inline(always)]
fn h2v2_group<S: Simd>(x: S, dst: &mut [u8], this: &[u8], nbr: &[u8], i: usize) {
    let prev = colsum3(x, &this[i - 1..], &nbr[i - 1..]);
    let cur = colsum3(x, &this[i..], &nbr[i..]);
    let next = colsum3(x, &this[i + 1..], &nbr[i + 1..]);
    h2_out::<S, 4>(x, &mut dst[2 * i..], prev, cur, next, 8, 7);
}

struct UpK<'a> {
    dst: &'a mut [u8],
    this: &'a [u8],
    nbr: &'a [u8], // empty: h2v1
    n: usize,
}

impl Kernel for UpK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (dst, this, nbr, n) = (self.dst, self.this, self.nbr, self.n);
        let v2 = !nbr.is_empty();
        // the first group: the sample left of the row only reaches output 0, which the edge
        // value replaces -- a shifted copy stands in for it
        let y = Sse2;
        if v2 {
            let cur = colsum3(y, this, nbr);
            let next = colsum3(y, &this[1..], &nbr[1..]);
            h2_out::<Sse2, 4>(y, dst, y.bsll::<2>(cur), cur, next, 8, 7);
        } else {
            let cur = y.load_px(this);
            h2_out::<Sse2, 2>(y, dst, y.bsll::<2>(cur), cur, y.load_px(&this[1..]), 1, 2);
        }
        let g = S::BYTES / 2;
        let mut i = 8;
        while i + g <= n {
            if v2 { h2v2_group(x, dst, this, nbr, i) } else { h2v1_group(x, dst, this, i) }
            i += g;
        }
        while i < n {
            if v2 { h2v2_group(y, dst, this, nbr, i) } else { h2v1_group(y, dst, this, i) }
            i += 8;
        }
        // first and last column
        if v2 {
            let s = this[0] as u32 * 3 + nbr[0] as u32;
            dst[0] = ((s * 4 + 8) >> 4) as u8;
            let s = this[n - 1] as u32 * 3 + nbr[n - 1] as u32;
            dst[2 * n - 1] = ((s * 4 + 7) >> 4) as u8;
        } else {
            dst[0] = this[0];
            dst[2 * n - 1] = this[n - 1];
        }
    }
}

/// jpeg_up_h2v1: 2n samples from n: triangle filter like libjpeg's h2v1_fancy_upsample
/// (3/4 nearer + 1/4 further sample, edges replicated)
pub fn up_h2v1(dst: &mut [u8], src: &[u8], n: usize) {
    simd::dispatch(UpK { dst, this: src, nbr: &[], n });
}

/// jpeg_up_h2v2: one output row of libjpeg's h2v2_fancy_upsample: `this` is the nearer
/// input row, `nbr` the next nearest
pub fn up_h2v2(dst: &mut [u8], this: &[u8], nbr: &[u8], n: usize) {
    simd::dispatch(UpK { dst, this, nbr, n });
}

struct UpVK<'a> {
    dst: &'a mut [u8],
    this: &'a [u8],
    nbr: &'a [u8],
    n: usize,
    bias: u16,
}

#[inline(always)]
fn h1v2_group<S: Simd>(x: S, dst: &mut [u8], this: &[u8], nbr: &[u8], bias: u16) {
    let s = x.srl16::<2>(x.add16(colsum3(x, this, nbr), x.splat16(bias)));
    x.store_px(dst, s);
}

impl Kernel for UpVK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (g, n8, full) = groups::<S>(self.n);
        let (t, nb) = (&self.this[..n8], &self.nbr[..n8]);
        let (dm, dt) = self.dst[..n8].split_at_mut(full);
        for (d, (t, nb)) in dm.chunks_exact_mut(g).zip(t[..full].chunks_exact(g).zip(nb[..full].chunks_exact(g))) {
            h1v2_group(x, d, t, nb, self.bias);
        }
        for (d, (t, nb)) in dt.chunks_exact_mut(8).zip(t[full..].chunks_exact(8).zip(nb[full..].chunks_exact(8))) {
            h1v2_group(Sse2, d, t, nb, self.bias);
        }
    }
}

/// jpeg_up_h1v2: (3 * this + nbr + bias) >> 2 (libjpeg-turbo h1v2_fancy_upsample: bias 1 for
/// the upper, 2 for the lower row); n rounded up to 8
pub fn up_h1v2(dst: &mut [u8], this: &[u8], nbr: &[u8], n: usize, bias: u16) {
    simd::dispatch(UpVK { dst, this, nbr, n, bias });
}

/// one group of jpeg_rgb_ycc (G pixels: 4 per lane in each of the two registers)
#[inline(always)]
fn ycc_out_group<S: Simd>(x: S, src: &[u8], y: &mut [u8], cb: &mut [u8], cr: &mut [u8], mode: u32) {
    let dff = x.splat32(255);
    let (mut p0, mut p1) = x.load_lanes2(src);
    if mode != 0 {
        // over white: add 255 - a to B, G and R
        let a0 = x.xor(x.srl32::<24>(p0), dff);
        let a1 = x.xor(x.srl32::<24>(p1), dff);
        let s0 = x.sll32::<8>(a0);
        let s1 = x.sll32::<8>(a1);
        p0 = x.addus8(p0, x.or(x.or(a0, s0), x.sll32::<8>(s0)));
        p1 = x.addus8(p1, x.or(x.or(a1, s1), x.sll32::<8>(s1)));
    }
    let b = x.packs32(x.and(p0, dff), x.and(p1, dff));
    let g = x.packs32(x.and(x.srl32::<8>(p0), dff), x.and(x.srl32::<8>(p1), dff));
    let r = x.packs32(x.and(x.srl32::<16>(p0), dff), x.and(x.srl32::<16>(p1), dff));
    let (rgl, rgh) = (x.unpacklo16(r, g), x.unpackhi16(r, g));
    let (bgl, bgh) = (x.unpacklo16(b, g), x.unpackhi16(b, g));
    let d32768 = x.splat32(32768);
    let dcbcr = x.splat32((128 << 16) + 32767);
    let k16384 = pair(x, 16384, 0);
    // Y = 0.299 R + 0.337 G + 0.114 B + 0.250 G
    let (ky_rg, ky_bg) = (pair(x, 19595, 22086), pair(x, 7471, 16384));
    let lo = x.add32(x.add32(x.madd16(rgl, ky_rg), x.madd16(bgl, ky_bg)), d32768);
    let hi = x.add32(x.add32(x.madd16(rgh, ky_rg), x.madd16(bgh, ky_bg)), d32768);
    x.store_px(y, x.packs32(x.sra32::<16>(lo), x.sra32::<16>(hi)));
    // Cb = -0.16874 R - 0.33126 G + 0.5 B
    let kb_rg = pair(x, -11059, -21709);
    let lo = x.add32(x.add32(x.madd16(rgl, kb_rg), x.sll32::<1>(x.madd16(bgl, k16384))), dcbcr);
    let hi = x.add32(x.add32(x.madd16(rgh, kb_rg), x.sll32::<1>(x.madd16(bgh, k16384))), dcbcr);
    x.store_px(cb, x.packs32(x.sra32::<16>(lo), x.sra32::<16>(hi)));
    // Cr = 0.5 R - 0.41869 G - 0.08131 B
    let kr_bg = pair(x, -5329, -27439);
    let lo = x.add32(x.add32(x.madd16(bgl, kr_bg), x.sll32::<1>(x.madd16(rgl, k16384))), dcbcr);
    let hi = x.add32(x.add32(x.madd16(bgh, kr_bg), x.sll32::<1>(x.madd16(rgh, k16384))), dcbcr);
    x.store_px(cr, x.packs32(x.sra32::<16>(lo), x.sra32::<16>(hi)));
}

struct RgbYccK<'a> {
    src: &'a [u8],
    y: &'a mut [u8],
    cb: &'a mut [u8],
    cr: &'a mut [u8],
    n: usize,
    mode: u32,
}

impl Kernel for RgbYccK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (g, n8, full) = groups::<S>(self.n);
        let src = &self.src[..n8 * 4];
        let (ym, yt) = self.y[..n8].split_at_mut(full);
        let (bm, bt) = self.cb[..n8].split_at_mut(full);
        let (rm, rt) = self.cr[..n8].split_at_mut(full);
        let outs = ym.chunks_exact_mut(g).zip(bm.chunks_exact_mut(g)).zip(rm.chunks_exact_mut(g));
        for (sp, ((y, cb), cr)) in src[..full * 4].chunks_exact(4 * g).zip(outs) {
            ycc_out_group(x, sp, y, cb, cr, self.mode);
        }
        let outs = yt.chunks_exact_mut(8).zip(bt.chunks_exact_mut(8)).zip(rt.chunks_exact_mut(8));
        for (sp, ((y, cb), cr)) in src[full * 4..].chunks_exact(32).zip(outs) {
            ycc_out_group(Sse2, sp, y, cb, cr, self.mode);
        }
    }
}

/// jpeg_rgb_ycc: n BGRA pixels (rounded up to 8) -> Y, Cb, Cr samples (libjpeg
/// rgb_ycc_convert rounding).  mode 0: XRGB (alpha ignored), 1: PARGB composited over white
/// (c + 255 - a)
pub fn rgb_ycc(src: &[u8], y: &mut [u8], cb: &mut [u8], cr: &mut [u8], n: usize, mode: u32) {
    simd::dispatch(RgbYccK { src, y, cb, cr, n, mode });
}

struct DownK<'a> {
    dst: &'a mut [u8],
    r0: &'a [u8],
    r1: &'a [u8], // empty: h2v1
    n: usize,
}

/// one group of jpeg_down_h2v2 / jpeg_down_h2v1 (2 G input samples per row)
#[inline(always)]
fn down_group<S: Simd>(x: S, dst: &mut [u8], r0: &[u8], r1: &[u8]) {
    let m = x.splat16(255);
    let a = x.load(r0);
    if r1.is_empty() {
        let s = x.srl16::<1>(x.add16(x.add16(x.srl16::<8>(a), x.and(a, m)), x.splat32(1 << 16)));
        x.store_px(dst, s);
    } else {
        let b = x.load(r1);
        let s = x.add16(x.add16(x.srl16::<8>(a), x.srl16::<8>(b)), x.add16(x.and(a, m), x.and(b, m)));
        x.store_px(dst, x.srl16::<2>(x.add16(s, x.splat32(1 | (2 << 16)))));
    }
}

impl Kernel for DownK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, x: S) {
        let (g, n8, full) = groups::<S>(self.n);
        let r0 = &self.r0[..2 * n8];
        let (dm, dt) = self.dst[..n8].split_at_mut(full);
        if self.r1.is_empty() {
            for (d, a) in dm.chunks_exact_mut(g).zip(r0[..2 * full].chunks_exact(2 * g)) {
                down_group(x, d, a, &[]);
            }
            for (d, a) in dt.chunks_exact_mut(8).zip(r0[2 * full..].chunks_exact(16)) {
                down_group(Sse2, d, a, &[]);
            }
        } else {
            let r1 = &self.r1[..2 * n8];
            for (d, (a, b)) in dm.chunks_exact_mut(g).zip(r0[..2 * full].chunks_exact(2 * g).zip(r1[..2 * full].chunks_exact(2 * g))) {
                down_group(x, d, a, b);
            }
            for (d, (a, b)) in dt.chunks_exact_mut(8).zip(r0[2 * full..].chunks_exact(16).zip(r1[2 * full..].chunks_exact(16))) {
                down_group(Sse2, d, a, b);
            }
        }
    }
}

/// jpeg_down_h2v2: n output samples (rounded up to 8), each (sum of a 2x2 input box + bias)
/// >> 2 with bias 1, 2, 1, 2, ... (libjpeg)
pub fn down_h2v2(dst: &mut [u8], r0: &[u8], r1: &[u8], n: usize) {
    simd::dispatch(DownK { dst, r0, r1, n });
}

/// jpeg_down_h2v1: n output samples (rounded up to 8), each (sum of 2 horizontal input
/// samples + bias) >> 1 with bias 0, 1, 0, 1, ... (libjpeg h2v1_downsample)
pub fn down_h2v1(dst: &mut [u8], src: &[u8], n: usize) {
    simd::dispatch(DownK { dst, r0: src, r1: &[], n });
}

// ------------------------------------------------------------------ tables

/// zigzag position -> natural index (+16 guard entries)
pub static ZZ: [u8; 80] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, //
    12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, //
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, //
    58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63, //
    63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];

/// Annex K quantisation tables (natural order)
pub static STD_QL: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, //
    14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, //
    18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, //
    49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
pub static STD_QC: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, //
    24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

// Annex K Huffman tables: 16 code counts (lengths 1..16), then the symbols
pub static DC_L_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
pub static DC_L_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
pub static DC_C_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
pub static DC_C_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
pub static AC_L_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
pub static AC_L_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, //
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, //
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, //
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, //
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, //
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, //
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, //
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, //
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, //
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, //
    0xf9, 0xfa,
];
pub static AC_C_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
pub static AC_C_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, //
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, //
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, //
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, //
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, //
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, //
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, //
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, //
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, //
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, //
    0xf9, 0xfa,
];
