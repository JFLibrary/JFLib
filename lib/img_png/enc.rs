// enc -- the PNG encoder  [src/img/png.s: png_encode_cfg, png_encode, filters]
//
//   encode_cfg(sf, cfg) -> Ok(file) / Err(ERR_DIMS / ERR_UNSUPPORTED / ERR_NOMEM / ERR_CANCEL)
//   encode_classic(sf, level) -> Option<file>     png_encode / png_encode_ex (PNGD_CLASSIC)
//   put_chunk(out, type, data)                    length, type, data, CRC-32
//
// PngCfg::depth: 0 auto: A8 -> grey; <= 256 distinct straight colours (alpha 0 counts as
// one) -> 8-bit palette with per-entry tRNS (lossless); else RGB8 when opaque, RGBA8
// otherwise; 32 RGBA8; 24 RGB8 (premultiplied colour over white); 8 palette from
// quantize_ex(qcfg) (dither level + transparency threshold, the transparent entry in tRNS);
// PNGD_CLASSIC: A8 grey, XRGB RGB, else RGB when opaque / RGBA.  Per row the filter with the
// smallest sum of |signed bytes| is chosen (palette rows: filter 0); one zlib stream at
// cfg.level; chunks (complete, CRC included) right after IHDR; IDAT split every 1 GB.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_A8, FMT_ARGB, FMT_PARGB, FMT_XRGB};
use img_core::checksum::crc32;
use img_core::deflate::zlib_deflate;
use img_core::meta::{all_opaque, get_row};
use img_core::quantize::{IQF_KEEPALPHA, IQF_WHITE, quantize_ex};
use img_core::*;
use simd::{Kernel, Simd};

/// PngCfg::depth: png_encode's rule (A8 grey, XRGB RGB, else RGB when opaque / RGBA)
pub const PNGD_CLASSIC: u32 = 0xFFFF_FFFF;
const PNG_LEVEL: u32 = 6;

/// the options of encode_cfg (PNGCFG)
#[derive(Clone, Copy, Default)]
pub struct PngCfg<'a> {
    pub level: u32,       // deflate level 0..9
    pub depth: u32,       // 0 auto / 32 / 24 / 8 / PNGD_CLASSIC
    pub qcfg: u32,        // quantize_ex cfg for 8 bit (dither level | threshold << 8)
    pub chunks: &'a [u8], // complete chunks written after IHDR
    pub cancel: Option<&'a AtomicU32>,
}

/// append a chunk: length, type (4 bytes), data, CRC-32 of type + data  [png_put_chunk]
pub fn put_chunk(out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let s = out.len();
    out.extend_from_slice(typ);
    out.extend_from_slice(data);
    let c = crc32(0, &out[s..]);
    out.extend_from_slice(&c.to_be_bytes());
}

/// png_encode_ex: classic depth rule, deflate level 0..9 (png_encode: 6); None for an empty
/// surface or an unknown format
pub fn encode_classic(sf: &Surface, level: u32) -> Option<Vec<u8>> {
    encode_cfg(sf, &PngCfg { level, depth: PNGD_CLASSIC, ..Default::default() }).ok()
}

/// png_encode (level 6)
pub fn encode_default(sf: &Surface) -> Option<Vec<u8>> {
    encode_classic(sf, PNG_LEVEL)
}

// filters: a = left (0 for x < bpp), b = above (prev; zeroes on the first row), c = above-left;
// written as zipped slices so that the compiler vectorizes them (the assembly: SSE2, 16 / 8
// bytes per step)
fn filt_sub(dst: &mut [u8], cur: &[u8], bpp: usize) {
    let n = cur.len();
    let k = bpp.min(n);
    dst[..k].copy_from_slice(&cur[..k]);
    for (d, (&x, &a)) in dst[k..n].iter_mut().zip(cur[k..].iter().zip(&cur[..n - k])) {
        *d = x.wrapping_sub(a);
    }
}

fn filt_up(dst: &mut [u8], cur: &[u8], prev: &[u8]) {
    for (d, (&x, &b)) in dst.iter_mut().zip(cur.iter().zip(prev)) {
        *d = x.wrapping_sub(b);
    }
}

fn filt_avg(dst: &mut [u8], cur: &[u8], prev: &[u8], bpp: usize) {
    let n = cur.len();
    let k = bpp.min(n);
    for (d, (&x, &b)) in dst[..k].iter_mut().zip(cur.iter().zip(prev)) {
        *d = x.wrapping_sub(b >> 1);
    }
    for (d, ((&x, &a), &b)) in dst[k..n].iter_mut().zip(cur[k..].iter().zip(&cur[..n - k]).zip(&prev[k..n])) {
        *d = x.wrapping_sub(((a as u16 + b as u16) >> 1) as u8);
    }
}

const PAD: usize = 16; // zero bytes around a row: a / c of the first pixel, a whole last step

// filt_paeth on padded rows (cur / prev: PAD zero bytes, the n row bytes, PAD bytes; dst:
// n + PAD bytes): 4 * PX bytes per step in 16-bit lanes, as the assembly's SSE2 loop
struct PaethK<'a> {
    dst: &'a mut [u8],
    cur: &'a [u8],
    prev: &'a [u8],
    n: usize,
    bpp: usize,
}

impl Kernel for PaethK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let PaethK { dst, cur, prev, n, bpp } = self;
        let (z, m8) = (s.zero(), s.splat16(0xFF));
        let mut i = 0;
        while i < n {
            let o = PAD + i;
            let a = s.load_px(&cur[o - bpp..]);
            let b = s.load_px(&prev[o..]);
            let c = s.load_px(&prev[o - bpp..]);
            let x = s.load_px(&cur[o..]);
            let bc = s.sub16(b, c);
            let ac = s.sub16(a, c);
            let abc = s.add16(bc, ac); // a + b - 2c
            let pa = s.maxs16(bc, s.sub16(z, bc));
            let pb = s.maxs16(ac, s.sub16(z, ac));
            let pc = s.maxs16(abc, s.sub16(z, abc));
            let bsel = s.xor(s.and(s.xor(b, c), s.cmpgts16(pb, pc)), b); // pb > pc: c, else b
            let m = s.or(s.cmpgts16(pa, pb), s.cmpgts16(pa, pc)); // pa > pb or pa > pc: not a
            let pred = s.xor(s.and(s.xor(a, bsel), m), a);
            s.store_px(&mut dst[i..], s.and(s.sub16(x, pred), m8));
            i += 4 * S::PX;
        }
    }
}

// sum_abs: the sum over the bytes of |byte as signed| (pminub(v, -v), psadbw)
struct SumAbsK<'a>(&'a [u8]);

impl Kernel for SumAbsK<'_> {
    type Out = u64;
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> u64 {
        let b = self.0;
        let (z, nb) = (s.zero(), S::BYTES);
        let mut acc = z;
        let mut i = 0;
        while i + nb <= b.len() {
            let v = s.load(&b[i..]);
            acc = s.add64(acc, s.sadu8(s.minu8(v, s.sub8(z, v)), z));
            i += nb;
        }
        let mut t = [0u8; 32];
        s.store(&mut t, acc);
        let mut sum: u64 = t[..nb].chunks_exact(8).map(|q| u64::from_le_bytes([q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7]])).sum();
        for &v in &b[i..] {
            sum += (v as i8).unsigned_abs() as u64;
        }
        sum
    }
}

fn sum_abs(b: &[u8]) -> u64 {
    simd::dispatch(SumAbsK(b))
}

/// encode sf (FMT_XRGB, PARGB, ARGB or A8) with a PngCfg  [png_encode_cfg]
pub fn encode_cfg(sf: &Surface, cfg: &PngCfg) -> Result<Vec<u8>, i32> {
    if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() {
        return Err(ERR_DIMS);
    }
    if sf.fmt > FMT_ARGB {
        return Err(ERR_UNSUPPORTED);
    }
    let (w, h) = (sf.w as usize, sf.h as usize);
    let mut pal = [0u32; 256];
    let mut npal = 0usize;
    let mut idx: Vec<u8> = Vec::new();
    // png_quant: quantize_ex into an index image; 0 colours = none
    let quant = |qcfg: u32, pal: &mut [u32; 256], idx: &mut Vec<u8>| -> usize {
        *idx = vec![0u8; w * h + 64];
        let (n, _) = quantize_ex(sf, 256, qcfg, pal, idx);
        if n == 0 {
            *idx = Vec::new();
        }
        n as usize
    };
    // ---- colour type: (mode = bytes per pixel, kind = row source, PNG colour type)
    #[derive(PartialEq)]
    enum K {
        Grey,
        Pal,
        Rgb,
        Rgba,
    }
    let opaque_kind = || if sf.fmt == FMT_XRGB || all_opaque(sf) { K::Rgb } else { K::Rgba };
    let kind = match cfg.depth {
        32 => K::Rgba,
        24 => K::Rgb,
        8 => {
            npal = quant(cfg.qcfg & !(IQF_KEEPALPHA | IQF_WHITE), &mut pal, &mut idx);
            if npal == 0 {
                return Err(ERR_NOMEM);
            }
            K::Pal
        }
        _ if sf.fmt == FMT_A8 => K::Grey,
        PNGD_CLASSIC => opaque_kind(),
        _ => {
            // auto: an exact palette (alpha per entry) when there are <= 256 colours
            npal = quant(IQF_KEEPALPHA, &mut pal, &mut idx);
            if npal != 0 { K::Pal } else { opaque_kind() }
        }
    };
    let (mode, ctype) = match kind {
        K::Grey => (1usize, 0u8),
        K::Pal => (1, 3),
        K::Rgb => (3, 2),
        K::Rgba => (4, 6),
    };
    // tRNS length: last entry with alpha < 255, + 1
    let ntrns = if kind == K::Pal { (0..npal).rev().find(|&i| pal[i] >> 24 != 255).map_or(0, |i| i + 1) } else { 0 };
    // ---- rows
    let rb = w * mode;
    let mut raw = vec![0u8; (rb + 1) * h];
    // cur / prev: PAD zero bytes, the row, PAD bytes; filtered rows of rb + PAD bytes
    let mut curb = vec![0u8; rb + 2 * PAD];
    let mut prevb = vec![0u8; rb + 2 * PAD];
    let fs = rb + PAD;
    let mut filt = vec![0u8; fs * 4];
    let mut scratch = vec![0u8; w * 4];
    let stride = sf.stride as usize;
    for y in 0..h {
        if cfg.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
            return Err(ERR_CANCEL);
        }
        // png_src_row
        let cur = &mut curb[PAD..PAD + rb];
        match kind {
            K::Grey => cur.copy_from_slice(&sf.px[y * stride..y * stride + w]),
            K::Pal => cur.copy_from_slice(&idx[y * w..y * w + w]),
            K::Rgb => {
                get_row(sf, y, &mut scratch, FMT_PARGB);
                for (d, s) in cur.chunks_exact_mut(3).zip(scratch.chunks_exact(4)) {
                    let p = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    let v = p.wrapping_add((p >> 24 ^ 255) * 0x010101); // c + (255 - a): over white
                    d.copy_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                }
            }
            K::Rgba => {
                get_row(sf, y, cur, FMT_ARGB);
                for p in cur.chunks_exact_mut(4) {
                    p.swap(0, 2); // B G R A -> R G B A
                }
            }
        }
        let o = y * (rb + 1);
        let (cur, prev) = (&curb[PAD..PAD + rb], &prevb[PAD..PAD + rb]);
        let best = if kind == K::Pal {
            0
        } else {
            let (f1, rest) = filt.split_at_mut(fs);
            let (f2, rest) = rest.split_at_mut(fs);
            let (f3, f4) = rest.split_at_mut(fs);
            filt_sub(&mut f1[..rb], cur, mode);
            filt_up(&mut f2[..rb], cur, prev);
            filt_avg(&mut f3[..rb], cur, prev, mode);
            simd::dispatch(PaethK { dst: f4, cur: &curb, prev: &prevb, n: rb, bpp: mode });
            let mut best = 0usize;
            let mut bs = sum_abs(cur);
            for f in 1..5 {
                let s = sum_abs(&filt[(f - 1) * fs..(f - 1) * fs + rb]);
                if s < bs {
                    bs = s;
                    best = f;
                }
            }
            best
        };
        raw[o] = best as u8;
        if best == 0 {
            raw[o + 1..o + 1 + rb].copy_from_slice(&curb[PAD..PAD + rb]);
        } else {
            raw[o + 1..o + 1 + rb].copy_from_slice(&filt[(best - 1) * fs..(best - 1) * fs + rb]);
        }
        core::mem::swap(&mut curb, &mut prevb);
    }
    let z = zlib_deflate(&raw, cfg.level as i32);
    drop(raw);
    // ---- file
    let mut out = Vec::with_capacity(z.len() + (z.len() >> 30) * 12 + 81 + cfg.chunks.len() + npal * 3 + ntrns);
    out.extend_from_slice(&img_core::io::PNG_SIG);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&(w as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = ctype;
    put_chunk(&mut out, b"IHDR", &ihdr);
    out.extend_from_slice(cfg.chunks);
    if kind == K::Pal {
        let mut pl = Vec::with_capacity(npal * 3);
        for &c in &pal[..npal] {
            pl.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
        }
        put_chunk(&mut out, b"PLTE", &pl);
        if ntrns != 0 {
            let tr: Vec<u8> = pal[..ntrns].iter().map(|&c| (c >> 24) as u8).collect();
            put_chunk(&mut out, b"tRNS", &tr);
        }
    }
    for part in z.chunks(1 << 30) {
        put_chunk(&mut out, b"IDAT", part);
    }
    if z.is_empty() {
        put_chunk(&mut out, b"IDAT", &[]);
    }
    put_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}
