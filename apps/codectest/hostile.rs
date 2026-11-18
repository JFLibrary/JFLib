// hostile -- codectest --hostile: crafted JPEG XL / JPEG XR inputs (built here, a few bytes to a
// few KB) that the decoders must refuse quickly, without a panic and without allocating or
// working far beyond the input or the pixel budget; and controls: the same kind of input made
// valid, and encoder round trips, still decode.  Exit code 1 when a check fails.
//
//   jpeg_scans       20000 empty scans of a 2048 x 2048 progressive JPEG with restarts (200 KB)
//   png_iccp         60 iCCP chunks, each a zlib bomb past the 16 MB metadata limit (1 MB)
//   jxl_box64        a container box with a 64-bit size reaching to the end of u64 (28 bytes)
//   jxl_crop         a frame of its own size (custom crop) larger than the pixel budget
//   jxl_palettes     273 palette transforms of 137727 entries each on a 1 x 1 image
//   jxl_squeeze      65416 squeeze steps on one channel of a 1 x 1 image (74 KB), 65120 on a
//                    palette's meta channel (no shift to count)
//   jxl_spline_rows  the spline draw cache of a frame 2^30 rows high
//   jxl_patch_area   20480 patches of 128 x 128 on a 128 x 128 frame
//   jxr_index        a 16384 x 16384 image with an index table and no tile data (86 bytes)
//   jxl_rt, jxr_rt   controls: the encoders' files decode (exactly when lossless)
//
// --only NAME runs the groups above whose names start with NAME.  (build_rs.py test codectest
// runs it through tests/hostile.uis; --script is ignored)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU32;

use gfx::FMT_ARGB;
use img_core::{ImgDec, ImgMeta, ImgOpt, Surface, ERR_HDR, ERR_LIMIT, ERR_TRUNC, IOF_LOSSLESS};
use img_jxl::ewr::Jw;
use img_jxl::writer::IOF_JXL_PROGRESSIVE;

// JPEG XL U32 distributions (offset, bits) x 4
static D_SIZE: [u32; 8] = [1, 9, 1, 13, 1, 18, 1, 30];
static D_BITS: [u32; 8] = [8, 0, 10, 0, 12, 0, 1, 6];
static D_NEC: [u32; 8] = [0, 0, 1, 0, 2, 4, 1, 12];
static D_0123: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 0];
static D_1248: [u32; 8] = [1, 0, 2, 0, 4, 0, 8, 0];
static D_PASSES: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 3];
static D_ORIGIN: [u32; 8] = [0, 8, 256, 11, 2304, 14, 18688, 30];
static D_BMODE: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 2];
static D_NAME: [u32; 8] = [0, 0, 0, 4, 16, 5, 48, 10];
static D_TOC: [u32; 8] = [0, 10, 1024, 14, 17408, 22, 4211712, 30];
static D_NT: [u32; 8] = [0, 0, 1, 0, 2, 4, 18, 8];
static D_BEGIN: [u32; 8] = [0, 3, 8, 6, 72, 10, 1096, 13];
static D_NUMC: [u32; 8] = [1, 0, 3, 0, 4, 0, 1, 13];
static D_NBCOL: [u32; 8] = [0, 8, 256, 10, 1280, 12, 5376, 16];
static D_NBDELTA: [u32; 8] = [0, 0, 1, 8, 257, 10, 1281, 16];
static D_NSQ: [u32; 8] = [0, 0, 1, 4, 9, 6, 41, 8];
static D_SQNC: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 4];

const JFT_REGULAR: u32 = 0;
const JFT_REFONLY: u32 = 2;
const JFF_PATCHES: u32 = 2;
/// a refusal must come within this many microseconds
const QUICK_US: u64 = 1_000_000;

struct H {
    ok: u32,
    failed: u32,
    only: Option<String>, // --only NAME: the groups whose names start with NAME
}

impl H {
    fn on(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|o| name.starts_with(o.as_str()))
    }

    fn check(&mut self, name: &str, cond: bool, what: String) {
        if cond {
            self.ok += 1;
            plat::write_stdout(format!("ok   {name}: {what}\n").as_bytes());
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {name}: {what}\n").as_bytes());
        }
    }

    /// f() must give the error `want`, within QUICK_US
    fn refuse(&mut self, name: &str, want: i32, f: impl FnOnce() -> i32) {
        let t0 = plat::time_us();
        let r = f();
        let us = plat::time_us().wrapping_sub(t0);
        self.check(name, r == want && us < QUICK_US, format!("result {r} (want {want}) in {} ms", us / 1000));
    }
}

/// a JPEG XL decode with that pixel budget (0 = the default): (result, w, h)
fn jxl_dec(data: &[u8], maxpx: u64) -> (i32, i32, i32) {
    let dec = ImgDec { maxpx, ..Default::default() };
    let (mut sf, mut m) = (Surface::default(), ImgMeta::default());
    let r = img_jxl::decode(data, &mut sf, &dec, &mut m);
    (r, sf.w, sf.h)
}

/// a JPEG XR decode, with a cancel flag or not
fn jxr_dec(data: &[u8], cancel: Option<&AtomicU32>) -> (i32, Surface) {
    let dec = ImgDec { cancel, ..Default::default() };
    let (mut sf, mut m) = (Surface::default(), ImgMeta::default());
    let r = img_jxr::decode(data, &mut sf, &dec, &mut m);
    (r, sf)
}

fn done(mut w: Jw) -> Vec<u8> {
    w.pad();
    w.buf
}

// ------------------------------------------------------------------ JPEG XL writer pieces

/// the codestream's image headers: w x h, 8-bit integer samples, no XYB, sRGB, no extra
/// channels; byte aligned
fn jxl_head(w: &mut Jw, xs: u32, ys: u32) {
    w.bits(0x0AFF, 16);
    w.bits(0, 1); // SizeHeader: not div8
    w.u32(ys, &D_SIZE);
    w.bits(0, 3); // no ratio
    w.u32(xs, &D_SIZE);
    w.bits(0, 1); // ImageMetadata: not all_default
    w.bits(0, 1); // no extra fields
    w.bits(0, 1); // integer samples
    w.u32(8, &D_BITS);
    w.bits(1, 1); // modular_16_bit_buffers
    w.u32(0, &D_NEC);
    w.bits(0, 1); // xyb_encoded
    w.bits(1, 1); // colour encoding all_default (sRGB)
    w.u64(0); // extensions
    w.bits(1, 1); // TransformData all_default
    w.pad();
}

/// a modular frame (one pass, no YCbCr, no upsampling, no loop filters, blend mode replace):
/// header, a TOC of one section, the section
fn jxl_frame(w: &mut Jw, typ: u32, flags: u32, crop: Option<(u32, u32)>, last: bool, sec: &[u8]) {
    w.bits(0, 1); // not all_default
    w.u32(typ, &D_0123);
    w.bits(1, 1); // modular
    w.u64(flags);
    w.bits(0, 1); // no YCbCr
    w.u32(1, &D_1248); // upsampling 1
    w.bits(1, 2); // group size shift 1 (256)
    if typ != JFT_REFONLY {
        w.u32(1, &D_PASSES);
    }
    match crop {
        Some((xs, ys)) => {
            w.bits(1, 1);
            if typ != JFT_REFONLY {
                w.u32(0, &D_ORIGIN); // x0, y0 = 0
                w.u32(0, &D_ORIGIN);
            }
            w.u32(xs, &D_ORIGIN);
            w.u32(ys, &D_ORIGIN);
        }
        None => w.bits(0, 1),
    }
    let last = typ == JFT_REGULAR && last;
    if typ == JFT_REGULAR {
        w.u32(0, &D_BMODE); // replace (the crops here are not partial: no source)
        w.bits(last as u64, 1);
    }
    if !last {
        w.u32(0, &D_0123); // save as reference 0
        w.bits(1, 1); // before the colour transform
    }
    w.u32(0, &D_NAME);
    w.bits(0, 1); // loop filter: not all_default
    w.bits(0, 1); // no gaborish
    w.bits(0, 2); // no EPF
    w.u64(0); // loop filter extensions
    w.u64(0); // frame header extensions
    w.bits(0, 1); // TOC: no permutation
    w.pad();
    w.u32(sec.len() as u32, &D_TOC);
    w.pad();
    w.data(sec);
}

/// an entropy code whose every symbol is 0: prefix codes, one cluster of one symbol
fn ent_zero(w: &mut Jw, nctx: u32) {
    w.bits(0, 1); // no LZ77
    if nctx > 1 {
        w.bits(1, 1); // simple context map, 0 bits per entry
        w.bits(0, 2);
    }
    w.bits(1, 1); // prefix codes
    w.bits(15, 4); // HybridUintConfig: split exponent 15
    w.bits(0, 1); // alphabet size 1
}

fn varlen3(w: &mut Jw, v: u32) {
    if v == 0 {
        w.bits(0, 1);
        return;
    }
    let n = 31 - v.leading_zeros();
    w.bits(1, 1);
    w.bits(n as u64, 3);
    if n != 0 {
        w.bits((v - (1 << n)) as u64, n);
    }
}

/// an entropy code whose context i always gives vals[i] (< 256, at most 8 distinct values):
/// ANS, one single-symbol histogram per value (no bits per symbol), then the reader's initial
/// state
fn ent_const(w: &mut Jw, vals: &[u32]) {
    let mut cl: Vec<u32> = Vec::new();
    let mut map = Vec::new();
    for &v in vals {
        let k = match cl.iter().position(|&c| c == v) {
            Some(k) => k,
            None => {
                cl.push(v);
                cl.len() - 1
            }
        };
        map.push(k as u64);
    }
    w.bits(0, 1); // no LZ77
    w.bits(1, 1); // simple context map, 3 bits per entry
    w.bits(3, 2);
    for &k in &map {
        w.bits(k, 3);
    }
    w.bits(0, 1); // ANS
    w.bits(3, 2); // log alphabet size 8
    for _ in &cl {
        w.bits(8, 4); // split exponent 8
    }
    for &v in &cl {
        w.bits(1, 1); // simple histogram of one symbol
        w.bits(0, 1);
        varlen3(w, v);
    }
    w.bits(0x13_0000, 32); // ANS state (stays: one symbol)
}

/// a modular transform of the group header
#[derive(Clone)]
enum Tr {
    /// a palette of channel 0: nb_colours, nb_deltas
    Pal(u32, u32),
    /// squeeze steps: horizontal, in place, begin channel, number of channels
    Sq(Vec<[u32; 4]>),
}

/// the rest of a single-group modular frame's section: default DC quantisation, no global
/// tree; the group header with these transforms, a one-leaf tree (zero predictor), every
/// sample 0
fn jxl_modular(w: &mut Jw, trs: &[Tr]) {
    w.bits(1, 1); // default DC quantisation
    w.bits(0, 1); // no global tree
    w.bits(0, 1); // group: local tree
    w.bits(1, 1); // default weighted predictor header
    w.u32(trs.len() as u32, &D_NT);
    for tr in trs {
        match tr {
            Tr::Pal(nc, nd) => {
                w.u32(1, &D_0123); // palette
                w.u32(0, &D_BEGIN);
                w.u32(1, &D_NUMC);
                w.u32(*nc, &D_NBCOL);
                w.u32(*nd, &D_NBDELTA);
                w.bits(0, 4); // predictor 0
            }
            Tr::Sq(steps) => {
                w.u32(2, &D_0123); // squeeze
                w.u32(steps.len() as u32, &D_NSQ);
                for s in steps {
                    w.bits(s[0] as u64, 1);
                    w.bits(s[1] as u64, 1);
                    w.u32(s[2], &D_BEGIN);
                    w.u32(s[3], &D_SQNC);
                }
            }
        }
    }
    ent_zero(w, 6); // tree: one leaf
    ent_zero(w, 1); // samples
}

/// a w x h RGB image of one modular frame with these transforms
fn jxl_modular_file(xs: u32, ys: u32, trs: &[Tr]) -> Vec<u8> {
    jxl_modular_frames(xs, ys, &[trs.to_vec()])
}

/// a w x h RGB image of modular frames, each with its transforms (all but the last saved as
/// reference 0, not shown)
fn jxl_modular_frames(xs: u32, ys: u32, frames: &[Vec<Tr>]) -> Vec<u8> {
    let mut w = Jw::new();
    jxl_head(&mut w, xs, ys);
    for (i, trs) in frames.iter().enumerate() {
        let mut s = Jw::new();
        jxl_modular(&mut s, trs);
        jxl_frame(&mut w, JFT_REGULAR, 0, None, i + 1 == frames.len(), &done(s));
    }
    done(w)
}

/// a w x h RGB image of one modular frame with these palettes (nb_colours, nb_deltas)
fn jxl_palette_file(xs: u32, ys: u32, palettes: &[(u32, u32)]) -> Vec<u8> {
    jxl_modular_file(xs, ys, &palettes.iter().map(|&(c, d)| Tr::Pal(c, d)).collect::<Vec<_>>())
}

/// the steps in transforms of at most 296 (D_NSQ's largest)
fn sq_transforms(steps: &[[u32; 4]]) -> Vec<Tr> {
    steps.chunks(296).map(|c| Tr::Sq(c.to_vec())).collect()
}

/// a 16 x 16 image whose frame has its own size xs x ys (origin 0, 0); data: with the section
fn jxl_crop_file(xs: u32, ys: u32, data: bool) -> Vec<u8> {
    let mut w = Jw::new();
    jxl_head(&mut w, 16, 16);
    let sec = if data {
        let mut s = Jw::new();
        jxl_modular(&mut s, &[]);
        done(s)
    } else {
        Vec::new()
    };
    jxl_frame(&mut w, JFT_REGULAR, 0, Some((xs, ys)), true, &sec);
    let mut d = done(w);
    d.extend_from_slice(&[0; 8]);
    d
}

/// a 128 x 128 image: a reference frame (saved before the colour transform), then a frame with
/// nref patches of the whole reference frame, cnt positions each (all at 0, 0; mode add)
fn jxl_patch_file(nref: u32, cnt: u32) -> Vec<u8> {
    let mut w = Jw::new();
    jxl_head(&mut w, 128, 128);
    let mut s = Jw::new();
    jxl_modular(&mut s, &[]);
    jxl_frame(&mut w, JFT_REFONLY, 0, None, false, &done(s));
    let mut s = Jw::new();
    // contexts: count, reference, size - 1, ref position, first position, mode, offsets,
    // positions - 1, alpha, clamp
    ent_const(&mut s, &[nref, 0, 127, 0, 0, 2, 0, cnt - 1, 0, 0]);
    jxl_modular(&mut s, &[]);
    jxl_frame(&mut w, JFT_REGULAR, JFF_PATCHES, None, true, &done(s));
    done(w)
}

// ------------------------------------------------------------------ JPEG XR writer pieces

/// a JPEG XR file: the container with the size and the codestream
fn jxr_file(xs: u32, ys: u32, cs: &[u8]) -> Vec<u8> {
    let mut d = vec![0x49, 0x49, 0xBC, 0x01, 8, 0, 0, 0];
    d.extend_from_slice(&4u16.to_le_bytes());
    let ioff = 8 + 2 + 4 * 12 + 4;
    for (tag, v) in [(0xBC80u16, xs), (0xBC81, ys), (0xBCC0, ioff), (0xBCC1, cs.len() as u32)] {
        d.extend_from_slice(&tag.to_le_bytes());
        d.extend_from_slice(&4u16.to_le_bytes()); // LONG
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(cs);
    d
}

/// a codestream: Y_ONLY 8 bits, DC band only, uniform DC QP 0, short header, one tile, an index
/// table of one entry (its VLW_ESC bytes), SubsequentBytes 0, then the tile data
fn jxr_cs(xs: u32, ys: u32, entry: &[u8], tile: &[u8]) -> Vec<u8> {
    let mut c = b"WMPHOTO\0".to_vec();
    c.extend_from_slice(&[0x10, 0x04, 0x80, 0x01]); // version 1; index table; short header; YONLY, BD8
    c.extend_from_slice(&((xs - 1) as u16).to_be_bytes());
    c.extend_from_slice(&((ys - 1) as u16).to_be_bytes());
    c.extend_from_slice(&[0x03, 0x80, 0x00]); // plane: YONLY, DC only, DC QP uniform 0
    c.extend_from_slice(&[0x00, 0x01]); // index table start code
    c.extend_from_slice(entry);
    c.extend_from_slice(&[0x00, 0x00]);
    c.extend_from_slice(tile);
    c
}

/// an opaque w x h test image: few colours (palette) or a gradient with a pattern
fn test_image(w: i32, h: i32, few: bool) -> Surface {
    let mut sf = Surface::new(w, h, FMT_ARGB);
    let s = sf.stride as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v: u32 = if few {
                [0xFF20_4060, 0xFFFF_FFFF, 0xFF00_0000, 0xFFC0_3010, 0xFF10_A0F0][(x / 5 + y / 7 + (x * y) % 3) % 5]
            } else {
                0xFF00_0000 | ((x * 4) as u32 & 255) << 16 | ((y * 3) as u32 & 255) << 8 | (((x ^ y) * 7) as u32 & 255)
            };
            sf.px[y * s + x * 4..y * s + x * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    sf
}

fn same_pixels(a: &Surface, b: &Surface) -> bool {
    if a.w != b.w || a.h != b.h {
        return false;
    }
    let rb = a.w as usize * 4;
    (0..a.h as usize).all(|y| {
        a.px[y * a.stride as usize..y * a.stride as usize + rb] == b.px[y * b.stride as usize..y * b.stride as usize + rb]
    })
}

// ------------------------------------------------------------------ checks

/// a progressive grey JPEG of w x h with a restart interval of 1 MCU and `scans` DC scans
/// without any entropy-coded data (each SOS segment straight after the last)
fn jpeg_empty_scans(w: u16, h: u16, scans: usize) -> Vec<u8> {
    let mut d = vec![0xFF, 0xD8];
    d.extend_from_slice(&[0xFF, 0xDB, 0, 67, 0]); // DQT: table 0, all 1
    d.extend_from_slice(&[1u8; 64]);
    d.extend_from_slice(&[0xFF, 0xC2, 0, 11, 8]); // SOF2: 8 bits, h, w, 1 component (id 1, 1x1, table 0)
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&[1, 1, 0x11, 0]);
    d.extend_from_slice(&[0xFF, 0xDD, 0, 4, 0, 1]); // DRI: 1 MCU
    for _ in 0..scans {
        d.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0x00, 0, 0, 0x00]); // SOS: DC first of component 1
    }
    d.extend_from_slice(&[0xFF, 0xD9]);
    d
}

pub fn run(only: Option<&str>) -> i32 {
    let mut t = H { ok: 0, failed: 0, only: only.map(String::from) };

    // 0: 20000 empty scans of a 2048 x 2048 progressive JPEG (200 KB): each went over every
    // block (the restart interval keeps it going) -- about 1.3e9 blocks; now the scans stop at
    // 64 x the image's blocks and the image so far comes back
    if t.on("jpeg_scans") {
        let d = jpeg_empty_scans(2048, 2048, 20_000);
        let t0 = plat::time_us();
        let r = img_jpeg::decode_fl(&d, 0);
        let us = plat::time_us().wrapping_sub(t0);
        t.check("jpeg_scans", r.as_ref().is_ok_and(|s| s.w == 2048 && s.h == 2048) && us < 4 * QUICK_US,
            format!("{} bytes: {} in {} ms", d.len(), if r.is_ok() { "the image" } else { "an error" }, us / 1000));
    }

    // 0b: 60 iCCP chunks, each inflating past the 16 MB metadata limit: every one was tried again
    // (about 32 MB of inflating each, two seconds); now the first one only, its profile dropped
    if t.on("png_iccp") {
        let bomb = img_core::deflate::zlib_deflate(&vec![0u8; 17 << 20], 9);
        let mut d = img_core::io::PNG_SIG.to_vec();
        img_png::put_chunk(&mut d, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
        let mut iccp = b"bomb\0\0".to_vec();
        iccp.extend_from_slice(&bomb);
        for _ in 0..60 {
            img_png::put_chunk(&mut d, b"iCCP", &iccp);
        }
        img_png::put_chunk(&mut d, b"IEND", &[]);
        let mut m = ImgMeta::default();
        let t0 = plat::time_us();
        let r = img_png::glue::scan(&d, &mut m, img_core::IMGD_ICC);
        let us = plat::time_us().wrapping_sub(t0);
        t.check("png_iccp", r == 0 && m.icc.is_empty() && m.w == 1 && us < QUICK_US / 4,
            format!("{} bytes: result {r}, profile {} bytes, in {} ms", d.len(), m.icc.len(), us / 1000));
    }

    // 1: box with a 64-bit size up to u64::MAX - 3 (the next box position overflowed)
    if t.on("jxl_box64") {
        let mut d = vec![0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A];
        d.extend_from_slice(&1u32.to_be_bytes());
        d.extend_from_slice(b"jxlc");
        d.extend_from_slice(&(u64::MAX - 15).to_be_bytes());
        let mut m = ImgMeta::default();
        t.check("jxl_box64/probe", !img_jxl::probe(&d, &mut m), format!("{} bytes not recognised", d.len()));
        t.refuse("jxl_box64/decode", ERR_HDR, || jxl_dec(&d, 0).0);
    }

    // 2: a frame larger than the pixel budget (the image itself is within it)
    if t.on("jxl_crop") {
        let d = jxl_crop_file(64, 64, true);
        let (r, w, h) = jxl_dec(&d, 0);
        t.check("jxl_crop/control", r == 0 && w == 16 && h == 16, format!("64 x 64 frame on 16 x 16: result {r}, {w} x {h}"));
        t.refuse("jxl_crop/maxpx", ERR_LIMIT, || jxl_dec(&d, 1024).0);
        let d = jxl_crop_file(16384, 32768, false);
        t.refuse("jxl_crop/2^29", ERR_LIMIT, || jxl_dec(&d, 0).0);
    }

    // 3: palette channels beyond the image's samples (each below the old 2^20 threshold)
    if t.on("jxl_palettes") {
        let d = jxl_palette_file(1, 1, &[(1, 0)]);
        let (r, w, h) = jxl_dec(&d, 0);
        t.check("jxl_palettes/control", r == 0 && w == 1 && h == 1, format!("one palette: result {r}, {w} x {h}"));
        let d = jxl_palette_file(1, 1, &[(70911, 66816); 273]);
        t.refuse("jxl_palettes/273", ERR_LIMIT, || jxl_dec(&d, 0).0);
    }

    // 3b: squeeze steps on one 1 x 1 channel, each adding an empty channel (up to 65536, each
    // inserted into the middle of the list: 232 s from 74 KB); refused like libjxl: shift > 30
    if t.on("jxl_squeeze") {
        let d = jxl_modular_file(16, 16, &[Tr::Sq(vec![[1, 1, 0, 3], [0, 1, 0, 3]])]);
        let (r, w, h) = jxl_dec(&d, 0);
        t.check("jxl_squeeze/control", r == 0 && w == 16 && h == 16, format!("two squeezes: result {r}, {w} x {h}"));
        let d = jxl_modular_file(1, 1, &sq_transforms(&vec![[1, 1, 0, 1]; 221 * 296]));
        t.refuse("jxl_squeeze/1x1", ERR_HDR, || jxl_dec(&d, 0).0);
        // a palette's meta channel has no shift to count: the channel moves are (56 s before),
        // over all frames of the decode
        let meta = |steps: usize| {
            let mut trs = vec![Tr::Pal(1, 0)];
            trs.extend(sq_transforms(&vec![[1, 1, 0, 1]; steps]));
            trs
        };
        let (r, w, h) = jxl_dec(&jxl_modular_file(1, 1, &meta(1000)), 0);
        t.check("jxl_squeeze/meta_1000", r == 0 && w == 1 && h == 1, format!("1000 steps on the meta channel: result {r}, {w} x {h}"));
        let d = jxl_modular_file(1, 1, &meta(220 * 296));
        t.refuse("jxl_squeeze/meta", ERR_LIMIT, || jxl_dec(&d, 0).0);
        let (r, w, h) = jxl_dec(&jxl_modular_frames(1, 1, &[meta(4000)]), 0);
        t.check("jxl_squeeze/frames_1", r == 0 && w == 1 && h == 1, format!("one frame of 4000 steps: result {r}, {w} x {h}"));
        let d = jxl_modular_frames(1, 1, &vec![meta(4000); 6]);
        t.refuse("jxl_squeeze/frames_6", ERR_LIMIT, || jxl_dec(&d, 0).0);
    }

    // 4: the spline draw cache of a 2^30 rows high frame (no blobs)
    if t.on("jxl_spline_rows") {
        let q = img_jxl::spline::Quantized::default();
        let t0 = plat::time_us();
        let ok = img_jxl::spline::Splines::new(&q, 64, 1 << 30, 0.0, 1.0).is_ok_and(|s| s.is_empty());
        let us = plat::time_us().wrapping_sub(t0);
        t.check("jxl_spline_rows", ok && us < QUICK_US, format!("empty cache in {} ms", us / 1000));
    }

    // 5: patches whose rectangles cover the frame 20480 times
    if t.on("jxl_patch_area") {
        let d = jxl_patch_file(1, 1);
        let (r, w, h) = jxl_dec(&d, 0);
        t.check("jxl_patch_area/control", r == 0 && w == 128 && h == 128, format!("one patch: result {r}, {w} x {h}"));
        let d = jxl_patch_file(80, 256);
        t.refuse("jxl_patch_area/20480", ERR_LIMIT, || jxl_dec(&d, 0).0);
    }

    // 6: JPEG XR with an index table: the size against the data also there
    if t.on("jxr_index") {
        let d = jxr_file(16, 16, &jxr_cs(16, 16, &[0x00, 0x00], &[0x00, 0x00, 0x01, 0x00, 0, 0, 0, 0]));
        let (r, sf) = jxr_dec(&d, None);
        t.check("jxr_index/control", r == 0 && sf.w == 16 && sf.h == 16, format!("16 x 16: result {r}, {} x {}", sf.w, sf.h));
        let d = jxr_file(16384, 16384, &jxr_cs(16384, 16384, &[0xFF], &[]));
        t.refuse("jxr_index/absent", ERR_TRUNC, || jxr_dec(&d, None).0);
        // the cancel flag set: the tiles poll it (ERR_CANCEL), the refusal comes before the
        // buffers and the tiles (the decode used to end with ERR_TRUNC only after them)
        let stop = AtomicU32::new(1);
        t.refuse("jxr_index/before_tiles", ERR_TRUNC, || jxr_dec(&d, Some(&stop)).0);
    }

    // controls: the encoders' files (palettes, squeeze, VarDCT, JPEG XR) still decode
    if t.on("jxl_rt") {
        for (name, few, flags) in [
            ("jxl_rt/palette", true, IOF_LOSSLESS),
            ("jxl_rt/squeeze", false, IOF_LOSSLESS | IOF_JXL_PROGRESSIVE),
            ("jxl_rt/lossless", false, IOF_LOSSLESS),
        ] {
            let src = test_image(67, 45, few);
            let opt = ImgOpt { quality: 100, flags, ..ImgOpt::from_bytes(&[]) };
            let mut out = Vec::new();
            let e = img_jxl::enc::encode(&src, &opt, &mut out);
            let (mut sf, mut m) = (Surface::default(), ImgMeta::default());
            let r = if e == 0 { img_jxl::decode(&out, &mut sf, &ImgDec::default(), &mut m) } else { e };
            t.check(name, r == 0 && same_pixels(&src, &sf), format!("{} bytes: result {r}, exact", out.len()));
        }
        for (name, few) in [("jxl_rt/lossy_q90", false), ("jxl_rt/lossy_flat", true)] {
            let src = test_image(300, 200, few);
            let opt = ImgOpt { quality: 90, ..ImgOpt::from_bytes(&[]) };
            let mut out = Vec::new();
            let e = img_jxl::enc::encode(&src, &opt, &mut out);
            let (r, w, h) = if e == 0 { jxl_dec(&out, 0) } else { (e, 0, 0) };
            t.check(name, r == 0 && w == 300 && h == 200, format!("{} bytes: result {r}, {w} x {h}", out.len()));
        }
    }
    if t.on("jxr_rt") {
        let src = test_image(67, 45, false);
        let opt = ImgOpt { flags: IOF_LOSSLESS, ..ImgOpt::from_bytes(&[]) };
        let mut out = Vec::new();
        let e = img_jxr::enc::encode(&src, &opt, &mut out);
        let (r, sf) = if e == 0 { jxr_dec(&out, None) } else { (e, Surface::default()) };
        t.check("jxr_rt/lossless", r == 0 && same_pixels(&src, &sf), format!("{} bytes: result {r}, exact", out.len()));
    }
    plat::write_stdout(format!("hostile: {} of {} checks passed\n", t.ok, t.ok + t.failed).as_bytes());
    if t.failed != 0 { 1 } else { 0 }
}
