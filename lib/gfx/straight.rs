// straight -- row kernels on straight (not premultiplied) 0xAARRGGBB pixels  [src/gfx/straight.s]
//
//   premul_row(dst, src)            straight -> premultiplied: c' = round(c * a / 255)
//   unpremul_row(dst, src)          premultiplied -> straight, the rounding of png.s:
//                                   c = min(255, (c * ceil(255 * 2^24 / a) + 2^23) >> 24)
//   lerp_mask_row(dst, a, b, m)     dst = round((a * (255 - m) + b * m) / 255), 4 channels
//   lerp_mask_into_a / _into_b      the same with dst = a / dst = b
//   erase_row(dst, cov, strength)   alpha = round(alpha * (255 - e) / 255), e = round(cov *
//                                   strength / 255); colour kept
//   view_row(dst, src, x0, y0, cfg)     straight src over a checkerboard -> XRGB row
//   view_row_pm(dst, src, x0, y0, cfg)  premultiplied src over a checkerboard -> XRGB row
//   *_px: the same for one pixel (u32), for callers whose rows alias
//
// Rows hold 4 bytes per pixel (B, G, R, A); a kernel works on as many pixels as its
// shortest row has.  The assembly's fast paths (opaque / transparent groups) are exact
// shortcuts of these per-pixel formulas, so the results are the same bytes.

use rt::fx::div255;
use simd::{Kernel, Simd};

/// the checkerboard of view_row: cell size (>= 1), colour of the even cells (cx + cy) and
/// of the odd ones (alpha ignored)
#[derive(Clone, Copy)]
pub struct ViewCfg {
    pub cell: i32,
    pub c0: u32,
    pub c1: u32,
}

// st_rcp[a] = ceil(255 * 2^24 / a) = floor((255 * 2^24 - 1) / a) + 1 (png.s unpm_init)
static RCP: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 1;
    while i < 256 {
        t[i] = 0xFEFF_FFFF / i as u32 + 1;
        i += 1;
    }
    t
};

#[inline]
fn px_of(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline]
fn ch(p: u32, k: u32) -> u32 {
    (p >> (k * 8)) & 255
}

pub fn premul_px(p: u32) -> u32 {
    let a = p >> 24;
    (p & 0xFF00_0000) | div255(ch(p, 2) * a) << 16 | div255(ch(p, 1) * a) << 8 | div255(ch(p, 0) * a)
}

pub fn unpremul_px(p: u32) -> u32 {
    let a = p >> 24;
    if a == 0 {
        return 0;
    }
    if a == 255 {
        return p;
    }
    let r = RCP[a as usize] as u64;
    let f = |c: u32| {
        let v = ((c as u64 * r + (1 << 23)) >> 24) as u32;
        core::hint::select_unpredictable(v > 255, 255, v) // cmova (random data: no branch)
    };
    (p & 0xFF00_0000) | f(ch(p, 2)) << 16 | f(ch(p, 1)) << 8 | f(ch(p, 0))
}

pub fn lerp_px(a: u32, b: u32, m: u32) -> u32 {
    let mut out = 0;
    for k in 0..4 {
        out |= div255(ch(a, k) * (255 - m) + ch(b, k) * m) << (k * 8);
    }
    out
}

pub fn erase_px(p: u32, cov: u32, strength: u32) -> u32 {
    let e = div255(cov * (strength & 255));
    let a = div255((p >> 24) * (255 - e));
    (p & 0x00FF_FFFF) | a << 24
}

/// the opaque checkerboard colour of pixel (x, y)
pub fn view_bg(x: i32, y: i32, cfg: &ViewCfg) -> u32 {
    let cell = cfg.cell.max(1);
    let cx = x.div_euclid(cell);
    let cy = y.div_euclid(cell);
    let c = if cx.wrapping_add(cy) & 1 != 0 { cfg.c1 } else { cfg.c0 };
    c | 0xFF00_0000
}

/// straight s over bg: round((s * a + bg * (255 - a)) / 255), alpha 255
pub fn view_mix(s: u32, bg: u32) -> u32 {
    let a = s >> 24;
    let mut out = 0xFF00_0000;
    for k in 0..3 {
        out |= div255(ch(s, k) * a + ch(bg, k) * (255 - a)) << (k * 8);
    }
    out
}

/// premultiplied s over bg: s + round(bg * (255 - a) / 255) (saturated), alpha 255
pub fn view_mix_pm(s: u32, bg: u32) -> u32 {
    let a = s >> 24;
    let mut out = 0xFF00_0000;
    for k in 0..3 {
        out |= (ch(s, k) + div255(ch(bg, k) * (255 - a))).min(255) << (k * 8);
    }
    out
}

// ---- rows: SIMD kernels (simd::Kernel) with the assembly's fast paths; tails per pixel

// movemask8 value of an all-true compare
#[inline(always)]
fn all<S: Simd>() -> u32 {
    if S::BYTES == 16 { 0xFFFF } else { 0xFFFF_FFFF }
}

// round(v / 255) for v <= 65025
#[inline(always)]
fn sc<S: Simd>(s: S, v: S::V) -> S::V {
    s.mulhiu16(s.add16(v, s.splat16(128)), s.splat16(257))
}

enum StK<'a> {
    Premul(&'a mut [u8], Option<&'a [u8]>),
    Unpremul(&'a mut [u8], Option<&'a [u8]>),
    Lerp(&'a mut [u8], Option<&'a [u8]>, Option<&'a [u8]>, &'a [u8]), // None: dst itself
    Erase(&'a mut [u8], &'a [u8], u32),
    View(&'a mut [u8], &'a [u8], bool), // dst already holds the checkerboard; premultiplied src
}

impl Kernel for StK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        match self {
            StK::Premul(d, src) => premul_k(s, d, src),
            StK::Unpremul(d, src) => unpremul_k(s, d, src),
            StK::Lerp(d, a, b, m) => lerp_k(s, d, a, b, m),
            StK::Erase(d, c, st) => erase_k(s, d, c, st),
            StK::View(d, src, pm) => view_k(s, d, src, pm),
        }
    }
}

// k_premul_row: groups of BYTES / 4 pixels; all opaque: unchanged, all transparent: 0
#[inline(always)]
fn premul_k<S: Simd>(s: S, dst: &mut [u8], src: Option<&[u8]>) {
    let n = src.map_or(dst.len(), |x| x.len().min(dst.len())) / 4;
    let nb = S::BYTES;
    let (z, d255) = (s.zero(), s.splat32(255));
    let (rgbw, aw255) = (s.splat64(0x0000_FFFF_FFFF_FFFF), s.splat64(0x00FF_0000_0000_0000));
    let full = n * 4 / nb * nb;
    let mut o = 0;
    while o < full {
        let v = s.load(&src.unwrap_or(&*dst)[o..o + nb]);
        let a = s.srl32::<24>(v);
        let r = if s.movemask8(s.cmpeq32(a, d255)) == all::<S>() {
            v
        } else if s.movemask8(s.cmpeq32(a, z)) == all::<S>() {
            z
        } else {
            let (lo, hi) = (s.unpacklo8(v, z), s.unpackhi8(v, z));
            let ml = s.or(s.and(s.shuf16::<0xFF>(lo), rgbw), aw255); // multipliers a, a, a, 255
            let mh = s.or(s.and(s.shuf16::<0xFF>(hi), rgbw), aw255);
            s.packus16(sc(s, s.mullo16(lo, ml)), sc(s, s.mullo16(hi, mh)))
        };
        s.store(&mut dst[o..o + nb], r);
        o += nb;
    }
    for i in full / 4..n {
        let p = px_of(&src.unwrap_or(&*dst)[i * 4..]);
        dst[i * 4..i * 4 + 4].copy_from_slice(&premul_px(p).to_le_bytes());
    }
}

// k_unpremul_row: groups of BYTES / 4 pixels checked for all opaque / all transparent, the
// others per pixel (table of reciprocals)
#[inline(always)]
fn unpremul_k<S: Simd>(s: S, dst: &mut [u8], src: Option<&[u8]>) {
    let n = src.map_or(dst.len(), |x| x.len().min(dst.len())) / 4;
    let nb = S::BYTES;
    let (z, d255) = (s.zero(), s.splat32(255));
    let full = n * 4 / nb * nb;
    let mut o = 0;
    while o < full {
        let v = s.load(&src.unwrap_or(&*dst)[o..o + nb]);
        let a = s.srl32::<24>(v);
        if s.movemask8(s.cmpeq32(a, d255)) == all::<S>() {
            s.store(&mut dst[o..o + nb], v);
        } else if s.movemask8(s.cmpeq32(a, z)) == all::<S>() {
            s.store(&mut dst[o..o + nb], z);
        } else {
            let mut t = [0u8; 32];
            t[..nb].copy_from_slice(&src.unwrap_or(&*dst)[o..o + nb]);
            for q in t[..nb].chunks_exact_mut(4) {
                let v = unpremul_px(px_of(q));
                q.copy_from_slice(&v.to_le_bytes());
            }
            dst[o..o + nb].copy_from_slice(&t[..nb]);
        }
        o += nb;
    }
    for i in full / 4..n {
        let p = px_of(&src.unwrap_or(&*dst)[i * 4..]);
        dst[i * 4..i * 4 + 4].copy_from_slice(&unpremul_px(p).to_le_bytes());
    }
}

// k_lerp_mask_row: PX pixels per step; m all 0: a, all 255: b (a / b None: dst itself)
#[inline(always)]
fn lerp_k<S: Simd>(s: S, dst: &mut [u8], a: Option<&[u8]>, b: Option<&[u8]>, m: &[u8]) {
    let len = |r: Option<&[u8]>| r.map_or(usize::MAX, |r| r.len() / 4);
    let n = m.len().min(dst.len() / 4).min(len(a)).min(len(b));
    let (px, nb) = (S::PX, 4 * S::PX);
    let w255 = s.splat16(255);
    let full = n / px * px;
    let mut i = 0;
    while i < full {
        let o = i * 4;
        let mm = &m[i..i + px];
        let ar = s.load_raw(&a.unwrap_or(&*dst)[o..o + nb]);
        let br = s.load_raw(&b.unwrap_or(&*dst)[o..o + nb]);
        let r = if mm.iter().all(|&v| v == 0) {
            ar
        } else if mm.iter().all(|&v| v == 255) {
            br
        } else {
            let mv = s.bcast_px(mm);
            let v = s.add16(s.mullo16(s.widen_px(ar), s.sub16(w255, mv)), s.mullo16(s.widen_px(br), mv));
            let mut t = [0u8; 16];
            s.store_px(&mut t, sc(s, v));
            s.load_raw(&t)
        };
        s.store_raw(&mut dst[o..o + nb], r);
        i += px;
    }
    for i in full..n {
        let pa = px_of(&a.unwrap_or(&*dst)[i * 4..]);
        let pb = px_of(&b.unwrap_or(&*dst)[i * 4..]);
        let v = lerp_px(pa, pb, m[i] as u32);
        dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

// k_erase_row: BYTES / 4 pixels per step (KE_PX); no coverage: unchanged
#[inline(always)]
fn erase_k<S: Simd>(s: S, dst: &mut [u8], cov: &[u8], strength: u32) {
    let n = cov.len().min(dst.len() / 4);
    let nb = S::BYTES;
    let g = nb / 4; // pixels per step
    let (z, w255, st) = (s.zero(), s.splat16(255), s.splat16((strength & 255) as u16));
    let rgb = s.splat32(0x00FF_FFFF);
    let full = n / g * g;
    let mut i = 0;
    while i < full {
        let c = &cov[i..i + g];
        if c.iter().any(|&v| v != 0) {
            let o = i * 4;
            let v = s.load(&dst[o..o + nb]);
            let e = sc(s, s.mullo16(s.unpacklo8(s.load_lane32(c), z), st)); // round(cov * strength / 255)
            let al = s.srl32::<24>(v);
            let al = s.packs32(al, al); // alphas in u16 lanes 0..3
            let na = sc(s, s.mullo16(al, s.sub16(w255, e)));
            let na = s.sll32::<24>(s.unpacklo16(na, z));
            s.store(&mut dst[o..o + nb], s.or(s.and(v, rgb), na));
        }
        i += g;
    }
    for i in full..n {
        let v = erase_px(px_of(&dst[i * 4..]), cov[i] as u32, strength);
        dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

// k_view_row / k_view_row_pm over the checkerboard in dst: groups of BYTES / 4 pixels; all
// opaque: src, all transparent: the checkerboard (premultiplied: bg + src saturated), else
// KV_MIX2 / KW_MIX2 on PX pixels
#[inline(always)]
fn view_k<S: Simd>(s: S, dst: &mut [u8], src: &[u8], pm: bool) {
    let n = src.len().min(dst.len()) / 4;
    let (nb, hb) = (S::BYTES, 4 * S::PX);
    let (z, d255, w255) = (s.zero(), s.splat32(255), s.splat16(255));
    let aff = s.splat32(0xFF00_0000);
    let aw255 = s.splat64(0x00FF_0000_0000_0000);
    let mix = |sv: S::V, bg: S::V| -> S::V {
        let a = s.shuf16::<0xFF>(sv);
        if pm {
            s.add16(sc(s, s.mullo16(bg, s.sub16(w255, a))), sv) // s + round(bg * (255 - a) / 255)
        } else {
            sc(s, s.add16(s.mullo16(bg, s.sub16(w255, a)), s.mullo16(sv, a)))
        }
    };
    let full = n * 4 / nb * nb;
    let mut o = 0;
    while o < full {
        let v = s.load(&src[o..o + nb]);
        let a = s.srl32::<24>(v);
        if s.movemask8(s.cmpeq32(a, d255)) == all::<S>() {
            s.store(&mut dst[o..o + nb], s.or(v, aff));
        } else if s.movemask8(s.cmpeq32(a, z)) == all::<S>() {
            if pm {
                let bg = s.load(&dst[o..o + nb]);
                s.store(&mut dst[o..o + nb], s.or(s.addus8(v, bg), aff));
            }
        } else {
            for h in [o, o + hb] {
                let c = mix(s.load_px(&src[h..]), s.load_px(&dst[h..]));
                s.store_px(&mut dst[h..h + hb], s.or(c, aw255)); // alpha 255 (saturated)
            }
        }
        o += nb;
    }
    for i in full / 4..n {
        let bg = px_of(&dst[i * 4..]);
        let p = px_of(&src[i * 4..]);
        let v = if pm { view_mix_pm(p, bg) } else { view_mix(p, bg) };
        dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}

// view_fill: the opaque checkerboard colours of (x0 .. x0 + n - 1, y0), cell by cell
fn view_fill(dst: &mut [u8], x0: i32, y0: i32, cfg: &ViewCfg) {
    let n = dst.len() / 4;
    let cell = cfg.cell.max(1);
    let cx = x0.div_euclid(cell);
    let mut left = (cell - x0.rem_euclid(cell)) as usize; // pixels left in the first cell
    let cy = y0.div_euclid(cell);
    let (mut c, mut d) = (cfg.c0 | 0xFF00_0000, cfg.c1 | 0xFF00_0000);
    if cx.wrapping_add(cy) & 1 != 0 {
        core::mem::swap(&mut c, &mut d);
    }
    let mut i = 0;
    while i < n {
        let k = left.min(n - i);
        for px in dst[i * 4..(i + k) * 4].chunks_exact_mut(4) {
            px.copy_from_slice(&c.to_le_bytes());
        }
        i += k;
        core::mem::swap(&mut c, &mut d);
        left = cell as usize;
    }
}

/// k_premul_row
pub fn premul_row(dst: &mut [u8], src: &[u8]) {
    simd::dispatch(StK::Premul(dst, Some(src)));
}

pub fn premul_inplace(buf: &mut [u8]) {
    simd::dispatch(StK::Premul(buf, None));
}

/// k_unpremul_row
pub fn unpremul_row(dst: &mut [u8], src: &[u8]) {
    simd::dispatch(StK::Unpremul(dst, Some(src)));
}

pub fn unpremul_inplace(buf: &mut [u8]) {
    simd::dispatch(StK::Unpremul(buf, None));
}

/// k_lerp_mask_row: pixels = m.len() (bounded by the rows)
pub fn lerp_mask_row(dst: &mut [u8], a: &[u8], b: &[u8], m: &[u8]) {
    simd::dispatch(StK::Lerp(dst, Some(a), Some(b), m));
}

/// k_lerp_mask_row with dst = a
pub fn lerp_mask_into_a(a: &mut [u8], b: &[u8], m: &[u8]) {
    simd::dispatch(StK::Lerp(a, None, Some(b), m));
}

/// k_lerp_mask_row with dst = b
pub fn lerp_mask_into_b(a: &[u8], b: &mut [u8], m: &[u8]) {
    simd::dispatch(StK::Lerp(b, Some(a), None, m));
}

/// k_erase_row: pixels = cov.len() (bounded by the row)
pub fn erase_row(dst: &mut [u8], cov: &[u8], strength: u32) {
    simd::dispatch(StK::Erase(dst, cov, strength));
}

/// k_view_row: straight src over the checkerboard; (x0, y0) = checkerboard coordinates of
/// the first pixel
pub fn view_row(dst: &mut [u8], src: &[u8], x0: i32, y0: i32, cfg: &ViewCfg) {
    let n = dst.len().min(src.len()) / 4;
    view_fill(&mut dst[..n * 4], x0, y0, cfg);
    simd::dispatch(StK::View(&mut dst[..n * 4], &src[..n * 4], false));
}

/// k_view_row_pm: premultiplied src over the checkerboard
pub fn view_row_pm(dst: &mut [u8], src: &[u8], x0: i32, y0: i32, cfg: &ViewCfg) {
    let n = dst.len().min(src.len()) / 4;
    view_fill(&mut dst[..n * 4], x0, y0, cfg);
    simd::dispatch(StK::View(&mut dst[..n * 4], &src[..n * 4], true));
}
