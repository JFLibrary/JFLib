// ft -- file types: the FileType trait, the safe views of a decode / encode call and the
// trampolines the host calls                 [ASMLIB/inc/plugin.inc FT_*, img.inc; PLUGINS.md §3]
//
//   trait FileType { const PROBE / DECODE / ENCODE; fn probe(data, meta); fn decode(data, d);
//                    fn encode(img, opt) -> Block }
//   FtDesc::new::<T>(name, exts, mime, caps)     (abi.rs) points at the trampolines
//   Decoder: flags() (IMGD_*), meta(), cancelled(), alloc(w, h) (PA_SURFALLOC: a zeroed
//            straight FMT_ARGB surface the host owns), row_mut(y), width / height / stride
//   Image:   the straight FMT_ARGB surface to encode: width, height, row(y)
//
// Same contract as the built-in codecs (IMGCODEC): any thread, possibly several calls at once,
// no globals.  probe: a magic check (fill meta when given).  decode: never read beyond the
// data; an error after alloc is fine (the host frees the surface).  encode: the output is a
// Block (PA_ALLOC), which the host frees.

use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use crate::{Block, ERR_FAIL, ERR_UNSUPPORTED, ImgDec, ImgMeta, ImgOpt, Surface, api};

/// a file type: implement it on a unit struct and describe it with FtDesc::new::<T>(..);
/// the consts say which functions it has (at least DECODE or ENCODE)
pub trait FileType: 'static {
    const PROBE: bool = false;
    const DECODE: bool = false;
    const ENCODE: bool = false;

    /// data starts with this format's signature (fill meta when given: w, h, bits, model,
    /// channels, flags)
    fn probe(data: &[u8], meta: Option<&mut ImgMeta>) -> bool {
        let _ = (data, meta);
        false
    }

    /// decode data into d (d.alloc(w, h), then the rows) -> Ok / Err(ERR_*)
    fn decode(data: &[u8], d: &mut Decoder<'_>) -> Result<(), i32> {
        let _ = (data, d);
        Err(ERR_UNSUPPORTED)
    }

    /// encode img -> the file's bytes in a Block (Block::new(cap), push / extend) / Err(ERR_*)
    fn encode(img: &Image<'_>, opt: &ImgOpt) -> Result<Block, i32> {
        let _ = (img, opt);
        Err(ERR_UNSUPPORTED)
    }
}

// a byte slice from the host's (data, len)
//
// # Safety
// data points at len readable bytes (or len is 0) for 'a.
unsafe fn data_slice<'a>(data: *const u8, len: usize) -> &'a [u8] {
    if data.is_null() || len == 0 || len > isize::MAX as usize {
        return &[];
    }
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(data, len) }
}

// ------------------------------------------------------------------ decoder

/// one decode call: the options and the host's SURFACE record to fill
pub struct Decoder<'a> {
    sf: *mut Surface,
    dec: *const ImgDec,
    /// after alloc: the surface
    s: Option<Surface>,
    _life: PhantomData<&'a mut Surface>,
}

impl<'a> Decoder<'a> {
    /// IMGD_* of the call (IMGD_PROBE: metadata only, pixels optional)
    pub fn flags(&self) -> u32 {
        // SAFETY: the host's IMGDEC (or null), alive during the call.
        unsafe { self.dec.as_ref() }.map_or(0, |d| d.flags)
    }

    /// the IMGMETA to fill (None: the host wants none)
    pub fn meta(&mut self) -> Option<&mut ImgMeta> {
        // SAFETY: ID_META of the host's IMGDEC: an IMGMETA owned by the host, not accessed by
        // anyone else during the call; &mut self keeps one reference at a time.
        unsafe { self.dec.as_ref().and_then(|d| d.meta.as_mut()) }
    }

    /// ID_CANCEL is set: return Err(ERR_CANCEL)
    pub fn cancelled(&self) -> bool {
        // SAFETY: ID_CANCEL points at a u32 the host may set at any time (or is null).
        unsafe { self.dec.as_ref().and_then(|d| d.cancel.as_ref()) }.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }

    /// PA_SURFALLOC: a zeroed straight FMT_ARGB surface w x h for the result (once; checked
    /// against the decode limits: ERR_DIMS / ERR_LIMIT / ERR_NOMEM)
    pub fn alloc(&mut self, w: i32, h: i32) -> Result<(), i32> {
        if self.s.is_some() {
            return Err(ERR_FAIL);
        }
        let a = api().ok_or(ERR_FAIL)?;
        let r = (a.raw().surfalloc)(self.sf, w, h, self.dec);
        if r != 0 {
            return Err(r);
        }
        // SAFETY: the host wrote the SURFACE record sf points at.
        let s = unsafe { *self.sf };
        let ok = !s.px.is_null()
            && s.px as usize % 4 == 0
            && s.w == w
            && s.h == h
            && w >= 0
            && h >= 0
            && s.stride as i64 >= w as i64 * 4
            && s.stride % 4 == 0;
        if !ok {
            return Err(ERR_FAIL);
        }
        self.s = Some(s);
        Ok(())
    }

    /// the size after alloc (0 before)
    pub fn width(&self) -> i32 {
        self.s.map_or(0, |s| s.w)
    }

    pub fn height(&self) -> i32 {
        self.s.map_or(0, |s| s.h)
    }

    /// row y of the result: width pixels 0xAARRGGBB (after alloc, 0 <= y < height, else a
    /// panic)
    pub fn row_mut(&mut self, y: i32) -> &mut [u32] {
        let s = self.s.expect("Decoder::row_mut before alloc");
        assert!(0 <= y && y < s.h, "Decoder::row_mut: y outside the image");
        let p = (s.px as usize).wrapping_add(y as usize * s.stride as usize);
        // SAFETY: row y of the surface PA_SURFALLOC made for this call (w pixels inside the
        // stride, aligned); only this call writes it, &mut self keeps one row alive at a time.
        unsafe { core::slice::from_raw_parts_mut(p as *mut u32, s.w as usize) }
    }
}

// ------------------------------------------------------------------ encoder input

/// the image to encode: straight FMT_ARGB (the host converts)
pub struct Image<'a> {
    s: Surface,
    _life: PhantomData<&'a Surface>,
}

impl<'a> Image<'a> {
    pub fn width(&self) -> i32 {
        self.s.w
    }

    pub fn height(&self) -> i32 {
        self.s.h
    }

    /// row y: width pixels 0xAARRGGBB (0 <= y < height, else a panic)
    pub fn row(&self, y: i32) -> &'a [u32] {
        assert!(0 <= y && y < self.s.h, "Image::row: y outside the image");
        let p = (self.s.px as usize).wrapping_add(y as usize * self.s.stride as usize);
        // SAFETY: row y of the host's surface (checked in encode_tramp), read-only during the
        // call.
        unsafe { core::slice::from_raw_parts(p as *const u32, self.s.w as usize) }
    }
}

// ------------------------------------------------------------------ trampolines

/// FT_PROBE of T
pub(crate) extern "win64" fn probe_tramp<T: FileType>(data: *const u8, len: usize, meta: *mut ImgMeta) -> u32 {
    // SAFETY: the host passes len readable bytes and its IMGMETA (or null) for the call.
    let (d, m) = unsafe { (data_slice(data, len), meta.as_mut()) };
    T::probe(d, m) as u32
}

/// FT_DECODE of T
pub(crate) extern "win64" fn decode_tramp<T: FileType>(data: *const u8, len: usize, sf: *mut Surface, dec: *const ImgDec) -> i32 {
    if sf.is_null() {
        return ERR_FAIL;
    }
    // SAFETY: the host passes len readable bytes for the call.
    let d = unsafe { data_slice(data, len) };
    let mut dc = Decoder { sf, dec, s: None, _life: PhantomData };
    match T::decode(d, &mut dc) {
        Ok(()) => 0,
        Err(e) => {
            if e < 0 {
                e
            } else {
                ERR_FAIL
            }
        }
    }
}

/// FT_ENCODE of T
pub(crate) extern "win64" fn encode_tramp<T: FileType>(sf: *const Surface, opt: *const ImgOpt, buf: *mut *mut u8, len: *mut usize) -> i32 {
    // SAFETY: the host's SURFACE and IMGOPT (or null), alive during the call.
    let (Some(s), Some(o)) = (unsafe { sf.as_ref() }, unsafe { opt.as_ref() }) else { return ERR_FAIL };
    let ok = !s.px.is_null() && s.px as usize % 4 == 0 && s.w >= 0 && s.h >= 0 && s.stride as i64 >= s.w as i64 * 4 && s.stride % 4 == 0;
    if !ok || buf.is_null() || len.is_null() {
        return ERR_FAIL;
    }
    let img = Image { s: *s, _life: PhantomData };
    match T::encode(&img, o) {
        Ok(b) => {
            let (p, n) = b.into_raw();
            // SAFETY: the host's out cells (checked for null above).
            unsafe {
                *buf = p;
                *len = n;
            }
            0
        }
        Err(e) => {
            if e < 0 {
                e
            } else {
                ERR_FAIL
            }
        }
    }
}
