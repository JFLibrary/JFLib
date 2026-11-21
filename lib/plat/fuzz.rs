// fuzz -- the decoders of other programs' data, callable with any bytes: the entry points of
// plattest --test wirefuzz.  Not API (hidden): what they decode is reached through plat's own
// functions; these exist so a corpus of random and mutated bytes can reach every decoder
// without a compositor, a bus or another program.
//
//   clip_wide_text(b)   CF_UNICODETEXT of another program (Windows' clipboard)
//   hdrop(b)            a DROPFILES block (Windows' drops) -> the paths
//   dbus(b)             one D-Bus message (Linux: any client of the session bus can send one)
//   wl_args(msg)        one Wayland event under every event signature (Linux)
//
// clip_html_decode, clip_dib_to_bmp, clip_bmp_fix, clipimg_split and drop_uri_list, the other
// decoders of clipboard and drop data, are plat's API already.  docs/PLAT_INPUTS.md lists every
// way other programs' data comes in.

use alloc::string::String;
use alloc::vec::Vec;

/// CF_UNICODETEXT bytes -> UTF-8 (as Windows' clip_get(CLIP_TEXT))
#[doc(hidden)]
pub fn clip_wide_text(b: &[u8]) -> Option<Vec<u8>> {
    crate::clip_wide_text(b)
}

/// a DROPFILES block -> its paths (at most 4096, as a drop takes)
#[doc(hidden)]
pub fn hdrop(b: &[u8]) -> Vec<String> {
    crate::hdrop_list(b, 4096)
}

/// one D-Bus message from the start of b -> Ok(Some(its length)) / Ok(None) incomplete /
/// Err(bytes to skip, or 0: the connection must close)
#[doc(hidden)]
#[cfg(target_os = "linux")]
pub fn dbus(b: &[u8]) -> Result<Option<usize>, usize> {
    use crate::linux::linux_dbus::{decode, Bad};
    match decode(b) {
        Ok(m) => Ok(m.map(|(_, n)| n)),
        Err(Bad::Fatal) => Err(0),
        Err(Bad::Skip(n)) => Err(n),
    }
}

/// one Wayland event message (header and body) -> how many of the protocol's event
/// signatures take it
#[doc(hidden)]
#[cfg(target_os = "linux")]
pub fn wl_args(msg: &[u8]) -> usize {
    crate::linux::wl_fuzz_args(msg)
}
