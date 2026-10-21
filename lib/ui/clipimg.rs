// clipimg -- (K1) images on the clipboard                            [src/ui/ui_clipimg.s]
//
//   clip_set_image(sf) -> 0 / -1        sf (FMT_ARGB straight, FMT_PARGB or FMT_XRGB) as
//                                       CLIP_IMAGE: PNG (deflate level 1) + a 32-bit BMP
//                                       (BITMAPV5HEADER, BI_BITFIELDS, straight alpha)
//   clip_get_image() -> Ok(sf) / Err    the clipboard image (CLIP_IMAGE: PNG, else the DIB /
//                                       BMP) decoded into a new FMT_ARGB surface (straight,
//                                       like paint.net layers); ERR_FAIL (-1) = none
//   clip_bmp_from_sf(sf) -> BMP file    the BMP of clip_set_image
//
// A CLIP_IMAGE clipboard record (the asm's CLIPIMG: PNG and BMP pointers) travels as bytes:
// u64 PNG length (LE), the PNG, u64 BMP length, the BMP (clipimg_record / clipimg_parse);
// plat::clip_set(CLIP_IMAGE, ..) takes this record.

use alloc::vec;
use alloc::vec::Vec;

use super::consts::*;
use super::Ui;
use gfx::{Surface, FMT_ARGB, FMT_PARGB, FMT_XRGB};

const V5_SIZE: usize = 124;
const HDR_SIZE: usize = 14 + V5_SIZE;
const PNG_CLIP_LEVEL: u32 = 1;

/// the CLIP_IMAGE record: u64 PNG length, PNG, u64 BMP length, BMP
pub fn clipimg_record(png: Option<&[u8]>, bmp: Option<&[u8]>) -> Vec<u8> {
    let mut r = Vec::new();
    for part in [png, bmp] {
        let p = part.unwrap_or(&[]);
        r.extend_from_slice(&(p.len() as u64).to_le_bytes());
        r.extend_from_slice(p);
    }
    r
}

/// the parts of a CLIP_IMAGE record (an empty part = none); None when it is malformed
pub fn clipimg_parse(r: &[u8]) -> Option<(Option<&[u8]>, Option<&[u8]>)> {
    let mut parts = [None, None];
    let mut o = 0usize;
    for p in parts.iter_mut() {
        let n = u64::from_le_bytes(r.get(o..o + 8)?.try_into().ok()?) as usize;
        o += 8;
        let d = r.get(o..o.checked_add(n)?)?;
        o += n;
        *p = if n == 0 { None } else { Some(d) };
    }
    Some((parts[0], parts[1]))
}

/// clip_bmp_from_sf(sf) -> the BMP file of clip_set_image (None: an empty / huge surface or
/// another format)
pub fn clip_bmp_from_sf(sf: &Surface) -> Option<Vec<u8>> {
    let (w, h) = (sf.w as u32, sf.h as u32);
    if w.wrapping_sub(1) >= 65535 || h.wrapping_sub(1) >= 65535 {
        return None;
    }
    if sf.fmt != FMT_XRGB && sf.fmt != FMT_PARGB && sf.fmt != FMT_ARGB {
        return None;
    }
    let row = w as usize * 4;
    let size = row * h as usize + HDR_SIZE;
    let mut b = vec![0u8; size];
    let put32 = |b: &mut [u8], o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    b[0] = b'B';
    b[1] = b'M';
    put32(&mut b, 2, size as u32);
    put32(&mut b, 10, HDR_SIZE as u32);
    put32(&mut b, 14, V5_SIZE as u32);
    put32(&mut b, 18, w);
    put32(&mut b, 22, h); // bottom-up
    b[26] = 1;
    b[28] = 32;
    put32(&mut b, 30, 3); // BI_BITFIELDS
    put32(&mut b, 34, (size - HDR_SIZE) as u32);
    put32(&mut b, 38, 2835);
    put32(&mut b, 42, 2835);
    put32(&mut b, 54, 0x00FF_0000);
    put32(&mut b, 58, 0x0000_FF00);
    put32(&mut b, 62, 0x0000_00FF);
    put32(&mut b, 66, 0xFF00_0000);
    put32(&mut b, 70, 0x7352_4742); // LCS_sRGB
    put32(&mut b, 122, 4); // LCS_GM_IMAGES
    // rows bottom-up
    for y in 0..h as usize {
        let d = HDR_SIZE + (h as usize - y - 1) * row;
        let s = y * sf.stride as usize;
        let (dst, src) = (&mut b[d..d + row], &sf.px[s..s + row]);
        if sf.fmt == FMT_PARGB {
            gfx::straight::unpremul_row(dst, src);
        } else {
            dst.copy_from_slice(src);
            if sf.fmt == FMT_XRGB {
                for x in 0..w as usize {
                    dst[x * 4 + 3] = 255; // XRGB: alpha 255
                }
            }
        }
    }
    Some(b)
}

impl Ui {
    /// ui_clip_set_image(sf) -> 0 / -1: sf as CLIP_IMAGE (PNG level 1 + BMP)
    pub fn clip_set_image(&mut self, sf: &Surface) -> i32 {
        let png = img_png::enc::encode_classic(sf, PNG_CLIP_LEVEL);
        let bmp = clip_bmp_from_sf(sf);
        let Some(png) = png else { return -1 }; // no PNG = no memory: nothing is set
        let rec = clipimg_record(Some(&png), bmp.as_deref());
        self.clip_set(CLIP_IMAGE, &rec)
    }

    /// ui_clip_get_image -> the clipboard image in a new FMT_ARGB surface, or ERR_* (-1 =
    /// no image)
    pub fn clip_get_image(&mut self) -> Result<Surface, i32> {
        let Some(data) = self.clip_get(CLIP_IMAGE) else { return Err(-1) };
        let dec = img_core::ImgDec { flags: img_core::IMGD_STRAIGHT, frame: 0, hint: 0, maxpx: 0, maxmem: 0, cancel: None };
        img_core::io::decode_ex(&data, Some(&dec), None)
    }
}
