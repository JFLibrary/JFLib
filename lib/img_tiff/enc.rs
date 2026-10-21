// enc -- the TIFF writer                                              [src/img/tiff.s]
//
//   encode(sf, opt, out) -> 0 / ERR_*                                 tiff_encode
//
// Little-endian TIFF, one page, strips of ~64 KB: opt.bitdepth 32 (RGBA, ExtraSamples 2 =
// unassociated), 24 (RGB, composited over white), 8 (palette: quantized over white with
// the dithering level of opt.dither), 0 = auto (not opaque -> 32; opaque with <= 256
// colours -> exact palette; else 24, like the BMP encoder); opt.tiffcomp none / LZW
// (default) / Deflate (8, level 6) / PackBits, horizontal differencing (Predictor 2) for
// LZW and Deflate at 24 / 32 bit; resolution from opt.meta (dpix / dpiy, resunit cm or
// inch; default 96 dpi); PackBits rows are packed separately (TIFF 6.0; ffmpeg decodes row
// by row); ICC (34675) with IOF_EMBED_ICC and XMP (700) with IOF_EMBED_XMP from opt.meta.
// Files that would exceed classic TIFF's 4 GB offsets give ERR_LIMIT.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_A8, FMT_ARGB, FMT_XRGB, Surface};
use img_core::*;

use crate::dec::{C_DEFLATE, C_LZW, C_NONE, C_PACKBITS};
use img_core::deflate::{DeflateState, deflate_bound, zlib_deflate_to};
use img_core::meta;
use img_core::quantize::{IQF_KEEPALPHA, IQF_WHITE, quantize_alloc};

const T_W: u16 = 256;
const T_H: u16 = 257;
const T_BPS: u16 = 258;
const T_COMP: u16 = 259;
const T_PHOT: u16 = 262;
const T_SOFF: u16 = 273;
const T_SPP: u16 = 277;
const T_RPS: u16 = 278;
const T_SCNT: u16 = 279;
const T_XRES: u16 = 282;
const T_YRES: u16 = 283;
const T_PLANAR: u16 = 284;
const T_RUNIT: u16 = 296;
const T_PRED: u16 = 317;
const T_CMAP: u16 = 320;
const T_EXTRA: u16 = 338;
const T_XMP: u16 = 700;
const T_ICC: u16 = 34675;

const TE_STRIPSZ: u32 = 65536; // target strip size (bytes before compression)
const TE_HSLOTS: usize = 8192; // LZW hash slots

/// one IFD entry (TEE_*); values: inline (u32) or out of line (bytes)
struct Ent<'a> {
    tag: u16,
    typ: u16,
    count: u32,
    val: u32,
    ptr: Option<&'a [u8]>, // the values in file order (None = inline)
}

/// the encoder context (TE_*)
struct Te {
    out: Vec<u8>, // the file
    w: u32,
    h: u32,
    spp: u32,  // 3 / 4 / 1
    bd: u32,   // 32 / 24 / 8
    comp: u32, // TIFF compression
    pred: u32, // 1 / 2
    rowb: u32, // bytes per row
    rps: u32,  // rows per strip
    nstr: u32, // strips
    hgen: u32,  // LZW hash generation
    unit: u32, // ResolutionUnit 2 / 3
    xres: [u8; 8],
    yres: [u8; 8],
    pal: [u32; 256], // palette (straight ARGB)
    idx: Vec<u8>,    // palette indices (w * h)
    hkey: Option<alloc::boxed::Box<[u32; TE_HSLOTS]>>, // LZW hash: keys (generation << 20 | prefix << 8 | byte)
    hcode: Option<alloc::boxed::Box<[u16; TE_HSLOTS]>>, // codes
}

/// encode sf as a TIFF file into out (cleared first; the file on success)  [tiff_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    out.clear();
    let mut te = Te {
        out: Vec::new(),
        w: 0,
        h: 0,
        spp: 0,
        bd: 0,
        comp: 0,
        pred: 0,
        rowb: 0,
        rps: 0,
        nstr: 0,
        hgen: 0,
        unit: 0,
        xres: [0; 8],
        yres: [0; 8],
        pal: [0; 256],
        idx: Vec::new(),
        hkey: None,
        hcode: None,
    };
    match te.run(sf, opt) {
        Ok(()) => {
            *out = core::mem::take(&mut te.out);
            0
        }
        Err(e) => e,
    }
}

// XResolution / YResolution value: inch: dpi (whole numbers as n / 1, else 16.16 / 65536);
// cm: dots per cm * 10000 / 10000  [te_rational]
fn rational(dpi16: u32, unit: u32) -> [u8; 8] {
    let (n, d) = if unit == 3 {
        let v = (dpi16 as u64 * 1_000_000 + 254 * 65536 / 2) / (254 * 65536);
        (v.min(0xFFFF_FFFF) as u32, 10000u32)
    } else if dpi16 & 0xFFFF == 0 {
        (dpi16 >> 16, 1)
    } else {
        (dpi16, 65536)
    };
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&n.to_le_bytes());
    b[4..].copy_from_slice(&d.to_le_bytes());
    b
}

// te_vbytes: count * type size
fn vbytes(e: &Ent) -> u64 {
    let sz = match e.typ {
        3 => 2,
        4 => 4,
        5 => 8,
        _ => 1,
    };
    e.count as u64 * sz
}

// append code with width bits to the LZW output  [TE_PUTC]
#[inline(always)]
fn putc(o: &mut Vec<u8>, acc: &mut u64, nb: &mut u32, code: u32, width: u32) {
    *acc = (*acc << width) | code as u64;
    *nb += width;
    while *nb >= 8 {
        *nb -= 8;
        o.push((*acc >> *nb) as u8);
    }
}

impl Te {
    // the whole encode  [te_run]
    fn run(&mut self, sf: &Surface, opt: &ImgOpt) -> Result<(), i32> {
        self.w = sf.w as u32;
        self.h = sf.h as u32;
        // metadata to embed
        let meta = opt.meta;
        let mut icc: &[u8] = &[];
        let mut xmp: &[u8] = &[];
        if let Some(m) = meta {
            if opt.flags & IOF_EMBED_ICC != 0 {
                icc = &m.icc;
            }
            if opt.flags & IOF_EMBED_XMP != 0 {
                xmp = &m.xmp;
            }
        }
        // resolution (16.16 dpi; 96 when unknown)
        let (mut dx, mut dy, ru) = match meta {
            Some(m) => (m.dpix, m.dpiy, m.resunit),
            None => (0, 0, 0),
        };
        if dx == 0 {
            dx = 96 << 16;
        }
        if dy == 0 {
            dy = dx;
        }
        self.unit = if ru == IMR_CM { 3 } else { 2 };
        self.xres = rational(dx, self.unit);
        self.yres = rational(dy, self.unit);
        // bit depth
        let bd = match opt.bitdepth {
            32 => 32,
            24 => 24,
            8 => {
                let dl = if opt.dither == 0 { 7 } else { (opt.dither - 1).min(8) }; // level + 1 (0 = default 7)
                match quantize_alloc(sf, 256, dl | IQF_WHITE, &mut self.pal) {
                    Some((_, _, idx)) => self.idx = idx,
                    None => return Err(ERR_NOMEM),
                }
                8
            }
            _ => {
                // auto: opaque -> an exact palette when <= 256 colours
                if sf.fmt == FMT_XRGB || sf.fmt == FMT_A8 || meta::all_opaque(sf) {
                    match quantize_alloc(sf, 256, IQF_KEEPALPHA, &mut self.pal) {
                        Some((_, _, idx)) => {
                            self.idx = idx;
                            8
                        }
                        None => 24,
                    }
                } else {
                    32
                }
            }
        };
        self.bd = bd;
        self.spp = match bd {
            8 => 1,
            24 => 3,
            _ => 4,
        };
        let mut cmap = Vec::new();
        if bd == 8 {
            // ColorMap: 256 entries per channel, v * 257
            let mut c = [0u16; 768];
            for (i, &p) in self.pal.iter().enumerate() {
                c[i] = ((p >> 16) & 0xFF) as u16 * 257;
                c[256 + i] = ((p >> 8) & 0xFF) as u16 * 257;
                c[512 + i] = (p & 0xFF) as u16 * 257;
            }
            cmap = c.iter().flat_map(|v| v.to_le_bytes()).collect();
        }
        self.comp = match opt.tiffcomp {
            TIFC_NONE => C_NONE,
            TIFC_DEFLATE => C_DEFLATE,
            TIFC_PACKBITS => C_PACKBITS,
            _ => C_LZW,
        };
        self.pred = if bd != 8 && (self.comp == C_LZW || self.comp == C_DEFLATE) { 2 } else { 1 };
        // rows per strip: ~TE_STRIPSZ bytes, at least one row
        self.rowb = self.w.wrapping_mul(self.spp);
        let mut rps = TE_STRIPSZ.checked_div(self.rowb).unwrap_or(0);
        if rps == 0 {
            rps = 1;
        }
        rps = rps.min(self.h);
        self.rps = rps;
        self.nstr = if rps == 0 { 0 } else { (self.h + rps - 1) / rps };
        // buffers
        let rowb = self.rowb as usize;
        let mut row = vec![0u8; self.w as usize * 4 + 64];
        let mut strip = vec![0u8; rowb * rps as usize + 64];
        let mut offs = vec![0u32; self.nstr as usize];
        let mut cnts = vec![0u32; self.nstr as usize];
        if self.comp == C_LZW {
            self.hkey = Some(alloc::boxed::Box::new([0u32; TE_HSLOTS]));
            self.hcode = Some(alloc::boxed::Box::new([0u16; TE_HSLOTS]));
        }
        let mut z: Option<(alloc::boxed::Box<DeflateState>, Vec<u8>)> = None; // one state + one buffer for all strips
        // header "II*\0" + IFD offset (patched by ifd)
        self.out.extend_from_slice(&[0x49, 0x49, 0x2A, 0, 0, 0, 0, 0]);
        // strips
        for s in 0..self.nstr {
            if opt.cancelled() {
                return Err(ERR_CANCEL);
            }
            let y0 = s * rps;
            let rows = (self.h - y0).min(rps);
            for r in 0..rows {
                self.row(sf, y0 + r, &mut row, &mut strip[r as usize * rowb..]);
            }
            if self.pred == 2 {
                self.te_pred(&mut strip, rows);
            }
            let raw = rowb * rows as usize;
            let start = self.out.len();
            offs[s as usize] = start as u32;
            match self.comp {
                C_NONE => self.out.extend_from_slice(&strip[..raw]),
                C_PACKBITS => {
                    for r in 0..rows as usize {
                        packbits(&mut self.out, &strip[r * rowb..(r + 1) * rowb]); // every row on its own (TIFF 6.0)
                    }
                }
                C_DEFLATE => {
                    let (zs, zbuf) = z.get_or_insert_with(|| (DeflateState::new(), vec![0u8; deflate_bound(rowb * rps as usize)]));
                    let Ok(n) = zlib_deflate_to(Some(zs), &strip[..raw], 6, zbuf) else {
                        return Err(ERR_NOMEM);
                    };
                    self.out.extend_from_slice(&zbuf[..n]);
                }
                _ => self.lzw(&strip[..raw]),
            }
            // classic TIFF: 32-bit offsets (the IFD and its values follow the strips)
            if self.out.len() as u64 > 0xFFF0_0000 {
                return Err(ERR_LIMIT);
            }
            cnts[s as usize] = (self.out.len() - start) as u32;
        }
        let offs_b: Vec<u8> = offs.iter().flat_map(|v| v.to_le_bytes()).collect();
        let cnts_b: Vec<u8> = cnts.iter().flat_map(|v| v.to_le_bytes()).collect();
        let bpsv = [8u8, 0, 8, 0, 8, 0, 8, 0]; // u16[4] BitsPerSample
        // the IFD entries of the page (tag order)  [te_entries]
        let spp = self.spp;
        let one = self.nstr == 1;
        let mut ents: Vec<Ent> = Vec::new();
        let mut add = |tag, typ, count, val, ptr| ents.push(Ent { tag, typ, count, val, ptr });
        add(T_W, 4, 1, self.w, None);
        add(T_H, 4, 1, self.h, None);
        if spp <= 2 {
            add(T_BPS, 3, spp, 0x0008_0008, None);
        } else {
            add(T_BPS, 3, spp, 0, Some(&bpsv[..spp as usize * 2]));
        }
        add(T_COMP, 3, 1, self.comp, None);
        add(T_PHOT, 3, 1, if bd == 8 { 3 } else { 2 }, None); // RGB, or palette
        if one {
            add(T_SOFF, 4, 1, offs[0], None);
        } else {
            add(T_SOFF, 4, self.nstr, 0, Some(&offs_b));
        }
        add(T_SPP, 3, 1, spp, None);
        add(T_RPS, 4, 1, self.rps, None);
        if one {
            add(T_SCNT, 4, 1, cnts[0], None);
        } else {
            add(T_SCNT, 4, self.nstr, 0, Some(&cnts_b));
        }
        let (xres, yres) = (self.xres, self.yres);
        add(T_XRES, 5, 1, 0, Some(&xres));
        add(T_YRES, 5, 1, 0, Some(&yres));
        add(T_PLANAR, 3, 1, 1, None);
        add(T_RUNIT, 3, 1, self.unit, None);
        if self.pred == 2 {
            add(T_PRED, 3, 1, 2, None);
        }
        if bd == 8 {
            add(T_CMAP, 3, 768, 0, Some(&cmap));
        }
        if bd == 32 {
            add(T_EXTRA, 3, 1, 2, None); // unassociated alpha
        }
        if !xmp.is_empty() && xmp.len() <= 0x7FFF_FFFF {
            add(T_XMP, 1, xmp.len() as u32, 0, Some(xmp));
        }
        if !icc.is_empty() && icc.len() <= 0x7FFF_FFFF {
            add(T_ICC, 7, icc.len() as u32, 0, Some(icc));
        }
        self.ifd(&ents)
    }

    // write the IFD (word aligned) and the values that do not fit into an entry, patch
    // the header  [te_ifd]
    fn ifd(&mut self, ents: &[Ent]) -> Result<(), i32> {
        if self.out.len() & 1 != 0 {
            self.out.push(0);
        }
        // size: entries + next pointer + values
        let n = ents.len();
        let mut size = n as u64 * 12 + 6;
        for e in ents {
            if e.ptr.is_some() {
                size += (vbytes(e) + 1) & !1;
            }
        }
        let pos = self.out.len();
        if pos as u64 + size > 0xFFFF_FFFF {
            return Err(ERR_LIMIT); // every offset must fit into 32 bits
        }
        self.out.resize(pos + size as usize, 0);
        let o = &mut self.out;
        o[4..8].copy_from_slice(&(pos as u32).to_le_bytes()); // header -> IFD
        o[pos..pos + 2].copy_from_slice(&(n as u16).to_le_bytes());
        let mut val = pos + n * 12 + 6; // value position
        let mut at = pos + 2; // entry position
        for e in ents {
            o[at..at + 2].copy_from_slice(&e.tag.to_le_bytes());
            o[at + 2..at + 4].copy_from_slice(&e.typ.to_le_bytes());
            o[at + 4..at + 8].copy_from_slice(&e.count.to_le_bytes());
            o[at + 8..at + 12].copy_from_slice(&e.val.to_le_bytes());
            if let Some(p) = e.ptr {
                o[at + 8..at + 12].copy_from_slice(&(val as u32).to_le_bytes()); // value offset
                let nb = vbytes(e) as usize;
                o[val..val + nb].copy_from_slice(&p[..nb]);
                val += (nb + 1) & !1;
            }
            at += 12;
        }
        o[at..at + 4].fill(0); // no next IFD
        o.truncate(val);
        Ok(())
    }

    // row y as TIFF samples: RGBA (32), RGB over white (24), palette indices (8)  [te_row]
    fn row(&self, sf: &Surface, y: u32, tmp: &mut [u8], dst: &mut [u8]) {
        let w = self.w as usize;
        if self.bd == 8 {
            dst[..w].copy_from_slice(&self.idx[y as usize * w..(y as usize + 1) * w]);
            return;
        }
        meta::get_row(sf, y as usize, tmp, FMT_ARGB);
        if self.bd == 32 {
            for (o, q) in dst.chunks_exact_mut(4).zip(tmp.chunks_exact(4)).take(w) {
                o.copy_from_slice(&[q[2], q[1], q[0], q[3]]); // B G R A -> R G B A
            }
            return;
        }
        for (o, q) in dst.chunks_exact_mut(3).zip(tmp.chunks_exact(4)).take(w) {
            let a = q[3] as u32;
            if a == 255 {
                o.copy_from_slice(&[q[2], q[1], q[0]]);
            } else {
                // composite over white: c' = (c a + 255 (255 - a) + 127) / 255
                let k = (255 - a) * 255 + 127;
                let f = |c: u8| (((c as u32 * a + k) * 0x8081) >> 23) as u8; // / 255 (exact for < 2^16 + 2^8)
                o.copy_from_slice(&[f(q[2]), f(q[1]), f(q[0])]);
            }
        }
    }

    // horizontal differencing (Predictor 2) of the first `rows` rows of the strip: each
    // byte minus the byte one pixel (spp bytes) to the left  [te_pred]
    fn te_pred(&self, strip: &mut [u8], rows: u32) {
        let rowb = self.rowb as usize;
        let spp = self.spp as usize;
        for r in 0..rows as usize {
            let row = &mut strip[r * rowb..(r + 1) * rowb];
            for i in (spp..rowb).rev() {
                row[i] = row[i].wrapping_sub(row[i - spp]);
            }
        }
    }

    // TIFF LZW (libtiff compatible: MSB first, clear first, one-code-early width change,
    // clear when code 4094 is reached, EOI) appended to the output; strings are found
    // through a hash of (prefix << 8 | byte) with generation-stamped slots (the generation
    // persists across strips)  [te_lzw]
    fn lzw(&mut self, src: &[u8]) {
        let n = src.len();
        self.out.reserve(n + (n >> 1) + (n >> 10) + 64);
        let mut o = core::mem::take(&mut self.out);
        let (Some(hkey), Some(hcode)) = (self.hkey.as_deref_mut(), self.hcode.as_deref_mut()) else {
            return;
        };
        self.hgen = lzw_strip(src, &mut o, hkey, hcode, self.hgen);
        self.out = o;
    }
}

// next hash generation (the table is cleared when the 12-bit stamp wraps)  [TE_NEWGEN]
#[inline(always)]
fn newgen(hkey: &mut [u32; TE_HSLOTS], hgen: u32) -> u32 {
    if hgen + 1 >= 4096 {
        hkey.fill(0);
        1
    } else {
        hgen + 1
    }
}

// one strip of te_lzw: returns the hash generation to keep
#[allow(unused_assignments)] // the bump after the last code may restart the table
#[inline(never)]
fn lzw_strip(src: &[u8], o: &mut Vec<u8>, hkey: &mut [u32; TE_HSLOTS], hcode: &mut [u16; TE_HSLOTS], hgen: u32) -> u32 {
    let mut hgen = newgen(hkey, hgen);
    let (mut acc, mut nb) = (0u64, 0u32);
    let mut width = 9u32;
    let mut next = 258u32;
    putc(o, &mut acc, &mut nb, 256, 9);
    // a code was added: clear at 4094, else one bit wider at 2^width  [TE_BUMP]
    macro_rules! bump {
        () => {
            next += 1;
            if next == 4094 {
                putc(o, &mut acc, &mut nb, 256, width);
                width = 9;
                next = 258;
                hgen = newgen(hkey, hgen);
            } else if width < 12 && next >= 1 << width {
                width += 1;
            }
        };
    }
    if let Some((&first, rest)) = src.split_first() {
        let mut prefix = first as u32;
        let mut gtag = hgen << 20;
        for &b in rest {
            let key = prefix << 8 | b as u32;
            let mut h = (key.wrapping_mul(0x9E37_79B1) >> (32 - 13)) as usize;
            let sv = gtag | key;
            let hit = loop {
                let e = hkey[h];
                if e == sv {
                    break true;
                }
                if e >> 20 != hgen {
                    break false; // free (older generation)
                }
                h = (h + 1) & (TE_HSLOTS - 1);
            };
            if hit {
                prefix = hcode[h] as u32;
                continue;
            }
            hkey[h] = sv; // new code for prefix + byte
            hcode[h] = next as u16;
            putc(o, &mut acc, &mut nb, prefix, width);
            prefix = b as u32;
            bump!();
            gtag = hgen << 20;
        }
        putc(o, &mut acc, &mut nb, prefix, width);
        bump!();
    }
    putc(o, &mut acc, &mut nb, 257, width);
    if nb != 0 {
        o.push((acc << (8 - nb)) as u8); // flush
    }
    hgen
}

// PackBits (one row): runs of 3+ equal bytes as repeats, everything else as literals (at
// most 128 each)  [te_packbits]
fn packbits(o: &mut Vec<u8>, src: &[u8]) {
    let n = src.len();
    let mut i = 0;
    while i < n {
        let b = src[i];
        let mut j = i + 1; // run end
        while j < n && j - i < 128 && src[j] == b {
            j += 1;
        }
        if j - i >= 3 {
            o.push((1 - (j - i) as i32) as u8); // repeat: 1 - n, byte
            o.push(b);
            i = j;
            continue;
        }
        j = i; // literal until a run of 3 or 128 bytes
        while j < n && j - i < 128 {
            if j + 2 < n && src[j] == src[j + 1] && src[j] == src[j + 2] {
                break;
            }
            j += 1;
        }
        o.push((j - i - 1) as u8);
        o.extend_from_slice(&src[i..j]);
        i = j;
    }
}
