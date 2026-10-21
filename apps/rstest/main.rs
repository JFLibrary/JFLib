// rstest -- headless test runner of the resampling / transform / mip / gradient / path data /
// dash kernels  [apps/rstest/src/main.s, ops_xf.s, ops_mip.s, ops_self.s, ops_grad.s,
// ops_dash.s]
//
// Driven by tools/resample_check.py, xform_check.py, gradient_check.py, pathdata_check.py and
// dash_check.py (the same protocol as the assembly version, so the tools run unchanged with
// --out build):
//
//   rstest --shot x.png --op NAME ARGS...     run one op
//   rstest --shot x.png --ops FILE            run every line of FILE ("NAME ARGS..."; empty
//                                             lines and lines starting with '#' skipped)
//   rstest --shot x.png --selftest            quick built-in checks (tests/selftest.uis)
//
// Every op prints one line: "ok NAME us=N" (N = microseconds per repetition of the timed
// part) or "err NAME CODE"; the exit code is 0 when every op succeeded, else 1.  Images are
// "RSR1" .raw files {u32 magic, u32 w, u32 h, u32 fmt, rows of w * bpp bytes}; any other
// name goes through img_core::io (load_ex with IMGD_STRAIGHT -> FMT_ARGB / save_opt; the
// core codecs of img_all).  The assembly keeps every surface between 0xCD guard bytes (err
// -200 when they change) and `stride N` forces huge strides; Rust slices cannot write past
// their buffer, so neither has a counterpart here (stride is accepted and ignored).
//
// Ops (integers decimal, colours / f64 bit patterns hex, f64 = 16 hex digits):
//   resize IN OUT W H FILTER FLAGS DFMT REPS SPLIT FILL
//        resample::resize into a W x H surface of DFMT filled with FILL; SPLIT > 1:
//        Resize::prepare + SPLIT row bands on the thread pool (rows)
//   weights S D FILTER OUT       axis_weights -> "RSW1", n, rb, nmax, id, D, D * rb bytes
//   mip IN OUT REPS SPLIT        mip_down of the whole image (SPLIT > 1: mip_down_rows bands)
//   mipchain IN PREFIX LEVELS    MipChain::alloc + update(None); level k -> PREFIX k ".raw"
//   mipinc BASE PATCH PREFIX X0 Y0 X1 Y1
//        chain of BASE, then the rectangle of PATCH copied into BASE and propagated with
//        update(rect); levels written like mipchain
//   affine IN OUT W H DFMT FILTER FLAGS FILL REPS SPLIT CX0 CY0 CX1 CY1 A B C D E F
//        xform::affine into a W x H surface of DFMT filled with FILL; CX0 = -1: no clip;
//        SPLIT > 1: Affine::prepare + SPLIT row bands on the thread pool
//   xfrot IN OUT W H DFMT FILTER FLAGS ANGLE TX TY
//        F = T(TX, TY) * R(ANGLE) from the identity with translate / rotate, then affine
//        with invert(F)
//   rot90 IN OUT DIR             ops::rotate90 (reference for the 90 degree test)
//   xfmat OUT TOKENS...          matrix helpers: starts with the identity; T tx ty, S sx sy,
//                                R angle cx cy, I (invert; singular -> err), M a b c d e f
//                                (m = m * that), P x y (append m.apply(x, y) to the output);
//                                OUT = the 6 f64 of m, then the applied points
//   sincos OUT ANGLE...          sincos of every angle -> sin, cos (f64) each
//   cubiclut OUT                 the 256 x 4 i16 bicubic phase table
//   grad IN OUT W H X0 Y0 TYPE REPEAT FLAGS COLA COLB SX SY EX EY REPS MODE   gfx::gradient
//   pdraster PATH OUT W H SCALE TX TY REPS     path data parsed, emitted, rasterized (+ OUT.txt)
//   dash PATH OUT W H WIDTH PAT CAP0 CAP1 DCAP JOIN OFFSET REPS             gfx::dash
//   pdfuzz IN OUT                              every line of IN parsed, emitted, stroked
//   stride N                                   (forced guard stride of the assembly: accepted)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering::SeqCst};

use base::Args;
use gfx::dash::{DASH_DASHDOT, DASH_MAXPAT, DStyle, LCAP_ARROW, LCAP_ARROW_FILLED, LCAP_ROUND, dash_pattern, dash_stroke};
use gfx::gradient::{GT_LINEAR, GT_SPIRAL_CW, Grad, grad_alpha_span, grad_span, grad_t_span};
use gfx::path::Path;
use gfx::pathdata::PData;
use gfx::raster::Raster;
use gfx::tmask::{TM_REPLACE, TMask};
use gfx::{ERR_NOMEM, FMT_A8, FMT_ARGB, FMT_PARGB, JOIN_MITER, RULE_NONZERO, Surface};
use img_core::io::{FileIo, load_ex, save_opt, set_file_io};
use img_core::mip::{MipChain, mip_down_rows};
use img_core::ops::{self, Band, bpp};
use img_core::resample::{RS_BICUBIC, RS_BICUBIC_SMOOTH, RS_BILINEAR, RS_FILTER_COUNT, RS_GAMMA, RS_NEAREST, Resize, axis_weights, resize};
use img_core::xform::{self, Affine, CUBIC_LUT, Matrix, affine, invert, sincos};
use img_core::{IMGD_STRAIGHT, ImgDec};
use rt::sync::SpinLock;

const MAXTOK: usize = 40;
const RAW_MAGIC: u32 = 0x3152_5352; // "RSR1"
const WGT_MAGIC: u32 = 0x3157_5352; // "RSW1"
const ERR_ARGS: i32 = -201;
const ERR_OP: i32 = -202;
const ERR_TRUNC: i32 = -1;
const ERR_HDR: i32 = -2;
const ERR_DIMS: i32 = -12;
const ERR_IO: i32 = -13;
const FZ_W: u32 = 256;
// fuzz transform: x / 4 + 32
const FZM: Matrix = Matrix { a: 0.25, b: 0.0, c: 32.0, d: 0.0, e: 0.25, f: 32.0 };

ui::main!(rstest());

fn rstest() -> i32 {
    set_file_io(FileIo { read: plat::read_file, write: plat::write_file });
    img_all::use_core_codecs(); // the codecs the assembly registers statically
    let argv = plat::args();
    let args = Args(argv.clone());
    if args.flag("--selftest") {
        return selftest();
    }
    let mut rt = Rt { us: 0, path: Path::default(), ras: Raster::default() };
    if let Some(f) = args.value("--ops") {
        return run_file(&mut rt, f);
    }
    // --op NAME ARGS...: the tokens after --op
    if let Some(i) = argv.iter().skip(1).position(|a| a == "--op") {
        let toks: Vec<&str> = argv[i + 2..].iter().take(MAXTOK).map(|s| s.as_str()).collect();
        if !toks.is_empty() {
            return dispatch(&mut rt, &toks);
        }
    }
    plat::write_stderr(b"usage: rstest --shot x.png (--op NAME ARGS... | --ops FILE | --selftest)\n");
    1
}

struct Rt {
    us: u64,
    path: Path,
    ras: Raster,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

// one op per line  [rt_run_file]
fn run_file(rt: &mut Rt, path: &str) -> i32 {
    let Some(text) = plat::read_file(path) else {
        plat::write_stderr(b"err file\n");
        return 1;
    };
    let mut fails = 0;
    for line in text.split(|&c| c == 10) {
        let toks: Vec<&str> = line
            .split(|&c| c <= 32)
            .filter(|t| !t.is_empty())
            .take(MAXTOK)
            .map(|t| core::str::from_utf8(t).unwrap_or(""))
            .collect();
        if toks.is_empty() || toks[0].starts_with('#') {
            continue;
        }
        fails |= dispatch(rt, &toks);
    }
    fails
}

type Op = fn(&mut Rt, &[&str]) -> i32;

// run one op and print its result line  [rt_dispatch]
fn dispatch(rt: &mut Rt, a: &[&str]) -> i32 {
    rt.us = 0;
    let f: Option<Op> = match a[0] {
        "resize" => Some(op_resize),
        "weights" => Some(op_weights),
        "mip" => Some(op_mip),
        "mipchain" => Some(op_mipchain),
        "mipinc" => Some(op_mipinc),
        "affine" => Some(op_affine),
        "xfrot" => Some(op_xfrot),
        "rot90" => Some(op_rot90),
        "xfmat" => Some(op_xfmat),
        "sincos" => Some(op_sincos),
        "cubiclut" => Some(op_cubiclut),
        "grad" => Some(op_grad),
        "stride" => Some(op_stride),
        "pdraster" => Some(op_pdraster),
        "dash" => Some(op_dash),
        "pdfuzz" => Some(op_pdfuzz),
        _ => None,
    };
    let r = match f {
        Some(f) => f(rt, a),
        None => ERR_OP,
    };
    if r == 0 {
        out(&format!("ok {} us={}\n", a[0], rt.us));
        0
    } else {
        out(&format!("err {} {}\n", a[0], r));
        1
    }
}

// ---------------------------------------------------------------- arguments

// parse_int: blanks, optional sign, decimal digits (wrapping) -> (value, rest)
fn parse_int(s: &[u8]) -> (i32, &[u8]) {
    let mut i = 0;
    while i < s.len() && s[i] == b' ' {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: u32 = 0;
    while i < s.len() && s[i].wrapping_sub(b'0') <= 9 {
        v = v.wrapping_mul(10).wrapping_add((s[i] - b'0') as u32);
        i += 1;
    }
    let v = if neg { v.wrapping_neg() } else { v };
    (v as i32, &s[i..])
}

fn int(a: &[&str], i: usize) -> i32 {
    parse_int(a[i].as_bytes()).0
}

// parse_hex: optional "#" / "0x", hex digits -> u32
fn hex(a: &[&str], i: usize) -> u32 {
    let mut s = a[i].as_bytes();
    if s.first() == Some(&b'#') {
        s = &s[1..];
    } else if s.len() >= 2 && s[0] == b'0' && s[1] | 32 == b'x' {
        s = &s[2..];
    }
    hex_digits(s) as u32
}

// up to 16 hex digits (stops at the first other character)  [rt_hex64]
fn hex64(a: &[&str], i: usize) -> u64 {
    hex_digits(a[i].as_bytes())
}

// an f64 argument (its bit pattern in hex)
fn f64a(a: &[&str], i: usize) -> f64 {
    f64::from_bits(hex64(a, i))
}

fn hex_digits(s: &[u8]) -> u64 {
    let mut v = 0u64;
    for &c in s {
        let d = if c.wrapping_sub(b'0') <= 9 {
            c - b'0'
        } else if (c | 32).wrapping_sub(b'a') <= 5 {
            (c | 32) - b'a' + 10
        } else {
            break;
        };
        v = v << 4 | d as u64;
    }
    v
}

// the reported time = total / max(reps, 1)  [rt_set_us]
fn set_us(rt: &mut Rt, total: u64, reps: i32) {
    let d = if reps == 0 { 1 } else { reps as u32 as u64 };
    rt.us = total / d;
}

// ---------------------------------------------------------------- images

// a w x h surface of fmt with packed rows, filled with fill (u32, or its low byte for
// FMT_A8)  [rt_alloc]
fn alloc(w: i32, h: i32, fmt: u8, fill: u32) -> Result<Surface, i32> {
    if w as u32 > 65535 || h as u32 > 65535 {
        return Err(ERR_DIMS);
    }
    let rb = w as usize * bpp(fmt);
    let n = w as usize * h as usize;
    let px = if fmt == FMT_A8 { vec![fill as u8; n] } else { fill.to_le_bytes().repeat(n) };
    Ok(Surface { px, w, h, stride: rb as i32, fmt, flags: 0, opac: 255, blend: 0 })
}

// row y of a surface (w * bpp bytes)
fn row_of(sf: &mut Surface, y: i32) -> &mut [u8] {
    let (o, n) = (y as usize * sf.stride as usize, sf.w as usize * bpp(sf.fmt));
    &mut sf.px[o..o + n]
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

// ".raw" file or any image (straight ARGB) into a surface  [rt_load]
fn load(path: &str) -> Result<Surface, i32> {
    if !path.ends_with(".raw") {
        let dec = ImgDec { flags: IMGD_STRAIGHT, ..ImgDec::default() };
        let img = load_ex(path, Some(&dec), None)?;
        let mut sf = alloc(img.w, img.h, FMT_ARGB, 0)?;
        let n = img.w as usize * 4;
        for y in 0..img.h {
            let o = y as usize * img.stride as usize;
            row_of(&mut sf, y).copy_from_slice(&img.px[o..o + n]);
        }
        return Ok(sf);
    }
    let b = plat::read_file(path).ok_or(ERR_IO)?;
    if b.len() < 16 || u32_at(&b, 0) != RAW_MAGIC {
        return Err(ERR_HDR);
    }
    let mut sf = alloc(u32_at(&b, 4) as i32, u32_at(&b, 8) as i32, u32_at(&b, 12) as u8, 0)?;
    let n = sf.w as usize * bpp(sf.fmt) * sf.h as usize;
    if n + 16 > b.len() {
        return Err(ERR_TRUNC);
    }
    sf.px.copy_from_slice(&b[16..16 + n]);
    Ok(sf)
}

// surface into a ".raw" file (rows of w * bpp bytes) or any image format (from the
// extension)  [rt_save]
fn save(path: &str, sf: &Surface) -> i32 {
    if !path.ends_with(".raw") {
        return save_opt(path, sf, None);
    }
    let rb = sf.w as usize * bpp(sf.fmt);
    let mut b = Vec::with_capacity(16 + rb * sf.h as usize);
    for v in [RAW_MAGIC, sf.w as u32, sf.h as u32, sf.fmt as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for y in 0..sf.h as usize {
        let o = y * sf.stride as usize;
        b.extend_from_slice(&sf.px[o..o + rb]);
    }
    if plat::write_file(path, &b) { 0 } else { ERR_IO }
}

// ---------------------------------------------------------------- bands on the thread pool

// f(band, y0, y1) over k row bands i * H / k .. (i + 1) * H / k on the thread pool (k <= 1:
// one call on this thread); the result ORs the bands' errors  [rt_bands]
fn bands(dst: &mut Surface, k: i32, f: &(dyn Fn(&mut Band, i32, i32) -> i32 + Sync)) -> i32 {
    let h = dst.h;
    if k <= 1 {
        return f(&mut Band::whole(dst), 0, h);
    }
    let cells: Vec<SpinLock<Option<Band>>> = ops::bands(dst, k as usize).into_iter().map(|b| SpinLock::new(Some(b))).collect();
    let err = AtomicI32::new(0);
    let _ = plat::parallel_for(cells.len(), &|i, _| {
        let band = cells[i].lock().take();
        if let Some(mut b) = band {
            let (y0, y1) = (b.y0, b.y1);
            let r = f(&mut b, y0, y1);
            if r != 0 {
                err.fetch_or(r, SeqCst);
            }
        }
    });
    err.load(SeqCst)
}

// ---------------------------------------------------------------- resize

// resize IN OUT W H FILTER FLAGS DFMT REPS SPLIT FILL  [op_resize]
fn op_resize(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 11 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (filter, flags, reps, split) = (int(a, 5) as u32, int(a, 6) as u32, int(a, 8), int(a, 9));
    let mut dst = match alloc(int(a, 3), int(a, 4), int(a, 7) as u8, hex(a, 10)) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let t0 = plat::time_us();
    for _ in 0..reps.max(0) {
        let r = if split > 1 {
            match Resize::prepare(&src, &dst, filter, flags) {
                Ok(c) => bands(&mut dst, split, &|b, y0, y1| c.rows(b, y0, y1)),
                Err(e) => e,
            }
        } else {
            resize(&src, &mut dst, filter, flags)
        };
        if r != 0 {
            return r;
        }
    }
    set_us(rt, plat::time_us() - t0, reps);
    save(a[2], &dst)
}

// weights S D FILTER OUT  [op_weights]
fn op_weights(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 5 {
        return ERR_ARGS;
    }
    let d = int(a, 2);
    let aw = match axis_weights(int(a, 1), d, int(a, 3) as u32) {
        Ok(w) => w,
        Err(e) => return e,
    };
    let mut b = Vec::with_capacity(24 + aw.tab.len());
    for v in [WGT_MAGIC, aw.n, aw.rb, aw.nmax, aw.id as u32, d as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&aw.tab);
    if plat::write_file(a[4], &b) { 0 } else { ERR_IO }
}

// stride N: the assembly's forced guard stride; nothing to do for Rust buffers  [op_stride]
fn op_stride(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 2 { ERR_ARGS } else { 0 }
}

// ---------------------------------------------------------------- mip

// mip IN OUT REPS SPLIT  [op_mip]
fn op_mip(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 5 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (reps, split) = (int(a, 3), int(a, 4));
    let mut dst = match alloc(((src.w as u32 + 1) >> 1) as i32, ((src.h as u32 + 1) >> 1) as i32, FMT_PARGB, 0) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let t0 = plat::time_us();
    for _ in 0..reps.max(0) {
        let r = bands(&mut dst, split, &|b, y0, y1| mip_down_rows(&src, b, 0, src.w, y0, y1));
        if r != 0 {
            return r;
        }
    }
    set_us(rt, plat::time_us() - t0, reps);
    save(a[2], &dst)
}

// every level to PREFIX k ".raw"  [mc_save]
fn mc_save(chain: &MipChain, prefix: &str) -> i32 {
    for k in 1..=chain.n {
        let Some(sf) = chain.level(k) else { break };
        let r = save(&format!("{}{}.raw", prefix, k), sf);
        if r != 0 {
            return r;
        }
    }
    0
}

// mipchain IN PREFIX LEVELS  [op_mipchain]
fn op_mipchain(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 4 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some(mut chain) = MipChain::alloc(src.w, src.h, int(a, 3)) else { return ERR_NOMEM };
    let t0 = plat::time_us();
    let r = chain.update(&src, None);
    set_us(rt, plat::time_us() - t0, 1);
    if r != 0 {
        return r;
    }
    mc_save(&chain, a[2])
}

// mipinc BASE PATCH PREFIX X0 Y0 X1 Y1  [op_mipinc]
fn op_mipinc(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 8 {
        return ERR_ARGS;
    }
    let mut base = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let patch = match load(a[2]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let rect = [int(a, 4), int(a, 5), int(a, 6), int(a, 7)];
    let Some(mut chain) = MipChain::alloc(base.w, base.h, 0) else { return ERR_NOMEM };
    let r = chain.update(&base, None);
    if r != 0 {
        return r;
    }
    // the patch rectangle into the base (pixels outside either image skipped)
    for y in rect[1]..rect[3] {
        for x in rect[0]..rect[2] {
            if (x as u32) < base.w.min(patch.w) as u32 && (y as u32) < base.h.min(patch.h) as u32 {
                let (bo, po) = (y as usize * base.stride as usize + x as usize * 4, y as usize * patch.stride as usize + x as usize * 4);
                base.px[bo..bo + 4].copy_from_slice(&patch.px[po..po + 4]);
            }
        }
    }
    let t0 = plat::time_us();
    let r = chain.update(&base, Some(rect));
    set_us(rt, plat::time_us() - t0, 1);
    if r != 0 {
        return r;
    }
    mc_save(&chain, a[3])
}

// ---------------------------------------------------------------- transforms

// affine IN OUT W H DFMT FILTER FLAGS FILL REPS SPLIT CX0 CY0 CX1 CY1 A B C D E F  [op_affine]
fn op_affine(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 21 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (filter, flags, reps, split) = (int(a, 6) as u32, int(a, 7) as u32, int(a, 9), int(a, 10));
    let clip = [int(a, 11), int(a, 12), int(a, 13), int(a, 14)];
    let m = Matrix { a: f64a(a, 15), b: f64a(a, 16), c: f64a(a, 17), d: f64a(a, 18), e: f64a(a, 19), f: f64a(a, 20) };
    let mut dst = match alloc(int(a, 3), int(a, 4), int(a, 5) as u8, hex(a, 8)) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let clip = if clip[0] == -1 { None } else { Some(clip) };
    let t0 = plat::time_us();
    for _ in 0..reps.max(0) {
        let r = if split > 1 {
            match Affine::prepare(&src, &dst, &m, filter, flags, clip) {
                Ok(xa) => bands(&mut dst, split, &|b, y0, y1| xa.rows(b, y0, y1)),
                Err(e) => e,
            }
        } else {
            affine(&src, &mut dst, &m, filter, flags, clip)
        };
        if r != 0 {
            return r;
        }
    }
    set_us(rt, plat::time_us() - t0, reps);
    save(a[2], &dst)
}

// xfrot IN OUT W H DFMT FILTER FLAGS ANGLE TX TY  [op_xfrot]
fn op_xfrot(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 11 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut dst = match alloc(int(a, 3), int(a, 4), int(a, 5) as u8, 0) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let mut f = Matrix::IDENTITY;
    xform::translate(&mut f, f64a(a, 9), f64a(a, 10));
    xform::rotate(&mut f, f64a(a, 8), 0.0, 0.0);
    let Some(inv) = invert(&f) else { return ERR_ARGS };
    let r = affine(&src, &mut dst, &inv, int(a, 6) as u32, int(a, 7) as u32, None);
    if r != 0 {
        return r;
    }
    save(a[2], &dst)
}

// rot90 IN OUT DIR  [op_rot90]
fn op_rot90(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 4 {
        return ERR_ARGS;
    }
    let src = match load(a[1]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match ops::rotate90(&src, int(a, 3)) {
        Ok(d) => save(a[2], &d),
        Err(_) => ERR_NOMEM,
    }
}

// xfmat OUT TOKENS...  [op_xfmat]
fn op_xfmat(_: &mut Rt, a: &[&str]) -> i32 {
    let argc = a.len();
    if argc < 2 {
        return ERR_ARGS;
    }
    let mut m = Matrix::IDENTITY;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut i = 2;
    while i < argc {
        match a[i].as_bytes()[0] {
            b'T' | b'S' => {
                if i + 2 >= argc {
                    return ERR_ARGS;
                }
                if a[i].as_bytes()[0] == b'T' {
                    xform::translate(&mut m, f64a(a, i + 1), f64a(a, i + 2));
                } else {
                    xform::scale(&mut m, f64a(a, i + 1), f64a(a, i + 2));
                }
                i += 3;
            }
            b'R' => {
                if i + 3 >= argc {
                    return ERR_ARGS;
                }
                xform::rotate(&mut m, f64a(a, i + 1), f64a(a, i + 2), f64a(a, i + 3));
                i += 4;
            }
            b'I' => {
                let Some(v) = invert(&m) else { return ERR_ARGS };
                m = v;
                i += 1;
            }
            b'M' => {
                if i + 6 >= argc {
                    return ERR_ARGS;
                }
                let b = Matrix { a: f64a(a, i + 1), b: f64a(a, i + 2), c: f64a(a, i + 3), d: f64a(a, i + 4), e: f64a(a, i + 5), f: f64a(a, i + 6) };
                m = xform::mul(&m, &b);
                i += 7;
            }
            b'P' => {
                if i + 2 >= argc || pts.len() >= 16 {
                    return ERR_ARGS;
                }
                pts.push(m.apply(f64a(a, i + 1), f64a(a, i + 2)));
                i += 3;
            }
            _ => return ERR_ARGS,
        }
    }
    let mut b = Vec::new();
    for v in [m.a, m.b, m.c, m.d, m.e, m.f] {
        b.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    for (x, y) in pts {
        b.extend_from_slice(&x.to_bits().to_le_bytes());
        b.extend_from_slice(&y.to_bits().to_le_bytes());
    }
    if plat::write_file(a[1], &b) { 0 } else { ERR_IO }
}

// sincos OUT A1 [A2 ...]: (sin, cos) f64 pairs of the angles (f64 bit patterns)  [op_sincos]
fn op_sincos(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 3 || a.len() > 34 {
        return ERR_ARGS;
    }
    let mut b = Vec::new();
    for i in 2..a.len() {
        let (s, c) = sincos(f64a(a, i));
        b.extend_from_slice(&s.to_bits().to_le_bytes());
        b.extend_from_slice(&c.to_bits().to_le_bytes());
    }
    if plat::write_file(a[1], &b) { 0 } else { ERR_IO }
}

// cubiclut OUT  [op_cubiclut]
fn op_cubiclut(_: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 2 {
        return ERR_ARGS;
    }
    let b: Vec<u8> = CUBIC_LUT.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    if plat::write_file(a[1], &b) { 0 } else { ERR_IO }
}

// ---------------------------------------------------------------- gradient

// grad IN OUT W H X0 Y0 TYPE REPEAT FLAGS COLA COLB SX SY EX EY REPS MODE  [op_grad]
fn op_grad(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 18 {
        return ERR_ARGS;
    }
    let (mode, reps) = (int(a, 17), int(a, 16));
    let (w, h, x0, y0) = (int(a, 3), int(a, 4), int(a, 5), int(a, 6));
    let g = Grad {
        typ: int(a, 7) as u8,
        repeat: int(a, 8) as u8,
        flags: int(a, 9) as u16,
        col_a: hex(a, 10),
        col_b: hex(a, 11),
        sx: f64a(a, 12),
        sy: f64a(a, 13),
        ex: f64a(a, 14),
        ey: f64a(a, 15),
    };
    let img = if a[1] == "-" { alloc(w, h, if mode == 2 { FMT_A8 } else { FMT_ARGB }, 0) } else { load(a[1]) };
    let mut img = match img {
        Ok(i) => i,
        Err(e) => return e,
    };
    let t0 = plat::time_us();
    let mut t = vec![0i32; img.w as usize];
    for _ in 0..reps.max(0) {
        for j in 0..img.h {
            let (iw, y) = (img.w as usize, y0.wrapping_add(j));
            let row = row_of(&mut img, j);
            match mode {
                2 => grad_alpha_span(&mut row[..iw], x0, y, &g),
                3 => {
                    grad_t_span(&mut t, x0, y, &g);
                    for (d, v) in row.chunks_exact_mut(4).zip(&t) {
                        d.copy_from_slice(&v.to_le_bytes());
                    }
                }
                _ => grad_span(&mut row[..iw * 4], x0, y, &g),
            }
        }
    }
    set_us(rt, plat::time_us() - t0, reps);
    save(a[2], &img)
}

// ---------------------------------------------------------------- path data, dashes

// read + parse (ERR_IO: no file)  [od_parse_file]
fn parse_file(path: &str, pd: &mut PData) -> (i32, usize) {
    pd.clear();
    match plat::read_file(path) {
        Some(t) => pd.parse(&t),
        None => (ERR_IO, 0),
    }
}

// the path into tm (replace) and tm into the A8 image  [od_raster]
fn raster(rt: &mut Rt, tm: &mut TMask, rule: u32, img: &mut Surface) -> i32 {
    let r = tm.raster_path(&rt.path, &mut rt.ras, rule, TM_REPLACE);
    if r != 0 {
        return r;
    }
    let w = img.w as usize;
    tm.store_a8(&mut img.px, w);
    0
}

// round(v * 1000) clamped to +-2e9 (cvtsd2si)  [od_bounds_int]
fn bounds_int(v: f64) -> i32 {
    rt::fx::cvtd(rt::fx::minsd(rt::fx::maxsd(v * 1000.0, -2000000000.0), 2000000000.0))
}

// pdraster PATH OUT W H SCALE TX TY REPS  [op_pdraster]
fn op_pdraster(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 9 {
        return ERR_ARGS;
    }
    let (w, h) = (int(a, 3) as u32, int(a, 4) as u32);
    let sc = int(a, 5) as f64 / 1000.0;
    let m = Matrix { a: sc, b: 0.0, c: int(a, 6) as f64 / 1000.0, d: 0.0, e: sc, f: int(a, 7) as f64 / 1000.0 };
    let mut pd = PData::new();
    let t0 = plat::time_us();
    let (code, off) = parse_file(a[1], &mut pd);
    set_us(rt, plat::time_us() - t0, 1);
    let (_, bb) = pd.bounds_box(None);
    let Some(mut tm) = TMask::new(w, h) else { return ERR_NOMEM };
    let mut img = match alloc(w as i32, h as i32, FMT_A8, 0) {
        Ok(i) => i,
        Err(e) => return e,
    };
    rt.path.reset();
    pd.emit(&mut rt.path, Some(&m));
    let r = raster(rt, &mut tm, pd.rule, &mut img);
    if r != 0 {
        return r;
    }
    let r = save(a[2], &img);
    if r != 0 {
        return r;
    }
    let b: Vec<i32> = bb.iter().map(|&v| bounds_int(v)).collect();
    let txt = format!("{} {} {} {} {} {} {} {}\n", code, off, pd.ncmd, pd.rule, b[0], b[1], b[2], b[3]);
    plat::write_file(&format!("{}.txt", a[2]), txt.as_bytes());
    0
}

// "-" solid, "dN" dash_pattern(N), else comma-separated entries / 1000  [od_pattern]
fn pattern(s: &str) -> Vec<f64> {
    let b = s.as_bytes();
    match b.first() {
        Some(b'-') => Vec::new(),
        Some(b'd') => dash_pattern(parse_int(&b[1..]).0 as u32).to_vec(),
        _ => {
            let mut v = Vec::new();
            let mut rest = b;
            while v.len() < DASH_MAXPAT {
                let (x, r) = parse_int(rest);
                v.push(x as f64 / 1000.0);
                if r.first() != Some(&b',') {
                    break;
                }
                rest = &r[1..];
            }
            v
        }
    }
}

// dash PATH OUT W H WIDTH PAT CAP0 CAP1 DCAP JOIN OFFSET REPS  [op_dash]
fn op_dash(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 13 {
        return ERR_ARGS;
    }
    let (w, h, reps) = (int(a, 3) as u32, int(a, 4) as u32, int(a, 12).max(1));
    let pat = pattern(a[6]);
    let st = DStyle {
        width: int(a, 5) as f64 / 1000.0,
        pat: &pat,
        cap0: int(a, 7) as u32,
        cap1: int(a, 8) as u32,
        dcap: int(a, 9) as u32,
        join: int(a, 10) as u32,
        offset: int(a, 11) as f64 / 1000.0,
        miter: 0.0,
    };
    let mut pd = PData::new();
    let (code, _) = parse_file(a[1], &mut pd);
    if code != 0 {
        return code;
    }
    let Some(mut tm) = TMask::new(w, h) else { return ERR_NOMEM };
    let mut img = match alloc(w as i32, h as i32, FMT_A8, 0) {
        Ok(i) => i,
        Err(e) => return e,
    };
    // REPS x (emit + stroke) timed, the last one rasterized
    let t0 = plat::time_us();
    for _ in 0..reps {
        rt.path.reset();
        pd.emit(&mut rt.path, None);
        let r = dash_stroke(&mut rt.path, &st);
        if r != 0 {
            return r;
        }
    }
    set_us(rt, plat::time_us() - t0, reps);
    let r = raster(rt, &mut tm, RULE_NONZERO, &mut img);
    if r != 0 {
        return r;
    }
    save(a[2], &img)
}

// pdfuzz IN OUT: every line parsed, emitted, filled, dash-stroked and filled again into a
// 256 x 256 mask; OUT gets "code offset ncmd" per line  [op_pdfuzz]
fn op_pdfuzz(rt: &mut Rt, a: &[&str]) -> i32 {
    if a.len() < 3 {
        return ERR_ARGS;
    }
    let Some(input) = plat::read_file(a[1]) else { return ERR_IO };
    let Some(mut tm) = TMask::new(FZ_W, FZ_W) else { return ERR_NOMEM };
    let st = DStyle {
        width: 3.0,
        pat: dash_pattern(DASH_DASHDOT),
        cap0: LCAP_ARROW,
        cap1: LCAP_ARROW_FILLED,
        dcap: LCAP_ROUND,
        join: JOIN_MITER,
        offset: 0.0,
        miter: 0.0,
    };
    let mut res = String::new();
    let mut pos = 0;
    while pos < input.len() {
        let start = pos;
        let mut e = pos;
        while e < input.len() && input[e] != 10 {
            e += 1;
        }
        pos = e + 1;
        let mut line = &input[start..e];
        if line.last() == Some(&13) {
            line = &line[..line.len() - 1]; // drop a CR
        }
        let mut pd = PData::new();
        let (mut code, off) = pd.parse(line);
        if code == 0 {
            rt.path.reset();
            pd.emit(&mut rt.path, Some(&FZM));
            tm.raster_path(&rt.path, &mut rt.ras, pd.rule, TM_REPLACE);
            rt.path.reset();
            pd.emit(&mut rt.path, Some(&FZM));
            code = dash_stroke(&mut rt.path, &st);
            if code == 0 {
                code = tm.raster_path(&rt.path, &mut rt.ras, RULE_NONZERO, TM_REPLACE);
            }
        }
        res.push_str(&format!("{} {} {}\n", code, off, pd.ncmd));
    }
    plat::write_file(a[2], res.as_bytes());
    0
}

// ---------------------------------------------------------------- self test  [ops_self.s]
//
//   1. resize to the same size is an exact copy for every interpolating filter
//   2. a constant image stays constant under every filter (up / down, straight / gamma)
//   3. affine with the identity (bilinear, bicubic) is an exact copy; a 90 degree turn built
//      with translate / rotate / invert equals ops::rotate90 (nearest, bilinear)
//   4. a mip chain of a constant image is constant on every level
//   5. grad_span: start pixel = A, end pixel = B, middle = (A + B + 1) / 2 per channel;
//      start == end gives B

// deterministic pseudo random premultiplied pixels (LCG)  [st_fill]
fn st_fill(sf: &mut Surface, seed: u32) {
    let mut s = seed;
    for y in 0..sf.h {
        for px in row_of(sf, y).chunks_exact_mut(4) {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            let a = s >> 24;
            let c = |k: u32| ((s >> k) & 255).min(a) << k;
            px.copy_from_slice(&(a << 24 | c(16) | c(8) | c(0)).to_le_bytes());
        }
    }
}

// every pixel = v  [st_const]
fn st_const(sf: &mut Surface, v: u32) {
    for y in 0..sf.h {
        for px in row_of(sf, y).chunks_exact_mut(4) {
            px.copy_from_slice(&v.to_le_bytes());
        }
    }
}

// same size and pixels  [st_eq]
fn st_eq(a: &Surface, b: &Surface) -> bool {
    if a.w != b.w || a.h != b.h {
        return false;
    }
    let n = a.w as usize * 4;
    (0..a.h as usize).all(|y| {
        let (oa, ob) = (y * a.stride as usize, y * b.stride as usize);
        a.px[oa..oa + n] == b.px[ob..ob + n]
    })
}

// every pixel is v  [st_isconst]
fn st_isconst(sf: &Surface, v: u32) -> bool {
    let n = sf.w as usize * 4;
    (0..sf.h as usize).all(|y| {
        let o = y * sf.stride as usize;
        sf.px[o..o + n].chunks_exact(4).all(|p| u32_at(p, 0) == v)
    })
}

fn st_fail(what: &str) -> i32 {
    plat::write_stderr(format!("selftest FAILED: {}\n", what).as_bytes());
    1
}

fn selftest() -> i32 {
    // ---- 1. identity resize, both formats
    for fmt in [FMT_PARGB, FMT_ARGB] {
        let mut a = Surface::new(11, 7, fmt);
        st_fill(&mut a, 77);
        let mut b = Surface::new(11, 7, fmt);
        for f in (0..RS_FILTER_COUNT).filter(|&f| f != RS_BICUBIC_SMOOTH) {
            if resize(&a, &mut b, f, 0) != 0 || !st_eq(&a, &b) {
                return st_fail("identity resize");
            }
        }
    }
    // ---- 2. constant images stay constant
    let mut a = Surface::new(13, 11, FMT_ARGB);
    st_const(&mut a, 0xFF80_4020);
    let mut b = Surface::new(5, 17, FMT_ARGB);
    let mut c = Surface::new(29, 3, FMT_ARGB);
    for k in 0..RS_FILTER_COUNT * 2 {
        let (f, fl) = (k >> 1, k & RS_GAMMA);
        if resize(&a, &mut b, f, fl) != 0 || !st_isconst(&b, 0xFF80_4020) || resize(&a, &mut c, f, fl) != 0 || !st_isconst(&c, 0xFF80_4020) {
            return st_fail("constant resize");
        }
    }
    // ---- 3. affine identity and 90 degree turn
    let mut a = Surface::new(9, 6, FMT_PARGB);
    st_fill(&mut a, 5);
    let mut b = Surface::new(9, 6, FMT_PARGB);
    for f in [RS_BILINEAR, RS_BICUBIC] {
        if affine(&a, &mut b, &Matrix::IDENTITY, f, 0, None) != 0 || !st_eq(&a, &b) {
            return st_fail("affine identity / rotate 90");
        }
    }
    // rotate90 clockwise into b; affine with invert(T(h, 0) R(pi / 2)) into c
    let Ok(b) = ops::rotate90(&a, 1) else { return st_fail("affine identity / rotate 90") };
    let mut c = Surface::new(6, 9, FMT_PARGB);
    let mut m = Matrix::IDENTITY;
    xform::translate(&mut m, 6.0, 0.0); // the source height
    xform::rotate(&mut m, f64::from_bits(0x3FF9_21FB_5444_2D18), 0.0, 0.0); // pi / 2
    let Some(m) = invert(&m) else { return st_fail("affine identity / rotate 90") };
    for f in [RS_NEAREST, RS_BILINEAR] {
        if affine(&a, &mut c, &m, f, 0, None) != 0 || !st_eq(&b, &c) {
            return st_fail("affine identity / rotate 90");
        }
    }
    // ---- 4. mip chain of a constant image
    let mut a = Surface::new(37, 5, FMT_ARGB);
    st_const(&mut a, 0xFF33_6699);
    let Some(mut chain) = MipChain::alloc(37, 5, 0) else { return st_fail("mip chain") };
    if chain.update(&a, None) != 0 {
        return st_fail("mip chain");
    }
    for k in 1..=chain.n {
        if !chain.level(k).is_some_and(|l| st_isconst(l, 0xFF33_6699)) {
            return st_fail("mip chain");
        }
    }
    if chain.n != 6 {
        return st_fail("mip chain"); // 37 x 5 -> 1 x 1 takes 6 levels
    }
    // ---- 5. gradient end points
    let half = f64::from_bits(0x3FE0_0000_0000_0000); // 0.5
    let mut g = Grad {
        typ: GT_LINEAR,
        col_a: 0x1020_4080,
        col_b: 0xF0E0_C0A1,
        sx: half,
        sy: half,
        ey: half,
        ex: f64::from_bits(0x4025_0000_0000_0000), // 10.5
        ..Grad::default()
    };
    let px = |b: &[u8], i: usize| u32_at(b, i * 4);
    let mut b = [0u8; 64];
    grad_span(&mut b, 0, 0, &g);
    let mut ok = px(&b, 0) == 0x1020_4080 && px(&b, 10) == 0xF0E0_C0A1;
    ok &= px(&b, 5) == 0x8080_8091 && px(&b, 15) == 0xF0E0_C0A1; // (A + B + 1) / 2; clamped
    g.ex = g.sx; // start == end: colour B
    g.typ = GT_SPIRAL_CW;
    grad_span(&mut b[..12], -5, 7, &g);
    ok &= px(&b, 2) == 0xF0E0_C0A1;
    if !ok {
        return st_fail("gradient");
    }
    out("selftest ok\n");
    0
}
