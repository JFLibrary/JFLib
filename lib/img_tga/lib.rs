// img_tga -- Truevision TGA reader / writer                            [src/img/tga.s]
//
//   probe(data, meta) -> recognised                tga_probe
//   decode(data, sf, dec, meta) -> 0 / ERR_*       tga_decode
//   encode(sf, opt, out) -> 0 / ERR_*              tga_encode
//   CODEC                                          tga_codec
//
// TGA has no magic number, so the format detection only reports TGA when the caller's hint
// (file extension) says so or probe accepts the header.  probe is deliberately strict: colour
// map type 0 / 1, image type 1 / 2 / 3 / 9 / 10 / 11, a colour map exactly when the type needs
// one (entry size 15 / 16 / 24 / 32, length > 0; without one all colour map fields 0), width and
// height > 0, a pixel depth that fits the type (colour mapped 8; true colour 15 / 16 / 24 / 32;
// grey 8 / 16), descriptor bits 6..7 zero, alpha bits 0 / 1 (16 bit), 0 / 8 (32 bit, 16-bit
// grey), 0 otherwise, at least one byte of pixel data after the ID and the colour map, and for
// uncompressed types all pixels present.
//
// Decoder (more lenient than the probe): image types 1 / 2 / 3 and RLE 9 / 10 / 11 (types 32 /
// 33 and 0: ERR_UNSUPPORTED); colour mapped with 8- or 16-bit indices and 15 / 16 / 24 / 32-bit
// entries starting at the first-entry index (indices outside the map: opaque black); true colour
// 15 / 16 / 24 / 32 bit; grey 8 bit and 16 bit (grey + alpha byte); a colour map of a
// true-colour image is skipped; the ID field is skipped; all four origins (descriptor bit 4
// right-to-left, bit 5 top-to-bottom); RLE packets may cross rows; interleaved images
// (descriptor bits 6..7) ERR_UNSUPPORTED.  5-bit channels expand by bit replication.
// Alpha: with a v2 footer ("TRUEVISION-XFILE.") and an extension area (size >= 495) the
// attributes type decides: 3 = straight alpha, 4 = premultiplied (FMT_PARGB result, colours
// clamped to alpha, IMF_SRC_PREMUL), anything else = opaque.  Without one the storage decides:
// 32-bit pixels / entries and 16-bit grey carry alpha, 16-bit pixels / entries when the
// descriptor has alpha bits; then an alpha channel that is zero everywhere counts as opaque.
// Missing or truncated pixel data: ERR_TRUNC (the minimum size -- RLE: one packet per 128
// pixels -- is checked before the surface is allocated).  Result: FMT_ARGB (straight).
//
// Encoder: 32-bit BGRA with straight alpha (descriptor alpha bits 8) or 24-bit BGR (bitdepth
// 24; transparent pixels are composited over white), bitdepth 0 / 32 = 32 bit, others
// ERR_UNSUPPORTED; IOF_RLE -> type 10 with packets that never cross a row (runs of >= 2 equal
// pixels become run packets); rows bottom-up (origin lower left); a v2 extension area
// (attributes type 3 for 32 bit, 0 for 24 bit) and footer.  Width / height above 65535:
// ERR_DIMS.  Codec iom: IOM_RLE | IOM_BITDEPTH.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use img_core::meta::{dims_check, get_row, surf_alloc};
use img_core::*;

// pixel kinds of the decoder
const TK_M8: u32 = 1; // 8-bit colour map index
const TK_M16: u32 = 2; // 16-bit colour map index
const TK_T16: u32 = 3; // 15 / 16-bit A1R5G5B5
const TK_T24: u32 = 4;
const TK_T32: u32 = 5;
const TK_G8: u32 = 6;
const TK_G16: u32 = 7; // grey + alpha byte

const TGA_EXTSIZE: usize = 495;
const TGA_FOOTSIZE: usize = 26;
const SIG: &[u8; 18] = b"TRUEVISION-XFILE.\0";
const MODELS: [u8; 4] = [0, IMM_PAL, IMM_RGB, IMM_GRAY];

/// tga_codec
pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode),
    encode: Some(encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_RLE | IOM_BITDEPTH,
    ..Codec::EMPTY
};

#[inline(always)]
fn u16_at(d: &[u8], o: usize) -> u32 {
    d[o] as u32 | (d[o + 1] as u32) << 8
}

/// tga_probe: the strict header check described in the module header; meta: size, depth,
/// model, alpha (w = h = 0 when not recognised)
pub fn probe(d: &[u8], m: &mut ImgMeta) -> bool {
    let ok = probe_hdr(d, m);
    if !ok {
        m.w = 0;
        m.h = 0;
    }
    ok
}

fn probe_hdr(d: &[u8], m: &mut ImgMeta) -> bool {
    if d.len() < 18 {
        return false;
    }
    let (cmt, it) = (d[1] as u32, d[2] as u32);
    if cmt > 1 {
        return false;
    }
    let cls = it & 0xF7; // 9 / 10 / 11 -> 1 / 2 / 3
    if cls.wrapping_sub(1) > 2 || (it > 3 && it < 9) {
        return false;
    }
    let (maplen, mapbits) = (u16_at(d, 5), d[7] as u32);
    let mut mapbytes = 0u64;
    if cls == 1 && cmt != 1 {
        return false;
    }
    if cls == 1 || cmt != 0 {
        // a colour map (true colour: one that is skipped)
        if maplen == 0 || !matches!(mapbits, 15 | 16 | 24 | 32) {
            return false;
        }
        mapbytes = ((mapbits + 7) >> 3) as u64 * maplen as u64;
    } else if maplen != 0 || mapbits != 0 || u16_at(d, 3) != 0 {
        return false;
    }
    let w = u16_at(d, 12);
    if w == 0 {
        return false;
    }
    m.w = w;
    let h = u16_at(d, 14);
    if h == 0 {
        return false;
    }
    m.h = h;
    let (depth, desc) = (d[16] as u32, d[17] as u32);
    if desc & 0xC0 != 0 {
        return false;
    }
    let abits = desc & 15;
    let ok = match (cls, depth) {
        (1, 8) | (3, 8) | (2, 15) | (2, 24) => abits == 0,
        (2, 16) => abits <= 1,
        (2, 32) | (3, 16) => abits == 0 || abits == 8,
        _ => false,
    };
    if !ok {
        return false;
    }
    // pixel data must start inside the file; uncompressed images complete
    let start = d[0] as u64 + mapbytes + 18;
    if start >= d.len() as u64 {
        return false;
    }
    if it <= 8 && start + ((depth + 7) >> 3) as u64 * w as u64 * h as u64 > d.len() as u64 {
        return false;
    }
    m.bits = if depth <= 16 && cls == 2 { 5 } else { 8 };
    m.model = MODELS[cls as usize];
    m.channels = if cls == 2 { 3 } else { 1 };
    if abits != 0 {
        m.channels += 1;
        m.flags |= IMF_ALPHA;
    }
    m.flags |= IMF_LOSSLESS;
    m.frames = 1;
    true
}

// ------------------------------------------------------------------ decoder

// the decoder context (TG_*)
struct Tg {
    aor: u32,   // 0xFF000000 when the alpha is not used
    a16: bool,  // bit 15 of 16-bit pixels / entries is alpha
    pal: Vec<u32>,
}

impl Tg {
    // tga_px: one stored pixel at d[o..] (kind k) -> straight BGRA
    #[inline(always)]
    fn px(&self, k: u32, d: &[u8], o: usize) -> u32 {
        match k {
            TK_T32 => u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) | self.aor,
            TK_T24 => u32::from_le_bytes([d[o], d[o + 1], d[o + 2], 0xFF]),
            TK_M8 => self.pal[d[o] as usize],
            TK_T16 => {
                let v = u16_at(d, o);
                let x5 = |c: u32| (c << 3) | (c >> 2);
                let rgb = x5(v & 31) | x5((v >> 5) & 31) << 8 | x5((v >> 10) & 31) << 16;
                let a = if self.a16 { (v >> 15).wrapping_neg() << 24 } else { 0xFF00_0000 };
                a | rgb
            }
            TK_G8 => d[o] as u32 * 0x010101 | 0xFF00_0000,
            TK_G16 => d[o] as u32 * 0x010101 | (d[o + 1] as u32) << 24 | self.aor,
            _ => self.pal[u16_at(d, o) as usize], // TK_M16
        }
    }
}

// the pixel loops (TGA_ROWSET / TGA_PUT): stream rows placed by the origin bits
struct Fill {
    w: usize,
    h: usize,
    topdown: bool, // descriptor bit 5
    rtl: bool,     // descriptor bit 4
    rle: bool,
    pos: usize, // pixel data
}

impl Fill {
    // byte offset of the first pixel of stream row `row`
    #[inline(always)]
    fn row_start(&self, sf: &Surface, row: usize) -> usize {
        let y = if self.topdown { row } else { self.h - 1 - row };
        y * sf.stride as usize + if self.rtl { (self.w - 1) * 4 } else { 0 }
    }

    // decode every pixel of kind K (B bytes per stored pixel) -> the OR of all pixels;
    // ERR_TRUNC (RLE data ends early) / ERR_CANCEL (polled at every new row)
    fn run<const K: u32, const B: usize>(&self, tg: &Tg, d: &[u8], sf: &mut Surface, dec: &ImgDec) -> Result<u32, i32> {
        let (w, h) = (self.w, self.h);
        let mut acc = 0u32;
        if !self.rle {
            // uncompressed: w * h * B bytes (checked before the surface was allocated)
            for row in 0..h {
                if row > 0 && dec.cancelled() {
                    return Err(ERR_CANCEL);
                }
                let src = &d[self.pos + row * w * B..self.pos + (row + 1) * w * B];
                let o = self.row_start(sf, row);
                if self.rtl {
                    let dst = &mut sf.px[o + 4 - w * 4..o + 4];
                    for (q, s) in dst.chunks_exact_mut(4).rev().zip(src.chunks_exact(B)) {
                        let v = tg.px(K, s, 0);
                        acc |= v;
                        q.copy_from_slice(&v.to_le_bytes());
                    }
                } else {
                    let dst = &mut sf.px[o..o + w * 4];
                    for (q, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(B)) {
                        let v = tg.px(K, s, 0);
                        acc |= v;
                        q.copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
            return Ok(acc);
        }
        // RLE: packets may cross rows
        let len = d.len();
        let step: isize = if self.rtl { -4 } else { 4 };
        let (mut row, mut col) = (0usize, 0usize);
        let mut o = self.row_start(sf, 0) as isize;
        let mut left = w * h;
        let mut pos = self.pos;
        while left > 0 {
            if pos >= len {
                return Err(ERR_TRUNC);
            }
            let hdr = d[pos] as usize;
            pos += 1;
            let run = hdr & 0x80 != 0;
            let mut v = 0;
            for k in 0..(hdr & 0x7F) + 1 {
                if !run || k == 0 {
                    if len - pos < B {
                        return Err(ERR_TRUNC);
                    }
                    v = tg.px(K, &d[pos..pos + B], 0);
                    pos += B;
                }
                acc |= v;
                let q = o as usize;
                sf.px[q..q + 4].copy_from_slice(&v.to_le_bytes());
                o += step;
                left -= 1;
                col += 1;
                if col == w {
                    col = 0;
                    row += 1;
                    if row < h {
                        if dec.cancelled() {
                            return Err(ERR_CANCEL);
                        }
                        o = self.row_start(sf, row) as isize;
                    }
                }
                if left == 0 {
                    break;
                }
            }
        }
        Ok(acc)
    }
}

/// tga_decode: decode (see the module header); meta is filled from the header; sf holds the
/// (partial) surface also when a later error is returned
pub fn decode(d: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    let len = d.len();
    if len < 18 {
        return ERR_TRUNC;
    }
    let it = d[2] as u32;
    if it == 0 || it == 32 || it == 33 {
        return ERR_UNSUPPORTED; // no image data, Huffman / delta / RLE, four-pass quadtree
    }
    let cls = it & 0xF7; // 9 / 10 / 11 -> 1 / 2 / 3: 1 mapped, 2 true colour, 3 grey
    if cls.wrapping_sub(1) > 2 {
        return ERR_HDR;
    }
    let rle = (it >> 3) & 1 != 0;
    // colour map (skipped for true colour / grey)
    let cmt = d[1] as u32;
    if cmt > 1 {
        return ERR_HDR;
    }
    let (mapfirst, maplen, mapbits) = (u16_at(d, 3), u16_at(d, 5), d[7] as u32);
    let mapoff = d[0] as usize + 18;
    let mapbytes = if cmt != 0 { ((mapbits + 7) >> 3) as usize * maplen as usize } else { 0 };
    let pix = mapoff + mapbytes;
    if pix > len {
        return ERR_TRUNC;
    }
    if cls == 1 && (cmt == 0 || maplen == 0) {
        return ERR_HDR; // mapped without a map
    }
    let depth = d[16] as u32;
    let desc = d[17] as u32;
    if desc & 0xC0 != 0 {
        return ERR_UNSUPPORTED; // interleaved
    }
    // kind; astore = alpha storage: 0 none, 1 byte (32 bit), 2 bit 15, 3 grey byte
    let (kind, astore) = match (cls, depth) {
        (2, 15) => (TK_T16, 0),
        (2, 16) => (TK_T16, 2),
        (2, 24) => (TK_T24, 0),
        (2, 32) => (TK_T32, 1),
        (3, 8) => (TK_G8, 0),
        (3, 16) => (TK_G16, 3),
        (1, 8) | (1, 16) => (
            if depth == 8 { TK_M8 } else { TK_M16 },
            match mapbits {
                15 | 24 => 0,
                16 => 2,
                32 => 1,
                _ => return ERR_UNSUPPORTED,
            },
        ),
        _ => return ERR_UNSUPPORTED,
    };
    let bpp = ((depth + 7) >> 3) as usize;
    let (w, h) = (u16_at(d, 12), u16_at(d, 14));
    // v2 footer + extension area: attributes type
    let mut ext = 0u32;
    if len >= 18 + TGA_FOOTSIZE && &d[len - 18..] == SIG {
        let eo = u32::from_le_bytes([d[len - 26], d[len - 25], d[len - 24], d[len - 23]]) as usize;
        if eo >= 18 && eo + TGA_EXTSIZE <= len - TGA_FOOTSIZE && u16_at(d, eo) as usize >= TGA_EXTSIZE {
            ext = d[eo + TGA_EXTSIZE - 1] as u32 + 1;
        }
    }
    let (mut used, mut premul, mut heur) = (false, false, false);
    if ext != 0 {
        if astore != 0 && (ext == 4 || ext == 5) {
            used = true; // type 3: straight alpha, type 4: premultiplied
            premul = ext == 5;
        }
    } else {
        used = if astore == 2 { desc & 15 != 0 } else { astore != 0 };
        heur = used;
    }
    let mut tg = Tg { aor: if used { 0 } else { 0xFF00_0000 }, a16: astore == 2 && used, pal: Vec::new() };
    // metadata
    m.w = w;
    m.h = h;
    m.bits = if kind == TK_T16 || (cls == 1 && mapbits <= 16) { 5 } else { 8 };
    m.model = MODELS[cls as usize];
    m.channels = (if cls == 2 { 3 } else { 1 }) + used as u8;
    m.flags |= IMF_LOSSLESS | if used { IMF_ALPHA | if premul { IMF_SRC_PREMUL } else { 0 } } else { 0 };
    if w == 0 || h == 0 {
        return ERR_DIMS;
    }
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    if dec.frame != 0 {
        return ERR_UNSUPPORTED;
    }
    // size limits, then enough data for w * h pixels before allocating anything (RLE: at least
    // one packet header + pixel per 128 pixels)
    let e = dims_check(w, h, dec);
    if e != 0 {
        return e;
    }
    let npx = w as usize * h as usize;
    let need = if rle { npx.div_ceil(128) * (bpp + 1) } else { npx * bpp };
    if need > len - pix {
        return ERR_TRUNC;
    }
    *sf = match surf_alloc(w, h, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if cls == 1 {
        // TG_PAL = u32[2^depth] from the colour map (opaque black outside [first, first + len))
        let n = 1usize << depth;
        tg.pal = vec![0xFF00_0000u32; n];
        let ek = if mapbits <= 16 { TK_T16 } else if mapbits == 24 { TK_T24 } else { TK_T32 };
        let eb = ((mapbits + 7) >> 3) as usize;
        for k in 0..maplen as usize {
            let i = mapfirst as usize + k;
            if i >= n {
                break; // beyond the index range
            }
            tg.pal[i] = tg.px(ek, d, mapoff + k * eb);
        }
    }
    // pixels: rows in stream order, placed by the origin bits (one loop per pixel kind)
    let fill = Fill { w: w as usize, h: h as usize, topdown: desc & 0x20 != 0, rtl: desc & 0x10 != 0, rle, pos: pix };
    let r = match kind {
        TK_T32 => fill.run::<TK_T32, 4>(&tg, d, sf, dec),
        TK_T24 => fill.run::<TK_T24, 3>(&tg, d, sf, dec),
        TK_T16 => fill.run::<TK_T16, 2>(&tg, d, sf, dec),
        TK_M8 => fill.run::<TK_M8, 1>(&tg, d, sf, dec),
        TK_M16 => fill.run::<TK_M16, 2>(&tg, d, sf, dec),
        TK_G8 => fill.run::<TK_G8, 1>(&tg, d, sf, dec),
        _ => fill.run::<TK_G16, 2>(&tg, d, sf, dec),
    };
    let acc = match r {
        Ok(acc) => acc, // OR of all pixels (alpha check)
        Err(e) => return e,
    };
    let (hu, wu, stride) = (h as usize, w as usize, sf.stride as usize);
    // an alpha channel that is 0 everywhere (storage-decided alpha) is opaque
    if heur && acc & 0xFF00_0000 == 0 {
        for y in 0..hu {
            for p in sf.px[y * stride..y * stride + wu * 4].chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        m.flags &= !IMF_ALPHA;
        m.channels -= 1;
    }
    if premul {
        // premultiplied pixels: clamp the colours to alpha, FMT_PARGB
        for y in 0..hu {
            for p in sf.px[y * stride..y * stride + wu * 4].chunks_exact_mut(4) {
                let a = p[3];
                p[0] = p[0].min(a);
                p[1] = p[1].min(a);
                p[2] = p[2].min(a);
            }
        }
        sf.fmt = FMT_PARGB;
    }
    0
}

// ------------------------------------------------------------------ encoder

/// tga_encode: TGA file (see the module header) -> 0 / ERR_DIMS / ERR_UNSUPPORTED / ERR_CANCEL;
/// out is empty on error
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    out.clear();
    let (w, h) = (sf.w as u32, sf.h as u32);
    if w.wrapping_sub(1) > IMG_MAXDIM - 1 || h.wrapping_sub(1) > IMG_MAXDIM - 1 {
        return ERR_DIMS;
    }
    let bpp: usize = match opt.bitdepth {
        0 | 32 => 4,
        24 => 3,
        _ => return ERR_UNSUPPORTED,
    };
    let rle = opt.flags & IOF_RLE != 0;
    let (wu, hu) = (w as usize, h as usize);
    let mut f = Vec::with_capacity((wu * bpp + wu / 128 + 2) * hu + 18 + TGA_EXTSIZE + TGA_FOOTSIZE);
    // header
    f.resize(18, 0);
    f[2] = if rle { 10 } else { 2 };
    f[12..14].copy_from_slice(&(w as u16).to_le_bytes());
    f[14..16].copy_from_slice(&(h as u16).to_le_bytes());
    f[16] = (bpp * 8) as u8;
    f[17] = if bpp == 4 { 8 } else { 0 }; // 8 alpha bits, origin lower left
    let mut row = vec![0u8; wu * 4];
    let mut pack = vec![0u8; wu * bpp];
    let mut vals = Vec::with_capacity(wu);
    // rows bottom-up
    for y in (0..hu).rev() {
        if opt.cancelled() {
            return ERR_CANCEL;
        }
        get_row(sf, y, &mut row, if bpp == 4 { FMT_ARGB } else { FMT_PARGB });
        // tge_pack: 24 bit: premultiplied pixels over white (c + 255 - a)
        if bpp == 4 {
            pack[..wu * 4].copy_from_slice(&row);
        } else {
            for (o, s) in pack.chunks_exact_mut(3).zip(row.chunks_exact(4)) {
                let c = u32::from_le_bytes([s[0], s[1], s[2], 0]);
                let v = c.wrapping_add((s[3] ^ 255) as u32 * 0x010101).to_le_bytes();
                o.copy_from_slice(&v[..3]);
            }
        }
        if rle {
            rle_row(&pack, &mut vals, wu, bpp, &mut f);
        } else {
            f.extend_from_slice(&pack[..wu * bpp]);
        }
    }
    // extension area (495 bytes, attributes type 3 = alpha for 32 bit) + footer
    let eo = f.len();
    f.resize(eo + TGA_EXTSIZE + TGA_FOOTSIZE, 0);
    f[eo..eo + 2].copy_from_slice(&(TGA_EXTSIZE as u16).to_le_bytes());
    f[eo + TGA_EXTSIZE - 1] = if bpp == 4 { 3 } else { 0 };
    f[eo + TGA_EXTSIZE..eo + TGA_EXTSIZE + 4].copy_from_slice(&(eo as u32).to_le_bytes());
    f[eo + TGA_EXTSIZE + 8..].copy_from_slice(SIG);
    *out = f;
    0
}

// tge_rle: run-length encode one packed row (packets inside the row); vals = the row's pixels
// as masked u32 (24 bit: the 3 colour bytes)
fn rle_row(pack: &[u8], vals: &mut Vec<u32>, w: usize, bpp: usize, f: &mut Vec<u8>) {
    vals.clear();
    if bpp == 4 {
        vals.extend(pack[..w * 4].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    } else {
        vals.extend(pack[..w * 3].chunks_exact(3).map(|c| u32::from_le_bytes([c[0], c[1], c[2], 0])));
    }
    let v = &vals[..w];
    let mut i = 0;
    while i < w {
        // run length at i
        let p = v[i];
        let mut run = 1;
        while i + run < w && run < 128 && v[i + run] == p {
            run += 1;
        }
        if run >= 2 {
            f.push((run + 127) as u8); // 0x80 | (run - 1)
            f.extend_from_slice(&pack[i * bpp..i * bpp + bpp]);
            i += run;
            continue;
        }
        // raw packet: pixels up to the start of a run of >= 2 (or 128)
        let mut cnt = 1;
        while i + cnt < w && cnt < 128 && !(i + cnt + 1 < w && v[i + cnt] == v[i + cnt + 1]) {
            cnt += 1;
        }
        f.push((cnt - 1) as u8);
        f.extend_from_slice(&pack[i * bpp..(i + cnt) * bpp]);
        i += cnt;
    }
}
