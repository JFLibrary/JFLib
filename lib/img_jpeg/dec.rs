// dec -- JPEG decoder  [src/img/jpeg_dec.s]
//
//   decode_fl(data, flags) -> Surface / ERR_*                     (jpeg_decode / jpeg_decode_fl)
//   decode_abbrev(tables, data, flags) -> Surface / ERR_*         (jpeg_decode_abbrev)
//
// Flags (JDF_*): JDF_STRAIGHT -> FMT_ARGB output (same bytes, JPEG is opaque); JDF_NO_ORIENT
// -> the EXIF orientation is not applied; JDF_RGB / JDF_YCC force the colour space of
// 3-component images (TIFF Photometric RGB / YCbCr), JDF_CMYK makes 4 components plain
// (non-inverted) CMYK.  decode_abbrev parses `tables` first (SOI, DQT / DHT / DRI / APPn /
// COM, EOI or end of data; SOF / SOS there -> ERR_HDR; empty = none), keeps those tables and
// then decodes `data` like decode_fl with JDF_NO_ORIENT (tables in `data` replace them per
// slot).
//
// Output: a new FMT_PARGB surface (alpha 255, SFF_OPAQUE).  Supported: baseline (SOF0),
// extended sequential 8-bit Huffman (SOF1) and progressive (SOF2, spectral selection +
// successive approximation, EOB runs); 1 (grey), 3 (YCbCr, or RGB with Adobe transform 0 /
// component ids 'R','G','B') and 4 components (CMYK / YCCK; with an Adobe APP14 marker the
// values are Adobe-inverted, without one plain CMYK); sampling factors 1..4 (h2v1 / h2v2 /
// h1v2 use libjpeg's "fancy" triangle upsampling -- h2v1 / h2v2 only for downsampled widths
// > 2 --, other ratios replicate samples); restart intervals; DHT / DQT between scans, 16-bit
// quantisation tables, missing Huffman tables (Annex K defaults), fill bytes, garbage between
// segments, missing EOI.  The EXIF orientation (APP1 "Exif", IFD0 tag 0x0112) is applied.
//
// Corrupt / short data: like libjpeg, when the entropy-coded data of a scan runs out (or hits
// an unexpected marker) the rest of that restart interval stays grey (zero coefficients) and
// lost restart markers are resynchronised like jpeg_resync_to_restart; when the file ends,
// everything decoded so far is returned as long as the frame header and at least one scan
// header were read, otherwise ERR_TRUNC.  An undecodable Huffman code or an invalid Huffman
// table gives ERR_CODE.
//
// Errors: ERR_TRUNC, ERR_HDR (no SOI, bad segment, bad SOF/SOS values, missing quantisation
// table, EOI without image), ERR_UNSUPPORTED (arithmetic coding, lossless / hierarchical,
// 12-bit, 2 components, DNL height), ERR_DIMS (w, h <= 32768 and w * h <= 2^28), ERR_CODE,
// ERR_NOMEM.
//
// The bit reader, the Huffman tables (9-bit lookup + AC fast path) and every branch follow
// the assembly, so damaged files decode to the same bytes.  Speed: the reader (input and bit
// buffer) is a local of the scan kept in registers -- only the byte-by-byte refill and the
// restart handling are out of line and take it by value; blocks with AC coefficients wait in
// pairs so that the IDCT runs on two at once (dsp::idct2); the output surface comes from
// try_vec's zeroed allocation (fresh pages, no clearing pass) and every orientation is
// converted in place (perf-load: orientation 1 used to go through a row buffer that was
// appended to the surface -- a second 48 MB copy at 12 MP).
//
// Streaming (perf-load; the assembly always decodes into whole sample planes first): a
// sequential image whose first scan holds every component once -- nearly every baseline
// file -- is decoded into a ring of three MCU rows per component instead of whole planes
// (12 MP 4:4:4: 3 x 96 KB instead of 36 MB, no page faults and no trip through memory for
// the planes), and the output rows are converted while the scan runs, one MCU row behind (the
// fancy upsamplers read one sample row of the next MCU row).  Every ring slot starts grey
// like the planes, so blocks a damaged scan leaves out stay grey; the rows after a scan that
// stopped early are converted at the end from grey slots.  The bytes are those of the plane
// path.  When the stream turns out to differ from what the plane path would see -- a second
// scan, or an APP0 JFIF / APP1 EXIF / APP14 Adobe segment after the scan that changes the
// colour space or the orientation -- the image is decoded again on the plane path
// (Dec::nostream).  A progressive image keeps its coefficient buffers, but the inverse DCT
// after the last scan and the output run MCU row by MCU row through the same ring.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{Surface, FMT_ARGB, FMT_PARGB, SFF_OPAQUE};
use img_core::{ERR_CODE, ERR_DIMS, ERR_HDR, ERR_NOMEM, ERR_TRUNC, ERR_UNSUPPORTED};

use crate::dsp;
use crate::{JDF_CMYK, JDF_NO_ORIENT, JDF_RGB, JDF_STRAIGHT, JDF_YCC};

const MAX_DIM: u32 = 32768;
const MAX_PIXELS: u64 = 1 << 28;

const BAND_LOG: u32 = 4; // rows converted before a transposed write-out
const BAND: u32 = 1 << BAND_LOG;

const UP_FULL: u32 = 0;
const UP_H2V1: u32 = 1;
const UP_H2V2: u32 = 2;
const UP_H1V2: u32 = 3;
const UP_ANY: u32 = 4;

const CS_GRAY: u32 = 0;
const CS_YCC: u32 = 1;
const CS_RGB: u32 = 2;
const CS_CMYK: u32 = 3;
const CS_YCCK: u32 = 4;

const M_EOF: u32 = 0x100; // Br::marker: the data ended

// Dec::run: the streamed decode cannot be finished like the plane path: decode again with
// nostream (never returned to a caller)
const RESTART: i32 = i32::MIN + 0x5354;

// block routines of a scan (JD_BLKFN)
const BF_SEQ: u32 = 0;
const BF_DCF: u32 = 1;
const BF_DCR: u32 = 2;
const BF_ACF: u32 = 3;
const BF_ACR: u32 = 4;

// jd_bias[s] = 1 - 2^s (EXTEND of a value whose top bit is clear)
static BIAS: [i32; 17] =
    [0, -1, -3, -7, -15, -31, -63, -127, -255, -511, -1023, -2047, -4095, -8191, -16383, -32767, -65535];

/// a Huffman decoding table (HT_*), rebuilt in place like the assembly's
#[derive(Clone)]
struct Huff {
    look: [u16; 512], // (length << 8) | symbol for codes of <= 9 bits, 0 = longer
    fac: [i16; 512],  // AC fast path: value << 8 | run << 4 | total bits, 0 = none
    maxc: [u32; 18],  // (last code + 1) << (16 - length)
    delta: [i32; 18], // symbol index - code, per length
    vals: [u8; 256],
    stat: u32, // 0 undefined, 1 ok, 2 bad table
}

impl Huff {
    fn new() -> Huff {
        Huff { look: [0; 512], fac: [0; 512], maxc: [0; 18], delta: [0; 18], vals: [0; 256], stat: 0 }
    }

    /// jd_build: a decoding table from the 16 code counts and the symbols; stat 1, or 2 for
    /// an invalid table
    fn build(&mut self, counts: &[u8], vals: &[u8], is_dc: bool) {
        self.look = [0; 512];
        self.fac = [0; 512];
        let n: usize = counts[..16].iter().map(|&c| c as usize).sum();
        for (i, &v) in vals[..n].iter().enumerate() {
            self.vals[i] = v;
            if is_dc && v > 15 {
                self.stat = 2; // DC categories are 0..15
                return;
            }
        }
        let mut code: u32 = 0;
        let mut si: u32 = 0; // symbol index
        for len in 1..=16u32 {
            let mut cnt = counts[len as usize - 1] as u32;
            self.delta[len as usize] = si.wrapping_sub(code) as i32;
            while cnt != 0 {
                if code >= 1 << len {
                    self.stat = 2; // over-subscribed
                    return;
                }
                if len <= 9 {
                    let sh = 9 - len;
                    let first = code << sh;
                    let e = self.vals[si as usize] as u16 | (len << 8) as u16;
                    for k in first..first + (1 << sh) {
                        self.look[k as usize] = e;
                    }
                }
                code += 1;
                si += 1;
                cnt -= 1;
            }
            if code >= 1 << len {
                self.stat = 2; // all-ones code (libjpeg rejects it)
                return;
            }
            self.maxc[len as usize] = code << (16 - len);
            code <<= 1;
        }
        self.maxc[17] = 0xFFFF_FFFF;
        if !is_dc {
            // fast AC entries: code + magnitude bits within 9 bits, value in -128..127
            for i in 0..512u32 {
                let e = self.look[i as usize] as u32;
                if e == 0 {
                    continue;
                }
                let clen = e >> 8;
                let s = e & 15;
                if s == 0 {
                    continue;
                }
                let run = (e & 255) >> 4;
                let total = clen + s;
                if total > 9 {
                    continue;
                }
                let mut v = ((i >> (9 - total)) & ((1 << s) - 1)) as i32;
                if v < 1 << (s - 1) {
                    v += (-1i32 << s) + 1; // negative value
                }
                if !(-128..=127).contains(&v) {
                    continue;
                }
                self.fac[i as usize] = ((v << 8) | (run << 4) as i32 | total as i32) as i16;
            }
        }
        self.stat = 1;
    }
}

/// the input side of the bit reader (rsi and the JD_* fields of the assembly); a local of
/// the scan, handed by value to the out-of-line slow paths so that it stays in registers
#[derive(Clone, Copy)]
struct Br<'a> {
    d: &'a [u8],
    pos: usize,
    marker: u32, // marker that stopped the reader (M_EOF = end)
    zbits: u32,  // zero bits fed after the data stopped
    err: i32,    // ERR_CODE from a bad code
}

/// the bit buffer (rbx / r12d of the assembly; MSB first), kept in registers by the block
/// routines: only the out-of-line refill / slow Huffman path see it, by value
#[derive(Clone, Copy)]
struct Bb {
    buf: u64,
    bits: u32,
}

impl Bb {
    /// NEED32
    #[inline(always)]
    fn need32(&mut self, br: &mut Br) {
        if self.bits < 32 {
            *self = br.fill(*self);
        }
    }

    /// HUFFDEC: one symbol (needs >= 16 buffered bits)
    #[inline(always)]
    fn huff(&mut self, br: &mut Br, t: &Huff) -> u32 {
        let e = t.look[(self.buf >> 55) as usize] as u32;
        if e != 0 {
            let l = e >> 8;
            self.buf <<= l;
            self.bits = self.bits.wrapping_sub(l);
            e & 255
        } else {
            let (buf, e) = huff_slow(t, self.buf);
            self.buf = buf;
            self.bits = self.bits.wrapping_sub((e >> 8) & 255);
            if e & 0x8000_0000 != 0 {
                br.err = ERR_CODE;
            }
            e & 255
        }
    }

    /// EXTEND: read s (1..15) bits and sign-extend them the JPEG way
    #[inline(always)]
    fn extend(&mut self, s: u32) -> i32 {
        let v = self.buf;
        self.buf <<= s;
        self.bits = self.bits.wrapping_sub(s);
        let bias = if (v as i64) < 0 { 0 } else { BIAS[s as usize] };
        ((v >> (64 - s)) as u32 as i32).wrapping_add(bias)
    }

    /// r (1..14) raw bits
    #[inline(always)]
    fn bits_n(&mut self, r: u32) -> u32 {
        let v = self.buf;
        self.buf <<= r;
        self.bits = self.bits.wrapping_sub(r);
        (v >> (64 - r)) as u32
    }

    /// one bit (bits must be > 0)
    #[inline(always)]
    fn bit(&mut self) -> bool {
        let v = self.buf;
        self.buf <<= 1;
        self.bits = self.bits.wrapping_sub(1);
        (v as i64) < 0
    }
}

/// jd_huff_slow: a code longer than 9 bits -> (buffer, symbol | consumed bits << 8 | bit 31
/// for a bad code: ERR_CODE, symbol 0)
#[inline(always)]
fn huff_slow(t: &Huff, buf: u64) -> (u64, u32) {
    let c16 = (buf >> 48) as u32;
    for c in 10..17u32 {
        if c16 < t.maxc[c as usize] {
            let idx = t.delta[c as usize].wrapping_add((buf >> (64 - c)) as u32 as i32);
            if idx as u32 > 255 {
                return (buf << c, c << 8 | 0x8000_0000);
            }
            return (buf << c, t.vals[idx as usize] as u32 | c << 8);
        }
    }
    (buf, 0x8000_0000)
}

impl Br<'_> {
    /// jd_fill: top up the bit buffer to more than 56 bits; after a marker or the end of the
    /// data zero bits are fed (counted in zbits)
    #[inline(always)]
    fn fill(&mut self, mut b: Bb) -> Bb {
        if let Some(w) = self.d.get(self.pos..self.pos + 8) {
            let v = u64::from_le_bytes(w.try_into().unwrap());
            let x = !v; // a zero byte here = an 0xFF byte in the data
            if x.wrapping_sub(0x0101_0101_0101_0101) & !x & 0x8080_8080_8080_8080 == 0 {
                b.buf |= v.swap_bytes() >> b.bits;
                let n = (63 - b.bits) >> 3;
                self.pos += n as usize;
                b.bits += n * 8;
                return b;
            }
        }
        let (br, b) = fill_slow(*self, b);
        *self = br;
        b
    }

    /// the byte-by-byte part of jd_fill (an 0xFF byte in the next 8, or fewer than 8 left)
    #[inline(always)]
    fn fill_bytes(&mut self, mut b: Bb) -> Bb {
        let end = self.d.len();
        while b.bits <= 56 {
            if self.marker == 0 {
                if self.pos >= end {
                    self.marker = M_EOF;
                } else {
                    let c = self.d[self.pos];
                    if c != 0xFF {
                        self.pos += 1;
                        b.buf |= (c as u64) << (56 - b.bits);
                        b.bits += 8;
                        continue;
                    }
                    let mut q = self.pos + 1;
                    loop {
                        if q >= end {
                            self.marker = M_EOF;
                            break;
                        }
                        let c = self.d[q];
                        if c == 0xFF {
                            q += 1;
                            continue;
                        }
                        if c == 0 {
                            self.pos = q + 1; // FF 00 -> data byte FF
                            break;
                        }
                        self.marker = c as u32;
                        self.pos = q - 1; // at the FF before the marker code
                        break;
                    }
                    if self.marker == 0 {
                        b.buf |= 0xFF << (56 - b.bits);
                        b.bits += 8;
                        continue;
                    }
                }
            }
            self.zbits += 8;
            b.bits += 8;
        }
        b
    }


    /// jd_find_marker: skip data up to the next marker: pos at its 0xFF, marker = its code,
    /// or M_EOF
    fn find_marker(&mut self) {
        let end = self.d.len();
        loop {
            if self.pos >= end {
                self.pos = end;
                self.marker = M_EOF;
                return;
            }
            if self.d[self.pos] != 0xFF {
                self.pos += 1;
                continue;
            }
            loop {
                let q = self.pos + 1;
                if q >= end {
                    self.pos = end;
                    self.marker = M_EOF;
                    return;
                }
                let c = self.d[q];
                if c == 0xFF {
                    self.pos += 1;
                    continue;
                }
                if c == 0 {
                    self.pos += 2; // stuffed FF 00
                    break;
                }
                self.marker = c as u32;
                return;
            }
        }
    }
}

/// fill_bytes out of line, the reader by value (it stays in registers in the block loops)
#[inline(never)]
fn fill_slow<'a>(mut br: Br<'a>, b: Bb) -> (Br<'a>, Bb) {
    let b = br.fill_bytes(b);
    (br, b)
}

/// a frame component (JC_*)
struct Comp {
    id: u32,
    h: u32,
    v: u32,
    tq: u32,
    bw: u32,     // blocks per row of the plane (MCU padded)
    bh: u32,     // block rows of the plane
    dw: u32,     // samples per row  ceil(W * h / hmax)
    dh: u32,     // rows             ceil(H * v / vmax)
    nbw: u32,    // blocks per row in a single-component scan  ceil(DW / 8)
    nbh: u32,    //
    stride: u32, // plane stride = BW * 8
    pred: i32,   // DC predictor
    // streaming: rows per ring slot (one MCU row; 0 = whole plane); the slot of the MCU row
    // being decoded starts at plane row `base`, it holds the plane rows from `first` on
    ring: u32,
    base: u32,
    first: u32,
    plane: Vec<u8>,
    coef: Vec<i16>, // progressive: 64 i16 per block
    dct: usize,     // table slots
    act: usize,
    qok: bool, // quantisation table latched
    up: u32,   // upsampling mode (UP_*)
    q: [i16; 64], // latched quantisation table, natural order (u16 values)
}

impl Comp {
    fn new() -> Comp {
        Comp {
            id: 0,
            h: 0,
            v: 0,
            tq: 0,
            bw: 0,
            bh: 0,
            dw: 0,
            dh: 0,
            nbw: 0,
            nbh: 0,
            stride: 0,
            pred: 0,
            ring: 0,
            base: 0,
            first: 0,
            plane: Vec::new(),
            coef: Vec::new(),
            dct: 0,
            act: 4,
            qok: false,
            up: 0,
            q: [0; 64],
        }
    }

    /// the byte offset of plane row r (streaming: in its ring slot)
    #[inline(always)]
    fn row_off(&self, r: u32) -> usize {
        let r = if self.ring == 0 { r } else { r / self.ring % 3 * self.ring + r % self.ring };
        self.stride as usize * r as usize
    }
}

/// the output stage (jd_output's locals): the surface, temp rows, the orientation
struct Out {
    sf: Surface,
    tmps: [Vec<u8>; 4], // one upsampling row per component
    band: Vec<u8>,      // BAND converted rows (mirrored / transposed orientations)
    bstride: usize,
    o: u32, // orientation 1..8
}

/// a streamed decode (see the file header): rows are converted while the scan runs
struct Stream {
    out: Out,
    bands: u32,   // MCU rows of the scan (block rows of a single-component scan)
    bho: u32,     // output rows per MCU row
    emitted: u32, // MCU rows converted
    prep: u32,    // MCU rows whose ring slot is ready (grey)
    snap: (u32, bool, bool, u32), // orient, jfif, adobe, xform at the start of the scan
}

/// the decoder context (JD_*)
struct Dec<'a> {
    ht: Vec<Huff>, // 8 tables: 0-3 DC, 4-7 AC
    comp: [Comp; 4],
    qt: [[i16; 64]; 4], // natural order
    qdef: [bool; 4],
    br: Br<'a>,
    w: u32,
    h: u32,
    nc: u32,
    hmax: u32,
    vmax: u32,
    mcux: u32,
    mcuy: u32,
    mode: u32, // 0 no frame yet, 1 sequential, 2 progressive
    scans: u32,
    ri: u32, // restart interval
    jfif: bool,
    adobe: bool,
    xform: u32,
    orient: u32,
    exif: bool,
    insuf: bool, // out of data in this restart interval
    ns: u32,
    ss: u32,
    se: u32,
    ah: u32,
    al: u32,
    eobrun: u32,
    rleft: u32,
    cspace: u32,
    cmode: u32,
    eoi: bool,
    rstnum: u32, // number of the next expected RSTn
    flags: u32,  // JDF_*
    tabonly: bool,
    scomp: [usize; 4], // components of the scan
    blkfn: u32,
    // a block waiting for a second one so that the IDCT runs on two at once (the planes are
    // read only after the scans; blocks are disjoint except a block decoded twice, which
    // keeps its order)
    pblk: [i16; 64],
    pend: Option<(usize, usize)>, // component, plane offset
    pair: bool,                   // AVX2: pair the IDCTs (SSE2 runs them one by one anyway)
    nostream: bool,               // the plane path only (planes allocated at SOF)
    planes: bool,                 // the sample planes (or rings) are allocated
    stream: Option<Stream>,
}

/// a zeroed / filled Vec, None when the memory is not there.  A probe reservation first
/// (vec! would abort instead of failing), then vec![v; n]: for a zero v that is
/// alloc_zeroed, whose fresh pages from the OS need no clearing pass (perf-load: the output
/// surface and the progressive coefficients were cleared byte by byte before)
pub(crate) fn try_vec<T: Clone>(n: usize, v: T) -> Option<Vec<T>> {
    let mut probe: Vec<T> = Vec::new();
    probe.try_reserve_exact(n).ok()?;
    drop(probe);
    Some(vec![v; n])
}

/// surf_alloc with a failure result instead of an abort
pub(crate) fn try_surface(w: u32, h: u32, fmt: u8) -> Option<Surface> {
    if w > 65535 || h > 65535 {
        return None;
    }
    let row = if fmt == gfx::FMT_A8 { w } else { w * 4 };
    let stride = (row + 63) & !63;
    let bytes = (stride as usize * h as usize).max(64);
    let px = try_vec(bytes, 0u8)?;
    Some(Surface { px, w: w as i32, h: h as i32, stride: stride as i32, fmt, flags: gfx::SFF_OWNED, opac: 255, blend: 0 })
}

impl<'a> Dec<'a> {
    fn new(flags: u32) -> Dec<'a> {
        Dec {
            ht: vec![Huff::new(); 8],
            comp: [Comp::new(), Comp::new(), Comp::new(), Comp::new()],
            qt: [[0; 64]; 4],
            qdef: [false; 4],
            br: Br { d: &[], pos: 0, marker: 0, zbits: 0, err: 0 },
            w: 0,
            h: 0,
            nc: 0,
            hmax: 0,
            vmax: 0,
            mcux: 0,
            mcuy: 0,
            mode: 0,
            scans: 0,
            ri: 0,
            jfif: false,
            adobe: false,
            xform: 0,
            orient: 0,
            exif: false,
            insuf: false,
            ns: 0,
            ss: 0,
            se: 0,
            ah: 0,
            al: 0,
            eobrun: 0,
            rleft: 0,
            cspace: 0,
            cmode: 0,
            eoi: false,
            rstnum: 0,
            flags,
            tabonly: false,
            scomp: [0; 4],
            blkfn: BF_SEQ,
            pblk: [0; 64],
            pend: None,
            pair: simd::has_avx2(),
            nostream: false,
            planes: false,
            stream: None,
        }
    }

    // ============================================================== markers

    /// jd_run: parse the marker segments of `d`, decode every scan, then build the output
    fn run(&mut self, d: &'a [u8], out: &mut Option<Surface>) -> i32 {
        let end = d.len();
        if end < 2 {
            return ERR_TRUNC;
        }
        if d[0] != 0xFF || d[1] != 0xD8 {
            return ERR_HDR;
        }
        let mut p = 2usize;
        loop {
            // .jr_marker
            if p >= end {
                break;
            }
            if d[p] != 0xFF {
                p += 1; // garbage between segments
                continue;
            }
            let mut m;
            loop {
                p += 1;
                if p >= end {
                    return self.eof(out);
                }
                m = d[p] as u32;
                if m != 0xFF {
                    break; // else a fill byte
                }
            }
            p += 1;
            if m == 0 {
                continue;
            }
            if m == 0xD9 {
                self.eoi = true;
                break;
            }
            if m == 0xD8 {
                return ERR_HDR; // second SOI
            }
            if m == 0x01 || (0xD0..=0xD7).contains(&m) {
                continue; // TEM, stray RSTn
            }
            // .jr_seg
            if end - p < 2 {
                break;
            }
            let len = (d[p] as usize) << 8 | d[p + 1] as usize;
            if len < 2 {
                return ERR_HDR;
            }
            if len > end - p {
                break; // segment runs past the end of the data
            }
            let seg = &d[p + 2..p + len];
            p += len;
            let r = match m {
                0xC4 => self.dht(seg),
                0xDB => self.dqt(seg),
                0xDA => {
                    if self.tabonly {
                        return ERR_HDR;
                    }
                    if self.stream.is_some() {
                        return RESTART; // a second scan: the plane path
                    }
                    let r = self.sos(seg);
                    if r != 0 {
                        return r;
                    }
                    if !self.planes && self.mode == 1 {
                        // the first scan of a sequential image: rings or planes
                        let r = if self.stream_ok() { self.stream_begin() } else { self.alloc_planes() };
                        if r != 0 {
                            return r;
                        }
                    }
                    let r = self.scan(d, p);
                    if r != 0 {
                        return r;
                    }
                    p = self.br.pos;
                    if self.br.marker == M_EOF {
                        break;
                    }
                    0
                }
                0xDD => {
                    if seg.len() < 2 {
                        return ERR_HDR;
                    }
                    self.ri = (seg[0] as u32) << 8 | seg[1] as u32;
                    0
                }
                0xE0 => {
                    if seg.len() >= 5 && &seg[..5] == b"JFIF\0" {
                        self.jfif = true;
                    }
                    0
                }
                0xE1 => {
                    self.exif_orient(seg);
                    0
                }
                0xEE => {
                    if seg.len() >= 12 && &seg[..5] == b"Adobe" {
                        self.adobe = true;
                        self.xform = seg[11] as u32;
                    }
                    0
                }
                0xC0..=0xC2 => {
                    if self.tabonly {
                        return ERR_HDR; // no frame in a tables-only stream
                    }
                    self.sof(seg, m)
                }
                0xC8 => 0,                              // JPG (reserved): skip
                0xC3..=0xCF | 0xDE | 0xDF => ERR_UNSUPPORTED, // SOF3, SOF5-15, DAC, DHP / EXP
                _ => 0,                                 // COM, APPn, others: skip
            };
            if r != 0 {
                return r;
            }
        }
        self.eof(out)
    }

    /// .jr_eof: the data ended (or EOI): finish the image
    fn eof(&mut self, out: &mut Option<Surface>) -> i32 {
        if self.tabonly {
            return 0; // tables-only stream: done
        }
        if self.scans == 0 {
            return if self.eoi { ERR_HDR } else { ERR_TRUNC };
        }
        if self.mode == 2 {
            if !self.nostream {
                return self.prog_stream(out);
            }
            self.prog_idct();
        }
        self.output(out)
    }

    /// jd_sof: frame header: dimensions, components; the coefficient buffers of a progressive
    /// image; the sample planes on the plane path (nostream), else only the check that they
    /// would fit (rings or planes follow at the first scan / after the last one)
    fn sof(&mut self, p: &[u8], marker: u32) -> i32 {
        if self.mode != 0 || p.len() < 6 {
            return ERR_HDR;
        }
        if p[0] != 8 {
            return ERR_UNSUPPORTED; // 12-bit precision
        }
        let h = (p[1] as u32) << 8 | p[2] as u32;
        let w = (p[3] as u32) << 8 | p[4] as u32;
        if w == 0 {
            return ERR_HDR;
        }
        if h == 0 {
            return ERR_UNSUPPORTED; // height from DNL
        }
        if w > MAX_DIM || h > MAX_DIM || h as u64 * w as u64 > MAX_PIXELS {
            return ERR_DIMS;
        }
        self.w = w;
        self.h = h;
        let nc = p[5] as u32;
        if nc == 0 {
            return ERR_HDR;
        }
        if nc > 4 || nc == 2 {
            return ERR_UNSUPPORTED;
        }
        if (p.len() as u32) < nc * 3 + 6 {
            return ERR_HDR;
        }
        self.nc = nc;
        let (mut hmax, mut vmax) = (0u32, 0u32);
        for i in 0..nc as usize {
            let c = &mut self.comp[i];
            c.id = p[6 + 3 * i] as u32;
            let hv = p[7 + 3 * i] as u32;
            let (h, v) = (hv >> 4, hv & 15);
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                return ERR_HDR;
            }
            c.h = h;
            c.v = v;
            hmax = hmax.max(h);
            vmax = vmax.max(v);
            let tq = p[8 + 3 * i] as u32;
            if tq > 3 {
                return ERR_HDR;
            }
            c.tq = tq;
        }
        self.hmax = hmax;
        self.vmax = vmax;
        self.mcux = (self.w + hmax * 8 - 1) / (hmax * 8);
        self.mcuy = (self.h + vmax * 8 - 1) / (vmax * 8);
        self.mode = if marker == 0xC2 { 2 } else { 1 };
        // per component geometry and buffers
        for i in 0..nc as usize {
            let c = &mut self.comp[i];
            c.bw = self.mcux * c.h;
            c.stride = c.bw * 8;
            c.bh = self.mcuy * c.v;
            c.dw = (self.w * c.h + hmax - 1) / hmax;
            c.nbw = (c.dw + 7) >> 3;
            c.dh = (self.h * c.v + vmax - 1) / vmax;
            c.nbh = (c.dh + 7) >> 3;
        }
        if self.nostream {
            return self.alloc_planes();
        }
        // the planes come later (sequential: the first scan decides between rings and planes,
        // stream_ok; progressive: rings after the last scan); fail here like the plane
        // allocation would
        let mut probe: [Vec<u8>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (c, p) in self.comp.iter().zip(probe.iter_mut()).take(nc as usize) {
            if p.try_reserve_exact(c.stride as usize * c.bh as usize * 8 + 64).is_err() {
                return ERR_NOMEM;
            }
        }
        if self.mode == 2 {
            for c in self.comp.iter_mut().take(nc as usize) {
                let Some(cf) = try_vec(c.bw as usize * c.bh as usize * 64, 0i16) else {
                    return ERR_NOMEM;
                };
                c.coef = cf;
            }
        }
        0
    }

    /// the value of samples no block reaches: grey for a sequential image, 0 for a
    /// progressive one (whose planes start cleared)
    fn grey(&self) -> u8 {
        if self.mode == 2 { 0 } else { 128 }
    }

    /// the sample planes (+ the coefficient buffers of a progressive image) of every
    /// component -> 0 / ERR_NOMEM
    fn alloc_planes(&mut self) -> i32 {
        let prog = self.mode == 2;
        for c in self.comp.iter_mut().take(self.nc as usize) {
            // plane: stride * BH * 8 + slack for the upsamplers' look-ahead
            let n = c.stride as usize * c.bh as usize * 8 + 64;
            // sequential: start grey
            let Some(pl) = try_vec(n, if prog { 0u8 } else { 128u8 }) else {
                return ERR_NOMEM;
            };
            c.plane = pl;
            if prog {
                let Some(cf) = try_vec(c.bw as usize * c.bh as usize * 64, 0i16) else {
                    return ERR_NOMEM;
                };
                c.coef = cf;
            }
        }
        self.planes = true;
        0
    }

    /// the first scan (sequential) can be streamed: it holds every component exactly once
    fn stream_ok(&self) -> bool {
        if self.nostream || self.mode != 1 || self.ns != self.nc {
            return false;
        }
        let mut seen = 0u32;
        for &ci in &self.scomp[..self.ns as usize] {
            seen |= 1 << ci;
        }
        seen == (1 << self.nc) - 1
    }

    /// start a streamed decode of a sequential image (first scan): the rings, the output
    /// stage -> 0 / RESTART when the memory is not there (the plane path then decides)
    fn stream_begin(&mut self) -> i32 {
        match self.ring_begin(self.ns == 1) {
            Ok(st) => {
                self.stream = Some(st);
                self.planes = true;
                0
            }
            Err(_) => RESTART,
        }
    }

    /// a ring of three MCU rows per component (single: block rows of a single-component
    /// image) holding grey(), and the output stage -> Stream / ERR_NOMEM
    fn ring_begin(&mut self, single: bool) -> Result<Stream, i32> {
        let fill = self.grey();
        for c in self.comp.iter_mut().take(self.nc as usize) {
            c.ring = if single { 8 } else { c.v * 8 };
            let Some(pl) = try_vec(c.stride as usize * c.ring as usize * 3 + 64, fill) else {
                return Err(ERR_NOMEM);
            };
            c.plane = pl;
        }
        let out = self.out_begin()?;
        let (bands, bho) = if single { (self.comp[0].nbh, 8) } else { (self.mcuy, self.vmax * 8) };
        Ok(Stream {
            out,
            bands,
            bho,
            emitted: 0,
            prep: bands.min(3),
            snap: (self.orient, self.jfif, self.adobe, self.xform),
        })
    }

    /// streaming: the scan starts MCU row my (block row of a single-component scan)
    fn stream_row_begin(&mut self, my: u32) {
        for c in self.comp.iter_mut().take(self.nc as usize) {
            c.first = my * c.ring;
            c.base = my % 3 * c.ring;
        }
    }

    /// streaming: MCU row my is complete: convert the one before it, make the slot of the
    /// next one grey
    fn stream_row_end(&mut self, my: u32) {
        self.flush_pend();
        let Some(mut st) = self.stream.take() else { return };
        if my >= 1 {
            self.stream_emit(&mut st, my - 1);
        }
        if my + 1 < st.bands && my + 1 >= st.prep {
            self.stream_prep(&mut st, my + 1);
        }
        self.stream = Some(st);
    }

    /// streaming: the ring slot of MCU row j starts grey
    fn stream_prep(&mut self, st: &mut Stream, j: u32) {
        let fill = self.grey();
        for c in self.comp.iter_mut().take(self.nc as usize) {
            let n = c.stride as usize * c.ring as usize;
            let s = (j % 3) as usize * n;
            c.plane[s..s + n].fill(fill);
        }
        st.prep = j + 1;
    }

    /// streaming: convert the output rows of MCU row j (they read the sample rows of MCU rows
    /// j - 1 .. j + 1).  The upsamplers read up to one byte past the end of a row, which in a
    /// plane is the first byte of the next row: behind the last slot of the ring the slack
    /// stands in for that row -- the first row of slot 0 when it holds the MCU row after the
    /// one in slot 2, else grey like the slack after the last plane row.
    fn stream_emit(&mut self, st: &mut Stream, j: u32) {
        let b = if j % 3 == 0 { j.wrapping_sub(1) } else if j % 3 == 2 { j } else { j + 1 }; // in slot 2
        let next = b <= j && b.wrapping_add(1) < st.bands;
        let fill = self.grey();
        for c in self.comp.iter_mut().take(self.nc as usize) {
            let t = c.stride as usize * c.ring as usize * 3;
            if next {
                c.plane.copy_within(0..16, t);
            } else {
                c.plane[t..t + 16].fill(fill);
            }
        }
        let y1 = ((j + 1) * st.bho).min(self.h);
        for y in j * st.bho..y1 {
            self.out_row(&mut st.out, y);
        }
        st.emitted = j + 1;
    }

    /// jd_dht: define Huffman tables
    fn dht(&mut self, mut p: &[u8]) -> i32 {
        while !p.is_empty() {
            if p.len() < 17 {
                return ERR_HDR;
            }
            let class = (p[0] >> 4) as usize;
            let id = (p[0] & 15) as usize;
            if class > 1 || id > 3 {
                return ERR_HDR;
            }
            let n: usize = p[1..17].iter().map(|&c| c as usize).sum();
            if n > 256 || p.len() < n + 17 {
                return ERR_HDR;
            }
            self.ht[id + class * 4].build(&p[1..17], &p[17..17 + n], class == 0);
            p = &p[n + 17..];
        }
        0
    }

    /// jd_std_table: the Annex K table for an undefined slot (0 DC luminance, 1-3 DC
    /// chrominance, 4 AC luminance, 5-7 AC chrominance)
    fn std_table(&mut self, slot: usize) {
        let t = &mut self.ht[slot];
        match slot {
            0 => t.build(&dsp::DC_L_BITS, &dsp::DC_L_VALS, true),
            1..=3 => t.build(&dsp::DC_C_BITS, &dsp::DC_C_VALS, true),
            4 => t.build(&dsp::AC_L_BITS, &dsp::AC_L_VALS, false),
            _ => t.build(&dsp::AC_C_BITS, &dsp::AC_C_VALS, false),
        }
    }

    /// jd_dqt: define quantisation tables
    fn dqt(&mut self, mut p: &[u8]) -> i32 {
        while !p.is_empty() {
            let prec = (p[0] >> 4) as usize; // 0: 8-bit, 1: 16-bit
            let id = (p[0] & 15) as usize;
            if prec > 1 || id > 3 {
                return ERR_HDR;
            }
            let need = ((prec + 1) << 6) + 1;
            if p.len() < need {
                return ERR_HDR;
            }
            self.qdef[id] = true;
            for k in 0..64 {
                let v = if prec == 0 { p[k + 1] as u16 } else { (p[2 * k + 1] as u16) << 8 | p[2 * k + 2] as u16 };
                self.qt[id][dsp::ZZ[k] as usize] = v as i16;
            }
            p = &p[need..];
        }
        0
    }

    /// jd_exif: APP1: take the orientation from the first EXIF block
    fn exif_orient(&mut self, p: &[u8]) {
        if self.exif || p.len() < 14 || &p[..6] != b"Exif\0\0" {
            return;
        }
        self.exif = true;
        let t = &p[6..]; // TIFF header
        let n = t.len() as u32;
        let be = match (t[0], t[1]) {
            (0x49, 0x49) => false,
            (0x4D, 0x4D) => true,
            _ => return,
        };
        let rd16 = |o: u32| {
            let (a, b) = (t[o as usize], t[o as usize + 1]);
            if be { (a as u32) << 8 | b as u32 } else { (b as u32) << 8 | a as u32 }
        };
        let mut ifd = u32::from_le_bytes([t[4], t[5], t[6], t[7]]);
        if be {
            ifd = ifd.swap_bytes();
        }
        if ifd > n - 2 {
            return;
        }
        let mut cnt = rd16(ifd);
        let mut e = ifd + 2;
        while cnt != 0 {
            if e + 12 > n {
                return;
            }
            if rd16(e) == 0x0112 {
                if rd16(e + 2) != 3 {
                    return; // SHORT
                }
                let v = rd16(e + 8);
                if (1..=8).contains(&v) {
                    self.orient = v;
                }
                return;
            }
            e += 12;
            cnt -= 1;
        }
    }

    /// jd_sos: scan header: components, tables, spectral selection; latches the
    /// quantisation tables; picks the block routine
    fn sos(&mut self, p: &[u8]) -> i32 {
        if self.mode == 0 || p.is_empty() {
            return ERR_HDR;
        }
        let ns = p[0] as usize;
        if !(1..=4).contains(&ns) || p.len() < ns * 2 + 4 {
            return ERR_HDR;
        }
        self.ns = ns as u32;
        let tail = &p[ns * 2..];
        self.ss = tail[1] as u32;
        self.se = tail[2] as u32;
        self.ah = (tail[3] >> 4) as u32;
        self.al = (tail[3] & 15) as u32;
        if self.mode == 2 {
            if self.ss == 0 {
                if self.se != 0 {
                    return ERR_HDR;
                }
            } else if self.se < self.ss || self.se > 63 || ns != 1 {
                return ERR_HDR;
            }
            if self.ah > 13 || self.al > 13 {
                return ERR_HDR;
            }
        } else {
            // sequential: ignore Ss / Se / Ah / Al
            self.ss = 0;
            self.ah = 0;
            self.al = 0;
            self.se = 63;
        }
        let mut blocks = 0u32;
        for i in 0..ns {
            let id = p[1 + 2 * i] as u32;
            let Some(ci) = (0..self.nc as usize).find(|&c| self.comp[c].id == id) else {
                return ERR_HDR;
            };
            self.scomp[i] = ci;
            blocks += self.comp[ci].h * self.comp[ci].v;
            let td = (p[2 + 2 * i] >> 4) as usize;
            let ta = (p[2 + 2 * i] & 15) as usize;
            if td > 3 || ta > 3 {
                return ERR_HDR;
            }
            self.comp[ci].dct = td;
            self.comp[ci].act = ta + 4;
            // tables this scan needs
            let (need_dc, need_ac) = if self.mode != 2 {
                (true, true)
            } else if self.ss != 0 {
                (false, true)
            } else {
                (self.ah == 0, false) // DC refinement: no table
            };
            if need_dc {
                let r = self.need_table(td);
                if r != 0 {
                    return r;
                }
            }
            if need_ac {
                let r = self.need_table(ta + 4);
                if r != 0 {
                    return r;
                }
            }
            let c = &mut self.comp[ci];
            if !c.qok {
                if !self.qdef[c.tq as usize] {
                    return ERR_HDR; // quantisation table never defined
                }
                c.q = self.qt[c.tq as usize];
                c.qok = true;
            }
        }
        if ns != 1 && blocks > 10 {
            return ERR_HDR;
        }
        self.blkfn = if self.mode != 2 {
            BF_SEQ
        } else if self.ss == 0 {
            if self.ah == 0 { BF_DCF } else { BF_DCR }
        } else if self.ah == 0 {
            BF_ACF
        } else {
            BF_ACR
        };
        self.scans += 1;
        0
    }

    /// .ss_table: make sure table slot is usable -> 0 / ERR_CODE
    fn need_table(&mut self, slot: usize) -> i32 {
        match self.ht[slot].stat {
            1 => 0,
            2 => ERR_CODE,
            _ => {
                self.std_table(slot);
                0
            }
        }
    }

    // ============================================================== entropy decoding

    /// .sc_reset: DC predictions and the EOB run
    fn reset_pred(&mut self) {
        self.eobrun = 0;
        for c in self.comp.iter_mut() {
            c.pred = 0;
        }
    }

    /// jd_scan: decode the entropy-coded segment of the current scan starting at pos; the
    /// reader is left at the marker that follows -> 0 / ERR_CODE
    fn scan(&mut self, d: &'a [u8], pos: usize) -> i32 {
        let mut br = Br { d, pos, marker: 0, zbits: 0, err: 0 };
        let r = self.scan_mcus(&mut br);
        self.flush_pend();
        self.br = br;
        r
    }

    /// the IDCT of the waiting block, alone
    fn flush_pend(&mut self) {
        if let Some((ci, off)) = self.pend.take() {
            let c = &mut self.comp[ci];
            dsp::idct(&self.pblk, &c.q, &mut c.plane[off..], c.stride as usize);
        }
    }

    /// the end of jd_blk_seq: the IDCT of a decoded block into the plane at off (paired with
    /// the waiting block), or the DC fill of a block without AC coefficients
    #[inline(always)]
    fn put_seq(&mut self, ci: usize, off: usize, blk: &[i16; 64]) {
        // an AC coefficient was stored <=> one is non-zero (stored values never are)
        if dsp::ac_zero(blk) {
            if self.pend == Some((ci, off)) {
                self.flush_pend();
            }
            let c = &mut self.comp[ci];
            dsp::dc_fill(&mut c.plane[off..], c.stride as usize, blk[0] as i32 * c.q[0] as u16 as i32);
            return;
        }
        if !self.pair {
            let c = &mut self.comp[ci];
            dsp::idct(blk, &c.q, &mut c.plane[off..], c.stride as usize);
            return;
        }
        match self.pend.take() {
            Some((pci, poff)) => {
                let mut out = [0u8; 128];
                dsp::idct2(&self.pblk, &self.comp[pci].q, blk, &self.comp[ci].q, &mut out);
                let pc = &mut self.comp[pci];
                dsp::put_block(&mut pc.plane[poff..], pc.stride as usize, &out[..64]);
                let c = &mut self.comp[ci];
                dsp::put_block(&mut c.plane[off..], c.stride as usize, &out[64..]);
            }
            None => {
                self.pblk = *blk;
                self.pend = Some((ci, off));
            }
        }
    }

    /// the MCUs of jd_scan with the reader br
    #[inline(always)]
    fn scan_mcus(&mut self, br: &mut Br<'a>) -> i32 {
        let mut bb = Bb { buf: 0, bits: 0 };
        self.insuf = false;
        self.rstnum = 0;
        self.reset_pred();
        self.rleft = self.ri;
        if self.ns == 1 {
            // single component: one block per MCU, ceil(DW/8) x ceil(DH/8) blocks
            let ci = self.scomp[0];
            let (nbw, nbh) = (self.comp[ci].nbw, self.comp[ci].nbh);
            let stream = self.stream.is_some();
            for my in 0..nbh {
                if stream {
                    self.stream_row_begin(my);
                }
                for mx in 0..nbw {
                    bb = self.restart(br, bb);
                    if !self.insuf {
                        self.block(ci, mx, my, br, &mut bb);
                        let r = self.check(br, bb);
                        if r != 0 {
                            return r;
                        }
                    }
                    if self.insuf && self.ri == 0 {
                        return 0; // nothing more to decode
                    }
                }
                if stream {
                    self.stream_row_end(my);
                }
            }
        } else {
            // interleaved: MCUX x MCUY MCUs of h x v blocks per component
            let stream = self.stream.is_some();
            for my in 0..self.mcuy {
                if stream {
                    self.stream_row_begin(my);
                }
                for mx in 0..self.mcux {
                    bb = self.restart(br, bb);
                    if !self.insuf {
                        for k in 0..self.ns as usize {
                            let ci = self.scomp[k];
                            let (h, v) = (self.comp[ci].h, self.comp[ci].v);
                            for by in 0..v {
                                for bx in 0..h {
                                    self.block(ci, mx * h + bx, my * v + by, br, &mut bb);
                                }
                            }
                        }
                        let r = self.check(br, bb);
                        if r != 0 {
                            return r;
                        }
                    }
                    if self.insuf && self.ri == 0 {
                        return 0;
                    }
                }
                if stream {
                    self.stream_row_end(my);
                }
            }
        }
        0
    }

    /// .sc_check: after an MCU -> error; flags running out of data
    #[inline(always)]
    fn check(&mut self, br: &Br, bb: Bb) -> i32 {
        if br.err != 0 {
            return br.err;
        }
        if br.zbits > bb.bits {
            self.insuf = true; // zero padding was consumed
        }
        0
    }

    /// .sc_restart: restart interval handling before each MCU (libjpeg read_restart_marker +
    /// jpeg_resync_to_restart: the expected RSTn is consumed; one of the next two expected
    /// RSTs, another marker or the end of the data is left alone (the interval then decodes
    /// from zero bits, i.e. stays grey); an earlier RST or an invalid marker is skipped;
    /// others are consumed.  The out-of-data flag is cleared unless we are left in front of a
    /// marker.)
    #[inline(always)]
    fn restart(&mut self, br: &mut Br<'a>, bb: Bb) -> Bb {
        if self.ri == 0 {
            return bb;
        }
        let (nbr, bb) = self.restart_slow(*br, bb);
        *br = nbr;
        bb
    }

    /// restart with a restart interval (the reader by value)
    #[inline(never)]
    fn restart_slow(&mut self, mut br: Br<'a>, mut bb: Bb) -> (Br<'a>, Bb) {
        if self.rleft == 0 {
            bb = Bb { buf: 0, bits: 0 }; // drop the remaining bits
            br.zbits = 0;
            if br.marker == 0 {
                br.find_marker();
            }
            loop {
                let m = br.marker;
                let e = self.rstnum;
                let take = if m == 0xD0 + e {
                    true // the expected RSTn
                } else if m == M_EOF {
                    break; // end of data: like libjpeg's fake EOI
                } else if m < 0xC0 {
                    false // invalid marker: go to the next one
                } else {
                    let dm = m.wrapping_sub(0xD0);
                    if dm > 7 {
                        break; // another valid marker: leave it
                    }
                    let dd = dm.wrapping_sub(e) & 7;
                    if dd == 1 || dd == 2 {
                        break; // one of the next two: empty interval
                    }
                    dd < 6 // an earlier restart (6, 7): advance; others: consume
                };
                br.pos += 2;
                br.marker = 0;
                if take {
                    break;
                }
                br.find_marker();
            }
            self.rstnum = (self.rstnum + 1) & 7;
            if br.marker == 0 {
                self.insuf = false;
            }
            self.reset_pred();
            self.rleft = self.ri;
        }
        self.rleft = self.rleft.wrapping_sub(1);
        (br, bb)
    }

    /// one block of the scan at block (bx, by) of component ci
    #[inline(always)]
    fn block(&mut self, ci: usize, bx: u32, by: u32, br: &mut Br, bb: &mut Bb) {
        if self.mode != 2 {
            let c = &mut self.comp[ci];
            // (streaming: the row in the ring slot of the current MCU row; else base = first = 0)
            let off = c.stride as usize * (c.base + by * 8 - c.first) as usize + bx as usize * 8;
            let mut blk = [0i16; 64];
            blk_seq(br, bb, c, &self.ht[c.dct], &self.ht[c.act], &mut blk);
            self.put_seq(ci, off, &blk);
            return;
        }
        let c = &mut self.comp[ci];
        let off = (c.bw as usize * by as usize + bx as usize) * 64;
        match self.blkfn {
            BF_DCF => {
                bb.need32(br);
                let s = bb.huff(br, &self.ht[c.dct]);
                let v = if s != 0 { bb.extend(s) } else { 0 };
                c.pred = c.pred.wrapping_add(v);
                c.coef[off] = (c.pred << self.al) as i16;
            }
            BF_DCR => {
                if bb.bits == 0 {
                    *bb = br.fill(*bb);
                }
                if bb.bit() {
                    c.coef[off] |= (1i32 << self.al) as i16;
                }
            }
            BF_ACF => {
                let blk: &mut [i16; 64] = (&mut c.coef[off..off + 64]).try_into().unwrap();
                self.eobrun = blk_acf(br, bb, &self.ht[c.act], blk, self.ss, self.se, self.al, self.eobrun);
            }
            _ => {
                let blk: &mut [i16; 64] = (&mut c.coef[off..off + 64]).try_into().unwrap();
                self.eobrun = blk_acr(br, bb, &self.ht[c.act], blk, self.ss, self.se, self.al, self.eobrun);
            }
        }
    }

    // ============================================================== output

    /// jd_prog_idct: progressive: inverse DCT of every coded block into the planes
    fn prog_idct(&mut self) {
        for ci in 0..self.nc as usize {
            let nbh = self.comp[ci].nbh;
            self.prog_blocks(ci, 0, nbh);
        }
    }

    /// the inverse DCT of block rows by0..by1 of component ci (plane rows through base /
    /// first, see Comp)
    fn prog_blocks(&mut self, ci: usize, by0: u32, by1: u32) {
        let pair = self.pair;
        let c = &mut self.comp[ci];
        let stride = c.stride as usize;
        let mut pend: Option<(usize, usize)> = None; // coefficients, plane offset of a waiting block
        for by in by0..by1 {
            let row = (c.base + by * 8 - c.first) as usize;
            for bx in 0..c.nbw as usize {
                let o = (c.bw as usize * by as usize + bx) * 64;
                let off = stride * row + bx * 8;
                let blk: &[i16; 64] = (&c.coef[o..o + 64]).try_into().unwrap();
                if dsp::ac_zero(blk) {
                    dsp::dc_fill(&mut c.plane[off..], stride, blk[0] as i32 * c.q[0] as u16 as i32);
                } else if !pair {
                    dsp::idct(blk, &c.q, &mut c.plane[off..], stride);
                } else if let Some((po, poff)) = pend.take() {
                    let pb: &[i16; 64] = (&c.coef[po..po + 64]).try_into().unwrap();
                    let mut out = [0u8; 128];
                    dsp::idct2(pb, &c.q, blk, &c.q, &mut out);
                    dsp::put_block(&mut c.plane[poff..], stride, &out[..64]);
                    dsp::put_block(&mut c.plane[off..], stride, &out[64..]);
                } else {
                    pend = Some((o, off));
                }
            }
        }
        if let Some((po, poff)) = pend {
            let pb: &[i16; 64] = (&c.coef[po..po + 64]).try_into().unwrap();
            dsp::idct(pb, &c.q, &mut c.plane[poff..], stride);
        }
    }

    /// a progressive image after its last scan: inverse DCT and output MCU row by MCU row
    /// through the ring (block rows of a single-component image), the output one MCU row
    /// behind -- the bytes of prog_idct + output on whole planes
    fn prog_stream(&mut self, out: &mut Option<Surface>) -> i32 {
        let single = self.nc == 1;
        let mut st = match self.ring_begin(single) {
            Ok(st) => st,
            Err(e) => return e,
        };
        for j in 0..st.bands {
            for ci in 0..self.nc as usize {
                let c = &mut self.comp[ci];
                c.first = j * c.ring;
                c.base = j % 3 * c.ring;
                if j >= 3 {
                    // the slot held MCU row j - 3: cleared like a plane
                    let n = c.stride as usize * c.ring as usize;
                    let s = (j % 3) as usize * n;
                    c.plane[s..s + n].fill(0);
                }
                let v = if single { 1 } else { c.v };
                let (by0, by1) = ((j * v).min(c.nbh), ((j + 1) * v).min(c.nbh));
                self.prog_blocks(ci, by0, by1);
            }
            if j >= 1 {
                self.stream_emit(&mut st, j - 1);
            }
        }
        let last = st.bands - 1;
        self.stream_emit(&mut st, last);
        *out = Some(st.out.sf);
        0
    }

    /// jd_cspace: the colour space like libjpeg's default_decompress_parms
    fn cspace(&mut self) {
        let nc = self.nc;
        let (mut cs, mut cm) = (CS_GRAY, 0);
        if nc == 4 {
            cs = CS_CMYK;
            cm = 1; // plain CMYK
            if self.adobe {
                cm = 0; // Adobe: inverted
                if self.xform != 0 {
                    cs = CS_YCCK;
                    cm = 2;
                }
            }
        } else if nc != 1 {
            cs = CS_YCC;
            if !self.jfif {
                if self.adobe {
                    if self.xform == 0 {
                        cs = CS_RGB;
                    }
                } else if self.comp[0].id == 82 && self.comp[1].id == 71 && self.comp[2].id == 66 {
                    cs = CS_RGB; // 'R' 'G' 'B'
                }
            }
        }
        // forced by the caller (JDF_RGB / JDF_YCC for 3, JDF_CMYK for 4 components)
        if nc == 3 {
            if self.flags & JDF_RGB != 0 {
                cs = CS_RGB;
            }
            if self.flags & JDF_YCC != 0 {
                cs = CS_YCC;
            }
        } else if nc == 4 && self.flags & JDF_CMYK != 0 {
            cs = CS_CMYK;
            cm = 1;
        }
        self.cspace = cs;
        self.cmode = cm;
    }

    /// jd_comp_row: samples of component ci for output row y at full resolution: Some(plane
    /// offset), or None = the upsampled row is in tmp
    fn comp_row(&self, ci: usize, y: u32, tmp: &mut [u8]) -> Option<usize> {
        let c = &self.comp[ci];
        match c.up {
            UP_FULL => Some(c.row_off(y)),
            UP_H2V1 => {
                dsp::up_h2v1(tmp, &c.plane[c.row_off(y)..], c.dw as usize);
                None
            }
            UP_ANY => {
                // replicate: source row y * v / vmax, column x * h / hmax
                let sy = y * c.v / self.vmax;
                let src = &c.plane[c.row_off(sy)..];
                let (mut acc, mut sx) = (0u32, 0usize);
                for t in tmp[..self.w as usize].iter_mut() {
                    *t = src[sx];
                    acc += c.h;
                    while acc >= self.hmax {
                        acc -= self.hmax;
                        sx += 1;
                    }
                }
                None
            }
            _ => {
                // H2V2 / H1V2: nearer row y / 2, next nearer row above (even y) or below (odd y)
                let this = y >> 1;
                let nbr = if y & 1 != 0 {
                    if this + 1 < c.dh { this + 1 } else { this }
                } else {
                    this.saturating_sub(1)
                };
                let a = &c.plane[c.row_off(this)..];
                let b = &c.plane[c.row_off(nbr)..];
                if c.up == UP_H2V2 {
                    dsp::up_h2v2(tmp, a, b, c.dw as usize);
                } else {
                    dsp::up_h1v2(tmp, a, b, c.dw as usize, ((y & 1) + 1) as u16);
                }
                None
            }
        }
    }

    /// jd_output: upsample, colour convert and orient the planes into the output surface
    /// (out_begin, then out_row for every row).  A streamed decode has converted its rows
    /// during the scan already: the rows after a scan that stopped early follow from grey
    /// ring slots.
    fn output(&mut self, out: &mut Option<Surface>) -> i32 {
        if let Some(mut st) = self.stream.take() {
            if st.snap != (self.orient, self.jfif, self.adobe, self.xform) {
                return RESTART; // a later APPn changed the colour space or the orientation
            }
            for j in st.emitted..st.bands {
                if j + 1 < st.bands && j + 1 >= st.prep {
                    self.stream_prep(&mut st, j + 1);
                }
                self.stream_emit(&mut st, j);
            }
            *out = Some(st.out.sf);
            return 0;
        }
        let mut st = match self.out_begin() {
            Ok(st) => st,
            Err(e) => return e,
        };
        for y in 0..self.h {
            self.out_row(&mut st, y);
        }
        *out = Some(st.sf);
        0
    }

    /// the output stage: colour space, upsampling modes, temp rows, the surface (width and
    /// height swapped for orientations 5..8) -> Out / ERR_NOMEM
    fn out_begin(&mut self) -> Result<Out, i32> {
        self.cspace();
        let o = if !(1..=8).contains(&self.orient) || self.flags & JDF_NO_ORIENT != 0 { 1 } else { self.orient };
        let (w, h) = (self.w, self.h);
        let nc = self.nc as usize;
        // temp rows: one upsampling row per component + BAND converted rows
        let rowsz = ((w + 127) & !63) as usize; // >= W + 64
        let bstride = ((w as usize * 4) + 127) & !63; // >= (W + 16) * 4
        let mut tmps: [Vec<u8>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for t in tmps.iter_mut().take(nc) {
            let Some(v) = try_vec(rowsz, 0u8) else { return Err(ERR_NOMEM) };
            *t = v;
        }
        let Some(band) = try_vec(bstride << BAND_LOG, 0u8) else { return Err(ERR_NOMEM) };
        // per component: upsampling mode
        for ci in 0..nc {
            let (hm, vm) = (self.hmax, self.vmax);
            let c = &mut self.comp[ci];
            c.up = if c.h == hm {
                if c.v == vm {
                    UP_FULL
                } else if 2 * c.v == vm {
                    UP_H1V2
                } else {
                    UP_ANY
                }
            } else if 2 * c.h == hm && c.dw > 2 {
                // libjpeg: fancy only for widths > 2
                if c.v == vm {
                    UP_H2V1
                } else if 2 * c.v == vm {
                    UP_H2V2
                } else {
                    UP_ANY
                }
            } else {
                UP_ANY
            };
        }
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        // (try_surface: fresh zero pages, no clearing pass; rows are converted in place)
        let Some(mut sf) = try_surface(ow, oh, FMT_PARGB) else {
            return Err(ERR_NOMEM);
        };
        sf.flags |= SFF_OPAQUE;
        if self.flags & JDF_STRAIGHT != 0 {
            sf.fmt = FMT_ARGB; // opaque: the same bytes
        }
        Ok(Out { sf, tmps, band, bstride, o })
    }

    /// one output row y (rows in order 0..H): mirrored rows (orientations 2, 3) are copied
    /// backwards; for the transposing orientations 5..8 BAND rows are converted first and then
    /// written out as runs of BAND pixels per destination row
    fn out_row(&self, st: &mut Out, y: u32) {
        let (w, h, nc, o, bstride) = (self.w, self.h, self.nc as usize, st.o, st.bstride);
        let ss = st.sf.stride as isize;
        let wn = w as usize;
        let n8 = (wn + 7) & !7;
        // sample rows of every component
        let mut offs = [None; 4];
        for (ci, t) in st.tmps.iter_mut().enumerate().take(nc) {
            offs[ci] = self.comp_row(ci, y, t);
        }
        let mut rows: [&[u8]; 4] = [&[]; 4];
        for ci in 0..nc {
            rows[ci] = match offs[ci] {
                Some(off) => &self.comp[ci].plane[off..],
                None => &st.tmps[ci][..],
            };
        }
        // destination of pixel 0 of this row and the step between pixels
        let yy = y as isize;
        let hy = (h - 1 - y) as isize;
        let w1 = (w - 1) as isize;
        let (base, dx): (isize, isize) = match o {
            2 => (ss * yy + w1 * 4, -4),
            3 => (ss * hy + w1 * 4, -4),
            4 => (ss * hy, 4),
            5 => (yy * 4, ss),
            6 => (hy * 4, ss),
            7 => (ss * w1 + hy * 4, -ss),
            8 => (ss * w1 + yy * 4, -ss),
            _ => (ss * yy, 4),
        };
        let brow = ((y & (BAND - 1)) as usize) * bstride;
        let (sf, band) = (&mut st.sf, &mut st.band);
        {
            let dst: &mut [u8] = if dx == 4 {
                &mut sf.px[base as usize..base as usize + n8 * 4]
            } else if o < 5 {
                &mut band[..n8 * 4]
            } else {
                &mut band[brow..brow + n8 * 4]
            };
            match self.cspace {
                CS_GRAY => dsp::gray_rgb(dst, rows[0], wn),
                CS_RGB => dsp::rgb_rgb(dst, rows[0], rows[1], rows[2], wn),
                CS_CMYK => dsp::cmyk_rgb(dst, rows[0], rows[1], rows[2], rows[3], wn, self.cmode),
                _ => {
                    // YCC and YCCK
                    dsp::ycc_rgb(dst, rows[0], rows[1], rows[2], wn);
                    if self.cspace == CS_YCCK {
                        dsp::cmyk_rgb(dst, rows[0], rows[1], rows[2], rows[3], wn, self.cmode);
                    }
                }
            }
        }
        if dx == 4 {
            return;
        }
        if o < 5 {
            // mirrored row: copy backwards
            let mut d = base;
            for i in 0..wn {
                let du = d as usize;
                sf.px[du..du + 4].copy_from_slice(&band[i * 4..i * 4 + 4]);
                d += dx;
            }
            return;
        }
        // transposed: flush after the last row of a band (or of the image)
        let rows_in = (y & (BAND - 1)) + 1;
        if y + 1 != h && rows_in != BAND {
            return;
        }
        // dy: the step between the destinations of consecutive source rows
        let dy: isize = if o == 6 || o == 7 { -4 } else { 4 };
        let mut d0 = base - (rows_in as isize - 1) * dy; // pixel 0 of the band's first row
        for x in 0..wn {
            let mut d = d0;
            let mut s = x * 4;
            for _ in 0..rows_in {
                let du = d as usize;
                sf.px[du..du + 4].copy_from_slice(&band[s..s + 4]);
                s += bstride;
                d += dy;
            }
            d0 += dx;
        }
    }
}

/// jd_blk_seq: sequential: decode a block, inverse DCT it into the plane at off
#[inline(always)]
fn blk_seq(br: &mut Br, bb: &mut Bb, c: &mut Comp, dct: &Huff, act: &Huff, blk: &mut [i16; 64]) {
    bb.need32(br);
    let s = bb.huff(br, dct);
    let v = if s != 0 { bb.extend(s) } else { 0 };
    c.pred = c.pred.wrapping_add(v);
    blk[0] = c.pred as i16;
    let mut k = 1usize;
    while k <= 63 {
        bb.need32(br);
        let e = act.fac[(bb.buf >> 55) as usize] as i32;
        if e != 0 {
            let n = (e & 15) as u32;
            bb.buf <<= n;
            bb.bits = bb.bits.wrapping_sub(n);
            k += ((e >> 4) & 15) as usize;
            blk[(dsp::ZZ[k] & 63) as usize] = (e >> 8) as i16;
            k += 1;
            continue;
        }
        let rs = bb.huff(br, act);
        let r = rs >> 4;
        let s = rs & 15;
        if s != 0 {
            k += r as usize;
            let v = bb.extend(s);
            blk[(dsp::ZZ[k] & 63) as usize] = v as i16;
            k += 1;
            continue;
        }
        if r != 15 {
            break; // EOB
        }
        k += 16; // ZRL
    }
}

/// jd_blk_acf: progressive AC first scan (spectral band Ss..Se, EOB runs) -> new EOB run
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn blk_acf(br: &mut Br, bb: &mut Bb, act: &Huff, blk: &mut [i16; 64], ss: u32, se: u32, al: u32, eobrun: u32) -> u32 {
    if eobrun != 0 {
        return eobrun - 1;
    }
    let mut k = ss;
    while k <= se {
        bb.need32(br);
        let rs = bb.huff(br, act);
        let r = rs >> 4;
        let s = rs & 15;
        if s != 0 {
            k += r;
            let v = bb.extend(s);
            blk[dsp::ZZ[k as usize] as usize] = (v << al) as i16;
            k += 1;
            continue;
        }
        if r == 15 {
            k += 16;
            continue;
        }
        let mut e = 1u32 << r;
        if r != 0 {
            e += bb.bits_n(r);
        }
        return e - 1;
    }
    0
}

/// one correction bit for a non-zero coefficient of a refinement scan
#[inline(always)]
fn refine(br: &mut Br, bb: &mut Bb, blk: &mut [i16; 64], k: u32, p1: i32) {
    let pos = dsp::ZZ[k as usize] as usize;
    let c = blk[pos] as i32;
    if c == 0 {
        return;
    }
    if bb.bits == 0 {
        *bb = br.fill(*bb);
    }
    if bb.bit() && c & p1 == 0 {
        blk[pos] = c.wrapping_add(if c >= 0 { p1 } else { -p1 }) as i16;
    }
}

/// jd_blk_acr: progressive AC refinement (libjpeg decode_mcu_AC_refine) -> new EOB run
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn blk_acr(br: &mut Br, bb: &mut Bb, act: &Huff, blk: &mut [i16; 64], ss: u32, se: u32, al: u32, mut eobrun: u32) -> u32 {
    let mut k = ss;
    let p1 = 1i32 << al;
    if eobrun == 0 {
        loop {
            if k > se {
                return eobrun;
            }
            bb.need32(br);
            let rs = bb.huff(br, act);
            let r = rs >> 4;
            let s = rs & 15;
            let t0: i32;
            if s != 0 {
                t0 = if bb.bit() { p1 } else { -p1 }; // new coefficient
            } else if r == 15 {
                t0 = 0; // ZRL
            } else {
                let mut e = 1u32 << r;
                if r != 0 {
                    e += bb.bits_n(r);
                }
                eobrun = e;
                break;
            }
            let mut left = r as i32; // zero coefficients still to skip
            loop {
                let pos = dsp::ZZ[k as usize] as usize;
                if blk[pos] != 0 {
                    refine(br, bb, blk, k, p1);
                } else {
                    left -= 1;
                    if left < 0 {
                        break;
                    }
                }
                k += 1;
                if k > se {
                    break;
                }
            }
            if t0 != 0 {
                blk[dsp::ZZ[k as usize] as usize] = t0 as i16;
            }
            k += 1;
        }
    }
    while k <= se {
        refine(br, bb, blk, k, p1);
        k += 1;
    }
    eobrun.wrapping_sub(1)
}

/// jpeg_decode_fl
pub fn decode_fl(data: &[u8], flags: u32) -> Result<Surface, i32> {
    match decode_run(&[], data, flags, false) {
        Err(RESTART) => decode_run(&[], data, flags, true),
        r => r,
    }
}

/// jpeg_decode_abbrev: an abbreviated image stream (JPEG-in-TIFF strip / tile) with the
/// tables of a tables-only stream (TIFF JPEGTables; empty = none); the orientation is never
/// applied
pub fn decode_abbrev(tables: &[u8], data: &[u8], flags: u32) -> Result<Surface, i32> {
    let flags = flags | JDF_NO_ORIENT;
    match decode_run(tables, data, flags, false) {
        Err(RESTART) => decode_run(tables, data, flags, true),
        r => r,
    }
}

/// decode_fl / decode_abbrev (tables-only stream first when not empty); nostream: the plane
/// path only -> Err(RESTART) when a streamed decode has to be done again on the plane path
fn decode_run(tables: &[u8], data: &[u8], flags: u32, nostream: bool) -> Result<Surface, i32> {
    let mut dec = Dec::new(flags);
    dec.nostream = nostream;
    let mut out = None;
    if !tables.is_empty() {
        dec.tabonly = true;
        let r = dec.run(tables, &mut out);
        if r != 0 {
            return Err(r);
        }
        dec.tabonly = false;
        dec.eoi = false;
        dec.jfif = false;
        dec.adobe = false;
        dec.xform = 0;
        dec.ri = 0;
    }
    let r = dec.run(data, &mut out);
    match out {
        Some(sf) if r == 0 => Ok(sf),
        _ => Err(if r == 0 { ERR_HDR } else { r }),
    }
}
