// img_all -- registration of the codec crates in the format table  [src/img/imgreg.s, imgio.s]
//
//   use_core_codecs()   PNG, JPEG, BMP, GIF (the assembly registers them statically: call it
//                       once at start-up, before worker threads decode)
//   use_all_codecs()    every codec of the library, the core formats included
//                       (img_use_all_codecs)
//
// Each codec crate exports `pub static CODEC: img_core::Codec`; the codec agents add one
// registration line for their crate below.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
pub mod psd;

use img_core::io::register;
use img_core::*;

/// register PNG, JPEG, BMP and GIF
pub fn use_core_codecs() {
    register(IMGF_PNG, Some(&img_png::CODEC));
    register(IMGF_JPEG, Some(&img_jpeg::CODEC)); // R6-jpeg
    register(IMGF_BMP, Some(&img_bmp::CODEC)); // R6-bgt
    register(IMGF_GIF, Some(&img_gif::CODEC)); // R6-bgt
}

/// register every codec of the library (the core formats included)  [img_use_all_codecs]
pub fn use_all_codecs() {
    use_core_codecs();
    register(IMGF_TIFF, Some(&img_tiff::CODEC)); // R6-tiff
    register(IMGF_TGA, Some(&img_tga::CODEC)); // R6-bgt
    // R9: each agent enables its line when its codec works (an empty codec would change
    // the dispatcher's answers for files of that format)
    register(IMGF_DDS, Some(&img_dds::CODEC)); // r9-dds
    register(IMGF_WEBP, Some(&img_webp::CODEC)); // R9-webp
    register(IMGF_AVIF, Some(&img_avif::CODEC)); // R9-av1 (+ r9-av1enc: the writer)
    register(IMGF_HEIC, Some(&img_heif::CODEC)); // R9-heif
    register(IMGF_JXL, Some(&img_jxl::CODEC)); // R9-jxl
    register(IMGF_JXR, Some(&img_jxr::CODEC)); // R9-jxr
    register(IMGF_PDN, Some(&img_pdn::CODEC)); // r9-pdn
    register(IMGF_PSD, Some(&psd::CODEC));
}
