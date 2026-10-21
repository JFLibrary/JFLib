// vp8l -- WebP lossless (VP8L) bitstream decoder, bit-exact with libwebp 1.3
//                                                                  [src/img/webp_vp8l.s]
//
//   decode(data, dec) -> Ok(surface) / Err(ERR_*)          vp8l_decode
//   decode_alpha(data, plane, w, h, dec) -> 0 / ERR_*      vp8l_decode_alpha
//   build_table(lengths, root_bits, table) -> entries      vp8l_build_table
//
// decode takes the payload of a "VP8L" chunk (signature byte 0x2F, 14-bit width - 1,
// 14-bit height - 1, alpha hint, 3-bit version 0) and returns a new FMT_ARGB surface
// (VP8L pixels are straight 0xAARRGGBB).  decode_alpha decodes the header-less image
// stream of a compressed ALPH chunk (w x h given) and writes its green channel to
// plane[w * h].
//
// Decoder (RFC 9649 / libwebp src/dec/vp8l_dec.c): transforms (predictor with the 14
// modes -- 14 / 15 act as mode 0 --, cross colour, subtract green, colour indexing with
// 1 / 2 / 4 / 8-bit pixel bundling; each type at most once, applied in reverse order),
// colour cache (1..11 bits, hash 0x1E35A7BD), meta prefix codes (entropy image; unused
// group indices are validated but not stored, like libwebp's mapping), simple and normal
// prefix codes (code-length code, repeat codes 16 / 17 / 18, max_symbol), LZ77 backward
// references with the 120-entry distance map.  Prefix codes must be complete (a single
// used symbol is a 0-bit code), exactly like libwebp's BuildHuffmanTable.  Tables: 8-bit
// root + second level (u32 entries: symbol << 16 | length).  End of stream: libwebp's
// rule -- an error once more bits were consumed than the data holds (streams shorter than
// 8 bytes count as 8 zero-padded bytes); bits past the end read as zero.  Distances before
// the image start and copies past its end: ERR_DIST; invalid codes: ERR_CODE; a repeated
// transform, bad cache bits / version / signature: ERR_HDR; out of data: ERR_TRUNC.
// ALPH streams that take libwebp's 8-bit alpha path finish their last <= 128 bits with an
// exact model of libwebp's VP8LBitReader (a8_tail).
//
// Pixels are kept as bytes (B, G, R, A per pixel); the main image is decoded contiguously
// into the buffer that becomes the surface (stride as surf_alloc), then its rows are
// spread to the stride in place, like the assembly.
//
// Memory: the pixels of each level (+ slack), tables (grown on demand), colour cache;
// dec.maxmem is checked before every big allocation; dec.cancel is polled once per
// decoded row of the level-0 image.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_ARGB, SFF_OWNED, Surface};
use img_core::meta::{dims_check, mem_check};
use img_core::*;
use simd::{Simd, Sse2};

const NUM_LIT: u32 = 256;
const NUM_LEN: u32 = 24;
const NUM_DIST: u32 = 40;
const MAX_CACHE_BITS: u32 = 11;
const HROOT: u32 = 8; // root table bits of the main codes
const CLROOT: u32 = 7; // root bits of the code-length code
const MAXLEN: usize = 15;
pub const MAX_ALPHA: usize = (NUM_LIT + NUM_LEN + (1 << MAX_CACHE_BITS)) as usize;
const PIXPAD: u64 = 64; // slack after every pixel buffer (16-byte loads)
pub const HASH_MUL: u32 = 0x1E35_A7BD;
const TAB_MIN: usize = 4096; // first table buffer capacity (entries)

const TR_PRED: u32 = 0; // transform types
const TR_CROSS: u32 = 1;
const TR_SUBG: u32 = 2;
const TR_CIDX: u32 = 3;

const GRF_TLIT: u32 = 1; // red, blue and alpha are 0-bit codes
const GRF_TCODE: u32 = 2; // the whole pixel is constant (green 0-bit literal too)

static ASIZE: [u32; 5] = [NUM_LIT + NUM_LEN, NUM_LIT, NUM_LIT, NUM_LIT, NUM_DIST];
static CL_ORDER: [u8; 19] = [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
static REP_BITS: [u8; 3] = [2, 3, 7];
static REP_OFFS: [u8; 3] = [3, 3, 11];

/// distance map: 0xYX with dx = 8 - X, dy = Y (libwebp kCodeToPlane)  [vl_code_to_plane]
pub static CODE_TO_PLANE: [u8; 120] = [
    0x18, 0x07, 0x17, 0x19, 0x28, 0x06, 0x27, 0x29, 0x16, 0x1A, 0x26, 0x2A, 0x38, 0x05, 0x37, 0x39, 0x15, 0x1B, 0x36,
    0x3A, 0x25, 0x2B, 0x48, 0x04, 0x47, 0x49, 0x14, 0x1C, 0x35, 0x3B, 0x46, 0x4A, 0x24, 0x2C, 0x58, 0x45, 0x4B, 0x34,
    0x3C, 0x03, 0x57, 0x59, 0x13, 0x1D, 0x56, 0x5A, 0x23, 0x2D, 0x44, 0x4C, 0x55, 0x5B, 0x33, 0x3D, 0x68, 0x02, 0x67,
    0x69, 0x12, 0x1E, 0x66, 0x6A, 0x22, 0x2E, 0x54, 0x5C, 0x43, 0x4D, 0x65, 0x6B, 0x32, 0x3E, 0x78, 0x01, 0x77, 0x79,
    0x53, 0x5D, 0x11, 0x1F, 0x64, 0x6C, 0x42, 0x4E, 0x76, 0x7A, 0x21, 0x2F, 0x75, 0x7B, 0x31, 0x3F, 0x63, 0x6D, 0x52,
    0x5E, 0x00, 0x74, 0x7C, 0x41, 0x4F, 0x10, 0x20, 0x62, 0x6E, 0x30, 0x73, 0x7D, 0x51, 0x5F, 0x40, 0x72, 0x7E, 0x61,
    0x6F, 0x50, 0x71, 0x7F, 0x60, 0x70,
];

// ------------------------------------------------------------------ pixel bytes

#[inline(always)]
fn rd(px: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(px[i * 4..i * 4 + 4].try_into().unwrap())
}

#[inline(always)]
fn wr(px: &mut [u8], i: usize, v: u32) {
    px[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
}

// ------------------------------------------------------------------ bit reader

/// the LSB-first bit reader of img_int.inc (BRL_*: img_core::bits::BrL) with every method
/// inlined and the byte-wise end-of-data refill out of line, so the state stays in
/// registers in the pixel loop
#[derive(Clone, Copy)]
struct Bits<'a> {
    data: &'a [u8],
    buf: u64,
    cnt: u32,
    pos: usize, // may pass data.len()
}

// BRL_REFILL near the end of the data: zero bytes are shifted in, pos keeps counting
#[cold]
#[inline(never)]
fn refill_slow(data: &[u8], mut buf: u64, mut cnt: u32, mut pos: usize) -> (u64, u32, usize) {
    while cnt <= 56 {
        let b = if pos < data.len() { data[pos] as u64 } else { 0 };
        pos += 1;
        buf |= b << cnt;
        cnt += 8;
    }
    (buf, cnt, pos)
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, buf: 0, cnt: 0, pos: 0 }
    }

    /// at least 56 bits in the buffer
    #[inline(always)]
    fn refill(&mut self) {
        if self.cnt >= 56 {
            return;
        }
        if let Some(b) = self.data.get(self.pos..self.pos + 8) {
            let v = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            self.buf |= v << self.cnt;
            self.pos += ((63 - self.cnt) >> 3) as usize;
            self.cnt |= 56;
        } else {
            (self.buf, self.cnt, self.pos) = refill_slow(self.data, self.buf, self.cnt, self.pos);
        }
    }

    #[inline(always)]
    fn skip(&mut self, n: u32) {
        self.buf = self.buf.wrapping_shr(n);
        self.cnt = self.cnt.wrapping_sub(n);
    }

    #[inline(always)]
    fn get(&mut self, n: u32) -> u64 {
        let v = self.buf & 1u64.wrapping_shl(n).wrapping_sub(1);
        self.skip(n);
        v
    }

    /// more padding bits consumed than are left (BRL_OVERRUN)
    #[inline(always)]
    fn overrun(&self) -> bool {
        let end = self.data.len();
        self.pos > end && ((self.pos - end) as u64) * 8 > self.cnt as u64
    }
}

/// a transform record (TF_*)
#[derive(Default)]
struct Tf {
    ty: u32,
    bits: u32,
    xs: u32, // output width of the transform
    ys: u32,
    data: Vec<u32>, // sub-image / 256-entry palette
}

/// a prefix-code group (GR_*): table offsets of green / red / blue / alpha / distance
#[derive(Clone, Copy, Default)]
struct Grp {
    t: [usize; 5],
    lit: u32, // A << 24 | R << 16 | B (| G << 8 for GRF_TCODE)
    fl: u32,  // GRF_*
}

/// the decoder context (VL_*)
struct Vl<'a> {
    br: Bits<'a>,
    cancel: Option<&'a AtomicU32>,
    maxmem: u64,
    ntr: u32,  // transforms read
    seen: u32, // transform types seen (bit mask)
    pix: Vec<u8>,
    tr: [Tf; 4],
    // prefix codes / cache / entropy image of the image being decoded
    ccb: u32,
    hb: u32,
    cache: [u32; 1 << MAX_CACHE_BITS],
    himg: Vec<u32>,
    hxs: u32,
    grp: Vec<Grp>,
    tab: Vec<u32>,
    tab_used: usize,
    map: Vec<u32>,
    cl: [u8; MAX_ALPHA + 8],
    clcl: [u8; 24],
    cltab: [u32; 128],
    isalpha: bool,
    a8: bool, // libwebp's 8-bit alpha path applies (see level0)
}

/// libwebp's 8-bit alpha path: the state handed over to a8_tail
struct A8 {
    cur: usize, // (pixel indices)
    end: usize,
    c: u64, // bits consumed so far
    col: u32,
    row: u32,
    xs: u32,
}

impl<'a> Vl<'a> {
    // vl_new: a zeroed context with the bit reader on data (streams shorter than 8 bytes
    // are read from a zero-padded copy: the caller passes that copy)
    fn new(data: &'a [u8], dec: Option<&'a ImgDec<'a>>) -> Vl<'a> {
        Vl {
            br: Bits::new(data),
            cancel: dec.and_then(|d| d.cancel),
            maxmem: dec.map_or(0, |d| d.maxmem),
            ntr: 0,
            seen: 0,
            pix: Vec::new(),
            tr: Default::default(),
            ccb: 0,
            hb: 0,
            cache: [0; 1 << MAX_CACHE_BITS],
            himg: Vec::new(),
            hxs: 0,
            grp: Vec::new(),
            tab: Vec::new(),
            tab_used: 0,
            map: Vec::new(),
            cl: [0; MAX_ALPHA + 8],
            clcl: [0; 24],
            cltab: [0; 128],
            isalpha: false,
            a8: false,
        }
    }

    // vl_free_meta: release the prefix codes / cache / entropy image of the image being
    // decoded
    fn free_meta(&mut self) {
        self.himg = Vec::new();
        self.grp = Vec::new();
        self.tab = Vec::new();
        self.tab_used = 0;
        self.map = Vec::new();
        self.ccb = 0;
        self.hb = 0;
    }

    // vl_alloc: the memory budget check before a big allocation (ERR_LIMIT)
    fn check(&self, bytes: u64) -> Result<(), i32> {
        let d = ImgDec { maxmem: self.maxmem, ..Default::default() };
        if mem_check(bytes, &d) != 0 { Err(ERR_LIMIT) } else { Ok(()) }
    }

    // vl_bits: the next n (0..32) bits, LSB first
    #[inline]
    fn bits(&mut self, n: u32) -> u32 {
        if self.br.cnt < n {
            self.br.refill();
        }
        self.br.get(n) as u32
    }

    // vl_eos: more bits were consumed than the data holds
    #[inline]
    fn eos(&self) -> bool {
        self.br.overrun()
    }

    // ------------------------------------------------------------------ image streams

    // vl_level0: the main image stream: transforms, colour cache, prefix codes (with
    // entropy image), pixels into pix (xs * ys pixels), then the inverse transforms
    fn level0(&mut self, xs0: u32, ys: u32) -> Result<(), i32> {
        let mut xs = xs0;
        while self.bits(1) != 0 {
            self.read_transform(&mut xs, ys)?;
        }
        let mut cb = 0;
        if self.bits(1) != 0 {
            cb = self.bits(4);
            if cb.wrapping_sub(1) > MAX_CACHE_BITS - 1 {
                return Err(ERR_HDR);
            }
        }
        self.read_codes(xs, ys, cb, true)?;
        // libwebp decodes ALPH streams with only colour indexing, no cache and 0-bit red /
        // blue / alpha codes through DecodeAlphaData, which accepts running out of data
        // while decoding the last symbol: a8 selects that end-of-stream rule
        if self.isalpha
            && self.ntr == 1
            && self.tr[0].ty == TR_CIDX
            && self.ccb == 0
            && self.grp.iter().all(|g| g.fl & GRF_TLIT != 0)
        {
            self.a8 = true;
        }
        if self.pix.is_empty() {
            let n = xs0 as u64 * ys as u64;
            self.check(n * 4 + PIXPAD)?;
            self.pix = vec![0u8; (n * 4 + PIXPAD) as usize];
        }
        let mut pix = core::mem::take(&mut self.pix);
        let r = self.decode_pixels(&mut pix, xs, ys, true);
        self.pix = pix;
        r?;
        self.free_meta();
        // inverse transforms, last read first
        for i in (0..self.ntr as usize).rev() {
            inverse(&mut self.pix, &self.tr[i]);
        }
        Ok(())
    }

    // vl_read_transform: one transform header (+ its sub-image); colour indexing narrows
    // *xsize
    fn read_transform(&mut self, xsize: &mut u32, ysize: u32) -> Result<(), i32> {
        let ty = self.bits(2);
        if self.seen & (1 << ty) != 0 {
            return Err(ERR_HDR);
        }
        self.seen |= 1 << ty;
        let k = (self.ntr & 3) as usize;
        self.ntr += 1;
        self.tr[k].ty = ty;
        self.tr[k].xs = *xsize;
        self.tr[k].ys = ysize;
        if ty == TR_SUBG {
            return Ok(());
        }
        if ty != TR_CIDX {
            // predictor / cross colour: bits 2..9, sub-image of the tiles
            let b = self.bits(3) + 2;
            self.tr[k].bits = b;
            let m = (1u32 << b) - 1;
            let (sx, sy) = ((self.tr[k].xs + m) >> b, (ysize + m) >> b);
            let d = self.subimage(sx, sy)?;
            self.tr[k].data = d;
            return Ok(());
        }
        let n = self.bits(8) + 1; // colours 1..256
        let b = if n > 16 {
            0
        } else if n > 4 {
            1
        } else if n > 2 {
            2
        } else {
            3
        };
        self.tr[k].bits = b;
        let m = (1u32 << b) - 1;
        *xsize = (self.tr[k].xs + m) >> b; // packed width
        let colours = self.subimage(n, 1)?;
        // expand the palette: 256 entries, delta coded per byte, rest 0
        self.check(256 * 4)?;
        let mut pal = vec![0u32; 256];
        pal[0] = colours[0];
        for i in 1..n as usize {
            pal[i] = add_px(colours[i], pal[i - 1]);
        }
        self.tr[k].data = pal;
        Ok(())
    }

    // vl_subimage: a level-1 image stream (colour cache, one prefix-code group, pixels)
    // into a new u32[xs * ys] buffer
    fn subimage(&mut self, xs: u32, ys: u32) -> Result<Vec<u32>, i32> {
        let r = self.subimage_body(xs, ys);
        self.free_meta();
        r
    }

    fn subimage_body(&mut self, xs: u32, ys: u32) -> Result<Vec<u32>, i32> {
        let mut cb = 0;
        if self.bits(1) != 0 {
            cb = self.bits(4);
            if cb.wrapping_sub(1) > MAX_CACHE_BITS - 1 {
                return Err(ERR_HDR);
            }
        }
        self.read_codes(xs, ys, cb, false)?;
        let n = xs as u64 * ys as u64;
        self.check(n * 4 + PIXPAD)?;
        let mut px = vec![0u8; (n * 4) as usize];
        self.decode_pixels(&mut px, xs, ys, false)?;
        Ok(px.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
    }

    // ------------------------------------------------------------------ prefix codes

    // vl_read_codes: the prefix-code groups of an image (entropy image when allow_meta and
    // signalled), its colour cache; everything in the meta fields
    fn read_codes(&mut self, xs: u32, ys: u32, cbits: u32, allow_meta: bool) -> Result<(), i32> {
        let mut nmax = 1u32;
        let mut stored = 1u32;
        if allow_meta && self.bits(1) != 0 {
            let hb = self.bits(3) + 2; // entropy image bits 2..9
            let m = (1u32 << hb) - 1;
            let (hxs, hys) = ((xs + m) >> hb, (ys + m) >> hb);
            let mut himg = self.subimage(hxs, hys)?;
            self.hb = hb;
            self.hxs = hxs;
            let tiles = (hxs * hys) as usize;
            // group = (pixel >> 8) & 0xFFFF, nmax = max + 1
            for v in himg[..tiles].iter_mut() {
                *v = (*v >> 8) & 0xFFFF;
                nmax = nmax.max(*v + 1);
            }
            self.check(nmax as u64 * 4)?;
            let mut map = vec![u32::MAX; nmax as usize];
            // slots in order of first use
            let mut used = 0u32;
            for v in himg[..tiles].iter_mut() {
                let g = *v as usize;
                if map[g] == u32::MAX {
                    map[g] = used;
                    used += 1;
                }
                *v = map[g];
            }
            stored = used;
            self.himg = himg;
            self.map = map;
        }
        if self.eos() {
            return Err(ERR_TRUNC);
        }
        self.check((stored as u64) << 6)?;
        self.grp = vec![Grp::default(); stored as usize];
        // read nmax groups of 5 codes
        for g in 0..nmax as usize {
            let (store, slot) = if self.map.is_empty() {
                (true, 0)
            } else if self.map[g] == u32::MAX {
                (false, 0)
            } else {
                (true, self.map[g] as usize)
            };
            for j in 0..5 {
                let mut asz = ASIZE[j];
                if j == 0 && cbits != 0 {
                    asz += 1 << cbits; // + cache size
                }
                let off = self.tab_used;
                self.read_code(asz, store)?;
                if store {
                    self.grp[slot].t[j] = off;
                }
            }
            if store {
                // trivial literal / code flags from the first root entries
                let gr = self.grp[slot];
                let (r, b, a) = (self.tab[gr.t[1]], self.tab[gr.t[2]], self.tab[gr.t[3]]);
                if (r | b | a) & 0xFF == 0 {
                    let mut lit = (a >> 16) << 24 | (r & 0xFFFF_0000) | (b >> 16);
                    let mut fl = GRF_TLIT;
                    let g0 = self.tab[gr.t[0]];
                    if g0 & 0xFF == 0 && g0 >> 16 < NUM_LIT {
                        lit |= (g0 >> 16) << 8;
                        fl = GRF_TLIT | GRF_TCODE;
                    }
                    self.grp[slot].lit = lit;
                    self.grp[slot].fl = fl;
                }
            }
        }
        self.ccb = cbits;
        if cbits != 0 {
            self.check(4u64 << cbits)?;
            self.cache[..1 << cbits].fill(0);
        }
        Ok(())
    }

    // vl_read_code: one prefix code: simple (1 or 2 symbols) or code-length coded; validated
    // with build; with store the table is appended to tab at tab_used -> entries
    fn read_code(&mut self, alphabet: u32, store: bool) -> Result<u32, i32> {
        self.cl[..alphabet as usize].fill(0);
        if self.bits(1) != 0 {
            let two = self.bits(1);
            let nb = if self.bits(1) == 0 { 1 } else { 8 };
            let s = self.bits(nb) as usize;
            self.cl[s] = 1; // symbol < 256 <= MAX_ALPHA
            if two != 0 {
                let s = self.bits(8) as usize;
                self.cl[s] = 1;
            }
        } else {
            let n = self.bits(4) + 4; // code-length codes 4..19
            self.clcl = [0; 24];
            for i in 0..n as usize {
                let v = self.bits(3) as u8;
                self.clcl[CL_ORDER[i] as usize] = v;
            }
            self.read_lengths(alphabet)?;
        }
        if self.eos() {
            return Err(ERR_TRUNC);
        }
        let entries = build(&self.cl[..alphabet as usize], HROOT, None);
        if entries == 0 {
            return Err(ERR_CODE);
        }
        if store {
            // grow the table buffer
            let need = self.tab_used + entries as usize;
            if need > self.tab.len() {
                let cap = (self.tab.len() * 2).max(need).max(TAB_MIN);
                self.check(cap as u64 * 4)?;
                self.tab.resize(cap, 0);
            }
            let at = self.tab_used;
            build(&self.cl[..alphabet as usize], HROOT, Some(&mut self.tab[at..]));
            self.tab_used += entries as usize;
        }
        Ok(entries)
    }

    // vl_read_lengths: code lengths coded with the code-length code in clcl (literals
    // 0..15, 16 repeat previous 3..6, 17 zeros 3..10, 18 zeros 11..138; optional
    // max_symbol) into cl
    fn read_lengths(&mut self, alphabet: u32) -> Result<(), i32> {
        if build(&self.clcl[..19], CLROOT, None) == 0 {
            return Err(ERR_CODE);
        }
        let mut t = [0u32; 128];
        build(&self.clcl[..19], CLROOT, Some(&mut t));
        self.cltab = t;
        let mut maxs = alphabet;
        if self.bits(1) != 0 {
            let nb = self.bits(3) * 2 + 2;
            maxs = self.bits(nb) + 2;
            if maxs > alphabet {
                return Err(ERR_CODE);
            }
        }
        let mut s = 0u32;
        let mut prev = 8u8; // previous non-zero length
        while s < alphabet {
            if maxs == 0 {
                break;
            }
            maxs -= 1;
            self.br.refill();
            let e = self.cltab[(self.br.buf & 127) as usize];
            self.br.skip(e & 0xFF);
            let code = e >> 16; // length code 0..18
            if code < 16 {
                self.cl[s as usize] = code as u8;
                s += 1;
                if code != 0 {
                    prev = code as u8;
                }
                continue;
            }
            let k = (code - 16) as usize;
            let v = if code == 16 { prev } else { 0 };
            let cnt = REP_OFFS[k] as u32 + self.br.get(REP_BITS[k] as u32) as u32;
            if s + cnt > alphabet {
                return Err(ERR_CODE);
            }
            self.cl[s as usize..(s + cnt) as usize].fill(v);
            s += cnt;
        }
        Ok(())
    }
}

/// libwebp's BuildHuffmanTable (vl_build / vp8l_build_table): a canonical prefix-code
/// decode table from u8 lengths (0..15, n <= 2328), root_bits 1..8; table = None only
/// validates and sizes.  Root entries with length > root_bits point to a second-level
/// table at entry + symbol.  Returns the number of entries used (0 = incomplete /
/// over-subscribed / empty code; a single used symbol gives a 0-length code).
pub fn build(lengths: &[u8], root: u32, mut table: Option<&mut [u32]>) -> u32 {
    let n = lengths.len();
    let mut cnt = [0u32; 16];
    let mut offs = [0u32; 17];
    for &l in lengths {
        cnt[(l & 15) as usize] += 1;
    }
    if cnt[0] as usize == n {
        return 0;
    }
    for l in 1..MAXLEN {
        if cnt[l] > 1 << l {
            return 0;
        }
        offs[l + 1] = cnt[l] + offs[l];
    }
    let mut sorted = [0u16; MAX_ALPHA + 8];
    for (i, &l) in lengths.iter().enumerate() {
        let l = (l & 15) as usize;
        if l != 0 {
            let d = offs[l] as usize;
            offs[l] += 1;
            if d < MAX_ALPHA {
                sorted[d] = i as u16;
            }
        }
    }
    let mut total = 1u32 << root; // total size = root table
    if offs[MAXLEN] == 1 {
        if let Some(t) = table {
            let e = (sorted[0] as u32) << 16; // 0-bit code
            t[..total as usize].fill(e);
        }
        return total;
    }
    let mut nodes = 1i32;
    let mut open = 1i32;
    let mut tsize = total;
    let mut low = u32::MAX;
    let mut next = 0usize;
    let root_size = total;
    let mut cur = 0usize; // current table (entry offset)
    let mut key = 0u32; // bit-reversed code
    let mut len = 1usize;
    while len <= root as usize {
        let o = open * 2;
        nodes += o;
        let o = o - cnt[len] as i32;
        if o < 0 {
            return 0;
        }
        open = o;
        while cnt[len] != 0 {
            cnt[len] -= 1;
            if let Some(t) = table.as_deref_mut() {
                let e = (sorted[next] as u32) << 16 | len as u32;
                next += 1;
                let step = 1u32 << len;
                let mut c = root_size;
                loop {
                    c -= step;
                    t[(key + c) as usize] = e;
                    if c == 0 {
                        break;
                    }
                }
            }
            key = next_key(key, len as u32);
        }
        len += 1;
    }
    while len <= MAXLEN {
        let o = open * 2;
        nodes += o;
        let o = o - cnt[len] as i32;
        if o < 0 {
            return 0;
        }
        open = o;
        while cnt[len] != 0 {
            let lo = key & (root_size - 1);
            if lo != low {
                cur += tsize as usize;
                // NextTableBitSize
                let mut l = len;
                let mut left = 1i32 << (len - root as usize);
                while l < MAXLEN {
                    left -= cnt[l] as i32;
                    if left <= 0 {
                        break;
                    }
                    l += 1;
                    left += left;
                }
                let tbits = (l - root as usize) as u32;
                tsize = 1 << tbits;
                total += tsize;
                low = lo;
                if let Some(t) = table.as_deref_mut() {
                    t[lo as usize] = ((cur as u32).wrapping_sub(lo)) << 16 | (tbits + root);
                }
            }
            cnt[len] -= 1;
            if let Some(t) = table.as_deref_mut() {
                let e = (sorted[next] as u32) << 16 | (len as u32 - root);
                next += 1;
                let step = 1u32 << (len as u32 - root);
                let base = cur + (key >> root) as usize;
                let mut c = tsize;
                loop {
                    c -= step;
                    t[base + c as usize] = e;
                    if c == 0 {
                        break;
                    }
                }
            }
            key = next_key(key, len as u32);
        }
        len += 1;
    }
    if (offs[MAXLEN] * 2).wrapping_sub(1) as i32 != nodes {
        return 0;
    }
    total
}

// libwebp GetNextKey: reverse(reverse(key, len) + 1, len)
#[inline]
fn next_key(key: u32, len: u32) -> u32 {
    let mut step = 1u32 << (len - 1);
    while key & step != 0 {
        step >>= 1;
    }
    if step != 0 { (key & (step - 1)) + step } else { key }
}

/// the public wrapper of build (vp8l_build_table): entries / 0
pub fn build_table(lengths: &[u8], root_bits: u32, table: Option<&mut [u32]>) -> u32 {
    build(lengths, root_bits, table)
}

// ------------------------------------------------------------------ pixel decoding

/// a group's tables for the pixel loop: the root tables as fixed-size slices
struct Roots<'t> {
    r: [&'t [u32; 1 << HROOT]; 5],
    t: [usize; 5],
    lit: u32,
    fl: u32,
}

// READSYM with the root table of code k of the group
#[inline(always)]
fn read_sym2(br: &mut Bits, tab: &[u32], g: &Roots, k: usize) -> u32 {
    let idx = (br.buf & 0xFF) as usize;
    let mut e = g.r[k][idx];
    if e & 0xFF > HROOT {
        br.skip(HROOT);
        let n2 = (e & 0xFF) - HROOT;
        let base = g.t[k] + idx + (e >> 16) as usize;
        e = tab[base + (br.buf as u32 & ((1 << n2) - 1)) as usize];
    }
    br.skip(e & 0xFF);
    e >> 16
}

// the prefix value of length / distance symbol s (< 4: s; else extra bits)
#[inline(always)]
fn prefix_value(br: &mut Bits, s: u32) -> u32 {
    if s < 4 {
        return s;
    }
    let eb = (s >> 1) - 1; // extra bits (s - 2) >> 1
    let base = (2 + (s & 1)) << eb;
    base + br.get(eb) as u32
}

// the distance of distance code dc >= 1 (the plane map for dc <= 120, at least 1)
#[inline(always)]
fn plane_dist(dc: u32, xs: u32) -> usize {
    (if dc > 120 {
        dc - 120
    } else {
        let v = CODE_TO_PLANE[(dc - 1) as usize] as i32;
        let d = (v >> 4) * xs as i32 + (8 - (v & 15));
        if d < 1 { 1 } else { d as u32 }
    }) as usize
}

impl Vl<'_> {
    // vl_decode_pixels: decode xs * ys ARGB pixels with the current prefix codes / cache /
    // entropy image into px (bytes)
    fn decode_pixels(&mut self, px: &mut [u8], xs: u32, ys: u32, level0: bool) -> Result<(), i32> {
        if self.a8 { self.decode_px::<true>(px, xs, ys, level0) } else { self.decode_px::<false>(px, xs, ys, level0) }
    }

    fn decode_px<const A8: bool>(&mut self, px: &mut [u8], xs: u32, ys: u32, level0: bool) -> Result<(), i32> {
        let end = xs as usize * ys as usize;
        let px = &mut px[..end * 4];
        let hb = self.hb;
        let hmask = if hb == 0 { u32::MAX } else { (1u32 << hb) - 1 };
        let shift = if self.ccb == 0 { 0 } else { 32 - self.ccb };
        let cmask = (1u32 << self.ccb).wrapping_sub(1) & ((1 << MAX_CACHE_BITS) - 1);
        let mut br = self.br;
        let tab = &self.tab[..];
        if self.grp.is_empty() {
            return Err(ERR_CODE);
        }
        // the root tables (HROOT bits) of every group
        let roots: Vec<Roots> = self
            .grp
            .iter()
            .map(|g| Roots {
                r: core::array::from_fn(|k| tab[g.t[k]..g.t[k] + (1 << HROOT)].try_into().unwrap()),
                t: g.t,
                lit: g.lit,
                fl: g.fl,
            })
            .collect();
        let grp = &roots[..];
        let himg = &self.himg[..];
        let hxs = self.hxs;
        let cache = &mut self.cache;
        let cancel = self.cancel;
        let data_bits = br.data.len() as i64 * 8;
        let mut cur = 0usize;
        let mut col = 0u32;
        let mut row = 0u32;
        let mut g: &Roots = &grp[0];
        // end of a decoded row: stop at an overrun, poll the cancel flag (level 0)
        macro_rules! rowend {
            () => {
                if !A8 && br.overrun() {
                    self.br = br;
                    return Err(ERR_TRUNC);
                }
                if level0 && cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
                    self.br = br;
                    return Err(ERR_CANCEL);
                }
            };
        }
        let mut lookup = false; // enter at .dp_lookup (after a copy)
        'pix: while cur < end {
            if !A8 || !lookup {
                if A8 {
                    // 8-bit alpha path: the last <= 128 bits go through a8_tail
                    let consumed = br.pos as i64 * 8 - br.cnt as i64;
                    if data_bits - consumed <= 128 {
                        let st = A8 { cur, end, c: consumed as u64, col, row, xs };
                        self.br = br;
                        return self.a8_tail(px, st);
                    }
                }
                if col & hmask == 0 && hb != 0 {
                    g = &grp[himg[((row >> hb) * hxs + (col >> hb)) as usize] as usize];
                }
            } else {
                lookup = false;
                if hb != 0 {
                    g = &grp[himg[((row >> hb) * hxs + (col >> hb)) as usize] as usize];
                }
            }
            let p;
            if g.fl & GRF_TCODE != 0 {
                p = g.lit;
            } else {
                br.refill();
                let s = read_sym2(&mut br, tab, g, 0);
                if s < NUM_LIT {
                    if g.fl & GRF_TLIT != 0 {
                        p = g.lit | s << 8;
                    } else {
                        let r = read_sym2(&mut br, tab, g, 1);
                        br.refill();
                        let b = read_sym2(&mut br, tab, g, 2);
                        let a = read_sym2(&mut br, tab, g, 3);
                        p = s << 8 | r << 16 | b | a << 24;
                    }
                } else if s < NUM_LIT + NUM_LEN {
                    // backward reference
                    let len = prefix_value(&mut br, s - NUM_LIT) as usize + 1;
                    let ds = read_sym2(&mut br, tab, g, 4);
                    br.refill();
                    let dist = plane_dist(prefix_value(&mut br, ds) + 1, xs);
                    if cur < dist || end - cur < len {
                        self.br = br;
                        return Err(ERR_DIST);
                    }
                    copy_fwd(px, cur, dist, len);
                    if shift != 0 {
                        for k in cur..cur + len {
                            let q = rd(px, k);
                            cache[(q.wrapping_mul(HASH_MUL) >> shift) as usize & ((1 << MAX_CACHE_BITS) - 1)] = q;
                        }
                    }
                    cur += len;
                    col += len as u32;
                    while col >= xs {
                        col -= xs;
                        row += 1;
                        rowend!();
                    }
                    if cur >= end {
                        break 'pix;
                    }
                    // a new group after every copy (the 8-bit alpha path skips its hand-over
                    // check there, like the assembly)
                    if A8 {
                        lookup = true;
                    } else if hb != 0 {
                        g = &grp[himg[((row >> hb) * hxs + (col >> hb)) as usize] as usize];
                    }
                    continue 'pix;
                } else {
                    if shift == 0 {
                        self.br = br;
                        return Err(ERR_CODE);
                    }
                    p = cache[((s - (NUM_LIT + NUM_LEN)) & cmask) as usize];
                }
            }
            wr(px, cur, p);
            if shift != 0 {
                cache[(p.wrapping_mul(HASH_MUL) >> shift) as usize & ((1 << MAX_CACHE_BITS) - 1)] = p;
            }
            cur += 1;
            col += 1;
            if col >= xs {
                col = 0;
                row += 1;
                rowend!();
            }
        }
        self.br = br;
        if !A8 && br.overrun() {
            return Err(ERR_TRUNC);
        }
        Ok(())
    }

    // vl_a8_tail: finish an ALPH image stream that takes libwebp's 8-bit alpha path from
    // the handed-over state: libwebp DecodeAlphaData with an exact model of its
    // VP8LBitReader -- 64-bit window val over the bytes before pos, reads val >> (bit_pos &
    // 63) (past the data this wraps inside the window instead of giving zeros), end of
    // stream when pos = len and bit_pos > 64 (ShiftBytes / ReadBits then reset bit_pos to
    // 0), running out of data while decoding the last symbol is accepted.  The starting
    // state is the canonical one for the consumed bit count (pos = min(len, 8 + C / 8)).
    fn a8_tail(&mut self, px: &mut [u8], st: A8) -> Result<(), i32> {
        let buf = self.br.data;
        let len = buf.len(); // >= 8 (short streams are padded)
        let mut r = Lbr { buf, len, pos: 0, bit_pos: 0, val: 0, eos: false };
        let pos = ((st.c >> 3) as usize + 8).min(len);
        r.pos = pos;
        r.bit_pos = (st.c as i64 - ((pos as i64 - 8) * 8)) as i32;
        r.val = u64::from_le_bytes(buf[pos - 8..pos].try_into().unwrap());
        let (mut cur, end) = (st.cur, st.end);
        let (mut col, mut row, xs) = (st.col, st.row, st.xs);
        let tab = &self.tab[..];
        while !r.eos && cur < end {
            let g = if self.hb != 0 {
                let hb = self.hb;
                self.grp[self.himg[((row >> hb) * self.hxs + (col >> hb)) as usize] as usize]
            } else {
                self.grp[0]
            };
            r.fill();
            let s = r.sym(tab, g.t[0]);
            if s < NUM_LIT {
                wr(px, cur, s << 8); // the index is the green byte
                cur += 1;
                col += 1;
                if col >= xs {
                    col = 0;
                    row += 1;
                }
            } else if s < NUM_LIT + NUM_LEN {
                // GetCopyLength
                let ls = s - NUM_LIT;
                let mut l = ls;
                if ls >= 4 {
                    let eb = (ls >> 1) - 1;
                    let base = (2 + (ls & 1)) << eb;
                    l = r.read_bits(eb) + base;
                }
                let length = l + 1;
                let ds = r.sym(tab, g.t[4]); // distance symbol
                r.fill();
                let mut d = ds; // GetCopyDistance
                if ds >= 4 {
                    let eb = (ds >> 1) - 1;
                    let base = (2 + (ds & 1)) << eb;
                    d = r.read_bits(eb) + base;
                }
                let dist = plane_dist(d + 1, xs);
                if cur < dist || end - cur < length as usize {
                    return Err(ERR_DIST);
                }
                copy_fwd(px, cur, dist, length as usize);
                cur += length as usize;
                col += length;
                while col >= xs {
                    col -= xs;
                    row += 1;
                }
            } else {
                return Err(ERR_CODE);
            }
            r.iseos(); // br->eos_ = VP8LIsEndOfStream(br)
        }
        r.iseos();
        if r.eos && cur < end {
            return Err(ERR_TRUNC);
        }
        Ok(())
    }
}

// pixels cur..cur + len = the pixels dist before, copied forwards (overlapping copies
// repeat)
#[inline(always)]
fn copy_fwd(px: &mut [u8], cur: usize, dist: usize, len: usize) {
    if dist >= len {
        px.copy_within((cur - dist) * 4..(cur - dist + len) * 4, cur * 4);
    } else if dist == 1 {
        let v = rd(px, cur - 1);
        for k in cur..cur + len {
            wr(px, k, v);
        }
    } else {
        for k in cur..cur + len {
            let v = rd(px, k - dist);
            wr(px, k, v);
        }
    }
}

/// libwebp's VP8LBitReader (the 8-bit alpha path of a8_tail)
struct Lbr<'a> {
    buf: &'a [u8],
    len: usize,
    pos: usize,
    bit_pos: i32,
    val: u64,
    eos: bool,
}

impl Lbr<'_> {
    // ReadSymbol: root lookup, second level after SetBitPos(+8), then SetBitPos(+length)
    fn sym(&mut self, tab: &[u32], t: usize) -> u32 {
        let v = self.val >> (self.bit_pos & 63);
        let mut at = t + (v & 0xFF) as usize;
        let mut e = tab[at];
        let mut l = e & 0xFF;
        if l > HROOT {
            self.bit_pos += HROOT as i32;
            at += (e >> 16) as usize; // second-level table
            let v = (self.val >> (self.bit_pos & 63)) as u32;
            e = tab[at + (v & ((1 << (l - HROOT)) - 1)) as usize];
            l = e & 0xFF;
        }
        self.bit_pos += l as i32;
        e >> 16
    }

    // ShiftBytes (+ end-of-stream reset)
    fn shift(&mut self) {
        while self.bit_pos >= 8 && self.pos < self.len {
            self.val >>= 8;
            self.val |= (self.buf[self.pos] as u64) << 56;
            self.pos += 1;
            self.bit_pos -= 8;
        }
        if self.eos || (self.pos == self.len && self.bit_pos > 64) {
            self.eos = true;
            self.bit_pos = 0;
        }
    }

    // VP8LReadBits
    fn read_bits(&mut self, n: u32) -> u32 {
        if self.eos {
            self.bit_pos = 0;
            return 0;
        }
        let v = (self.val >> (self.bit_pos & 63)) as u32 & ((1u32 << n) - 1);
        self.bit_pos += n as i32;
        self.shift();
        v
    }

    // VP8LFillBitWindow
    fn fill(&mut self) {
        if self.bit_pos < 32 {
            return;
        }
        if self.pos + 8 >= self.len {
            self.shift();
            return;
        }
        self.val >>= 32;
        let w = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.val |= (w as u64) << 32;
        self.bit_pos -= 32;
        self.pos += 4;
    }

    // eos |= pos = len and bit_pos > 64 (no reset)
    fn iseos(&mut self) {
        if self.pos == self.len && self.bit_pos > 64 {
            self.eos = true;
        }
    }
}

// ------------------------------------------------------------------ inverse transforms
// The assembly's SSE2 code: 4 pixels at a time where the result depends only on the row
// above (subtract green, cross colour, predictor modes T, TR, TL, avg(TL, T), avg(T, TR)),
// single pixels in registers for the modes that use the left neighbour.  Every pixel's
// result is the same per-byte arithmetic either way.

type V = <Sse2 as Simd>::V;

// per-byte wrapping add of two pixels (paddb)
#[inline(always)]
pub fn add_px(a: u32, b: u32) -> u32 {
    let ag = (a & 0xFF00_FF00).wrapping_add(b & 0xFF00_FF00);
    let rb = (a & 0x00FF_00FF).wrapping_add(b & 0x00FF_00FF);
    (ag & 0xFF00_FF00) | (rb & 0x00FF_00FF)
}

// per-byte floor((a + b) / 2) (AVG2: pavgb - ((a ^ b) & 1))
#[inline(always)]
pub fn avg2(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xFEFE_FEFE) >> 1).wrapping_add(a & b)
}

// AVG2 on 4 pixels
#[inline(always)]
fn avg2v(s: Sse2, a: V, b: V) -> V {
    s.sub8(s.avgu8(a, b), s.and(s.xor(a, b), s.splat8(1)))
}

// 4 pixels from pixel index i
#[inline(always)]
fn ld4(s: Sse2, px: &[u8], i: usize) -> V {
    s.load(&px[i * 4..i * 4 + 16])
}

#[inline(always)]
fn st4(s: Sse2, px: &mut [u8], i: usize, v: V) {
    s.store(&mut px[i * 4..i * 4 + 16], v)
}

// vl_inverse: apply one inverse transform in place
fn inverse(px: &mut [u8], t: &Tf) {
    match t.ty {
        TR_PRED => inv_pred(px, t),
        TR_CROSS => inv_cross(px, t),
        TR_SUBG => inv_subg(px, t.xs as usize * t.ys as usize),
        _ => inv_cidx(px, t),
    }
}

// vl_inv_subg: add green to red and blue (4 pixels at a time)
fn inv_subg(px: &mut [u8], n: usize) {
    let s = Sse2;
    let m = s.splat32(0xFF);
    let mut i = 0;
    while n - i >= 4 {
        let x = ld4(s, px, i);
        let g = s.and(s.srl32::<8>(x), m); // green
        let g = s.or(g, s.sll32::<16>(g)); // green in blue and red
        st4(s, px, i, s.add8(x, g));
        i += 4;
    }
    while i < n {
        let p = rd(px, i);
        let g = (p >> 8) & 0xFF;
        wr(px, i, add_px(p, g | g << 16));
        i += 1;
    }
}

// vl_inv_cidx: colour indexing: index = green (bits 0) or 8 >> bits-bit indices packed
// into the green of the narrowed image (first pixel in the low bits); expanded in place
// from the last pixel backwards
fn inv_cidx(px: &mut [u8], t: &Tf) {
    let pal: &[u32; 256] = t.data[..256].try_into().unwrap();
    let (xs, ys, bits) = (t.xs as usize, t.ys as usize, t.bits);
    if bits == 0 {
        for p in px[..xs * ys * 4].chunks_exact_mut(4) {
            p.copy_from_slice(&pal[p[1] as usize].to_le_bytes());
        }
        return;
    }
    let bpi = 8 >> bits; // bits per index
    let pmask = (1usize << bits) - 1; // position mask
    let pw = (xs + pmask) >> bits; // packed width
    let imask = (1u32 << bpi) - 1;
    for y in (0..ys).rev() {
        let (o, pr) = (y * xs, y * pw);
        for x in (0..xs).rev() {
            let g = px[(pr + (x >> bits)) * 4 + 1] as u32;
            let sh = (x & pmask) as u32 * bpi;
            wr(px, o + x, pal[((g >> sh) & imask) as usize]);
        }
    }
}

// ColorTransformDelta: (int8 a * int8 b) >> 5
#[inline(always)]
fn ctd(m: i32, c: u32) -> u32 {
    ((m * (c as u8 as i8 as i32)) >> 5) as u32
}

// CROSS on 4 pixels: m1 = CST(g2r) << 16 | CST(g2b), m2 = CST(r2b) << 16 (CST(m) =
// int8(m) * 8) (the math of libwebp's SSE2 version)
#[inline(always)]
fn cross4(s: Sse2, x: V, m1: V, m2: V) -> V {
    let k = s.splat32(0xFF00_FF00);
    let ag = s.and(x, k); // alpha, green
    let g = s.and(x, s.splat32(0xFF00));
    let g = s.or(g, s.sll32::<16>(g)); // green << 8 in both 16-bit halves
    let x = s.add8(x, s.mulhis16(g, m1)); // red', blue + delta
    let r = s.srl32::<16>(s.mulhis16(s.sll16::<8>(x), m2)); // red' * r2b >> 5
    let x = s.add8(x, r); // blue'
    s.or(s.andnot(k, x), ag)
}

// vl_inv_cross: cross-colour transform: per tile the multipliers green_to_red (blue
// byte), green_to_blue (green), red_to_blue (red)
fn inv_cross(px: &mut [u8], t: &Tf) {
    let s = Sse2;
    let (xs, ys, bits) = (t.xs as usize, t.ys as usize, t.bits);
    let tw = (xs + (1 << bits) - 1) >> bits; // tiles per row
    for y in 0..ys {
        let mrow = &t.data[(y >> bits) * tw..];
        let o = y * xs;
        let mut x = 0;
        while x < xs {
            let tile = x >> bits;
            let xe = ((tile + 1) << bits).min(xs); // tile end
            let m = mrow[tile];
            let g2r = m as u8 as i8 as i32;
            let g2b = (m >> 8) as u8 as i8 as i32;
            let r2b = (m >> 16) as u8 as i8 as i32;
            let m1 = s.splat32(((g2r << 19) as u32) | ((g2b << 3) as u32 & 0xFFFF));
            let m2 = s.splat32((r2b << 19) as u32);
            while xe - x >= 4 {
                let v = cross4(s, ld4(s, px, o + x), m1, m2);
                st4(s, px, o + x, v);
                x += 4;
            }
            while x < xe {
                let v = rd(px, o + x);
                let g = v >> 8;
                let r = ((v >> 16).wrapping_add(ctd(g2r, g))) & 0xFF;
                let b = (v.wrapping_add(ctd(g2b, g)).wrapping_add(ctd(r2b, r))) & 0xFF;
                wr(px, o + x, (v & 0xFF00_FF00) | r << 16 | b);
                x += 1;
            }
        }
    }
}

// Select(T, L, TL): sum |L - TL| > sum |T - TL| ? L : T (psadbw)
#[inline(always)]
fn select(s: Sse2, l: V, t: V, tl: V) -> V {
    let pa = s.to_i32(s.sadu8(l, tl));
    let pb = s.to_i32(s.sadu8(t, tl));
    if pa > pb { l } else { t }
}

// clamp(L + T - TL) per channel
#[inline(always)]
fn clamp_full(s: Sse2, l: V, t: V, tl: V) -> V {
    let z = s.zero();
    let a = s.unpacklo8(l, z);
    let b = s.unpacklo8(t, z);
    let c = s.unpacklo8(tl, z);
    let v = s.sub16(s.add16(a, b), c);
    s.packus16(v, z)
}

// a = Average2(L, T); clamp(a + (a - TL) / 2) per channel (truncated division)
#[inline(always)]
fn clamp_half(s: Sse2, l: V, t: V, tl: V) -> V {
    let z = s.zero();
    let a = s.unpacklo8(avg2v(s, l, t), z);
    let c = s.unpacklo8(tl, z);
    let d = s.sub16(a, c);
    let d = s.sra16::<1>(s.add16(d, s.srl16::<15>(d)));
    let v = s.add16(a, d);
    s.packus16(v, z)
}

// vl_inv_pred: predictor transform (modes 0..13, 14 / 15 act as 0; row 0: black then L;
// column 0: T; TR of the last column = the first pixel of the current row, as in libwebp)
fn inv_pred(px: &mut [u8], t: &Tf) {
    let (xs, ys, bits) = (t.xs as usize, t.ys as usize, t.bits);
    let tw = (xs + (1 << bits) - 1) >> bits; // tiles per row
    wr(px, 0, rd(px, 0).wrapping_add(0xFF00_0000));
    for x in 1..xs {
        wr(px, x, add_px(rd(px, x - 1), rd(px, x)));
    }
    for y in 1..ys {
        let o = y * xs;
        wr(px, o, add_px(rd(px, o - xs), rd(px, o)));
        let mrow = &t.data[(y >> bits) * tw..];
        let mut x = 1;
        while x < xs {
            let tile = x >> bits;
            let mode = (mrow[tile] >> 8) & 15;
            let xe = ((tile + 1) << bits).min(xs);
            pred_seg(px, o, x, xe, xs, mode);
            x = xe;
        }
    }
}

// one segment of a row with one predictor mode (the asm's .ip_mN handlers)
#[inline(always)]
fn pred_seg(px: &mut [u8], o: usize, x0: usize, x1: usize, xs: usize, mode: u32) {
    let s = Sse2;
    // 4 at a time: the prediction comes from the row above only ($v: 4 pixels at u, $p:
    // one pixel at u)
    macro_rules! up {
        ($u:ident, $v:expr, $p:expr) => {{
            let mut x = x0;
            while x1 - x >= 4 {
                let i = o + x;
                let $u = i - xs;
                let p = $v;
                let v = s.add8(p, ld4(s, px, i));
                st4(s, px, i, v);
                x += 4;
            }
            while x < x1 {
                let i = o + x;
                let $u = i - xs;
                let p = $p;
                wr(px, i, add_px(p, rd(px, i)));
                x += 1;
            }
        }};
    }
    // the modes with the left neighbour: one pixel at a time
    macro_rules! seq {
        ($l:ident, $t:ident, $tr:ident, $tl:ident, $e:expr) => {{
            let (head, tail) = px.split_at_mut(o * 4);
            let up = &head[(o - xs) * 4..];
            let cur = &mut tail[..xs * 4];
            let n = x1.min(xs - 1).max(x0); // pixels whose TR is in the row above
            let mut $l: V = s.load_lane32(&cur[(x0 - 1) * 4..x0 * 4]);
            let mut $tl: V = s.load_lane32(&up[(x0 - 1) * 4..x0 * 4]);
            let mut $t: V = s.load_lane32(&up[x0 * 4..x0 * 4 + 4]);
            for (c, trb) in cur[x0 * 4..n * 4].chunks_exact_mut(4).zip(up[(x0 + 1) * 4..(n + 1) * 4].chunks_exact(4)) {
                let $tr: V = s.load_lane32(trb);
                let _ = ($t, $tr, $tl);
                $l = s.add8($e, s.load_lane32(c));
                c.copy_from_slice(&(s.to_i32($l) as u32).to_le_bytes());
                $tl = $t;
                $t = $tr;
            }
            if n < x1 {
                // the last column: TR = the first pixel of this row
                let $tr: V = s.load_lane32(&cur[..4]);
                let _ = ($t, $tr, $tl);
                $l = s.add8($e, s.load_lane32(&cur[n * 4..n * 4 + 4]));
                cur[n * 4..n * 4 + 4].copy_from_slice(&(s.to_i32($l) as u32).to_le_bytes());
            }
            let _ = ($t, $tl, $l);
        }};
    }
    match mode {
        1 => seq!(l, t, tr, tl, l),
        2 => up!(u, ld4(s, px, u), rd(px, u)),
        3 => up!(u, ld4(s, px, u + 1), rd(px, u + 1)),
        4 => up!(u, ld4(s, px, u - 1), rd(px, u - 1)),
        5 => seq!(l, t, tr, tl, avg2v(s, avg2v(s, l, tr), t)),
        6 => seq!(l, t, tr, tl, avg2v(s, l, tl)),
        7 => seq!(l, t, tr, tl, avg2v(s, l, t)),
        8 => up!(u, avg2v(s, ld4(s, px, u - 1), ld4(s, px, u)), avg2(rd(px, u - 1), rd(px, u))),
        9 => up!(u, avg2v(s, ld4(s, px, u), ld4(s, px, u + 1)), avg2(rd(px, u), rd(px, u + 1))),
        10 => seq!(l, t, tr, tl, avg2v(s, avg2v(s, l, tl), avg2v(s, t, tr))),
        11 => seq!(l, t, tr, tl, select(s, l, t, tl)),
        12 => seq!(l, t, tr, tl, clamp_full(s, l, t, tl)),
        13 => seq!(l, t, tr, tl, clamp_half(s, l, t, tl)),
        _ => {
            for x in x0..x1 {
                wr(px, o + x, rd(px, o + x).wrapping_add(0xFF00_0000));
            }
        }
    }
}

// ------------------------------------------------------------------ public entry points

/// decode a VP8L bitstream (the payload of a "VP8L" chunk) into a new FMT_ARGB surface
/// (straight alpha)  [vp8l_decode]
pub fn decode(data: &[u8], dec: Option<&ImgDec>) -> Result<Surface, i32> {
    if data.len() < 5 {
        return Err(ERR_TRUNC);
    }
    if data[0] != 0x2F {
        return Err(ERR_HDR);
    }
    let hd = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    if hd & 0xE000_0000 != 0 {
        return Err(ERR_HDR); // version 0
    }
    let w = (hd & 0x3FFF) + 1;
    let h = ((hd >> 14) & 0x3FFF) + 1;
    let dd = dec.copied().unwrap_or_default();
    let r = dims_check(w, h, &dd);
    if r != 0 {
        return Err(r);
    }
    let mut pad = [0u8; 8];
    let src: &[u8] = if data.len() < 8 {
        pad[..data.len()].copy_from_slice(data);
        &pad
    } else {
        data
    };
    let mut v = Vl::new(src, dec);
    v.bits(8); // skip the 40 header bits
    v.bits(32);
    // the pixels are decoded contiguously into the surface buffer (stride as surf_alloc:
    // 64-byte multiple), then spread to the stride in place
    let rowb = w as usize * 4;
    let stride = (rowb + 63) & !63;
    let total = h as u64 * stride as u64 + PIXPAD;
    v.check(total)?;
    v.pix = vec![0u8; total as usize];
    v.level0(w, h)?;
    let mut pix = core::mem::take(&mut v.pix);
    if stride != rowb {
        for y in (1..h as usize).rev() {
            pix.copy_within(y * rowb..(y + 1) * rowb, y * stride); // row 0 stays in place
        }
    }
    pix.truncate((stride * h as usize).max(64));
    Ok(Surface { px: pix, w: w as i32, h: h as i32, stride: stride as i32, fmt: FMT_ARGB, flags: SFF_OWNED, opac: 255, blend: 0 })
}

/// decode the header-less VP8L image stream of a compressed ALPH chunk (w x h pixels, w
/// and h 1..16383) and store the green channel of every pixel in plane[w * h]
/// [vp8l_decode_alpha]
pub fn decode_alpha(data: &[u8], plane: &mut [u8], w: u32, h: u32, dec: Option<&ImgDec>) -> i32 {
    let mut pad = [0u8; 8];
    let src: &[u8] = if data.len() < 8 {
        pad[..data.len()].copy_from_slice(data);
        &pad
    } else {
        data
    };
    let mut v = Vl::new(src, dec);
    v.isalpha = true;
    if let Err(e) = v.level0(w, h) {
        return e;
    }
    let n = w as usize * h as usize;
    for (d, p) in plane[..n].iter_mut().zip(v.pix.chunks_exact(4)) {
        *d = p[1];
    }
    0
}
