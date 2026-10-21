// codectest -- headless test runner for the image codecs              [apps/codectest/src/main.s]
//
// Driven by tools/codec_check.py and the <codec>_check.py tools (tools/imgtest_common.py), with
// the same protocol as the assembly version, so the tools run unchanged with --out build.
//
//   --batch IN --result OUT        run every record of a batch file
//   --bench-inflate FILE.z [--reps N]
//   --bench-deflate FILE [--level L] [--reps N]
//   --bench-png FILE.png [--reps N]  time png_decode and png_encode
//   --inflate IN --to OUT          zlib stream -> raw bytes
//   --deflate IN --to OUT [--level L]
//   --convert IN --to OUT          img_load + img_save (format from the extension)
//   --convert-ex IN --to OUT [--straight] [--quality Q]
//                                  img_load_ex (hint from the extension) + img_save_opt
//   --core-only                    leave the non-core codecs unregistered
//   --jxlenc FILE                  (jxl-module) save JPEG XL with the writer of the module file
//                                  FILE (jxlenc.dll / .so; "off" = the assembly-compatible
//                                  writer) instead of the linked one (img_jxl_enc)
//
// Batch file: "CTB1", then records {u32 op, u32 arg, u32 len, u8 data[len]}.
// Result file: "CTR1", then per record {i32 result, u32 len, u8 data[len]}.  An op handler
// records exactly one result per record (Ct::record); an unknown op records -100.  The
// assembly places inputs and fixed-size outputs at page ends to catch overruns; Rust slices
// cannot run past their end, so that has no counterpart here.
//
// Op ranges (one module each, owned by the R6 agents -- rewrite/AGENTS.md):
//     1..10  ops_base (zlib, crc, adler, png / bmp / img decode, png / bmp encode)   R6-core
//    20..39  ops_jpeg     R6-jpeg      40..59  ops_gif   R6-bgt     60..79  ops_tiff  R6-tiff
//    80..99  ops_tga      R6-bgt      240..259 ops_meta  R6-core   260..279 ops_png   R6-core
//   280..299 ops_bmp      R6-bgt      300..319 ops_zlib  R6-core   320..339 ops_icc   R6-core
//   100..119 ops_dds  120..139 ops_webp  140..159 ops_avif  160..179 ops_heic  180..199 ops_jxl
//   200..219 ops_jxr  220..239 ops_pdn   (R9 agents)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use base::Args;
use gfx::{FMT_A8, Surface};
use img_core::ImgMeta;

mod ops_base;
mod ops_bmp;
mod ops_dds;
mod ops_webp;
mod ops_avif;
mod ops_heic;
mod ops_jxl;
mod ops_jxr;
mod ops_pdn;
mod ops_gif;
mod ops_icc;
mod ops_jpeg;
mod ops_meta;
mod ops_png;
mod ops_tga;
mod ops_tiff;
mod ops_zlib;

pub const R_UNKNOWN: i32 = -100;

ui::main!(codectest());

/// the runner state shared by the op modules
pub struct Ct {
    pub out: Vec<u8>,     // the result file being built
    pub core_only: bool,  // --core-only
    pub args: Args,
}

impl Ct {
    /// append {i32 result, u32 n, data}  [rb_record]
    pub fn record(&mut self, result: i32, data: &[u8]) {
        self.out.extend_from_slice(&result.to_le_bytes());
        self.out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out.extend_from_slice(data);
    }

    /// record {result, {u32 w, h, fmt | flags << 8, packed rows}} (4 bytes per pixel, 1 for
    /// FMT_A8; no rows without pixels)  [ct_surface_out]
    pub fn surface_out(&mut self, result: i32, sf: &Surface) {
        let d = surface_bytes(sf);
        self.record(result, &d);
    }
}

/// {u32 w, h, fmt | flags << 8} + the packed rows of a surface
pub fn surface_bytes(sf: &Surface) -> Vec<u8> {
    let bpp = if sf.fmt == FMT_A8 { 1 } else { 4 };
    let rb = sf.w.max(0) as usize * bpp;
    let h = if sf.px.is_empty() { 0 } else { sf.h.max(0) as usize };
    let mut d = Vec::with_capacity(12 + rb * h);
    d.extend_from_slice(&(sf.w as u32).to_le_bytes());
    d.extend_from_slice(&(sf.h as u32).to_le_bytes());
    d.extend_from_slice(&(sf.fmt as u32 | (sf.flags as u32) << 8).to_le_bytes());
    let stride = sf.stride as usize;
    for y in 0..h {
        d.extend_from_slice(&sf.px[y * stride..y * stride + rb]);
    }
    d
}

/// a new surface from {u32 w, h, fmt, packed rows} (4 bytes per pixel, 1 for FMT_A8); None
/// when the data is short or the size invalid  [ct_surface_in]
pub fn surface_in(data: &[u8]) -> Option<Surface> {
    if data.len() < 12 {
        return None;
    }
    let (w, h, fmt) = (u32_at(data, 0), u32_at(data, 4), u32_at(data, 8));
    if w > 65536 || h > 65536 {
        return None; // surf_alloc fails
    }
    let mut sf = Surface::new(w as i32, h as i32, fmt as u8);
    if sf.px.is_empty() {
        return None;
    }
    let rb = w as usize * if sf.fmt == FMT_A8 { 1 } else { 4 };
    if h as usize * rb + 12 > data.len() {
        return None;
    }
    let stride = sf.stride as usize;
    for y in 0..h as usize {
        sf.px[y * stride..y * stride + rb].copy_from_slice(&data[12 + y * rb..12 + (y + 1) * rb]);
    }
    Some(sf)
}

/// the 80-byte meta record: IMGMETA bytes 0..39, u64 icclen, exiflen, xmplen, IMGMETA bytes
/// 88..103  [mo_meta_rec]
pub fn meta_rec(m: &ImgMeta) -> [u8; 80] {
    let b = m.to_bytes();
    let mut r = [0u8; 80];
    r[..40].copy_from_slice(&b[..40]);
    r[40..48].copy_from_slice(&b[48..56]);
    r[48..56].copy_from_slice(&b[64..72]);
    r[56..64].copy_from_slice(&b[80..88]);
    r[64..80].copy_from_slice(&b[88..104]);
    r
}

pub fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// plat::parallel_for for img_core (which does not depend on plat)
fn par(n: usize, f: &(dyn Fn(usize) + Sync)) {
    let _ = plat::parallel_for(n, &|i, _| f(i));
}

fn codectest() -> i32 {
    let args = Args(plat::args());
    img_core::io::set_file_io(img_core::io::FileIo { read: plat::read_file, write: plat::write_file });
    img_core::set_parallel(par); // the codecs' parallel parts (YUV bands, ICC grids) on the pool
    let core_only = args.flag("--core-only");
    if !core_only {
        img_all::use_all_codecs();
    } else {
        img_all::use_core_codecs();
    }
    // jxl-module: the JPEG XL writer linked in, or with --jxlenc FILE that of the module file
    // (as PaintCraft loads it; the check tools compare both)
    match args.value("--jxlenc") {
        Some(f) => img_jxl::writer::use_module(Some(f)),
        None => img_jxl_enc::use_static(),
    }
    let mut ct = Ct { out: Vec::new(), core_only, args };
    if let Some(inp) = ct.args.value("--batch").map(alloc::string::String::from) {
        return run_batch(&mut ct, &inp);
    }
    if let Some(code) = ops_base::run_mode(&mut ct) {
        return code;
    }
    plat::write_stderr(
        b"usage: codectest --shot x.png (--batch IN --result OUT | --bench-inflate F | --bench-deflate F | --bench-png F | --inflate IN --to OUT | --deflate IN --to OUT | --convert IN --to OUT)\n",
    );
    1
}

/// read a file or print "error: cannot read PATH"  [read_file]
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let r = plat::read_file(path);
    if r.is_none() {
        plat::write_stderr(format!("error: cannot read {path}\n").as_bytes());
    }
    r
}

// run every record of the batch file  [run_batch]
fn run_batch(ct: &mut Ct, inp: &str) -> i32 {
    let Some(outp) = ct.args.value("--result").map(alloc::string::String::from) else {
        plat::write_stderr(b"error: --batch needs --result OUT\n");
        return 1;
    };
    let Some(file) = read_file(inp) else {
        return 1;
    };
    if file.len() < 4 || &file[..4] != b"CTB1" {
        plat::write_stderr(b"error: malformed batch file\n");
        return 1;
    }
    ct.out = b"CTR1".to_vec();
    let mut pos = 4;
    let mut n = 0u32;
    while file.len() - pos >= 12 {
        let (op, arg, len) = (u32_at(&file, pos), u32_at(&file, pos + 4), u32_at(&file, pos + 8) as usize);
        if len > file.len() - pos - 12 {
            plat::write_stderr(b"error: malformed batch file\n");
            return 1;
        }
        let data = &file[pos + 12..pos + 12 + len];
        pos += 12 + len;
        let before = ct.out.len();
        let handled = match op {
            1..=19 => ops_base::op(ct, op, arg, data),
            20..=39 => ops_jpeg::op(ct, op, arg, data),
            40..=59 => ops_gif::op(ct, op, arg, data),
            60..=79 => ops_tiff::op(ct, op, arg, data),
            80..=99 => ops_tga::op(ct, op, arg, data),
            100..=119 => ops_dds::op(ct, op, arg, data),
            120..=139 => ops_webp::op(ct, op, arg, data),
            140..=159 => ops_avif::op(ct, op, arg, data),
            160..=179 => ops_heic::op(ct, op, arg, data),
            180..=199 => ops_jxl::op(ct, op, arg, data),
            200..=219 => ops_jxr::op(ct, op, arg, data),
            220..=239 => ops_pdn::op(ct, op, arg, data),
            240..=259 => ops_meta::op(ct, op, arg, data),
            260..=279 => ops_png::op(ct, op, arg, data),
            280..=299 => ops_bmp::op(ct, op, arg, data),
            300..=319 => ops_zlib::op(ct, op, arg, data),
            320..=339 => ops_icc::op(ct, op, arg, data),
            _ => false,
        };
        if !handled {
            ct.out.truncate(before);
            ct.record(R_UNKNOWN, &[]);
        }
        n += 1;
    }
    if !plat::write_file(&outp, &ct.out) {
        plat::write_stderr(format!("error: cannot write {outp}\n").as_bytes());
        return 1;
    }
    plat::write_stdout(format!("batch: {n} records\n").as_bytes());
    0
}
