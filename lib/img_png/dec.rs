// dec -- the PNG decoder  [src/img/png.s: png_decode_fl and its helpers]
//
// All colour types (grey, RGB, palette, grey + alpha, RGBA), bit depths 1 / 2 / 4 / 8 / 16,
// PLTE, tRNS (palette alpha, grey / RGB colour key compared at full 16-bit precision), Adam7,
// all five filters.  CRCs of the critical chunks (and tRNS) are checked; other ancillary
// chunks are ignored.  The result is FMT_PARGB (premultiplied, exact rounding) or, with
// JDF_STRAIGHT, FMT_ARGB with the file's straight values (colour kept at alpha 0; a
// colour-key pixel keeps its colour with alpha 0).  16-bit samples: round(v / 257).
// w, h <= 32768 and w * h <= 2^28 (ERR_DIMS).  APNG: the default image is decoded.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, FMT_PARGB, SFF_OPAQUE};
use simd::{Kernel, Simd};
use img_core::checksum::crc32;
use img_core::inflate::zlib_inflate;
use img_core::*;

const MAX_DIM: u32 = 32768;
const MAX_PIXELS: u64 = 1 << 28;

const T_IHDR: u32 = u32::from_le_bytes(*b"IHDR");
const T_PLTE: u32 = u32::from_le_bytes(*b"PLTE");
const T_IDAT: u32 = u32::from_le_bytes(*b"IDAT");
const T_IEND: u32 = u32::from_le_bytes(*b"IEND");
const T_TRNS: u32 = u32::from_le_bytes(*b"tRNS");

// Adam7 passes 1..7 (index 0 = the whole image): x0, y0, log2 dx, log2 dy
static ADAM7: [[u32; 4]; 8] =
    [[0, 0, 0, 0], [0, 0, 3, 3], [4, 0, 3, 3], [0, 4, 2, 3], [2, 0, 2, 2], [0, 2, 1, 2], [1, 0, 1, 1], [0, 1, 0, 1]];
// valid bit depths per colour type (bit n = depth n)
static DEPTHS: [u32; 7] = [0x10116, 0, 0x10100, 0x116, 0x10100, 0, 0x10100];
pub(crate) static CHANS: [u32; 8] = [1, 0, 3, 1, 2, 0, 4, 0];

#[inline]
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
fn be16(d: &[u8], o: usize) -> u32 {
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

// round(v / 257) of a 16-bit sample (U16TO8)
#[inline]
fn u16to8(v: u32) -> u32 {
    (v * 255 + 32895) >> 16
}

// round(x * a / 255) (PMUL)
#[inline]
fn pmul(x: u32, a: u32) -> u32 {
    let t = x * a + 128;
    (t + (t >> 8)) >> 8
}

// the decoder context (PD_*)
struct Pd {
    lut: [u32; 256], // palette / grey -> pixel
    w: u32,
    h: u32,
    depth: u32,
    ctype: u32,
    ilace: u32,
    bits: u32, // bits per pixel
    bpp: usize, // filter distance in bytes
    npal: usize,
    plte: usize, // offset of the PLTE data
    trns: Option<(usize, usize)>, // tRNS data (offset, length)
    haskey: bool,
    key: [u32; 3], // 16-bit colour key (grey uses key[0])
    straight: bool,
}

impl Pd {
    // adam_dims: width / height of an Adam7 pass (pass 0 = the whole image)
    fn dims(&self, pass: usize) -> (u32, u32) {
        let a = &ADAM7[pass];
        let w = if self.w > a[0] { (self.w - a[0] + (1 << a[2]) - 1) >> a[2] } else { 0 };
        let h = if self.h > a[1] { (self.h - a[1] + (1 << a[3]) - 1) >> a[3] } else { 0 };
        (w, h)
    }

    // png_rowbytes: bytes of one row of n pixels
    fn rowbytes(&self, n: u32) -> usize {
        ((n as u64 * self.bits as u64 + 7) >> 3) as usize
    }

    // png_build_lut: parse tRNS, build the palette / grey lookup table
    fn build_lut(&mut self, data: &[u8]) {
        if let Some((t, n)) = self.trns {
            if self.ctype == 0 && n >= 2 {
                self.key[0] = be16(data, t);
                self.haskey = true;
            } else if self.ctype == 2 && n >= 6 {
                self.key = [be16(data, t), be16(data, t + 2), be16(data, t + 4)];
                self.haskey = true;
            }
        }
        if self.ctype == 0 && self.depth <= 8 {
            // grey: value i -> i * 255 / (2^depth - 1)
            let max = (1u32 << self.depth) - 1;
            let scale = 255 / max;
            for i in 0..=max {
                let g = (i * scale) * 0x010101;
                self.lut[i as usize] = if self.haskey && i == self.key[0] {
                    if self.straight { g } else { 0 }
                } else {
                    g | 0xFF00_0000
                };
            }
        } else if self.ctype == 3 {
            let tr = self.trns.unwrap_or((0, 0));
            for i in 0..256usize {
                if i >= self.npal {
                    self.lut[i] = 0xFF00_0000; // missing entries: opaque black
                    continue;
                }
                let p = self.plte + i * 3;
                let (mut r, mut g, mut b) = (data[p] as u32, data[p + 1] as u32, data[p + 2] as u32);
                let a = if self.trns.is_some() && i < tr.1 { data[tr.0 + i] as u32 } else { 255 };
                if a != 255 && !self.straight {
                    r = pmul(r, a);
                    g = pmul(g, a);
                    b = pmul(b, a);
                }
                self.lut[i] = a << 24 | r << 16 | g << 8 | b;
            }
        }
    }

    // png_conv_row: n pixels of one unfiltered row -> n consecutive pixels in dst (the
    // caller scatters an Adam7 pass row)
    fn conv_row(&self, src: &[u8], n: usize, dst: &mut [u8]) {
        let dst = &mut dst[..n * 4];
        let st = self.straight;
        let d = self.depth;
        match self.ctype {
            3 | 0 if self.ctype == 3 || d <= 8 => {
                if d == 8 {
                    for (o, &v) in dst.chunks_exact_mut(4).zip(&src[..n]) {
                        o.copy_from_slice(&self.lut[v as usize].to_le_bytes());
                    }
                } else {
                    let mask = (1u32 << d) - 1;
                    let per = (8 / d) as usize;
                    for (i, o) in dst.chunks_exact_mut(4).enumerate() {
                        let byte = src[i / per] as u32;
                        let sh = 8 - d * (i % per + 1) as u32;
                        o.copy_from_slice(&self.lut[((byte >> sh) & mask) as usize].to_le_bytes());
                    }
                }
            }
            0 => {
                // grey 16 bit (colour key compared on 16 bits)
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(2)) {
                    let v = u16::from_be_bytes([sp[0], sp[1]]) as u32;
                    let px = if self.haskey && v == self.key[0] {
                        if st { u16to8(v) * 0x010101 } else { 0 }
                    } else {
                        u16to8(v) * 0x010101 | 0xFF00_0000
                    };
                    o.copy_from_slice(&px.to_le_bytes());
                }
            }
            2 if d == 8 => {
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                    let (r, g, b) = (sp[0] as u32, sp[1] as u32, sp[2] as u32);
                    let px = if self.haskey && r == self.key[0] && g == self.key[1] && b == self.key[2] {
                        if st { r << 16 | g << 8 | b } else { 0 }
                    } else {
                        r << 16 | g << 8 | b | 0xFF00_0000
                    };
                    o.copy_from_slice(&px.to_le_bytes());
                }
            }
            2 => {
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(6)) {
                    let (r, g, b) = (be16(sp, 0), be16(sp, 2), be16(sp, 4));
                    let key = self.haskey && r == self.key[0] && g == self.key[1] && b == self.key[2];
                    let px = if key && !st {
                        0
                    } else {
                        u16to8(r) << 16 | u16to8(g) << 8 | u16to8(b) | if key { 0 } else { 0xFF00_0000 }
                    };
                    o.copy_from_slice(&px.to_le_bytes());
                }
            }
            4 => {
                let k = if d == 16 { 4 } else { 2 };
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(k)) {
                    let (mut g, a) =
                        if d == 16 { (u16to8(be16(sp, 0)), u16to8(be16(sp, 2))) } else { (sp[0] as u32, sp[1] as u32) };
                    let px = if !st && a == 0 {
                        0
                    } else {
                        if !st && a != 255 {
                            g = pmul(g, a);
                        }
                        g * 0x010101 | a << 24
                    };
                    o.copy_from_slice(&px.to_le_bytes());
                }
            }
            _ if d == 8 => {
                // RGBA 8: swap R / B, premultiply with the kernels of gfx (exact / 255)
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    o.copy_from_slice(&[sp[2], sp[1], sp[0], sp[3]]);
                }
                if !st {
                    gfx::straight::premul_inplace(dst);
                }
            }
            _ => {
                for (o, sp) in dst.chunks_exact_mut(4).zip(src.chunks_exact(8)) {
                    let (mut r, mut g, mut b, a) =
                        (u16to8(be16(sp, 0)), u16to8(be16(sp, 2)), u16to8(be16(sp, 4)), u16to8(be16(sp, 6)));
                    let px = if !st && a == 0 {
                        0
                    } else {
                        if !st && a != 255 {
                            r = pmul(r, a);
                            g = pmul(g, a);
                            b = pmul(b, a);
                        }
                        r << 16 | g << 8 | b | a << 24
                    };
                    o.copy_from_slice(&px.to_le_bytes());
                }
            }
        }
    }
}

// png_unfilter: undo the filters in place (each row: filter byte + rb data bytes)
fn unfilter(rows: &mut [u8], rb: usize, nrows: usize, bpp: usize) -> i32 {
    let stride = rb + 1;
    for r in 0..nrows {
        let (before, cur) = rows.split_at_mut(r * stride);
        let ft = cur[0];
        let cur = &mut cur[1..stride];
        let prev: Option<&[u8]> = if r == 0 { None } else { Some(&before[(r - 1) * stride + 1..r * stride]) };
        if ft > 4 {
            return ERR_BLOCK;
        }
        match (ft, prev) {
            (0, _) | (2, None) | (5.., _) => {}
            (1, _) | (4, None) => {
                for x in bpp..rb {
                    cur[x] = cur[x].wrapping_add(cur[x - bpp]);
                }
            }
            (2, Some(p)) => {
                for x in 0..rb {
                    cur[x] = cur[x].wrapping_add(p[x]);
                }
            }
            (3, None) => {
                for x in bpp..rb {
                    cur[x] = cur[x].wrapping_add(cur[x - bpp] >> 1);
                }
            }
            (3, Some(p)) if bpp == 4 => simd::dispatch_sse2(UnfK::<4> { cur, prev: p, paeth: false }),
            (3, Some(p)) if bpp == 3 => simd::dispatch_sse2(UnfK::<3> { cur, prev: p, paeth: false }),
            (4, Some(p)) if bpp == 4 => simd::dispatch_sse2(UnfK::<4> { cur, prev: p, paeth: true }),
            (4, Some(p)) if bpp == 3 => simd::dispatch_sse2(UnfK::<3> { cur, prev: p, paeth: true }),
            (3, Some(p)) => {
                for x in 0..bpp.min(rb) {
                    cur[x] = cur[x].wrapping_add(p[x] >> 1);
                }
                for x in bpp..rb {
                    let s = (cur[x - bpp] as u32 + p[x] as u32) >> 1;
                    cur[x] = cur[x].wrapping_add(s as u8);
                }
            }
            (_, Some(p)) => {
                for x in 0..bpp.min(rb) {
                    cur[x] = cur[x].wrapping_add(p[x]);
                }
                for x in bpp..rb {
                    let a = cur[x - bpp] as i32;
                    let b = p[x] as i32;
                    let c = p[x - bpp] as i32;
                    let pa = (b - c).abs();
                    let pb = (a - c).abs();
                    let pc = (a + b - 2 * c).abs();
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    cur[x] = cur[x].wrapping_add(pred as u8);
                }
            }
        }
    }
    0
}

// one row of 3- / 4-byte pixels with a prior row, Avg or Paeth: one pixel per step in u16
// lanes, the left pixel kept in a register (the assembly's xmm loop)
struct UnfK<'a, const BPP: usize> {
    cur: &'a mut [u8],
    prev: &'a [u8],
    paeth: bool,
}

#[inline(always)]
fn rd4(b: &[u8], x: usize) -> i32 {
    if x + 4 <= b.len() {
        u32::from_le_bytes([b[x], b[x + 1], b[x + 2], b[x + 3]]) as i32
    } else {
        let mut t = [0u8; 4];
        let k = b.len() - x;
        t[..k].copy_from_slice(&b[x..]);
        u32::from_le_bytes(t) as i32
    }
}

impl<const BPP: usize> Kernel for UnfK<'_, BPP> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let z = s.zero();
        let n = self.cur.len() / BPP * BPP;
        let mut a = z; // left pixel, u16 lanes
        let mut c = z; // above-left
        let mut x = 0;
        while x < n {
            let b = s.unpacklo8(s.from_i32(rd4(self.prev, x)), z);
            let pred = if self.paeth {
                let bc = s.sub16(b, c);
                let ac = s.sub16(a, c);
                let abc = s.add16(bc, ac);
                let pa = s.maxs16(bc, s.sub16(z, bc));
                let pb = s.maxs16(ac, s.sub16(z, ac));
                let pc = s.maxs16(abc, s.sub16(z, abc));
                let m1 = s.cmpgts16(pb, pc); // pb > pc: c, else b
                let bsel = s.xor(b, s.and(s.xor(b, c), m1));
                let m2 = s.or(s.cmpgts16(pa, pb), s.cmpgts16(pa, pc));
                s.xor(a, s.and(s.xor(a, bsel), m2))
            } else {
                s.srl16::<1>(s.add16(a, b))
            };
            let px = s.add8(s.from_i32(rd4(self.cur, x)), s.packus16(pred, pred));
            let v = (s.to_i32(px) as u32).to_le_bytes();
            self.cur[x..x + BPP].copy_from_slice(&v[..BPP]);
            c = b;
            a = s.unpacklo8(px, z);
            x += BPP;
        }
    }
}

/// decode a PNG file into a new surface: FMT_PARGB, or FMT_ARGB with JDF_STRAIGHT; on an
/// error out is left as it is  [png_decode_fl]
pub fn decode_fl(data: &[u8], out: &mut Surface, flags: u32) -> i32 {
    match decode_inner(data, flags & JDF_STRAIGHT != 0) {
        Ok(sf) => {
            *out = sf;
            0
        }
        Err(e) => e,
    }
}

/// decode into a new FMT_PARGB surface  [png_decode]
pub fn decode(data: &[u8], out: &mut Surface) -> i32 {
    decode_fl(data, out, 0)
}

fn decode_inner(data: &[u8], straight: bool) -> Result<Surface, i32> {
    let len = data.len();
    if len < 8 {
        return Err(ERR_TRUNC);
    }
    if data[..8] != img_core::io::PNG_SIG {
        return Err(ERR_HDR);
    }
    let mut pd = Pd {
        lut: [0; 256],
        w: 0,
        h: 0,
        depth: 0,
        ctype: 0,
        ilace: 0,
        bits: 0,
        bpp: 0,
        npal: 0,
        plte: 0,
        trns: None,
        haskey: false,
        key: [0; 3],
        straight,
    };
    // ---- chunk scan
    let mut p = 8usize;
    let mut ihdr = false;
    let mut iend = false;
    let mut idatsz = 0usize;
    loop {
        let rest = len - p;
        if rest == 0 {
            break;
        }
        if rest < 12 {
            return Err(ERR_TRUNC);
        }
        let n = be32(data, p);
        if n > 0x7FFF_FFFF {
            return Err(ERR_CHUNK);
        }
        let n = n as usize;
        if n > rest - 12 {
            return Err(ERR_TRUNC);
        }
        let typ = le32(data, p + 4);
        let crc_ok = || crc32(0, &data[p + 4..p + 8 + n]) == be32(data, p + 8 + n);
        if typ & 0x20 != 0 {
            // ancillary
            if !ihdr {
                return Err(ERR_CHUNK);
            }
            if typ == T_TRNS && crc_ok() {
                pd.trns = Some((p + 8, n)); // (a damaged tRNS is ignored)
            }
        } else {
            if !crc_ok() {
                return Err(ERR_CRC);
            }
            if typ == T_IHDR {
                if ihdr || n != 13 {
                    return Err(ERR_CHUNK);
                }
                ihdr = true;
                let (w, h) = (be32(data, p + 8), be32(data, p + 12));
                if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM || w as u64 * h as u64 > MAX_PIXELS {
                    return Err(ERR_DIMS);
                }
                pd.w = w;
                pd.h = h;
                let depth = data[p + 16] as u32;
                let ct = data[p + 17] as u32;
                if data[p + 18] != 0 || data[p + 19] != 0 || data[p + 20] > 1 {
                    return Err(ERR_HDR);
                }
                pd.ilace = data[p + 20] as u32;
                if ct > 6 || depth > 16 || DEPTHS[ct as usize] >> depth & 1 == 0 {
                    return Err(ERR_HDR);
                }
                pd.depth = depth;
                pd.ctype = ct;
                pd.bits = depth * CHANS[ct as usize];
                pd.bpp = ((pd.bits >> 3) as usize).max(1);
            } else if !ihdr {
                return Err(ERR_CHUNK); // IHDR must come first
            } else if typ == T_IDAT {
                idatsz += n;
            } else if typ == T_PLTE {
                if n == 0 || n > 768 || n % 3 != 0 {
                    return Err(ERR_CHUNK);
                }
                pd.npal = n / 3;
                pd.plte = p + 8;
            } else if typ == T_IEND {
                iend = true;
                break;
            } else {
                return Err(ERR_UNSUPPORTED); // unknown critical chunk
            }
        }
        p += n + 12;
    }
    if !ihdr {
        return Err(ERR_TRUNC);
    }
    if idatsz == 0 {
        return Err(if iend { ERR_CHUNK } else { ERR_TRUNC });
    }
    if pd.ctype == 3 && pd.npal == 0 {
        return Err(ERR_CHUNK);
    }
    // ---- gather IDAT
    let mut idat = Vec::with_capacity(idatsz);
    let mut p = 8usize;
    while len - p >= 12 {
        let n = be32(data, p) as usize;
        let typ = le32(data, p + 4);
        if typ == T_IEND {
            break;
        }
        if typ == T_IDAT {
            idat.extend_from_slice(&data[p + 8..p + 8 + n]);
        }
        p += n + 12;
    }
    // ---- inflate
    let passes = if pd.ilace != 0 { 1..8 } else { 0..1 };
    let mut rawsz = 0usize;
    for ps in passes.clone() {
        let (pw, ph) = pd.dims(ps);
        if pw != 0 && ph != 0 {
            rawsz += (pd.rowbytes(pw) + 1) * ph as usize;
        }
    }
    // deflate expands at most ~1032:1 -- do not allocate for impossible sizes
    if rawsz as u64 > idatsz as u64 * 1040 + 4096 {
        return Err(ERR_TRUNC);
    }
    let mut raw = vec![0u8; rawsz];
    let n = zlib_inflate(&idat, &mut raw)?;
    if n != rawsz {
        return Err(ERR_TRUNC);
    }
    drop(idat);
    // ---- unfilter
    let mut off = 0usize;
    for ps in passes.clone() {
        let (pw, ph) = pd.dims(ps);
        if pw == 0 || ph == 0 {
            continue;
        }
        let rb = pd.rowbytes(pw);
        let sz = (rb + 1) * ph as usize;
        let r = unfilter(&mut raw[off..off + sz], rb, ph as usize, pd.bpp);
        if r != 0 {
            return Err(r);
        }
        off += sz;
    }
    // ---- convert
    pd.build_lut(data);
    let mut sf = Surface::new(pd.w as i32, pd.h as i32, if straight { FMT_ARGB } else { FMT_PARGB });
    if sf.px.is_empty() {
        return Err(ERR_NOMEM);
    }
    let stride = sf.stride as usize;
    let mut off = 0usize;
    let mut tmp = if pd.ilace != 0 { vec![0u8; pd.w as usize * 4] } else { Vec::new() };
    for ps in passes {
        let (pw, ph) = pd.dims(ps);
        if pw == 0 || ph == 0 {
            continue;
        }
        let rb = pd.rowbytes(pw);
        let a = &ADAM7[ps];
        for j in 0..ph as usize {
            let y = (j << a[3]) + a[1] as usize;
            let src = &raw[off + 1..off + 1 + rb];
            if ps == 0 {
                pd.conv_row(src, pw as usize, &mut sf.px[y * stride..(y + 1) * stride]);
            } else {
                pd.conv_row(src, pw as usize, &mut tmp);
                let row = &mut sf.px[y * stride..(y + 1) * stride];
                for (i, px) in tmp[..pw as usize * 4].chunks_exact(4).enumerate() {
                    let o = (a[0] as usize + (i << a[2])) * 4;
                    row[o..o + 4].copy_from_slice(px);
                }
            }
            off += rb + 1;
        }
    }
    // opaque hint for types without any alpha
    if pd.ctype != 4 && pd.ctype != 6 && pd.trns.is_none() {
        sf.flags |= SFF_OPAQUE;
    }
    Ok(sf)
}
