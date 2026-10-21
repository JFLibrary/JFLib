// ops_base -- codectest ops 1..10 and the non-batch modes            [apps/codectest/src/main.s]
//
//   op 1 zlib_inflate  arg = output capacity  -> output bytes
//   op 2 inflate_raw   arg = output capacity  -> output bytes
//   op 3 zlib_deflate  arg = level            -> compressed stream
//   op 4 crc32         arg = initial crc      -> u32
//   op 5 adler32       arg = initial adler    -> u32
//   op 6 png_decode                           -> u32 w, u32 h, w*h PARGB u32
//   op 7 bmp_decode                           -> same
//   op 8 png_encode    data = u32 w, h, fmt, pixels (4 or 1 bytes, packed) -> file bytes
//   op 9 bmp_encode    data = same            -> file bytes
//   op 10 img_decode                          -> like op 6
// run_mode: --bench-inflate / --bench-deflate / --bench-png / --inflate / --deflate /
// --convert-ex / --convert (the stdout / stderr lines of the assembly).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Ct, read_file, u32_at};
use gfx::{FMT_A8, Surface};
use img_core::checksum::{adler32, crc32};
use img_core::deflate::zlib_deflate;
use img_core::inflate::{inflate_raw, zlib_inflate};
use img_core::io;
use img_core::{ERR_NOMEM, ERR_OVERFLOW, IMGD_STRAIGHT, IMGF_BMP, ImgDec, ImgOpt};

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        1 | 2 => {
            let mut out = vec![0u8; arg as usize];
            let r = if op == 1 { zlib_inflate(data, &mut out) } else { inflate_raw(data, &mut out) };
            match r {
                Ok(n) => ct.record(n as i32, &out[..n]),
                Err(e) => ct.record(e, &[]),
            }
        }
        3 => {
            let z = zlib_deflate(data, arg as i32);
            ct.record(0, &z);
        }
        4 => ct.record(0, &crc32(arg, data).to_le_bytes()),
        5 => ct.record(0, &adler32(arg, data).to_le_bytes()),
        6 | 7 | 10 => {
            let mut sf = Surface::default();
            let r = match op {
                6 => img_png::decode_fl(data, &mut sf, 0),
                7 => match io::codec(IMGF_BMP).and_then(|c| c.decode_fl) {
                    Some(f) => f(data, &mut sf, 0),
                    None => return false, // BMP not registered: unknown op
                },
                _ => match io::decode(data) {
                    Ok(s) => {
                        sf = s;
                        0
                    }
                    Err(e) => e,
                },
            };
            if r != 0 {
                ct.record(r, &[]);
            } else {
                let (w, h) = (sf.w.max(0) as usize, sf.h.max(0) as usize);
                let mut d = Vec::with_capacity(8 + w * h * 4);
                d.extend_from_slice(&(w as u32).to_le_bytes());
                d.extend_from_slice(&(h as u32).to_le_bytes());
                let stride = sf.stride as usize;
                for y in 0..h {
                    d.extend_from_slice(&sf.px[y * stride..y * stride + w * 4]);
                }
                ct.record(0, &d);
            }
        }
        8 | 9 => {
            let save = if op == 8 {
                Some(png_save as fn(&Surface, u32) -> Option<Vec<u8>>)
            } else {
                match io::codec(IMGF_BMP).and_then(|c| c.save) {
                    Some(f) => Some(f),
                    None => return false, // BMP not registered: unknown op
                }
            };
            let file = enc_surface(data).and_then(|sf| save.and_then(|f| f(&sf, 0)));
            match file {
                Some(f) => ct.record(0, &f),
                None => ct.record(ERR_NOMEM, &[]),
            }
        }
        _ => return false,
    }
    true
}

fn png_save(sf: &Surface, _q: u32) -> Option<Vec<u8>> {
    img_png::enc::encode_default(sf)
}

// the surface of ops 8 / 9: {u32 w, h, fmt, packed rows} (rows copied without a length
// check in the assembly; short data here: no surface)
fn enc_surface(data: &[u8]) -> Option<Surface> {
    if data.len() < 12 {
        return None;
    }
    let (w, h, fmt) = (u32_at(data, 0), u32_at(data, 4), u32_at(data, 8));
    if w > 65536 || h > 65536 {
        return None;
    }
    let mut sf = Surface::new(w as i32, h as i32, fmt as u8);
    if sf.px.is_empty() {
        return None;
    }
    let rb = w as usize * if sf.fmt == FMT_A8 { 1 } else { 4 };
    if 12 + rb * h as usize > data.len() {
        return None;
    }
    let stride = sf.stride as usize;
    for y in 0..h as usize {
        sf.px[y * stride..y * stride + rb].copy_from_slice(&data[12 + y * rb..12 + (y + 1) * rb]);
    }
    Some(sf)
}

/// the non-batch modes (--inflate, --deflate, --convert, --bench-*): Some(exit code)
pub fn run_mode(ct: &mut Ct) -> Option<i32> {
    let a = &ct.args;
    let v = |n: &str| a.value(n).map(String::from);
    if let Some(p) = v("--bench-inflate") {
        return Some(bench_inflate(ct, &p));
    }
    if let Some(p) = v("--bench-deflate") {
        return Some(bench_deflate(ct, &p));
    }
    if let Some(p) = v("--bench-png") {
        return Some(bench_png(ct, &p));
    }
    if let Some(p) = v("--inflate") {
        return Some(cmd_inflate(ct, &p));
    }
    if let Some(p) = v("--deflate") {
        return Some(cmd_deflate(ct, &p));
    }
    if let Some(p) = v("--convert-ex") {
        return Some(cmd_convert_ex(ct, &p));
    }
    if let Some(p) = v("--convert") {
        return Some(cmd_convert(ct, &p));
    }
    None
}

fn out(s: String) {
    plat::write_stdout(s.as_bytes());
}

fn err(s: String) {
    plat::write_stderr(s.as_bytes());
}

// zlib_inflate into a buffer that doubles until the data fits (start: len * mul + 1 MB)
fn inflate_sized(src: &[u8], mul: usize) -> Result<Vec<u8>, i32> {
    let mut cap = src.len() * mul + (1 << 20);
    loop {
        let mut buf = vec![0u8; cap];
        match zlib_inflate(src, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return Ok(buf);
            }
            Err(ERR_OVERFLOW) => cap <<= 1,
            Err(e) => return Err(e),
        }
    }
}

// bench_inflate(path): time zlib_inflate on a zlib file
fn bench_inflate(ct: &Ct, path: &str) -> i32 {
    let Some(data) = read_file(path) else { return 1 };
    let reps = ct.args.int("--reps", 5).max(1) as u64;
    let sized = match inflate_sized(&data, 4) {
        Ok(v) => v,
        Err(e) => {
            err(format!("inflate error {e}\n"));
            return 1;
        }
    };
    let n = sized.len();
    // the sizing run's buffer: its pages are touched, as the assembly's buffer is (a fresh
    // zeroed vec would add first-touch page faults to the first repetition)
    let mut buf = sized;
    let t0 = plat::time_us();
    for _ in 0..reps {
        let _ = zlib_inflate(&data, &mut buf);
    }
    let us = (plat::time_us() - t0) / reps;
    out(format!("inflate in={} out={} us={}\n", data.len(), n, us));
    0
}

// bench_deflate(path): time zlib_deflate
fn bench_deflate(ct: &Ct, path: &str) -> i32 {
    let Some(data) = read_file(path) else { return 1 };
    let level = ct.args.int("--level", 6);
    let reps = ct.args.int("--reps", 3).max(1) as u64;
    let t0 = plat::time_us();
    let mut zl = 0;
    for _ in 0..reps {
        zl = zlib_deflate(&data, level).len();
    }
    let us = (plat::time_us() - t0) / reps;
    out(format!("deflate in={} out={} us={}\n", data.len(), zl, us));
    0
}

// bench_png(path): time png_decode and png_encode of a PNG file
fn bench_png(ct: &Ct, path: &str) -> i32 {
    let Some(data) = read_file(path) else { return 1 };
    let reps = ct.args.int("--reps", 5).max(1) as u64;
    let t0 = plat::time_us();
    let mut sf = Surface::default();
    for _ in 0..reps {
        let r = img_png::decode_fl(&data, &mut sf, 0);
        if r != 0 {
            err(format!("png_decode error {r}\n"));
            return 1;
        }
    }
    let dec_us = plat::time_us() - t0;
    let t0 = plat::time_us();
    let mut ol = 0;
    for _ in 0..reps {
        match img_png::enc::encode_default(&sf) {
            Some(f) => ol = f.len(),
            None => return 1,
        }
    }
    let enc_us = (plat::time_us() - t0) / reps;
    out(format!(
        "png w={} h={} in={} decode_us={} encode_us={} out={}\n",
        sf.w,
        sf.h,
        data.len(),
        dec_us / reps,
        enc_us,
        ol
    ));
    0
}

// cmd_inflate(path): zlib stream -> raw bytes in --to
fn cmd_inflate(ct: &Ct, path: &str) -> i32 {
    let Some(data) = read_file(path) else { return 1 };
    let raw = match inflate_sized(&data, 8) {
        Ok(v) => v,
        Err(e) => {
            err(format!("inflate error {e}\n"));
            return 1;
        }
    };
    let Some(to) = ct.args.value("--to") else { return 1 };
    if !plat::write_file(to, &raw) {
        return 1;
    }
    out(format!("inflated {} bytes\n", raw.len()));
    0
}

// cmd_deflate(path): --to gets the zlib stream at --level (default 6)
fn cmd_deflate(ct: &Ct, path: &str) -> i32 {
    let Some(data) = read_file(path) else { return 1 };
    let z = zlib_deflate(&data, ct.args.int("--level", 6));
    let Some(to) = ct.args.value("--to") else { return 1 };
    if !plat::write_file(to, &z) {
        return 1;
    }
    out(format!("deflated {} -> {} bytes\n", data.len(), z.len()));
    0
}

// cmd_convert_ex(path): img_load_ex (IMGD_STRAIGHT with --straight), img_save_opt to --to
// (quality from --quality)
fn cmd_convert_ex(ct: &Ct, path: &str) -> i32 {
    let dec = ImgDec { flags: if ct.args.flag("--straight") { IMGD_STRAIGHT } else { 0 }, ..Default::default() };
    let sf = match io::load_ex(path, Some(&dec), None) {
        Ok(s) => s,
        Err(e) => {
            err(format!("img_load_ex {path}: error {e}\n"));
            return 1;
        }
    };
    let Some(to) = ct.args.value("--to") else { return 1 };
    let opt = ImgOpt { quality: ct.args.int("--quality", 0) as u32, ..Default::default() };
    let r = io::save_opt(to, &sf, Some(&opt));
    if r != 0 {
        err(format!("img_save_opt {to}: error {r}\n"));
        return 1;
    }
    out(format!("converted {} ({}x{} fmt {}) -> {}\n", path, sf.w, sf.h, sf.fmt, to));
    0
}

// cmd_convert(path): img_load, img_save to --to
fn cmd_convert(ct: &Ct, path: &str) -> i32 {
    let sf = match io::load(path) {
        Ok(s) => s,
        Err(e) => {
            err(format!("img_load {path}: error {e}\n"));
            return 1;
        }
    };
    let Some(to) = ct.args.value("--to") else { return 1 };
    let r = io::save(to, &sf);
    if r != 0 {
        err(format!("img_save {to}: error {r}\n"));
        return 1;
    }
    out(format!("converted {} ({}x{}) -> {}\n", path, sf.w, sf.h, to));
    0
}
