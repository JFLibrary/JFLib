// ccitt -- CCITT bilevel decoding for TIFF (compression 2 / 3 / 4)   [src/img/tiff_ccitt.s]
//
//   decode(src, dst, &prm, &mut work) -> bytes written (rows * rowb) / ERR_*
//
// prm: width, rows, bytes per output row, compression (2 Modified Huffman "CCITT RLE": 1D
// rows, byte aligned, no EOLs; 3 T.4 / Group 3: every row starts with an EOL, T4Options
// bit 0 = 2D rows allowed (a tag bit after the EOL: 1 = 1D, 0 = 2D), fill bits before EOLs;
// 4 T.6 / Group 4: 2D rows relative to the previous row, no EOLs), T4Options / T6Options.
// work: two lines of changing elements (grown on demand, kept by the caller between
// strips); the 13-bit run-length lookup tables are static here.
//
// The input is read MSB first (tiff.rs reverses FillOrder 2 data before).  The output is
// libtiff's convention: bit 1 = black run (a pixel after an odd number of colour changes),
// rows MSB first.  Behaviour follows libtiff's tif_fax3.c: EOL synchronisation (an EOL is
// searched when none was seen, fill bits skipped), a premature EOL ends the row (the rest
// white), a row that runs past the width is clipped, uncompressed mode (2D extension) and
// invalid codes are ERR_CODE, and when the data ends the current row is completed white;
// the remaining rows are white and the strip counts as decoded when at least one row was
// complete (else ERR_TRUNC).  Every code consumes input, the change lists are bounded
// (2 * width + 16 entries), so no input can loop or overflow.

use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_HDR, ERR_TRUNC};

/// the parameter block (CCP_*)
pub struct Prm {
    pub w: u32,    // width in pixels (1..65536)
    pub rows: u32, // rows to decode
    pub rowb: u32, // bytes per output row
    pub comp: u32, // 2 / 3 / 4
    pub opt: u32,  // T4Options / T6Options
}

/// the work buffer: two lines of changing elements (CW_LINES; CCP_WORK / CCP_WCAP)
#[derive(Default)]
pub struct Work {
    lines: Vec<u32>,
    wcap: u32,
}

// run kinds (1 = terminating code)
const CK_MAKEUP: u32 = 2;
const CK_EOL: u32 = 3;

// 2D modes (MODES: length << 4 | mode, indexed by the next 7 bits)
const MD_P: u8 = 1;
const MD_H: u8 = 2;
const MD_EXT: u8 = 10;

// ---------------------------------------------------------------- the MSB-first bit reader
// (img_int.inc BRM_*: 64-bit buffer, zero bytes past the end, overrun = more bits consumed
// than the data has)
struct Br<'a> {
    d: &'a [u8],
    buf: u64,
    cnt: u64,
    ptr: usize,
}

impl Br<'_> {
    #[inline(always)]
    fn refill(&mut self) {
        if self.cnt >= 56 {
            return;
        }
        let end = self.d.len();
        if end >= self.ptr + 8 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&self.d[self.ptr..self.ptr + 8]);
            self.buf |= u64::from_be_bytes(b) >> self.cnt;
            self.ptr += ((63 - self.cnt) >> 3) as usize;
            self.cnt |= 56;
        } else {
            while self.cnt <= 56 {
                let t = if self.ptr < end { self.d[self.ptr] as u64 } else { 0 };
                self.ptr += 1;
                self.buf |= t << (56 - self.cnt);
                self.cnt += 8;
            }
        }
    }
    #[inline(always)]
    fn peek(&self, n: u32) -> u64 {
        self.buf >> (64 - n)
    }
    #[inline(always)]
    fn skip(&mut self, n: u64) {
        self.buf <<= n;
        self.cnt = self.cnt.wrapping_sub(n);
    }
    /// BRM_OVERRUN: more bits consumed than the data has
    #[inline(always)]
    fn overrun(&self) -> bool {
        let end = self.d.len();
        self.ptr > end && ((self.ptr - end) as u64) * 8 > self.cnt
    }
}

// ---------------------------------------------------------------- tables
// white / black run codes: (code, length | kind << 8, run)
static WHITE: [(u16, u16, u16); 104] = [
    (0x35, 0x108, 0), (0x07, 0x106, 1), (0x07, 0x104, 2), (0x08, 0x104, 3), (0x0B, 0x104, 4),
    (0x0C, 0x104, 5), (0x0E, 0x104, 6), (0x0F, 0x104, 7), (0x13, 0x105, 8), (0x14, 0x105, 9),
    (0x07, 0x105, 10), (0x08, 0x105, 11), (0x08, 0x106, 12), (0x03, 0x106, 13), (0x34, 0x106, 14),
    (0x35, 0x106, 15), (0x2A, 0x106, 16), (0x2B, 0x106, 17), (0x27, 0x107, 18), (0x0C, 0x107, 19),
    (0x08, 0x107, 20), (0x17, 0x107, 21), (0x03, 0x107, 22), (0x04, 0x107, 23), (0x28, 0x107, 24),
    (0x2B, 0x107, 25), (0x13, 0x107, 26), (0x24, 0x107, 27), (0x18, 0x107, 28), (0x02, 0x108, 29),
    (0x03, 0x108, 30), (0x1A, 0x108, 31), (0x1B, 0x108, 32), (0x12, 0x108, 33), (0x13, 0x108, 34),
    (0x14, 0x108, 35), (0x15, 0x108, 36), (0x16, 0x108, 37), (0x17, 0x108, 38), (0x28, 0x108, 39),
    (0x29, 0x108, 40), (0x2A, 0x108, 41), (0x2B, 0x108, 42), (0x2C, 0x108, 43), (0x2D, 0x108, 44),
    (0x04, 0x108, 45), (0x05, 0x108, 46), (0x0A, 0x108, 47), (0x0B, 0x108, 48), (0x52, 0x108, 49),
    (0x53, 0x108, 50), (0x54, 0x108, 51), (0x55, 0x108, 52), (0x24, 0x108, 53), (0x25, 0x108, 54),
    (0x58, 0x108, 55), (0x59, 0x108, 56), (0x5A, 0x108, 57), (0x5B, 0x108, 58), (0x4A, 0x108, 59),
    (0x4B, 0x108, 60), (0x32, 0x108, 61), (0x33, 0x108, 62), (0x34, 0x108, 63), (0x1B, 0x205, 64),
    (0x12, 0x205, 128), (0x17, 0x206, 192), (0x37, 0x207, 256), (0x36, 0x208, 320), (0x37, 0x208, 384),
    (0x64, 0x208, 448), (0x65, 0x208, 512), (0x68, 0x208, 576), (0x67, 0x208, 640), (0xCC, 0x209, 704),
    (0xCD, 0x209, 768), (0xD2, 0x209, 832), (0xD3, 0x209, 896), (0xD4, 0x209, 960), (0xD5, 0x209, 1024),
    (0xD6, 0x209, 1088), (0xD7, 0x209, 1152), (0xD8, 0x209, 1216), (0xD9, 0x209, 1280), (0xDA, 0x209, 1344),
    (0xDB, 0x209, 1408), (0x98, 0x209, 1472), (0x99, 0x209, 1536), (0x9A, 0x209, 1600), (0x18, 0x206, 1664),
    (0x9B, 0x209, 1728), (0x08, 0x20B, 1792), (0x0C, 0x20B, 1856), (0x0D, 0x20B, 1920), (0x12, 0x20C, 1984),
    (0x13, 0x20C, 2048), (0x14, 0x20C, 2112), (0x15, 0x20C, 2176), (0x16, 0x20C, 2240), (0x17, 0x20C, 2304),
    (0x1C, 0x20C, 2368), (0x1D, 0x20C, 2432), (0x1E, 0x20C, 2496), (0x1F, 0x20C, 2560),
];

static BLACK: [(u16, u16, u16); 104] = [
    (0x37, 0x10A, 0), (0x02, 0x103, 1), (0x03, 0x102, 2), (0x02, 0x102, 3), (0x03, 0x103, 4),
    (0x03, 0x104, 5), (0x02, 0x104, 6), (0x03, 0x105, 7), (0x05, 0x106, 8), (0x04, 0x106, 9),
    (0x04, 0x107, 10), (0x05, 0x107, 11), (0x07, 0x107, 12), (0x04, 0x108, 13), (0x07, 0x108, 14),
    (0x18, 0x109, 15), (0x17, 0x10A, 16), (0x18, 0x10A, 17), (0x08, 0x10A, 18), (0x67, 0x10B, 19),
    (0x68, 0x10B, 20), (0x6C, 0x10B, 21), (0x37, 0x10B, 22), (0x28, 0x10B, 23), (0x17, 0x10B, 24),
    (0x18, 0x10B, 25), (0xCA, 0x10C, 26), (0xCB, 0x10C, 27), (0xCC, 0x10C, 28), (0xCD, 0x10C, 29),
    (0x68, 0x10C, 30), (0x69, 0x10C, 31), (0x6A, 0x10C, 32), (0x6B, 0x10C, 33), (0xD2, 0x10C, 34),
    (0xD3, 0x10C, 35), (0xD4, 0x10C, 36), (0xD5, 0x10C, 37), (0xD6, 0x10C, 38), (0xD7, 0x10C, 39),
    (0x6C, 0x10C, 40), (0x6D, 0x10C, 41), (0xDA, 0x10C, 42), (0xDB, 0x10C, 43), (0x54, 0x10C, 44),
    (0x55, 0x10C, 45), (0x56, 0x10C, 46), (0x57, 0x10C, 47), (0x64, 0x10C, 48), (0x65, 0x10C, 49),
    (0x52, 0x10C, 50), (0x53, 0x10C, 51), (0x24, 0x10C, 52), (0x37, 0x10C, 53), (0x38, 0x10C, 54),
    (0x27, 0x10C, 55), (0x28, 0x10C, 56), (0x58, 0x10C, 57), (0x59, 0x10C, 58), (0x2B, 0x10C, 59),
    (0x2C, 0x10C, 60), (0x5A, 0x10C, 61), (0x66, 0x10C, 62), (0x67, 0x10C, 63), (0x0F, 0x20A, 64),
    (0xC8, 0x20C, 128), (0xC9, 0x20C, 192), (0x5B, 0x20C, 256), (0x33, 0x20C, 320), (0x34, 0x20C, 384),
    (0x35, 0x20C, 448), (0x6C, 0x20D, 512), (0x6D, 0x20D, 576), (0x4A, 0x20D, 640), (0x4B, 0x20D, 704),
    (0x4C, 0x20D, 768), (0x4D, 0x20D, 832), (0x72, 0x20D, 896), (0x73, 0x20D, 960), (0x74, 0x20D, 1024),
    (0x75, 0x20D, 1088), (0x76, 0x20D, 1152), (0x77, 0x20D, 1216), (0x52, 0x20D, 1280), (0x53, 0x20D, 1344),
    (0x54, 0x20D, 1408), (0x55, 0x20D, 1472), (0x5A, 0x20D, 1536), (0x5B, 0x20D, 1600), (0x64, 0x20D, 1664),
    (0x65, 0x20D, 1728), (0x08, 0x20B, 1792), (0x0C, 0x20B, 1856), (0x0D, 0x20B, 1920), (0x12, 0x20C, 1984),
    (0x13, 0x20C, 2048), (0x14, 0x20C, 2112), (0x15, 0x20C, 2176), (0x16, 0x20C, 2240), (0x17, 0x20C, 2304),
    (0x1C, 0x20C, 2368), (0x1D, 0x20C, 2432), (0x1E, 0x20C, 2496), (0x1F, 0x20C, 2560),
];

/// a 13-bit lookup table from a code list: run | length << 16 | kind << 24; indices
/// starting with 11 zero bits are EOL (length 11)  [cc_table]
const fn table(codes: &[(u16, u16, u16); 104]) -> [u32; 8192] {
    let mut t = [0u32; 8192];
    let mut k = 0;
    while k < codes.len() {
        let (code, lk, run) = codes[k];
        let len = (lk & 0xFF) as u32;
        let e = ((lk as u32 >> 8) << 24) | (len << 16) | run as u32;
        let sh = 13 - len;
        let mut i = (code as u32) << sh;
        let mut n = 1u32 << sh;
        while n > 0 {
            t[(i & 8191) as usize] = e;
            i += 1;
            n -= 1;
        }
        k += 1;
    }
    let eol = (CK_EOL << 24) | (11 << 16);
    t[0] = eol;
    t[1] = eol;
    t[2] = eol;
    t[3] = eol;
    t
}

static WT: [u32; 8192] = table(&WHITE);
static BT: [u32; 8192] = table(&BLACK);

// 2D mode codes by the next 7 bits: length << 4 | mode
static MODES: [u8; 128] = {
    let mut m = [0u8; 128];
    let head = [0x00, 0x7A, 0x79, 0x76, 0x68, 0x68, 0x65, 0x65];
    let mut i = 0;
    while i < 128 {
        m[i] = if i < 8 {
            head[i]
        } else if i < 16 {
            0x41
        } else if i < 32 {
            0x32
        } else if i < 48 {
            0x37
        } else if i < 64 {
            0x34
        } else {
            0x13
        };
        i += 1;
    }
    m
};

// a1 - b1 of the vertical modes (by MD_*)
static VDELTA: [i8; 11] = [0, 0, 0, 0, 1, 2, 3, -1, -2, -3, 0];

// how a run ended
enum Run {
    Len(u64),
    Eol,
    Bad,
    Eof,
}

// CC_RUN: one run length (make-up codes + a terminating code)
#[inline(always)]
fn run(br: &mut Br, tab: &[u32; 8192]) -> Run {
    let mut out = 0u64;
    loop {
        br.refill();
        let e = tab[br.peek(13) as usize];
        let len = ((e >> 16) & 0xFF) as u64;
        if len == 0 {
            return if br.overrun() { Run::Eof } else { Run::Bad }; // zeros past the end: end of data
        }
        br.skip(len);
        if br.overrun() {
            return Run::Eof;
        }
        let kind = e >> 24;
        out += (e & 0xFFFF) as u64;
        if kind == CK_MAKEUP {
            continue;
        }
        if kind == CK_EOL {
            return Run::Eol;
        }
        return Run::Len(out);
    }
}

// the frame of ccitt_decode (L_*): the two change lines live in work.lines
struct Fr {
    refl: usize, // start of the reference line in lines
    cur: usize,  // start of the current line
    row: u32,
    m: u32,   // changes in the current line
    w: u32,   // width
    cap: u32, // change capacity - 4 (sentinels)
}

// cc_add: append change v to the current line; false = list full
#[inline(always)]
fn add(fr: &mut Fr, lines: &mut [u32], v: u32) -> bool {
    if fr.m >= fr.cap {
        return false;
    }
    lines[fr.cur + fr.m as usize] = v;
    fr.m += 1;
    true
}

// cc_close: an open black span (odd count) is closed at its last change, so the rest of
// the row is white; 4 sentinels (width)
fn close(fr: &mut Fr, lines: &mut [u32]) {
    let c = fr.cur;
    let mut m = fr.m as usize;
    if m & 1 != 0 {
        lines[c + m] = lines[c + m - 1];
        m += 1;
        fr.m = m as u32;
    }
    lines[c + m..c + m + 4].fill(fr.w);
}

// cc_render: output row fr.row from the change list: black between change 2i and 2i + 1
fn render(fr: &Fr, lines: &[u32], dst: &mut [u8], rowb: u32) {
    let start = fr.row as usize * rowb as usize;
    let row = &mut dst[start..start + rowb as usize];
    row.fill(0);
    let cur = &lines[fr.cur..];
    let w = fr.w;
    let mut i = 0usize;
    while (i as u32) < fr.m {
        let mut x = cur[i];
        let to = cur[i + 1].min(w);
        i += 2;
        while x < to {
            let b = (x >> 3) as usize;
            if b >= rowb as usize {
                break;
            }
            if x & 7 == 0 && x + 8 <= to {
                row[b] = 0xFF; // a whole byte
                x += 8;
            } else {
                row[b] |= 0x80 >> (x & 7);
                x += 1;
            }
        }
    }
}

// cc_work: the change lines exist for the width
fn work(wk: &mut Work, w: u32) {
    if wk.lines.is_empty() || w > wk.wcap {
        wk.lines = alloc::vec![0u32; (w as usize * 2 + 16) * 2];
        wk.wcap = w;
    }
}

/// decode a CCITT strip / tile into dst (rows * rowb bytes) -> bytes written / ERR_*
/// [ccitt_decode]
pub fn decode(src: &[u8], dst: &mut [u8], prm: &Prm, wk: &mut Work) -> i64 {
    let w = prm.w;
    if w == 0 || w > 65536 {
        return ERR_HDR as i64;
    }
    work(wk, w);
    let lines = &mut wk.lines[..];
    let mut br = Br { d: src, buf: 0, cnt: 0, ptr: 0 };
    let mut fr = Fr { refl: 0, cur: w as usize * 2 + 16, row: 0, m: 0, w, cap: w * 2 + 12 };
    lines[0..4].fill(w); // reference line of row 0: all white
    let mut eol = false; // the EOL of the next row was consumed
    let mut done = 0u32; // complete rows
    'rows: while fr.row < prm.rows {
        fr.m = 0;
        let mut two_d = prm.comp == 4;
        if prm.comp != 2 && prm.comp != 4 {
            // T.4: synchronise on the EOL (libtiff SYNC_EOL)
            if !eol {
                loop {
                    br.refill();
                    if br.peek(11) == 0 {
                        break;
                    }
                    br.skip(1);
                    if br.overrun() {
                        break 'rows;
                    }
                }
            }
            loop {
                br.refill();
                if br.peek(8) != 0 {
                    break;
                }
                br.skip(8);
                if br.overrun() {
                    break 'rows;
                }
            }
            while br.peek(1) == 0 {
                br.skip(1);
            }
            br.skip(1); // the EOL's 1 bit
            if br.overrun() {
                break 'rows;
            }
            eol = false;
            if prm.opt & 1 != 0 {
                br.refill();
                let t = br.peek(1);
                br.skip(1); // 1 = 1D row, 0 = 2D row
                if br.overrun() {
                    break 'rows;
                }
                two_d = t == 0;
            }
        }
        if !two_d {
            // ------------------------------------------------------ 1D row
            let mut a0 = 0u64;
            let mut col = 0u32;
            loop {
                let tab = if col == 0 { &WT } else { &BT };
                match run(&mut br, tab) {
                    Run::Len(r) => {
                        a0 = (a0 + r).min(w as u64);
                        if fr.m >= fr.cap {
                            return ERR_CODE as i64;
                        }
                        lines[fr.cur + fr.m as usize] = a0 as u32;
                        fr.m += 1;
                        col ^= 1;
                        if a0 >= w as u64 {
                            break;
                        }
                    }
                    Run::Eol => {
                        eol = true; // premature EOL: rest of the row white
                        break;
                    }
                    Run::Bad => return ERR_CODE as i64,
                    Run::Eof => break 'rows,
                }
            }
        } else {
            // ------------------------------------------------------ 2D row
            let mut a0: i32 = -1;
            let mut col = 0u32;
            let mut k = 0u32;
            loop {
                if a0 >= w as i32 {
                    break;
                }
                // b1: first change on the reference line right of a0 with colour != a0's
                let rl = fr.refl;
                let mut c = if k == 0 { 0 } else { k - 1 };
                loop {
                    let e = lines[rl + c as usize];
                    if e >= w || (e as i32) > a0 {
                        break;
                    }
                    c += 1;
                }
                if (c ^ col) & 1 != 0 {
                    c += 1;
                }
                k = c;
                let b1 = lines[rl + c as usize];
                let b2 = lines[rl + c as usize + 1];
                // mode
                br.refill();
                let md = MODES[br.peek(7) as usize];
                let len = (md >> 4) as u64;
                let mode = md & 15;
                if len == 0 {
                    if br.peek(12) == 1 {
                        br.skip(11); // 000000000001: EOL
                        eol = true;
                        break;
                    }
                    if br.overrun() {
                        break 'rows;
                    }
                    return ERR_CODE as i64;
                }
                br.skip(len);
                if br.overrun() {
                    break 'rows;
                }
                if mode == MD_P {
                    a0 = b2 as i32; // a0 = b2, same colour
                } else if mode == MD_H {
                    let (t1, t2) = if col == 0 { (&WT, &BT) } else { (&BT, &WT) };
                    let r1 = match run(&mut br, t1) {
                        Run::Len(r) => r,
                        Run::Eol => {
                            eol = true;
                            break;
                        }
                        Run::Bad => return ERR_CODE as i64,
                        Run::Eof => break 'rows,
                    };
                    let r2 = match run(&mut br, t2) {
                        Run::Len(r) => r,
                        Run::Eol => {
                            eol = true;
                            break;
                        }
                        Run::Bad => return ERR_CODE as i64,
                        Run::Eof => break 'rows,
                    };
                    let a = (a0.max(0) as u32 as u64 + r1).min(w as u64) as u32; // a1
                    if !add(&mut fr, lines, a) {
                        return ERR_CODE as i64;
                    }
                    let a2 = (a as u64 + r2).min(w as u64) as u32;
                    if !add(&mut fr, lines, a2) {
                        return ERR_CODE as i64;
                    }
                    a0 = a2 as i32;
                } else if mode == MD_EXT {
                    return ERR_CODE as i64;
                } else {
                    let mut a1 = VDELTA[mode as usize] as i32 + b1 as i32; // a1 = b1 + delta
                    let lo = a0.max(0);
                    if a1 < lo {
                        a1 = lo;
                    }
                    if a1 > w as i32 {
                        a1 = w as i32;
                    }
                    if !add(&mut fr, lines, a1 as u32) {
                        return ERR_CODE as i64;
                    }
                    a0 = a1;
                    col ^= 1;
                }
            }
        }
        // ---------------------------------------------------------- row end
        close(&mut fr, lines);
        render(&fr, lines, dst, prm.rowb);
        done += 1;
        core::mem::swap(&mut fr.refl, &mut fr.cur); // this row is the next reference
        fr.row += 1;
        if prm.comp == 2 {
            let c = br.cnt & 7; // Modified Huffman: rows are byte aligned
            br.skip(c);
        }
    }
    if fr.row < prm.rows {
        // the data ended inside row fr.row: complete it white, the remaining rows white
        if done == 0 {
            return ERR_TRUNC as i64;
        }
        close(&mut fr, lines);
        render(&fr, lines, dst, prm.rowb);
        fr.row += 1;
        while fr.row < prm.rows {
            fr.m = 0;
            close(&mut fr, lines);
            render(&fr, lines, dst, prm.rowb);
            fr.row += 1;
        }
    }
    prm.rows as i64 * prm.rowb as i64
}
