// jxlref -- codectest --jxlref: JPEG XL files decoded against libjxl 0.7's pixels.  The files and
// libjxl's RGB8 decode lie in fixtures/jxl (NAME.jxl, NAME.rgb); a sample may differ by 1 (the
// decoders round differently, as in tools/jxl_plus_check.py's decode suite).  Exit code 1 when a
// file differs.
//
//   eWxH_SS_epfK   a JPEG of W x H with chroma subsampling SS (420, 422, 440) transcoded by libjxl
//                  (YCbCr, VarDCT), its frame header then given K EPF iterations and a sharpness
//                  LUT of 64.0 (libjxl writes JPEG frames without EPF, and their sharpness 0 would
//                  keep the filter off): the EPF runs on chroma planes upsampled to the frame's
//                  width, whose stride differs from plane 0's when the width is no multiple of
//                  16 (420 and 422 here; 440 is the control)
//
// (build_rs.py test codectest runs it through tests/jxlref.uis; --script is ignored)

use alloc::format;
use alloc::vec::Vec;

use img_core::{ImgDec, ImgMeta, Surface};

static FILES: [(&str, u32, u32, &[u8], &[u8]); 4] = [
    ("e40x24_420_epf1", 40, 24, include_bytes!("fixtures/jxl/e40x24_420_epf1.jxl"), include_bytes!("fixtures/jxl/e40x24_420_epf1.rgb")),
    ("e40x24_422_epf3", 40, 24, include_bytes!("fixtures/jxl/e40x24_422_epf3.jxl"), include_bytes!("fixtures/jxl/e40x24_422_epf3.rgb")),
    ("e40x24_440_epf2", 40, 24, include_bytes!("fixtures/jxl/e40x24_440_epf2.jxl"), include_bytes!("fixtures/jxl/e40x24_440_epf2.rgb")),
    ("e101x37_420_epf2", 101, 37, include_bytes!("fixtures/jxl/e101x37_420_epf2.jxl"), include_bytes!("fixtures/jxl/e101x37_420_epf2.rgb")),
];

/// the largest difference of a sample against the reference and how many differ by more than 1
fn compare(sf: &Surface, rgb: &[u8]) -> (u32, usize) {
    let (mut worst, mut off) = (0u32, 0usize);
    let s = sf.stride as usize;
    for y in 0..sf.h as usize {
        for x in 0..sf.w as usize {
            let p = &sf.px[y * s + x * 4..y * s + x * 4 + 4]; // B G R A
            let r = &rgb[(y * sf.w as usize + x) * 3..][..3];
            for (got, want) in [(p[2], r[0]), (p[1], r[1]), (p[0], r[2])] {
                let d = (got as i32 - want as i32).unsigned_abs();
                worst = worst.max(d);
                off += (d > 1) as usize;
            }
        }
    }
    (worst, off)
}

pub fn run() -> i32 {
    let (mut ok, mut failed) = (0u32, 0u32);
    let mut lines = Vec::new();
    for (name, w, h, jxl, rgb) in FILES {
        let (mut sf, mut m) = (Surface::default(), ImgMeta::default());
        let r = img_jxl::decode(jxl, &mut sf, &ImgDec::default(), &mut m);
        let good = r == 0 && sf.w == w as i32 && sf.h == h as i32 && rgb.len() == (w * h * 3) as usize;
        let (worst, off) = if good { compare(&sf, rgb) } else { (0, 0) };
        if good && worst <= 1 {
            ok += 1;
            lines.push(format!("ok   {name}: largest difference {worst}\n"));
        } else {
            failed += 1;
            lines.push(format!("FAIL {name}: result {r}, {} x {}, largest difference {worst}, {off} samples off by more than 1\n", sf.w, sf.h));
        }
    }
    for l in &lines {
        plat::write_stdout(l.as_bytes());
    }
    plat::write_stdout(format!("jxlref: {ok} of {} files match libjxl\n", ok + failed).as_bytes());
    if failed != 0 { 1 } else { 0 }
}
