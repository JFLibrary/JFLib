// fx -- effects and adjustments: the Effect trait, the safe view of a render / prepare call,
// and the trampolines the host calls                  [ASMLIB/inc/fx.inc; PLUGINS.md §3, §4]
//
//   trait Effect { type Aux; const PREPARE; fn render(fx, aux); fn prepare(fx) }
//   FxDesc::new::<E>(name, flags, category, props)    (abi.rs) points at the trampolines
//   Fx: width / height, the ROI (x0, y0, x1, y1), src_row(y) (the whole source row),
//       rows(y) (the ROI parts of the source and destination row y), dst_row(y), param_*(slot),
//       cancelled(), job()
//
// Rules of the host (fx.inc): render runs on many worker threads at once, each call for one
// ROI, and writes the destination only inside it; the result of a pixel depends only on the
// source, the parameters and aux (never on the ROI split); no writable globals; poll the
// cancel flag once per row.  Pixels are straight 0xAARRGGBB.  The destination of the Rust
// host is backed only inside the ROI -- Fx never hands out anything outside it.

use core::ffi::c_void;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use crate::{ERR_FAIL, ERR_NOMEM, FX_NSLOTS, FxArgs, Job, Surface, api};

/// an effect or adjustment: implement it on a unit struct and describe it with
/// FxDesc::new::<E>(..)
pub trait Effect: 'static {
    /// what prepare computes for render (FXA_AUX); `()` for effects without prepare
    type Aux: Sync + 'static;
    /// has a prepare step (called once per run on one worker before any render)
    const PREPARE: bool = false;

    /// write the destination inside the ROI -> Ok, Err(ERR_CANCEL) when cancelled, Err(ERR_*)
    fn render(fx: &mut Fx<'_>, aux: Option<&Self::Aux>) -> Result<(), i32>;

    /// global statistics, tables, ... over the whole source -> Ok(Some(aux)), Ok(None) (no aux)
    /// or Err(ERR_*) (only with PREPARE = true)
    fn prepare(fx: &Fx<'_>) -> Result<Option<Self::Aux>, i32> {
        let _ = fx;
        Ok(None)
    }
}

/// one call of render / prepare: the validated FXARGS
pub struct Fx<'a> {
    src: Surface,
    dst: Surface,
    /// the ROI: x0 <= x < x1, y0 <= y < y1 inside the image
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    param: *const u64,
    aux: *const c_void,
    job: *const Job,
    _life: PhantomData<&'a FxArgs>,
}

// SAFETY: through a shared Fx only the source (never written during the call), the parameter
// block, the ROI and the atomic cancel flag are read -- e.g. by parallel_for items of a
// prepare step; the destination needs &mut Fx.
unsafe impl Sync for Fx<'_> {}

impl<'a> Fx<'a> {
    /// the records behind a (None: a record the host could not have sent)
    ///
    /// # Safety
    /// a is the FXARGS of a host call (or null); the records stay valid during the call.
    unsafe fn from_raw(a: *const FxArgs) -> Option<Fx<'a>> {
        // SAFETY: the caller's contract; every pointer is checked for null first.
        let (a, src, dst) = unsafe {
            let a = a.as_ref()?;
            (a, *a.src.as_ref()?, *a.dst.as_ref()?)
        };
        let (w, h) = (src.w as i64, src.h as i64);
        let ok_src = !src.px.is_null()
            && src.px as usize % 4 == 0
            && w >= 0
            && h >= 0
            && src.stride as i64 >= w * 4
            && src.stride % 4 == 0;
        let ok_roi = 0 <= a.x0 && a.x0 <= a.x1 && a.x1 as i64 <= w && 0 <= a.y0 && a.y0 <= a.y1 && a.y1 as i64 <= h;
        if !ok_src || !ok_roi {
            return None;
        }
        Some(Fx {
            src,
            dst,
            x0: a.x0,
            y0: a.y0,
            x1: a.x1,
            y1: a.y1,
            param: a.param,
            aux: a.aux,
            job: a.job,
            _life: PhantomData,
        })
    }

    /// the image width (source and destination)
    pub fn width(&self) -> i32 {
        self.src.w
    }

    pub fn height(&self) -> i32 {
        self.src.h
    }

    /// the whole source row y (0 <= y < height, else a panic)
    pub fn src_row(&self, y: i32) -> &'a [u32] {
        assert!(0 <= y && y < self.src.h, "Fx::src_row: y outside the image");
        let p = (self.src.px as usize).wrapping_add(y as usize * self.src.stride as usize);
        // SAFETY: row y of the source surface: w pixels inside the stride (checked in
        // from_raw), 4-byte aligned, never written while the effect runs.
        unsafe { core::slice::from_raw_parts(p as *const u32, self.src.w as usize) }
    }

    /// the source pixel (x, y) (a panic outside the image)
    pub fn src_px(&self, x: i32, y: i32) -> u32 {
        assert!(0 <= x && x < self.src.w, "Fx::src_px: x outside the image");
        self.src_row(y)[x as usize]
    }

    /// the destination row y inside the ROI: index i is pixel x0 + i (y0 <= y < y1, else a
    /// panic)
    pub fn dst_row(&mut self, y: i32) -> &mut [u32] {
        assert!(self.y0 <= y && y < self.y1, "Fx::dst_row: y outside the ROI");
        let n = (self.x1 - self.x0) as usize;
        let d = &self.dst;
        assert!(!d.px.is_null() && d.stride as i64 >= n as i64 * 4 && d.stride % 4 == 0, "Fx::dst_row: no destination");
        let p = (d.px as usize)
            .wrapping_add((y as isize as usize).wrapping_mul(d.stride as isize as usize))
            .wrapping_add(self.x0 as usize * 4);
        assert!(p % 4 == 0, "Fx::dst_row: unaligned destination");
        // SAFETY: the host backs the destination inside the ROI (fx.inc); this call writes no
        // other pixels, other threads get disjoint ROIs, and &mut self keeps two rows of this
        // call from being alive at once.
        unsafe { core::slice::from_raw_parts_mut(p as *mut u32, n) }
    }

    /// the ROI parts of source and destination row y (index i = pixel x0 + i)
    pub fn rows(&mut self, y: i32) -> (&'a [u32], &mut [u32]) {
        let (x0, x1) = (self.x0 as usize, self.x1 as usize);
        let s = &self.src_row(y)[x0..x1];
        (s, self.dst_row(y))
    }

    /// the raw 8 bytes of parameter slot `slot` (0 when the host passed none)
    pub fn param(&self, slot: usize) -> u64 {
        assert!(slot < FX_NSLOTS, "Fx::param: slot >= FX_NSLOTS");
        if self.param.is_null() {
            return 0;
        }
        // SAFETY: the parameter block has FX_NSLOTS 8-byte slots (fx.inc); slot is checked.
        unsafe { self.param.add(slot).read_unaligned() }
    }

    /// FP_INT / FP_BOOL / FP_CHOICE / FP_SEED: the slot as i64
    pub fn param_int(&self, slot: usize) -> i64 {
        self.param(slot) as i64
    }

    /// FP_DEC / FP_ANGLE / FP_PAN: the slot as f64
    pub fn param_f64(&self, slot: usize) -> f64 {
        f64::from_bits(self.param(slot))
    }

    /// FP_BOOL: slot != 0
    pub fn param_bool(&self, slot: usize) -> bool {
        self.param(slot) != 0
    }

    /// FP_COLOR: the straight ARGB colour in the low half
    pub fn param_color(&self, slot: usize) -> u32 {
        self.param(slot) as u32
    }

    /// the run's cancel flag is set: return Err(ERR_CANCEL) (poll it once per row)
    pub fn cancelled(&self) -> bool {
        self.job().is_some_and(|j| j.cancel.load(Ordering::Relaxed) != 0)
    }

    /// the run's JOB (None: the host passed none)
    pub fn job(&self) -> Option<&'a Job> {
        // SAFETY: the host's JOB record (or null), alive during the call.
        unsafe { self.job.as_ref() }
    }
}

// ------------------------------------------------------------------ trampolines

/// FXD_RENDER of E
pub(crate) extern "win64" fn render_tramp<E: Effect>(a: *const FxArgs) -> i32 {
    // SAFETY: the host calls render with its FXARGS (render's contract).
    let Some(mut fx) = (unsafe { Fx::from_raw(a) }) else { return ERR_FAIL };
    // SAFETY: FXA_AUX is what prepare_tramp::<E> returned: an E::Aux in a host block (or 0).
    let aux = if E::PREPARE { unsafe { (fx.aux as *const E::Aux).as_ref() } } else { None };
    match E::render(&mut fx, aux) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// FXD_PREPARE of E: rax = aux | 0, edx = the error when rax is 0 (two return registers:
/// written in assembly around prepare_inner)
#[unsafe(naked)]
pub(crate) extern "win64" fn prepare_tramp<E: Effect>(a: *const FxArgs) -> *mut c_void {
    core::arch::naked_asm!(
        "sub rsp, 56",                 // shadow space + the error slot, rsp 16-aligned at the call
        "lea rdx, [rsp + 40]",
        "mov dword ptr [rdx], 0",
        "call {inner}",                // (args, &err) -> rax aux | 0
        "mov edx, dword ptr [rsp + 40]",
        "add rsp, 56",
        "ret",
        inner = sym prepare_inner::<E>,
    )
}

extern "win64" fn prepare_inner<E: Effect>(a: *const FxArgs, err: *mut i32) -> *mut c_void {
    let (p, e) = prepare_run::<E>(a);
    // SAFETY: err points at the slot of prepare_tramp's frame.
    unsafe { *err = e };
    p
}

fn prepare_run<E: Effect>(a: *const FxArgs) -> (*mut c_void, i32) {
    // SAFETY: the host calls prepare with its FXARGS.
    let Some(fx) = (unsafe { Fx::from_raw(a) }) else { return (core::ptr::null_mut(), ERR_FAIL) };
    match E::prepare(&fx) {
        Ok(Some(v)) => {
            const { assert!(core::mem::align_of::<E::Aux>() <= 64, "Effect::Aux: alignment above 64") };
            let Some(api) = api() else { return (core::ptr::null_mut(), ERR_NOMEM) };
            let p = (api.raw().alloc)(core::mem::size_of::<E::Aux>().max(1)) as *mut E::Aux;
            if p.is_null() {
                return (core::ptr::null_mut(), ERR_NOMEM);
            }
            // SAFETY: a fresh host block of the size of E::Aux, 64-byte aligned.
            unsafe { p.write(v) };
            (p as *mut c_void, 0)
        }
        Ok(None) => (core::ptr::null_mut(), 0),
        Err(e) => (core::ptr::null_mut(), if e < 0 { e } else { ERR_FAIL }),
    }
}

/// FXD_FREE of E: drop the aux and free its block
pub(crate) extern "win64" fn free_tramp<E: Effect>(aux: *mut c_void) {
    if aux.is_null() {
        return;
    }
    // SAFETY: aux is a block prepare_run::<E> filled with an E::Aux; the host frees it once,
    // after every render of the run.
    unsafe { core::ptr::drop_in_place(aux as *mut E::Aux) };
    if let Some(a) = api() {
        (a.raw().free)(aux as *mut u8);
    }
}
