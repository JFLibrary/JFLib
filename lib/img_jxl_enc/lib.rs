// img_jxl_enc -- the new JPEG XL writer (jxl-plus: a port of libjxl 0.11.1's encoder; jxl-tools,
// jxl-speed), split out of img_jxl so that PaintCraft can load it as a module (jxl-module)
//                                                                       [jxl-plus: no asm]
//
//   use_static()                   the app saves JPEG XL with this writer, linked in (codectest,
//                                  the check tools): img_jxl::writer::set_static(enew::encode_px)
//   enew::encode / encode_px / encode_with_splines   the writer itself (options: the header
//                                  of img_jxl/lib.rs)
//
// Who links what: img_jxl (reader, the assembly-compatible writer, the writer's options and
// samples in writer.rs) is in every app that registers JPEG XL; this crate only where the
// app calls use_static() or one of its functions (codectest).  PaintCraft does not: it calls
// img_jxl::writer::use_module(), and the module jxlenc (modules/jxlenc: this crate built
// as a no_std cdylib over the module runtime) runs encode_px on the samples the host read.
// The output is the same byte for byte either way, for any number of threads.
//
// Modules (moved from img_jxl, same names; `crate::<decoder module>` paths resolve through
// the re-exports below): enew (options, lossless frame), evdct (VarDCT frame), emod (modular
// streams, transforms), ema (MA tree learning), eans (entropy coding: ANS / prefix codes,
// clustering, LZ77), efrm (headers, TOC), eicc (ICC stream), etx (forward transforms, quant
// tables), dctk (the transforms as simd kernels; jxl-speed), eaq (adaptive quantization,
// gaborish), eacs (AC strategy search), ecfl (chroma from luma); jxl-tools: epatch (patch
// search, packing, dictionary), edots (dots for the dictionary), espline (test-only spline
// writer), enoise (photon noise LUT), efeat (the frame's feature sections), ebutter
// (Butteraugli), ebq (the effort 8 / 9 quantization loop).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

// the reader's modules under their old paths (crate::frm, crate::tab, ...)
pub use img_jxl::{
    bits, blend, brotli, dct, dec, emath, enc, ent, epf, ewr, frm, hdr, icc, jpeg, modular, mtr, noise, par, rnd, spline,
    tab, vac, vdc, venc, vqt, vrc, writer,
};
pub use img_jxl::{decode_linear, Recon};

pub mod dctk;
pub mod eacs;
pub mod eans;
pub mod eaq;
pub mod ebq;
pub mod ebutter;
pub mod ecfl;
pub mod edots;
pub mod efeat;
pub mod efrm;
pub mod eicc;
pub mod ema;
pub mod emod;
pub mod enew;
pub mod enoise;
pub mod epatch;
pub mod espline;
pub mod etx;
pub mod evdct;

/// save JPEG XL with this writer, linked into the app (instead of the module file)
pub fn use_static() {
    img_jxl::writer::set_static(enew::encode_px);
}

