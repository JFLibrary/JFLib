// img_gif -- GIF decoder and encoder, and the GIF parts of the core-format glue
//                                                  [src/img/gif.s, src/img/imgcore.s]
//
//   decode(data, sf) -> 0 / ERR_*                  gif_decode: FMT_PARGB
//   decode_fl(data, sf, flags) -> 0 / ERR_*        gif_decode_fl: JDF_STRAIGHT -> FMT_ARGB
//   encode(sf) -> Some(file) / None                gif_encode
//   encode_q(sf, qcfg) -> Some(file) / None        gif_encode_q (quantize_ex cfg)
//   meta(data, meta, flags) -> 0 / ERR_*           gif_meta (imgcore.s)
//   probe(data, meta) -> recognised                gif_probe
//   decode_ex(data, sf, dec, meta) -> 0 / ERR_*    gif_decode_ex
//   encode_opt(sf, opt, out) -> 0 / ERR_*          gif_encode_opt
//   CODEC                                          gif_codec
//
// Decoder: GIF87a / GIF89a, global and local colour tables, LZW minimum code sizes 1..8,
// clear codes anywhere (early clear, deferred clear = codes after the table filled up without
// a clear, full 4096-entry tables), interlace, the Graphic Control Extension transparency
// index.  Only the first image is decoded (animations show their first frame); the metadata
// scanner counts the frames.  The image is placed at its offset on a transparent canvas of the
// logical screen size, grown to the frame extent (left + width, top + height) where the screen
// is smaller; a 0-sized frame takes the screen size.  Application / comment / plain-text and
// unknown extensions are skipped (a GCE counts for the image only when it is the last one
// before it).
// Colours: index -> opaque palette colour; index past the colour table -> opaque black; no
// colour table at all -> grey ramp (index i -> i,i,i); the GCE transparent index -> 0 (also in
// FMT_ARGB output: GIF has only binary transparency, so both formats hold the same bytes).
// Code size rule (giflib / Wuffs): after every code other than a clear code the width grows by
// one bit when next_code >= 2^width (max 12 bits).
// Errors: ERR_TRUNC (data ends before the image descriptor is complete), ERR_HDR (no GIF87a /
// GIF89a signature), ERR_CHUNK (trailer or an unknown block before the first image), ERR_DIMS
// (canvas w or h > 32768, w * h > 2^28, or a 0-sized frame on a 0-sized screen),
// ERR_UNSUPPORTED (LZW minimum code size outside 1..8).  Once the image descriptor has been
// read the result is always 0: a truncated local colour table, missing, truncated or corrupt
// LZW data end the image and the pixels not decoded stay transparent.
//
// Encoder: GIF89a, one image.  img_core::quantize builds the palette: exact when the image has
// at most 256 colours (transparent counts as one), else Wu quantization to 256 colours (255 +
// transparent) with serpentine Floyd-Steinberg dithering (encode: level 8, encode_q: the cfg's
// level 0..8).  Alpha < threshold (encode: 128) becomes transparent (GCE written only when such
// pixels exist).  Input FMT_PARGB / FMT_ARGB / FMT_XRGB / FMT_A8.  Global colour table of the
// next power of two >= the colour count (min 2), LZW minimum code size max(2, table bits),
// variable code width up to 12 bits, a clear code whenever the 4096-entry table is full.
//
// Metadata (gif_meta): logical screen, colour table depth, frame count (walks all blocks),
// first frame delay, NETSCAPE loop count, transparency, interlace of the first frame, an ICC
// profile from an "ICCRGBG1" "012" application extension (IMGD_ICC).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use img_core::io::{core_decode, core_probe};
use img_core::quantize::{self, IQ_THRESH_SHIFT, IQF_KEEPALPHA, IQF_WHITE};
use img_core::*;

const GIF_MAX_DIM: u32 = 32768;
const GIF_MAX_PIX: u64 = 1 << 28;
const GIF_SLACK: usize = 8192; // index buffer slack (longest string)
const GIF_HBITS: u32 = 14; // encoder hash table: 2^14 u32 slots
const GIF_HSIZE: usize = 1 << GIF_HBITS;
const GIF_HASHMUL: u32 = 0x9E37_79B1;
const ICC_MAX: u64 = 1 << 24; // largest ICC profile gathered from GIF sub-blocks
const ILACE: [(u32, u32); 4] = [(0, 8), (4, 8), (2, 4), (1, 2)]; // interlace passes: first row, step


/// gif_codec
pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode_ex),
    encode: Some(encode_opt),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: GIF_IOM,
    scan: Some(meta),
    decode_fl: Some(decode_fl),
    save: Some(save),
};

// the classic encoder of img_save (gif_encode)
fn save(sf: &Surface, _quality: u32) -> Option<Vec<u8>> {
    encode(sf)
}

#[inline(always)]
fn u16_at(d: &[u8], o: usize) -> u32 {
    d[o] as u32 | (d[o + 1] as u32) << 8
}

// ================================================================== decoder

/// gif_decode: the first image as a new FMT_PARGB surface (sf is replaced; untouched on error)
pub fn decode(data: &[u8], sf: &mut Surface) -> i32 {
    decode_fl(data, sf, 0)
}

/// gif_decode_fl: gif_decode; flags JDF_STRAIGHT: FMT_ARGB surface (same bytes)
pub fn decode_fl(data: &[u8], sf: &mut Surface, flags: u32) -> i32 {
    let end = data.len();
    if end < 6 {
        return ERR_TRUNC;
    }
    if &data[..4] != b"GIF8" || !(&data[4..6] == b"7a" || &data[4..6] == b"9a") {
        return ERR_HDR;
    }
    if end < 13 {
        return ERR_TRUNC;
    }
    let (sw, sh) = (u16_at(data, 6), u16_at(data, 8));
    let sflags = data[10] as u32;
    let mut pos = 13usize;
    let mut gct: Option<usize> = None;
    let mut ngct = 0u32;
    let mut trans = -1i32;
    if sflags & 0x80 != 0 {
        let n = 2u32 << (sflags & 7);
        let bytes = 3 * n as usize;
        if end - pos < bytes {
            return ERR_TRUNC;
        }
        gct = Some(pos);
        ngct = n;
        pos += bytes;
    }
    // ---------------------------------------------------------- blocks
    loop {
        if pos >= end {
            return ERR_TRUNC;
        }
        let b = data[pos];
        pos += 1;
        if b == 0x2C {
            break;
        }
        if b != 0x21 {
            return ERR_CHUNK; // trailer or garbage before any image
        }
        if pos >= end {
            return ERR_TRUNC;
        }
        let label = data[pos];
        pos += 1;
        if label == 0xF9 && end - pos >= 5 && data[pos] >= 4 {
            // Graphic Control Extension: sub-block {flags, delay u16, transparent index}
            trans = -1;
            if data[pos + 1] & 1 != 0 {
                trans = data[pos + 4] as i32;
            }
        }
        loop {
            if pos >= end {
                return ERR_TRUNC;
            }
            let n = data[pos] as usize;
            pos += 1;
            if n == 0 {
                break;
            }
            if end - pos < n {
                return ERR_TRUNC;
            }
            pos += n;
        }
    }
    // ---------------------------------------------------------- image descriptor
    if end - pos < 9 {
        return ERR_TRUNC;
    }
    let (fx, fy) = (u16_at(data, pos), u16_at(data, pos + 2));
    let (mut fw, mut fh) = (u16_at(data, pos + 4), u16_at(data, pos + 6));
    let ifl = data[pos + 8] as u32;
    pos += 9;
    if fw == 0 || fh == 0 {
        (fw, fh) = (sw, sh); // 0-sized frame: screen size
        if fw == 0 || fh == 0 {
            return ERR_DIMS;
        }
    }
    let cw = (fw + fx).max(sw); // frame extent
    let ch = (fh + fy).max(sh);
    if cw > GIF_MAX_DIM || ch > GIF_MAX_DIM || cw as u64 * ch as u64 > GIF_MAX_PIX {
        return ERR_DIMS;
    }
    // from here on truncation is not an error
    let (mut pal, mut npal) = (gct, ngct);
    let mut mcs = 0u32;
    'hdr: {
        if ifl & 0x80 != 0 {
            let n = 2u32 << (ifl & 7);
            let bytes = 3 * n as usize;
            if end - pos < bytes {
                break 'hdr; // truncated local table: empty image
            }
            pal = Some(pos);
            npal = n;
            pos += bytes;
        }
        if pos >= end {
            break 'hdr;
        }
        let m = data[pos] as u32;
        pos += 1;
        if !(1..=8).contains(&m) {
            return ERR_UNSUPPORTED;
        }
        mcs = m;
    }
    let total = fw as usize * fh as usize;
    let mut stream = Vec::new();
    if mcs != 0 {
        // gather the data sub-blocks (a truncated last block counts as far as it goes)
        stream.reserve(end - pos + 16);
        while pos < end {
            let n = data[pos] as usize;
            pos += 1;
            if n == 0 {
                break;
            }
            let n = n.min(end - pos);
            stream.extend_from_slice(&data[pos..pos + n]);
            pos += n;
        }
    }
    let fmt = if flags & JDF_STRAIGHT != 0 { FMT_ARGB } else { FMT_PARGB };
    let mut out = Surface::new(cw as i32, ch as i32, fmt);
    if out.px.is_empty() {
        return ERR_NOMEM;
    }
    if mcs != 0 {
        let mut idx = vec![0u8; total + GIF_SLACK];
        let npix = lzw_decode(&mut stream, mcs, &mut idx, total);
        // ------------------------------------------------------ colours
        let mut lut = [0u32; 256];
        for (i, l) in lut.iter_mut().enumerate() {
            *l = match pal {
                None => 0xFF00_0000 | i as u32 * 0x010101, // no colour table: grey ramp
                Some(p) if (i as u32) < npal => {
                    let e = &data[p + 3 * i..p + 3 * i + 3];
                    0xFF00_0000 | (e[0] as u32) << 16 | (e[1] as u32) << 8 | e[2] as u32
                }
                Some(_) => 0xFF00_0000, // past the table: black
            };
        }
        if trans >= 0 {
            lut[trans as usize] = 0;
        }
        // ------------------------------------------------------ rows
        let stride = out.stride as usize;
        let (mut left, mut src) = (npix, 0usize);
        let (mut row, mut step, mut pass) = (0u32, 8u32, 0usize);
        while left > 0 {
            let n = left.min(fw as usize);
            left -= n;
            let o = (fy + row) as usize * stride + fx as usize * 4;
            for (d, &i) in out.px[o..o + n * 4].chunks_exact_mut(4).zip(&idx[src..src + n]) {
                d.copy_from_slice(&lut[i as usize].to_le_bytes());
            }
            src += n;
            if ifl & 0x40 == 0 {
                row += 1;
                continue;
            }
            row += step;
            while row >= fh {
                pass += 1;
                if pass >= 4 {
                    left = 0;
                    break;
                }
                (row, step) = ILACE[pass];
            }
        }
    }
    *sf = out;
    0
}

// the LZW decoder: indices of the stream (min code size mcs) into idx -> pixels decoded
// (at most total); stops at end of information, the end of the stream or a corrupt code
fn lzw_decode(stream: &mut Vec<u8>, mcs: u32, idx: &mut [u8], total: usize) -> usize {
    let nbits = stream.len() as u64 * 8;
    stream.extend_from_slice(&[0u8; 16]); // the 8-byte reads past the end see zeros
    let st: &[u8] = stream;
    let mut table = vec![0u64; 4096]; // len | offset << 32
    let clear = 1u32 << mcs;
    let mut bitpos = 0u64;
    let mut outpos = 0usize;
    'reset: loop {
        let mut width = mcs + 1;
        let mut mask = (1u32 << width) - 1;
        let mut next = clear + 2;
        let mut prev = 0u64; // previous string (len | pos << 32), 0 = none
        loop {
            if bitpos + width as u64 > nbits {
                break 'reset; // not enough bits left
            }
            let o = (bitpos >> 3) as usize;
            let v = u64::from_le_bytes(st[o..o + 8].try_into().unwrap());
            let code = (v >> (bitpos & 7)) as u32 & mask;
            bitpos += width as u64;
            let len: usize;
            if code < clear {
                idx[outpos] = code as u8; // literal
                len = 1;
            } else if code == clear {
                continue 'reset;
            } else if code == clear + 1 {
                break 'reset; // end of information
            } else if code < next {
                let e = table[code as usize & 4095];
                len = e as u32 as usize;
                copy16(idx, (e >> 32) as usize, outpos, len);
            } else {
                if code != next || prev == 0 {
                    break 'reset; // corrupt data / no previous string to extend
                }
                let pl = prev as u32 as usize;
                let src = (prev >> 32) as usize;
                copy16(idx, src, outpos, pl);
                idx[outpos + pl] = idx[src]; // its first byte appended
                len = pl + 1;
            }
            if prev != 0 && next < 4096 {
                table[next as usize] = prev + 1; // previous string + first byte
                next += 1;
            }
            if next > mask && width < 12 {
                width += 1;
                mask = mask * 2 + 1;
            }
            prev = (outpos as u64) << 32 | len as u64;
            outpos += len;
            if outpos >= total {
                break 'reset;
            }
        }
    }
    outpos.min(total)
}

// copy an earlier string (src + len <= dst) in 16-byte steps like the assembly; the bytes
// written past dst + len are overwritten by the next string or never read
#[inline(always)]
fn copy16(idx: &mut [u8], src: usize, dst: usize, len: usize) {
    let mut k = 0;
    loop {
        let t: [u8; 16] = idx[src + k..src + k + 16].try_into().unwrap();
        idx[dst + k..dst + k + 16].copy_from_slice(&t);
        k += 16;
        if k >= len {
            break;
        }
    }
}

// ================================================================== encoder

/// gif_encode: GIF89a file (dither level 8, threshold 128) or None (empty or larger than
/// 65535 x 65535, unknown format)
pub fn encode(sf: &Surface) -> Option<Vec<u8>> {
    encode_q(sf, 8 | 128 << IQ_THRESH_SHIFT)
}

/// gif_encode_q: gif_encode with a quantize_ex cfg (dither level | threshold <<
/// IQ_THRESH_SHIFT; IQF_* flags are ignored)
pub fn encode_q(sf: &Surface, qcfg: u32) -> Option<Vec<u8>> {
    let qcfg = qcfg & !(IQF_KEEPALPHA | IQF_WHITE);
    let (w, h) = (sf.w, sf.h);
    if w <= 0 || h <= 0 || w > 65535 || h > 65535 || sf.fmt > FMT_ARGB {
        return None;
    }
    let n = w as usize * h as usize;
    let mut idx = vec![0u8; n];
    let mut pal = [0u32; 256];
    let (ncol, trans) = quantize::quantize_ex(sf, 256, qcfg, &mut pal, &mut idx);
    if ncol == 0 {
        return None;
    }
    let mut bits = 1u32; // table bits: 2^bits >= colours
    while (1u32 << bits) < ncol {
        bits += 1;
    }
    let mcs = bits.max(2);
    let raw = lzw_encode(&idx, mcs);
    let size = raw.len() + raw.len().div_ceil(255) + (3 << bits) + 13 + 10 + 1 + 2 + if trans >= 0 { 8 } else { 0 };
    let mut f = Vec::with_capacity(size);
    // header + logical screen descriptor (global table, 8-bit colour resolution)
    f.extend_from_slice(b"GIF89a");
    f.extend_from_slice(&(w as u16).to_le_bytes());
    f.extend_from_slice(&(h as u16).to_le_bytes());
    f.extend_from_slice(&[((bits - 1) | 0xF0) as u8, 0, 0]);
    // global colour table (entries past the colours are 0 = black)
    for &c in &pal[..1 << bits] {
        f.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    }
    // graphic control extension (transparency only)
    if trans >= 0 {
        f.extend_from_slice(&[0x21, 0xF9, 4, 1, 0, 0, trans as u8, 0]);
    }
    // image descriptor: at 0, 0, no local table, not interlaced
    f.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
    f.extend_from_slice(&(w as u16).to_le_bytes());
    f.extend_from_slice(&(h as u16).to_le_bytes());
    f.extend_from_slice(&[0, mcs as u8]);
    for blk in raw.chunks(255) {
        f.push(blk.len() as u8);
        f.extend_from_slice(blk);
    }
    f.extend_from_slice(&[0, 0x3B]); // block terminator, trailer
    Some(f)
}

// the LZW bit writer (GIF_EMIT): codes LSB first, full 32-bit words written out
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    #[inline(always)]
    fn emit(&mut self, code: u32, width: u32) {
        self.acc |= (code as u64) << self.n;
        self.n += width;
        if self.n >= 32 {
            self.out.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.n -= 32;
        }
    }
}

// gif_lzw_encode: the GIF LZW code stream (no sub-block framing) of n >= 1 indices < 2^mcs
fn lzw_encode(idx: &[u8], mcs: u32) -> Vec<u8> {
    let n = idx.len();
    let mut bw = Bits { out: Vec::with_capacity((n + n / 1024 + 16) * 3 / 2 + 64), acc: 0, n: 0 };
    let clear = 1u32 << mcs;
    let mut hash = vec![0u32; GIF_HSIZE]; // key << 12 | code, 0 = empty
    let mut width = mcs + 1;
    let mut next = clear + 2;
    bw.emit(clear, width); // clear code first
    let mut prefix = idx[0] as u32;
    for &d in &idx[1..] {
        let d = d as u32;
        let key = (prefix << 8 | d) << 12;
        let mut h = ((prefix << 8 | d).wrapping_mul(GIF_HASHMUL) >> (32 - GIF_HBITS)) as usize;
        loop {
            let s = hash[h];
            if s == 0 {
                break;
            }
            if s & !4095 == key {
                prefix = s & 4095;
                break;
            }
            h = (h + 1) & (GIF_HSIZE - 1);
        }
        if hash[h] != 0 {
            continue; // hit: the string grows
        }
        if next < 4096 {
            hash[h] = key | next; // new string = prefix + index
            bw.emit(prefix, width);
            next += 1;
            if next > 1 << width && width < 12 {
                width += 1;
            }
        } else {
            bw.emit(prefix, width);
            bw.emit(clear, width); // table full: clear code
            hash.fill(0);
            width = mcs + 1;
            next = clear + 2;
        }
        prefix = d;
    }
    bw.emit(prefix, width);
    if next >= 1 << width && width < 12 {
        width += 1; // the decoder grows before EOI too
    }
    bw.emit(clear + 1, width);
    let mut left = bw.n as i32;
    while left > 0 {
        bw.out.push(bw.acc as u8);
        bw.acc >>= 8;
        left -= 8;
    }
    bw.out
}

// ================================================================== metadata, IMGCODEC glue

// gif_blocks: position after a sub-block chain (the 0 terminator), None when the data ends first
fn blocks(d: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= d.len() {
            return None;
        }
        let n = d[pos] as usize;
        pos += 1;
        if n == 0 {
            return Some(pos);
        }
        pos += n;
    }
}

// gif_icc: gather the sub-block chain at pos into meta.icc (at most ICC_MAX bytes; a chain cut
// off by the end of the data is ignored)
fn icc(d: &[u8], pos: usize, meta: &mut ImgMeta) {
    let mut p = pos;
    let mut total = 0u64;
    loop {
        if p >= d.len() {
            return;
        }
        let n = d[p] as usize;
        p += 1;
        if n == 0 {
            break;
        }
        total += n as u64;
        p += n;
        if total > ICC_MAX {
            return;
        }
    }
    if p > d.len() || total == 0 {
        return;
    }
    let mut blob = Vec::with_capacity(total as usize);
    let mut p = pos;
    loop {
        let n = d[p] as usize;
        p += 1;
        if n == 0 {
            break;
        }
        blob.extend_from_slice(&d[p..p + n]);
        p += n;
    }
    meta.icc = blob;
}

/// gif_meta: logical screen and a walk over all blocks -> 0 / ERR_TRUNC / ERR_HDR
pub fn meta(d: &[u8], m: &mut ImgMeta, flags: u32) -> i32 {
    let len = d.len();
    if len < 6 {
        return ERR_TRUNC;
    }
    if &d[..4] != b"GIF8" || !(&d[4..6] == b"7a" || &d[4..6] == b"9a") {
        return ERR_HDR;
    }
    if len < 13 {
        return ERR_TRUNC;
    }
    m.w = u16_at(d, 6);
    m.h = u16_at(d, 8);
    m.model = IMM_PAL;
    m.channels = 1;
    m.flags |= IMF_LOSSLESS;
    m.bits = 8;
    let f = d[10] as u32;
    let mut pos = 13usize;
    if f & 0x80 != 0 {
        m.bits = ((f & 7) + 1) as u8;
        pos += 6 << (f & 7);
    }
    let mut frames = 0u32;
    while pos < len {
        let b = d[pos];
        if b == 0x21 {
            if pos + 2 > len {
                break;
            }
            let label = d[pos + 1];
            pos += 2;
            if label == 0xF9 {
                if pos + 5 <= len && d[pos] >= 4 {
                    if d[pos + 1] & 1 != 0 {
                        m.flags |= IMF_ALPHA;
                    }
                    if frames == 0 {
                        m.delay = u16_at(d, pos + 2) * 10;
                    }
                }
            } else if label == 0xFF && pos + 16 <= len && d[pos] == 11 {
                let id = &d[pos + 1..pos + 12];
                if id == b"ICCRGBG1012" {
                    // the sub-blocks after the identifier hold an ICC profile
                    if flags & IMGD_ICC != 0 && m.icc.is_empty() {
                        icc(d, pos + 12, m);
                    }
                } else if id == b"NETSCAPE2.0" && d[pos + 12] == 3 && d[pos + 13] == 1 {
                    m.loops = u16_at(d, pos + 14);
                }
            }
        } else if b == 0x2C {
            if pos + 10 > len {
                break;
            }
            frames += 1;
            let f = d[pos + 9] as usize;
            if frames == 1 && f & 0x40 != 0 {
                m.flags |= IMF_INTERLACED;
            }
            pos += 10;
            if f & 0x80 != 0 {
                pos += 6 << (f & 7);
            }
            pos += 1; // LZW minimum code size
        } else {
            break; // trailer or garbage
        }
        match blocks(d, pos) {
            Some(p) => pos = p,
            None => break,
        }
    }
    m.frames = frames.max(1);
    if frames > 1 {
        m.flags |= IMF_ANIMATED;
    }
    0
}

/// gif_probe: header + metadata scan, no pixels
pub fn probe(data: &[u8], m: &mut ImgMeta) -> bool {
    core_probe(data, m, meta)
}

/// gif_decode_ex: the first frame through decode_fl (core_decode); meta.w / h become the canvas
/// size decode_fl produced
pub fn decode_ex(data: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    let e = core_decode(data, sf, dec, m, meta, decode_fl);
    if e == 0 && dec.flags & IMGD_PROBE == 0 {
        m.w = sf.w as u32;
        m.h = sf.h as u32;
    }
    e
}

/// gif_encode_opt: encode_q with the dither level ImgOpt::dither and the threshold
/// ImgOpt::athresh -> 0 / ERR_NOMEM (also for a surface encode_q refuses) / ERR_CANCEL
pub fn encode_opt(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    out.clear();
    if opt.cancelled() {
        return ERR_CANCEL;
    }
    match encode_q(sf, quantize::opt_qcfg(opt)) {
        Some(f) => {
            *out = f;
            0
        }
        None => ERR_NOMEM,
    }
}
