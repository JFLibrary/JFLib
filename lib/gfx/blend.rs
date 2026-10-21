// blend -- pixel row kernels (premultiplied 0xAARRGGBB, bytes B G R A)   [src/gfx/blend.s, raster.s]
//
//   fill_solid(row, colour)          store colour into every pixel
//   fill_over(row, pcol, ia)         blend the premultiplied colour over every pixel
//   mask_over(row, mask, pcol)       the colour through an 8-bit coverage row
//   blit_over(dst, src, opacity)     premultiplied src over dst (x opacity 0..255)
//   a8_over(dst, cov, alpha)         8-bit target: c = cov * alpha / 255, dst = c + dst * (255 - c) / 255
//
// "over": dst = src + dst * (255 - src_alpha) / 255 with exact rounding (div255) and the
// saturation of the SSE2 kernels (packuswb).  fill_over, mask_over and a8_over are SIMD
// kernels (simd::Kernel; their fast paths are exact shortcuts, so SSE2 and AVX2 agree);
// blit_over is the assembly's SSE2 loop on pixel pairs, because its pair skip (both
// transparent: dst kept) is visible for invalid premultiplied input.

use rt::fx::div255;
use simd::{Kernel, Simd};

/// k_fill_solid: store one colour into every pixel of the row
pub fn fill_solid(row: &mut [u8], c: u32) {
    let b = c.to_le_bytes();
    for px in row.chunks_exact_mut(4) {
        px.copy_from_slice(&b);
    }
}

/// k_fill_over: blend the premultiplied colour over every pixel
pub fn fill_over(row: &mut [u8], pcol: &[u32; 4], ia: u32) {
    simd::dispatch(FillOver { row, pcol: *pcol, ia });
}

/// k_mask_over: blend the colour through an 8-bit coverage row (mask.len() pixels)
pub fn mask_over(row: &mut [u8], mask: &[u8], pcol: &[u32; 4]) {
    simd::dispatch(MaskOver { row, mask, pcol: *pcol });
}

// round(v / 255) = ((v + 128) * 257) >> 16 for v <= 65025 (pmulhuw)
#[inline(always)]
fn sc<S: Simd>(s: S, v: S::V) -> S::V {
    s.mulhiu16(s.add16(v, s.splat16(128)), s.splat16(257))
}

// the premultiplied colour in the 4 u16 lanes of every pixel
#[inline(always)]
fn pcol_lanes<S: Simd>(s: S, pcol: &[u32; 4]) -> S::V {
    let v = (pcol[0] & 0xFFFF) as u64 | ((pcol[1] & 0xFFFF) as u64) << 16 | ((pcol[2] & 0xFFFF) as u64) << 32 | ((pcol[3] & 0xFFFF) as u64) << 48;
    s.splat64(v)
}

// dst lanes * (255 - a) / 255 + c (pixel lanes, saturated when packed)
#[inline(always)]
fn over<S: Simd>(s: S, d: S::V, ia: S::V, c: S::V) -> S::V {
    s.add16(sc(s, s.mullo16(d, ia)), c)
}

struct FillOver<'a> {
    row: &'a mut [u8],
    pcol: [u32; 4],
    ia: u32,
}

impl Kernel for FillOver<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let FillOver { row, pcol, ia } = self;
        let n = row.len() / 4;
        let nb = S::BYTES;
        let (z, pc, iv) = (s.zero(), pcol_lanes(s, &pcol), s.splat16((ia & 0xFFFF) as u16));
        let full = n * 4 / nb * nb;
        let mut o = 0;
        while o < full {
            let v = s.load(&row[o..o + nb]);
            let lo = over(s, s.unpacklo8(v, z), iv, pc);
            let hi = over(s, s.unpackhi8(v, z), iv, pc);
            s.store(&mut row[o..o + nb], s.packus16(lo, hi));
            o += nb;
        }
        for px in row[full..n * 4].chunks_exact_mut(4) {
            for ch in 0..4 {
                let v = div255(px[ch] as u32 * ia) + pcol[ch];
                px[ch] = v.min(255) as u8;
            }
        }
    }
}

struct MaskOver<'a> {
    row: &'a mut [u8],
    mask: &'a [u8],
    pcol: [u32; 4],
}

impl Kernel for MaskOver<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let MaskOver { row, mask, pcol } = self;
        let n = mask.len().min(row.len() / 4);
        let px = S::PX;
        let pc = pcol_lanes(s, &pcol);
        let w255 = s.splat16(255);
        let solid = pcol[3] >= 255; // opaque colour: full coverage stores it
        let pc32 = (pcol[0] | pcol[1] << 8 | pcol[2] << 16 | pcol[3] << 24).to_le_bytes();
        let full = n / px * px;
        let mut i = 0;
        while i < full {
            let m = &mask[i..i + px];
            let o = i * 4;
            if m.iter().all(|&v| v == 0) {
                // no coverage: unchanged
            } else if solid && m.iter().all(|&v| v == 255) {
                for d in row[o..o + 4 * px].chunks_exact_mut(4) {
                    d.copy_from_slice(&pc32);
                }
            } else {
                let c = sc(s, s.mullo16(s.bcast_px(m), pc)); // c' = c * m / 255
                let ia = s.xor(s.shuf16::<0xFF>(c), w255); // 255 - alpha'
                let v = over(s, s.load_px(&row[o..o + 4 * px]), ia, c);
                s.store_px(&mut row[o..o + 4 * px], v);
            }
            i += px;
        }
        for (d, &m) in row[full * 4..n * 4].chunks_exact_mut(4).zip(&mask[full..n]) {
            if m == 0 {
                continue;
            }
            let m = m as u32;
            let c = [div255(pcol[0] * m), div255(pcol[1] * m), div255(pcol[2] * m), div255(pcol[3] * m)];
            let ia = 255 - c[3];
            for ch in 0..4 {
                let v = div255(d[ch] as u32 * ia) + c[ch];
                d[ch] = v.min(255) as u8;
            }
        }
    }
}

// one pixel of k_blit_over: src scaled by op (unless op is 255), then over dst
#[inline]
fn blit_px(d: &mut [u8], s: &[u8], op: u32, scale: bool) {
    let mut c = [s[0] as u32, s[1] as u32, s[2] as u32, s[3] as u32];
    if scale {
        for v in c.iter_mut() {
            *v = div255(*v * op);
        }
    }
    let ia = 255 - c[3];
    for ch in 0..4 {
        d[ch] = (div255(d[ch] as u32 * ia) + c[ch]).min(255) as u8;
    }
}

/// k_blit_over: premultiplied src over dst, scaled by opacity (pixels = min of both lengths)
pub fn blit_over(dst: &mut [u8], src: &[u8], op: u32) {
    let n = (dst.len() / 4).min(src.len() / 4);
    // pixel pairs with SSE2, as in the assembly
    let x = simd::Sse2;
    let (w255, ov) = (x.splat16(255), x.splat16(op as u16));
    let mut i = 0;
    while i + 2 <= n {
        let o = i * 4;
        let s8 = u64::from_le_bytes(src[o..o + 8].try_into().unwrap());
        if op == 255 {
            let a = s8 & 0xFF00_0000_FF00_0000;
            if a == 0 {
                i += 2; // both transparent
                continue;
            }
            if a == 0xFF00_0000_FF00_0000 {
                dst[o..o + 8].copy_from_slice(&src[o..o + 8]); // both opaque
                i += 2;
                continue;
            }
        }
        let mut c = x.load_px(&src[o..o + 8]);
        if op != 255 {
            c = sc(x, x.mullo16(c, ov));
        }
        let ia = x.xor(x.shuf16::<0xFF>(c), w255);
        let v = over(x, x.load_px(&dst[o..o + 8]), ia, c);
        x.store_px(&mut dst[o..o + 8], v);
        i += 2;
    }
    if i < n {
        // the single last pixel is always scaled (x 255 / 255 is exact)
        blit_px(&mut dst[i * 4..i * 4 + 4], &src[i * 4..i * 4 + 4], op, true);
    }
}

/// k_a8_over: 8-bit coverage target, c = cov * alpha / 255, dst = c + dst * (255 - c) / 255
pub fn a8_over(dst: &mut [u8], cov: &[u8], alpha: u32) {
    simd::dispatch(A8Over { dst, cov, alpha });
}

// k_a8_over: BYTES pixels per step in 16-bit lanes (every intermediate fits: t + 128 <=
// 65153, + (t >> 8) <= 65407); groups without coverage are skipped, full coverage at alpha
// 255 stores 255; the tail per pixel
// round(v / 255) = (t + (t >> 8)) >> 8, t = v + 128 (functions, not closures: a closure is
// compiled on its own, outside the AVX2 context of simd::dispatch)
#[inline(always)]
fn a8_round<S: Simd>(s: S, v: S::V) -> S::V {
    let t = s.add16(v, s.splat16(128));
    s.srl16::<8>(s.add16(t, s.srl16::<8>(t)))
}

// 8 coverage / dst lanes: c = cov * alpha / 255, dst = c + dst * (255 - c) / 255
#[inline(always)]
fn a8_half<S: Simd>(s: S, cv: S::V, dv: S::V, av: S::V) -> S::V {
    let c = a8_round(s, s.mullo16(cv, av));
    s.add16(a8_round(s, s.mullo16(dv, s.sub16(s.splat16(255), c))), c)
}

struct A8Over<'a> {
    dst: &'a mut [u8],
    cov: &'a [u8],
    alpha: u32,
}

impl Kernel for A8Over<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let A8Over { dst, cov, alpha } = self;
        let n = dst.len().min(cov.len());
        let nb = S::BYTES;
        let all = if nb == 16 { 0xFFFF } else { 0xFFFF_FFFF };
        let (z, ff, av) = (s.zero(), s.splat8(255), s.splat16(alpha as u16));
        let full = n / nb * nb;
        let mut o = 0;
        while o < full {
            let cv = s.load(&cov[o..o + nb]);
            if s.movemask8(s.cmpeq8(cv, z)) != all {
                let v = if alpha == 255 && s.movemask8(s.cmpeq8(cv, ff)) == all {
                    ff
                } else {
                    let dv = s.load(&dst[o..o + nb]);
                    let lo = a8_half(s, s.unpacklo8(cv, z), s.unpacklo8(dv, z), av);
                    let hi = a8_half(s, s.unpackhi8(cv, z), s.unpackhi8(dv, z), av);
                    s.packus16(lo, hi)
                };
                s.store(&mut dst[o..o + nb], v);
            }
            o += nb;
        }
        for (d, &m) in dst[full..n].iter_mut().zip(&cov[full..n]) {
            let c = div255(m as u32 * alpha);
            *d = (c + div255(*d as u32 * (255 - c))) as u8;
        }
    }
}
