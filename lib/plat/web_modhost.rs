// Browser module boundary. Static codec implementations keep their normal shared-library
// fallback; native DLL/SO encoder modules are neither loaded nor claimed available.

use alloc::{string::String, vec::Vec};
use core::sync::atomic::AtomicU32;

/// Empty because the browser does not load a native JPEG XL encoder module.
pub const JXLENC_FILE: &str = "";
/// No native module owns a browser address.
pub fn module_at(_address: u64) -> Option<(&'static str, u64)> {
    None
}
/// Native module paths do not activate code in the browser.
pub fn jxlenc_set_path(_path: Option<String>) {}
/// No native JPEG XL module path is available.
pub fn jxlenc_file() -> Option<String> {
    None
}
/// The native module encoder is unavailable; codecs may use their static fallback.
pub fn jxlenc_ready() -> bool {
    false
}
/// Shared JPEG XL request structure retained for the static fallback selection.
pub struct JxlEncReq<'a> {
    /// Pixel width.
    pub w: u32,
    /// Pixel height.
    pub h: u32,
    /// Grayscale source.
    pub gray: bool,
    /// Alpha is present.
    pub alpha: bool,
    /// Owned interleaved source pixels.
    pub rgba: Vec<u8>,
    /// Requested effort.
    pub effort: u32,
    /// Requested quality.
    pub quality: u32,
    /// Codec flags.
    pub flags: u32,
    /// Optional ICC bytes.
    pub icc: Option<&'a [u8]>,
    /// Optional EXIF bytes.
    pub exif: Option<&'a [u8]>,
    /// Optional XMP bytes.
    pub xmp: Option<&'a [u8]>,
    /// Optional cooperative cancellation flag.
    pub cancel: Option<&'a AtomicU32>,
}
/// No native module result; the caller must select its compiled-in codec.
pub fn jxlenc_encode(_request: JxlEncReq<'_>) -> Option<Result<Vec<u8>, i32>> {
    None
}
