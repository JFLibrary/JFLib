// dec -- AV1 intra decoder driver: OBUs, sequence / frame header, frame buffers, tile groups,
// the post filter chain                                                  [src/img/av1.s]
//
//   Av1Dec::new / set_cancel / set_stage / set_nograin / obus / finish / qindex  (lib.rs)
//
// The decoder record AV of the assembly is split into Hdr (sequence and frame header values,
// read-only while the tiles decode), Frame (the frame buffers and the per-4x4 / per-unit
// arrays) and the tile state Ts (blk.rs, one for every tile, kept between tiles like the
// assembly's single TS record).  The assembly's plane pointers that may alias each other
// (CdefFrame = CurrFrame without CDEF, the upscaled frames = the frames without superres) are
// flags here (Frame::cdef_own, Frame::sr) with accessor functions.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU32;

use img_core::{ERR_HDR, ERR_NOMEM, ERR_TRUNC, ERR_UNSUPPORTED};

use crate::blk::{self, Ts};
use crate::tab::*;
use crate::{Pic, AVS_CDEF, AVS_FINAL, AVS_LF, AVS_RECON, AVS_SR};

// ---- limits / constants of the specification used here  [av1_int.inc]
pub(crate) const AV_MAXTILES: u32 = 64; // tile columns / rows each
pub(crate) const KEY_FRAME: u32 = 0;
pub(crate) const INTRA_ONLY_FRAME: u32 = 2;
pub(crate) const SEG_LVL_ALT_Q: usize = 0;
pub(crate) const SEG_LVL_REF_FRAME: usize = 5;
pub(crate) const TX_MODE_LARGEST: u32 = 1;
pub(crate) const ONLY_4X4: u32 = 0;

/// film grain parameters (FG_*)
#[derive(Clone, Copy)]
pub(crate) struct Fg {
    pub seed: u32,
    pub ny: u32, // num_y_points
    pub ncb: u32,
    pub ncr: u32,
    pub csfl: u32,    // chroma_scaling_from_luma
    pub scaling: u32, // grain_scaling_minus_8 + 8
    pub lag: u32,     // ar_coeff_lag
    pub arshift: u32, // ar_coeff_shift_minus_6 + 6
    pub gshift: u32,  // grain_scale_shift
    pub cbmult: u32,
    pub cblmult: u32,
    pub cboff: u32,
    pub crmult: u32,
    pub crlmult: u32,
    pub croff: u32,
    pub overlap: u32,
    pub clip: u32, // clip_to_restricted_range
    pub py: [[u8; 2]; 14], // point_y value, scaling
    pub pcb: [[u8; 2]; 10],
    pub pcr: [[u8; 2]; 10],
    pub ary: [u8; 24], // ar_coeffs_y_plus_128
    pub arcb: [u8; 25],
    pub arcr: [u8; 25],
}

impl Fg {
    const ZERO: Fg = Fg {
        seed: 0,
        ny: 0,
        ncb: 0,
        ncr: 0,
        csfl: 0,
        scaling: 0,
        lag: 0,
        arshift: 0,
        gshift: 0,
        cbmult: 0,
        cblmult: 0,
        cboff: 0,
        crmult: 0,
        crlmult: 0,
        croff: 0,
        overlap: 0,
        clip: 0,
        py: [[0; 2]; 14],
        pcb: [[0; 2]; 10],
        pcr: [[0; 2]; 10],
        ary: [0; 24],
        arcb: [0; 25],
        arcr: [0; 25],
    };
}

/// sequence header and frame header (the AV_* values of the assembly)
#[derive(Clone)]
pub(crate) struct Hdr {
    // sequence header
    pub profile: u32,
    pub reduced: u32, // reduced_still_picture_header
    pub opidc: u32,   // OperatingPointIdc
    pub dmip: u32,    // decoder_model_info_present_flag
    pub eqpi: u32,    // equal_picture_interval
    pub bdlen: u32,   // buffer_delay_length_minus_1 + 1
    pub brtlen: u32,  // buffer_removal_time_length_minus_1 + 1
    pub fptlen: u32,  // frame_presentation_time_length_minus_1 + 1
    pub nop: u32,     // operating points
    pub fwbits: u32,
    pub fhbits: u32,
    pub maxw: u32,
    pub maxh: u32,
    pub fidp: u32,  // frame_id_numbers_present_flag
    pub idlen: u32, // idLen
    pub sb128: u32,
    pub efi: u32,   // enable_filter_intra
    pub eief: u32,  // enable_intra_edge_filter
    pub eoh: u32,   // enable_order_hint
    pub sfsct: u32, // seq_force_screen_content_tools
    pub sfimv: u32, // seq_force_integer_mv
    pub ohbits: u32,
    pub esr: u32, // enable_superres
    pub ecdef: u32,
    pub elr: u32,
    pub bd: u32,
    pub mono: u32,
    pub np: u32,
    pub cdp: u32,
    pub prim: u32,
    pub trc: u32,
    pub mc: u32,
    pub full: u32,
    pub ssx: u32,
    pub ssy: u32,
    pub csp: u32,
    pub sepuv: u32,
    pub fgp: u32, // film_grain_params_present
    pub opidcs: [u16; 32],
    pub dmpop: [u8; 32],
    // frame header
    pub ftype: u32,
    pub showf: u32,
    pub showable: u32,
    pub erm: u32,
    pub dcu: u32, // disable_cdf_update
    pub sct: u32, // allow_screen_content_tools
    pub fso: u32,
    pub intrabc: u32,
    pub fw: u32, // FrameWidth (downscaled)
    pub fh: u32,
    pub upw: u32, // UpscaledWidth
    pub rw: u32,
    pub rh: u32,
    pub usesr: u32,
    pub srden: u32,
    pub micols: u32,
    pub mirows: u32,
    pub tcols: u32,
    pub trows: u32,
    pub tclog2: u32,
    pub trlog2: u32,
    pub tsb: u32, // TileSizeBytes
    pub baseq: u32,
    pub dqydc: i32,
    pub dqudc: i32,
    pub dquac: i32,
    pub dqvdc: i32,
    pub dqvac: i32,
    pub useqm: u32,
    pub segen: u32,
    pub segpreskip: u32,
    pub lastseg: u32,
    pub dqpres: u32,
    pub dqres: u32,
    pub dlfpres: u32,
    pub dlfres: u32,
    pub dlfmulti: u32,
    pub codedll: u32,
    pub allll: u32,
    pub lfsharp: u32,
    pub lfdeltaen: u32,
    pub cdefdamp: u32,
    pub cdefbits: u32,
    pub useslr: u32,
    pub txmode: u32,
    pub redtx: u32,
    pub applygrain: u32,
    pub fen: [[u8; 8]; 8],    // FeatureEnabled
    pub fdata: [[i16; 8]; 8], // FeatureData
    pub llarr: [u8; 8],       // LosslessArray
    pub qmlvl: [[u8; 8]; 3],  // SegQMLevel
    pub lflvl: [u8; 4],
    pub lfrefd: [i8; 8],
    pub lfmoded: [i8; 2],
    pub cdefyp: [u8; 8],
    pub cdefys: [u8; 8],
    pub cdefuvp: [u8; 8],
    pub cdefuvs: [u8; 8],
    pub frt: [u8; 3], // FrameRestorationType
    pub lrsize: [u32; 3],
    pub mics: [u16; 66], // MiColStarts
    pub mirs: [u16; 66],
    pub fg: Fg,
}

/// the header bit reader: f(n) MSB first, zero bits at and past `end` (bytes).  Deviation:
/// zero bits also past the data itself.  In the assembly an OBU whose header / leb128 size
/// runs past the data end gets a payload end beyond it (AV_BEND > len), and ab_f then reads
/// the bytes after the input buffer (codectest: access violation, e.g. "12 00 0a 06 18 15 7f
/// bd a0 08 32 97"); here those bits are zero (ERR_TRUNC for that stream).
pub(crate) struct Br<'d> {
    d: &'d [u8],
    pub pos: u64, // bit position
    pub end: u64,
}

impl Br<'_> {
    /// ab_f: next n bits (zero bits past end)
    pub fn f(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let by = self.pos >> 3;
            let bit = if by < self.end && (by as usize) < self.d.len() {
                (self.d[by as usize] >> (7 - (self.pos & 7))) as u32 & 1
            } else {
                0
            };
            v = v.wrapping_shl(1) | bit;
            self.pos += 1;
        }
        v
    }

    /// ab_su: su(n), n = 1..32
    fn su(&mut self, n: u32) -> i32 {
        let v = self.f(n);
        let s = 32 - n;
        (v.wrapping_shl(s) as i32).wrapping_shr(s)
    }

    /// ab_ns: ns(n) (n >= 1)
    fn ns(&mut self, n: u32) -> u32 {
        let w = 32 - n.leading_zeros();
        let m = 1u32.wrapping_shl(w).wrapping_sub(n);
        let v = self.f(w - 1);
        if v < m {
            return v;
        }
        let e = self.f(1);
        e.wrapping_add(v.wrapping_mul(2)).wrapping_sub(m)
    }

    /// ab_leb128
    fn leb128(&mut self) -> u64 {
        let mut v = 0u64;
        let mut i = 0u32;
        loop {
            let b = self.f(8);
            v |= ((b & 0x7F) as u64).wrapping_shl(7 * i);
            i += 1;
            if b & 0x80 == 0 || i >= 8 {
                return v;
            }
        }
    }

    /// ab_uvlc (values above 2^32 - 1 saturate)
    fn uvlc(&mut self) -> u32 {
        let mut lz = 0u32;
        loop {
            if self.f(1) != 0 {
                break;
            }
            lz += 1;
            if lz >= 40 {
                return u32::MAX;
            }
        }
        if lz >= 32 {
            return u32::MAX;
        }
        let v = self.f(lz) as u64 + ((1u64 << lz) - 1);
        v.min(0xFFFF_FFFF) as u32
    }
}

/// a loop restoration unit (LRU_*)
#[derive(Clone, Copy, Default)]
pub(crate) struct Lru {
    pub typ: u8,              // RESTORE_*
    pub set: u8,              // LrSgrSet
    pub wiener: [[i8; 3]; 2], // LrWiener[ pass ][ coeff ]
    pub xqd: [i16; 2],        // LrSgrXqd
}

/// which planes are the output (setout)
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Out {
    Cur,
    Cdef,
    UpCdef,
    Lr,
    Grain,
}

/// the frame buffers (planes u16, rows padded to a multiple of 64 samples) and the per-frame arrays
pub(crate) struct Frame {
    pub pl: [Vec<u16>; 3], // CurrFrame
    pub ps: [u32; 3],      // stride (samples)
    pub ph: [u32; 3],      // rows
    pub cpl: [Vec<u16>; 3], // CdefFrame when cdef_own (else CurrFrame)
    pub cdef_own: bool,
    pub upl: [Vec<u16>; 3],  // upscaled CurrFrame (superres only)
    pub ucpl: [Vec<u16>; 3], // upscaled CdefFrame (superres only)
    pub sr: bool,
    pub ups: [u32; 3],        // stride of the upscaled planes
    pub lrpl: [Vec<u16>; 3],  // LrFrame
    pub gpl: [Vec<u16>; 3],   // film grain output
    pub out: Out,
    pub ops: [u32; 3], // output stride
    pub ow: u32,       // output width
    pub mi: Vec<[u8; 16]>, // MIB records (blk.rs MIB_*)
    pub mis: u32,          // MIB stride (records per row)
    pub mir: u32,          // MIB rows
    pub cdidx: Vec<i8>,    // cdef_idx per 64x64 block
    pub cds: u32,
    pub lru: [Vec<Lru>; 3],
    pub lrurows: [u32; 3], // restoration unit rows / columns
    pub lrucols: [u32; 3],
    pub lrns: [u32; 3], // loop restoration stripes per plane (0: plane not filtered)
}

impl Frame {
    pub(crate) fn new() -> Frame { // r9-av1enc: crate-visible (the encoder's tile frames)
        Frame {
            pl: [Vec::new(), Vec::new(), Vec::new()],
            ps: [0; 3],
            ph: [0; 3],
            cpl: [Vec::new(), Vec::new(), Vec::new()],
            cdef_own: false,
            upl: [Vec::new(), Vec::new(), Vec::new()],
            ucpl: [Vec::new(), Vec::new(), Vec::new()],
            sr: false,
            ups: [0; 3],
            lrpl: [Vec::new(), Vec::new(), Vec::new()],
            gpl: [Vec::new(), Vec::new(), Vec::new()],
            out: Out::Cur,
            ops: [0; 3],
            ow: 0,
            mi: Vec::new(),
            mis: 0,
            mir: 0,
            cdidx: Vec::new(),
            cds: 0,
            lru: [Vec::new(), Vec::new(), Vec::new()],
            lrurows: [0; 3],
            lrucols: [0; 3],
            lrns: [0; 3],
        }
    }

    /// CdefFrame plane p
    pub fn cdef_pl(&self, p: usize) -> &[u16] {
        if self.cdef_own { &self.cpl[p] } else { &self.pl[p] }
    }

    /// upscaled CdefFrame plane p
    pub fn upcdef_pl(&self, p: usize) -> &[u16] {
        if self.sr { &self.ucpl[p] } else { self.cdef_pl(p) }
    }

    /// output plane p
    pub fn out_pl(&self, p: usize) -> &[u16] {
        match self.out {
            Out::Cur => &self.pl[p],
            Out::Cdef => self.cdef_pl(p),
            Out::UpCdef => self.upcdef_pl(p),
            Out::Lr => &self.lrpl[p],
            Out::Grain => &self.gpl[p],
        }
    }
}

/// setout: the output = those planes; frame done
fn setout(f: &mut Frame, state: &mut u32, out: Out, strides: [u32; 3], w: u32) {
    f.out = out;
    f.ops = strides;
    f.ow = w;
    *state = 3;
}

/// a zeroed Vec of n elements, ERR_NOMEM when the memory is not there.  The probe allocation
/// fails where the assembly's mem_alloc (page run from the OS) fails; the real one comes from
/// the zeroed page allocator, so untouched pages are never written (as with mem_alloc).
pub(crate) fn zvec<T: Copy + Default>(n: usize) -> Result<Vec<T>, i32>
where
    Vec<T>: ZeroedVec,
{
    let mut probe: Vec<T> = Vec::new();
    probe.try_reserve_exact(n).map_err(|_| ERR_NOMEM)?;
    drop(probe);
    Ok(<Vec<T> as ZeroedVec>::zeroed(n))
}

/// vec![0; n] for the element types used here (alloc_zeroed: no clearing loop)
pub(crate) trait ZeroedVec {
    fn zeroed(n: usize) -> Self;
}
impl ZeroedVec for Vec<u16> {
    fn zeroed(n: usize) -> Self {
        vec![0u16; n]
    }
}
impl ZeroedVec for Vec<u8> {
    fn zeroed(n: usize) -> Self {
        vec![0u8; n]
    }
}
impl ZeroedVec for Vec<i8> {
    fn zeroed(n: usize) -> Self {
        vec![0i8; n]
    }
}
impl ZeroedVec for Vec<i32> {
    fn zeroed(n: usize) -> Self {
        vec![0i32; n]
    }
}
impl ZeroedVec for Vec<[u8; 16]> {
    fn zeroed(n: usize) -> Self {
        vec![[0u8; 16]; n]
    }
}

// r9-av1enc: the encoder reads the parsed headers and the frame geometry
impl Av1Dec<'_> {
    pub(crate) fn enc_view(&self) -> (u32, &Hdr, &Frame) {
        (self.state, &self.h, &self.f)
    }
}

/// the AV1 intra decoder (AV)
pub struct Av1Dec<'a> {
    cancel: Option<&'a AtomicU32>,
    stage: u32,
    state: u32, // 0 nothing, 1 sequence header, 2 frame header, 3 frame done
    err: i32,
    tid: u32, // temporal_id / spatial_id of the current OBU
    sid: u32,
    nograin: u32,
    ntile: u32, // tiles decoded so far
    h: Box<Hdr>,
    f: Frame,
    ts: Box<Ts>,
}

impl Default for Av1Dec<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Av1Dec<'a> {
    /// av1_new
    pub fn new() -> Av1Dec<'a> {
        Av1Dec {
            cancel: None,
            stage: 0,
            state: 0,
            err: 0,
            tid: 0,
            sid: 0,
            nograin: 0,
            ntile: 0,
            h: Box::new(Hdr::ZERO),
            f: Frame::new(),
            ts: Ts::new(),
        }
    }

    /// av1_set_cancel: a flag polled per superblock (!= 0 cancels)
    pub fn set_cancel(&mut self, c: Option<&'a AtomicU32>) {
        self.cancel = c;
    }

    /// av1_set_stage: test hook, AVS_* (output after that step)
    pub fn set_stage(&mut self, stage: u32) {
        self.stage = stage;
    }

    /// av1_set_nograin: 1 = do not apply film grain
    pub fn set_nograin(&mut self, flag: u32) {
        self.nograin = flag;
    }

    /// av1_qindex: get_qindex( curq < 0, seg ) with CurrentQIndex = curq
    pub fn qindex(&self, seg: u32, curq: i32) -> u32 {
        qindex(&self.h, seg, curq)
    }

    /// av1_obus: OBUs in the low-overhead format
    pub fn obus(&mut self, data: &[u8]) -> Result<(), i32> {
        if self.err != 0 {
            return Err(self.err);
        }
        match self.obus_in(data) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.err = e;
                Err(e)
            }
        }
    }

    fn obus_in(&mut self, data: &[u8]) -> Result<(), i32> {
        let len = data.len() as u64;
        let mut p = 0u64;
        loop {
            if p >= len || self.state == 3 {
                return Ok(()); // (3: first shown frame decoded)
            }
            let mut b = Br { d: data, pos: p * 8, end: len };
            b.f(1); // obu_forbidden_bit
            let typ = b.f(4);
            let ext = b.f(1);
            let has = b.f(1);
            b.f(1);
            self.tid = 0;
            self.sid = 0;
            if ext != 0 {
                self.tid = b.f(3);
                self.sid = b.f(2);
                b.f(3);
            }
            let sz = if has != 0 {
                b.leb128()
            } else {
                let t = len - p - 1;
                if t < ext as u64 {
                    return Err(ERR_TRUNC);
                }
                t - ext as u64
            };
            let ps = b.pos >> 3; // payload start
            if sz > len.wrapping_sub(ps) {
                return Err(ERR_TRUNC);
            }
            let pe = ps.wrapping_add(sz); // payload end
            b.end = pe;
            // operating point selection
            let mut skip = false;
            if typ != 1 && typ != 2 && ext != 0 && self.h.opidc != 0 {
                let o = self.h.opidc;
                if o >> (self.tid & 31) & 1 == 0 || o >> ((self.sid + 8) & 31) & 1 == 0 {
                    skip = true;
                }
            }
            if !skip {
                match typ {
                    1 => {
                        seqhdr(&mut self.h, &mut b)?;
                        if self.state < 1 {
                            self.state = 1;
                        }
                    }
                    2 => {
                        if self.state == 2 {
                            self.state = 1;
                        }
                    }
                    3 | 7 => {
                        if self.state != 2 {
                            self.framehdr(&mut b)?;
                        }
                    }
                    6 => {
                        if self.state == 2 {
                            return Err(ERR_UNSUPPORTED); // (a second frame header in an OBU_FRAME)
                        }
                        self.framehdr(&mut b)?;
                        let off = (b.pos + 7) >> 3;
                        self.tg_at(data, off, pe)?;
                    }
                    4 => {
                        if self.state != 2 {
                            return Err(ERR_HDR);
                        }
                        let off = b.pos >> 3;
                        self.tg_at(data, off, pe)?;
                    }
                    _ => {}
                }
            }
            p = pe;
        }
    }

    // the tile group data[off .. pe] (pe may lie past the data only for an empty payload)
    fn tg_at(&mut self, data: &[u8], off: u64, pe: u64) -> Result<(), i32> {
        if pe < off {
            return Err(ERR_TRUNC);
        }
        let len = data.len() as u64;
        let tg = &data[off.min(len) as usize..pe.min(len) as usize];
        let r = self.tilegroup(tg);
        if r != 0 { Err(r) } else { Ok(()) }
    }

    /// av_framehdr: uncompressed_header of an intra frame, then the frame buffers; state 2
    fn framehdr(&mut self, b: &mut Br) -> Result<(), i32> {
        if self.state == 0 {
            return Err(ERR_HDR);
        }
        let h = &mut *self.h;
        if h.reduced != 0 {
            h.ftype = KEY_FRAME;
            h.showf = 1;
            h.showable = 0;
            h.erm = 1;
        } else {
            if b.f(1) != 0 {
                return Err(ERR_UNSUPPORTED); // show_existing_frame
            }
            h.ftype = b.f(2);
            if h.ftype != KEY_FRAME && h.ftype != INTRA_ONLY_FRAME {
                return Err(ERR_UNSUPPORTED);
            }
            h.showf = b.f(1);
            if h.showf == 0 {
                return Err(ERR_UNSUPPORTED); // (only shown frames are output)
            }
            if h.dmip != 0 && h.eqpi == 0 {
                b.f(h.fptlen); // temporal_point_info
            }
            h.showable = (h.ftype != KEY_FRAME) as u32;
            h.erm = 1;
            if h.ftype != KEY_FRAME {
                h.erm = b.f(1);
            }
        }
        h.dcu = b.f(1);
        h.sct = if h.sfsct == 2 { b.f(1) } else { h.sfsct };
        if h.sct != 0 && h.sfimv == 2 {
            b.f(1); // force_integer_mv
        }
        if h.fidp != 0 {
            b.f(h.idlen); // current_frame_id
        }
        h.fso = if h.reduced == 0 { b.f(1) } else { 0 };
        b.f(h.ohbits); // order_hint
        if h.dmip != 0 && b.f(1) != 0 {
            // buffer_removal_time_present_flag
            for i in 0..h.nop as usize {
                if i >= 32 {
                    break;
                }
                if h.dmpop[i] == 0 {
                    continue;
                }
                let o = h.opidcs[i] as u32;
                if o == 0 || (o >> (self.tid & 31) & 1 != 0 && o >> ((self.sid + 8) & 31) & 1 != 0) {
                    b.f(h.brtlen);
                }
            }
        }
        h.intrabc = 0;
        let refresh = if h.ftype != KEY_FRAME { b.f(8) } else { 0xFF };
        if refresh != 0xFF && h.erm != 0 && h.eoh != 0 {
            for _ in 0..8 {
                b.f(h.ohbits); // ref_order_hint[ i ]
            }
        }
        // frame_size
        let (mut w, mut hh) = (h.maxw, h.maxh);
        if h.fso != 0 {
            w = b.f(h.fwbits).wrapping_add(1);
            hh = b.f(h.fhbits).wrapping_add(1);
        }
        h.upw = w;
        h.fh = hh;
        // superres_params
        h.usesr = if h.esr != 0 { b.f(1) } else { 0 };
        h.srden = if h.usesr != 0 { b.f(3) + 9 } else { 8 };
        h.fw = (h.upw.wrapping_shl(3).wrapping_add(h.srden >> 1)) / h.srden;
        h.micols = ((h.fw.wrapping_add(7)) >> 3).wrapping_mul(2);
        h.mirows = ((h.fh.wrapping_add(7)) >> 3).wrapping_mul(2);
        // render_size
        if b.f(1) != 0 {
            h.rw = b.f(16) + 1;
            h.rh = b.f(16) + 1;
        } else {
            h.rw = h.upw;
            h.rh = h.fh;
        }
        if h.sct != 0 && h.upw == h.fw && b.f(1) != 0 {
            return Err(ERR_UNSUPPORTED); // allow_intrabc
        }
        if h.reduced == 0 && h.dcu == 0 {
            b.f(1); // disable_frame_end_update_cdf
        }
        tileinfo(h, b)?;
        quantseg(h, b);
        filters(h, b);
        grainparams(h, b);
        self.alloc()?;
        self.state = 2;
        self.ntile = 0;
        Ok(())
    }

    /// av_alloc: CurrFrame planes, MIB array, cdef_idx, loop restoration units, above arrays
    fn alloc(&mut self) -> Result<(), i32> {
        let h = &*self.h;
        self.f = Frame::new(); // framefree
        self.ts.above_free();
        // planes: stride = align64(width of the plane), rows = align64(height)
        for p in 0..h.np as usize {
            let (mut w, mut hh) = (h.micols.wrapping_shl(2), h.mirows.wrapping_shl(2));
            if p > 0 {
                w = w.wrapping_shr(h.ssx);
                hh = hh.wrapping_shr(h.ssy);
            }
            let s = w.wrapping_add(63) & !63;
            let r = hh.wrapping_add(63) & !63;
            self.f.ps[p] = s;
            self.f.ph[p] = r;
            self.f.pl[p] = zvec::<u16>(r as usize * s as usize)?;
        }
        self.f.mis = h.micols.wrapping_add(32);
        self.f.mir = h.mirows.wrapping_add(32);
        self.f.mi = zvec::<[u8; 16]>(self.f.mis as usize * self.f.mir as usize)?;
        // cdef_idx per 64x64
        self.f.cds = (h.micols.wrapping_add(15) >> 4).wrapping_add(2);
        let cr = (h.mirows.wrapping_add(15) >> 4).wrapping_add(2);
        self.f.cdidx = zvec::<i8>(cr.wrapping_mul(self.f.cds) as usize)?;
        // loop restoration units
        for p in 0..h.np as usize {
            if h.frt[p] == 0 {
                continue;
            }
            let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
            let us = h.lrsize[p];
            let rnd = |v: u32, s: u32| v.wrapping_add((1u32 << s) >> 1) >> s; // Round2
            let mut rows = (rnd(h.fh, sy).wrapping_add(us >> 1)) / us;
            if rows == 0 {
                rows = 1;
            }
            let mut cols = (rnd(h.upw, sx).wrapping_add(us >> 1)) / us;
            if cols == 0 {
                cols = 1;
            }
            self.f.lrurows[p] = rows;
            self.f.lrucols[p] = cols;
            let n = cols.wrapping_mul(rows) as usize;
            let mut v: Vec<Lru> = Vec::new();
            v.try_reserve_exact(n).map_err(|_| ERR_NOMEM)?;
            v.resize(n, Lru::default());
            self.f.lru[p] = v;
        }
        self.ts.above_alloc(h.micols.wrapping_add(64))?;
        Ok(())
    }

    /// av_tilegroup: tile_group_obu: decode its tiles; after the last tile of the frame the post
    /// filters (wrapup)
    fn tilegroup(&mut self, data: &[u8]) -> i32 {
        let len = data.len() as u64;
        let mut b = Br { d: data, pos: 0, end: len };
        let nt = self.h.tcols.wrapping_mul(self.h.trows); // NumTiles
        let mut start = 0u32;
        let mut end = nt.wrapping_sub(1);
        if nt > 1 && b.f(1) != 0 {
            let bits = self.h.tclog2 + self.h.trlog2;
            start = b.f(bits);
            end = b.f(bits);
            if end >= nt || start > end {
                return ERR_HDR;
            }
        }
        if start != self.ntile {
            return ERR_HDR; // tile groups must follow each other
        }
        let mut pos = (b.pos + 7) >> 3;
        let mut t = start;
        while t <= end {
            if pos > len {
                return ERR_TRUNC;
            }
            let mut size = len - pos; // bytes left
            if t != end {
                let tsb = self.h.tsb as u64;
                if size < tsb {
                    return ERR_TRUNC;
                }
                b.pos = pos * 8;
                let v = b.f(self.h.tsb * 8);
                // le(TileSizeBytes): reverse the bytes read MSB first
                let mut le = 0u32;
                let mut x = v;
                for _ in 0..self.h.tsb {
                    le = (le << 8) | (x & 0xFF);
                    x >>= 8;
                }
                size = le as u64 + 1;
                pos += tsb;
                if size > len - pos {
                    return ERR_TRUNC;
                }
            }
            if size == 0 {
                return ERR_TRUNC;
            }
            let tr = (t / self.h.tcols) as usize;
            let tc = (t % self.h.tcols) as usize;
            let h = &*self.h;
            let bounds = [h.mirs[tr] as u32, h.mirs[tr + 1] as u32, h.mics[tc] as u32, h.mics[tc + 1] as u32];
            let e = blk::tile(&mut self.ts, h, &mut self.f, &data[pos as usize..(pos + size) as usize], bounds, self.cancel);
            if e != 0 {
                return e;
            }
            pos += size;
            t += 1;
            self.ntile = t;
        }
        if end == nt.wrapping_sub(1) {
            return self.wrapup();
        }
        0
    }

    /// av_wrapup: decode_frame_wrapup: deblocking, CDEF, superres, loop restoration, film grain
    /// (each test stage stops after its step)
    fn wrapup(&mut self) -> i32 {
        let st = self.stage;
        let h = &*self.h;
        let f = &mut self.f;
        let state = &mut self.state;
        if st == AVS_RECON {
            let ps = f.ps;
            setout(f, state, Out::Cur, ps, h.fw);
            return 0;
        }
        if h.lflvl[0] != 0 || h.lflvl[1] != 0 {
            crate::lf::loopfilter(h, f);
        }
        if st == AVS_LF {
            let ps = f.ps;
            setout(f, state, Out::Cur, ps, h.fw);
            return 0;
        }
        // CDEF: CdefFrame = copy of CurrFrame, then filtered (else the same planes)
        if h.codedll == 0 && h.ecdef != 0 {
            for p in 0..h.np as usize {
                let n = f.ph[p] as usize * f.ps[p] as usize;
                let mut v: Vec<u16> = Vec::new();
                if v.try_reserve_exact(n).is_err() {
                    return ERR_NOMEM;
                }
                v.extend_from_slice(&f.pl[p][..n]);
                f.cpl[p] = v;
            }
            f.cdef_own = true;
            crate::cdef::cdef(h, f);
        }
        if st == AVS_CDEF {
            let ps = f.ps;
            setout(f, state, Out::Cdef, ps, h.fw);
            return 0;
        }
        // superres (allocates the upscaled planes); else aliases
        if h.usesr != 0 {
            let e = crate::lr::superres(h, f);
            if e != 0 {
                return e;
            }
            f.sr = true;
        } else {
            f.ups = f.ps;
        }
        let ups = f.ups;
        if st == AVS_SR {
            setout(f, state, Out::UpCdef, ups, h.upw);
            return 0;
        }
        if h.useslr != 0 {
            let e = crate::lr::lr(h, f);
            if e != 0 {
                return e;
            }
            setout(f, state, Out::Lr, ups, h.upw);
        } else {
            setout(f, state, Out::UpCdef, ups, h.upw);
        }
        if st == AVS_FINAL && h.applygrain != 0 && self.nograin == 0 {
            return crate::fg::grain(h, f); // replaces the output planes
        }
        0
    }

    /// av1_finish: the decoded frame
    pub fn finish(&mut self) -> Result<Pic<'_>, i32> {
        if self.err != 0 {
            return Err(self.err);
        }
        if self.state != 3 {
            return Err(ERR_TRUNC);
        }
        let h = &*self.h;
        let f = &self.f;
        let np = h.np as usize;
        let pl = |p: usize| if p < np { f.out_pl(p) } else { &[][..] };
        Ok(Pic {
            w: f.ow,
            h: h.fh,
            bd: h.bd,
            subx: h.ssx,
            suby: h.ssy,
            mono: h.mono,
            cdp: h.cdp,
            prim: h.prim,
            trc: h.trc,
            mc: h.mc,
            full: h.full,
            csp: h.csp,
            planes: [pl(0), pl(1), pl(2)],
            strides: f.ops,
            rw: h.rw,
            rh: h.rh,
        })
    }
}

impl Hdr {
    const ZERO: Hdr = Hdr {
        profile: 0,
        reduced: 0,
        opidc: 0,
        dmip: 0,
        eqpi: 0,
        bdlen: 0,
        brtlen: 0,
        fptlen: 0,
        nop: 0,
        fwbits: 0,
        fhbits: 0,
        maxw: 0,
        maxh: 0,
        fidp: 0,
        idlen: 0,
        sb128: 0,
        efi: 0,
        eief: 0,
        eoh: 0,
        sfsct: 0,
        sfimv: 0,
        ohbits: 0,
        esr: 0,
        ecdef: 0,
        elr: 0,
        bd: 0,
        mono: 0,
        np: 0,
        cdp: 0,
        prim: 0,
        trc: 0,
        mc: 0,
        full: 0,
        ssx: 0,
        ssy: 0,
        csp: 0,
        sepuv: 0,
        fgp: 0,
        opidcs: [0; 32],
        dmpop: [0; 32],
        ftype: 0,
        showf: 0,
        showable: 0,
        erm: 0,
        dcu: 0,
        sct: 0,
        fso: 0,
        intrabc: 0,
        fw: 0,
        fh: 0,
        upw: 0,
        rw: 0,
        rh: 0,
        usesr: 0,
        srden: 0,
        micols: 0,
        mirows: 0,
        tcols: 0,
        trows: 0,
        tclog2: 0,
        trlog2: 0,
        tsb: 0,
        baseq: 0,
        dqydc: 0,
        dqudc: 0,
        dquac: 0,
        dqvdc: 0,
        dqvac: 0,
        useqm: 0,
        segen: 0,
        segpreskip: 0,
        lastseg: 0,
        dqpres: 0,
        dqres: 0,
        dlfpres: 0,
        dlfres: 0,
        dlfmulti: 0,
        codedll: 0,
        allll: 0,
        lfsharp: 0,
        lfdeltaen: 0,
        cdefdamp: 0,
        cdefbits: 0,
        useslr: 0,
        txmode: 0,
        redtx: 0,
        applygrain: 0,
        fen: [[0; 8]; 8],
        fdata: [[0; 8]; 8],
        llarr: [0; 8],
        qmlvl: [[0; 8]; 3],
        lflvl: [0; 4],
        lfrefd: [0; 8],
        lfmoded: [0; 2],
        cdefyp: [0; 8],
        cdefys: [0; 8],
        cdefuvp: [0; 8],
        cdefuvs: [0; 8],
        frt: [0; 3],
        lrsize: [0; 3],
        mics: [0; 66],
        mirs: [0; 66],
        fg: Fg::ZERO,
    };
}

// ------------------------------------------------------------------ sequence header

/// av_seqhdr: sequence_header_obu
fn seqhdr(h: &mut Hdr, b: &mut Br) -> Result<(), i32> {
    h.profile = b.f(3);
    if h.profile > 2 {
        return Err(ERR_UNSUPPORTED);
    }
    b.f(1); // still_picture
    h.reduced = b.f(1);
    h.dmip = 0;
    h.eqpi = 0;
    h.opidc = 0;
    h.nop = 1;
    if h.reduced != 0 {
        b.f(5); // seq_level_idx[ 0 ]
    } else {
        if b.f(1) != 0 {
            // timing_info_present_flag
            b.f(32);
            b.f(32);
            h.eqpi = b.f(1);
            if h.eqpi != 0 {
                b.uvlc();
            }
            h.dmip = b.f(1);
            if h.dmip != 0 {
                h.bdlen = b.f(5) + 1;
                b.f(32);
                h.brtlen = b.f(5) + 1;
                h.fptlen = b.f(5) + 1;
            }
        }
        let idd = b.f(1); // initial_display_delay_present_flag
        h.nop = b.f(5) + 1;
        for i in 0..h.nop as usize {
            h.opidcs[i] = b.f(12) as u16;
            if b.f(5) > 7 {
                b.f(1); // seq_tier
            }
            h.dmpop[i] = 0;
            if h.dmip != 0 {
                let v = b.f(1);
                h.dmpop[i] = v as u8;
                if v != 0 {
                    b.f(h.bdlen);
                    b.f(h.bdlen);
                    b.f(1);
                }
            }
            if idd != 0 && b.f(1) != 0 {
                b.f(4);
            }
        }
        h.opidc = h.opidcs[0] as u32;
    }
    h.fwbits = b.f(4) + 1;
    h.fhbits = b.f(4) + 1;
    h.maxw = b.f(h.fwbits).wrapping_add(1);
    h.maxh = b.f(h.fhbits).wrapping_add(1);
    h.fidp = 0;
    if h.reduced == 0 {
        h.fidp = b.f(1);
        if h.fidp != 0 {
            let d = b.f(4); // delta_frame_id_length_minus_2
            h.idlen = b.f(3) + d + 3;
        }
    }
    h.sb128 = b.f(1);
    h.efi = b.f(1);
    h.eief = b.f(1);
    if h.reduced != 0 {
        h.eoh = 0;
        h.sfsct = 2;
        h.sfimv = 2;
        h.ohbits = 0;
    } else {
        b.f(4); // interintra, masked, warped, dual filter
        h.eoh = b.f(1);
        if h.eoh != 0 {
            b.f(2); // jnt_comp, ref_frame_mvs
        }
        h.sfsct = if b.f(1) != 0 { 2 } else { b.f(1) };
        h.sfimv = 2;
        if h.sfsct != 0 && b.f(1) == 0 {
            h.sfimv = b.f(1);
        }
        h.ohbits = 0;
        if h.eoh != 0 {
            h.ohbits = b.f(3) + 1;
        }
    }
    h.esr = b.f(1);
    h.ecdef = b.f(1);
    h.elr = b.f(1);
    // color_config
    let hbd = b.f(1);
    let mut bd = 8;
    if hbd != 0 {
        bd = 10;
        if h.profile == 2 && b.f(1) != 0 {
            bd = 12;
        }
    }
    h.bd = bd;
    h.mono = if h.profile == 1 { 0 } else { b.f(1) };
    h.np = if h.mono != 0 { 1 } else { 3 };
    h.cdp = b.f(1);
    h.prim = 2;
    h.trc = 2;
    h.mc = 2;
    if h.cdp != 0 {
        h.prim = b.f(8);
        h.trc = b.f(8);
        h.mc = b.f(8);
    }
    h.csp = 0;
    h.sepuv = 0;
    if h.mono != 0 {
        h.full = b.f(1);
        h.ssx = 1;
        h.ssy = 1;
    } else {
        if h.prim == 1 && h.trc == 13 && h.mc == 0 {
            h.full = 1;
            h.ssx = 0;
            h.ssy = 0;
        } else {
            h.full = b.f(1);
            let (sx, sy) = match h.profile {
                0 => (1, 1),
                1 => (0, 0),
                _ => {
                    if h.bd == 12 {
                        let x = b.f(1);
                        if x != 0 { (x, b.f(1)) } else { (0, 0) }
                    } else {
                        (1, 0)
                    }
                }
            };
            h.ssx = sx;
            h.ssy = sy;
            if sx & sy != 0 {
                h.csp = b.f(2);
            }
        }
        h.sepuv = b.f(1);
    }
    h.fgp = b.f(1);
    Ok(())
}

// ------------------------------------------------------------------ frame header parts

/// tile_log2: smallest k with blk << k >= target (k < 32)
fn tile_log2(blk: u32, target: u32) -> u32 {
    let mut k = 0u32;
    loop {
        if blk.wrapping_shl(k) >= target {
            return k;
        }
        k += 1;
        if k >= 31 {
            return k;
        }
    }
}

/// av_tileinfo: tile_info( )
fn tileinfo(h: &mut Hdr, b: &mut Br) -> Result<(), i32> {
    let sbshift = if h.sb128 != 0 { 5 } else { 4 };
    let mask = (1u32 << sbshift) - 1;
    let sbcols = h.micols.wrapping_add(mask) >> sbshift;
    let sbrows = h.mirows.wrapping_add(mask) >> sbshift;
    let sbsize = sbshift + 2;
    let max_w_sb = 4096u32 >> sbsize;
    let max_area_sb = (4096u32 * 2304) >> (2 * sbsize);
    let min_log2_cols = tile_log2(max_w_sb, sbcols);
    let max_log2_cols = tile_log2(1, sbcols.min(64));
    let max_log2_rows = tile_log2(1, sbrows.min(64));
    let min_log2_tiles = tile_log2(max_area_sb, sbcols.wrapping_mul(sbrows)).max(min_log2_cols);
    if b.f(1) != 0 {
        // uniform_tile_spacing_flag
        let mut l = min_log2_cols;
        while l < max_log2_cols {
            if b.f(1) == 0 {
                break;
            }
            l += 1;
        }
        h.tclog2 = l;
        let tw = (sbcols.wrapping_add(1u32.wrapping_shl(l)).wrapping_sub(1)).wrapping_shr(l);
        let (mut i, mut s) = (0u32, 0u32);
        while s < sbcols {
            if i >= AV_MAXTILES {
                return Err(ERR_HDR);
            }
            h.mics[i as usize] = s.wrapping_shl(sbshift) as u16;
            i += 1;
            s = s.wrapping_add(tw);
        }
        h.mics[i as usize] = h.micols as u16;
        h.tcols = i;
        let mut l = min_log2_tiles as i32 - h.tclog2 as i32;
        if l < 0 {
            l = 0;
        }
        let mut l = l as u32;
        while l < max_log2_rows {
            if b.f(1) == 0 {
                break;
            }
            l += 1;
        }
        h.trlog2 = l;
        let th = (sbrows.wrapping_add(1u32.wrapping_shl(l)).wrapping_sub(1)).wrapping_shr(l);
        let (mut i, mut s) = (0u32, 0u32);
        while s < sbrows {
            if i >= AV_MAXTILES {
                return Err(ERR_HDR);
            }
            h.mirs[i as usize] = s.wrapping_shl(sbshift) as u16;
            i += 1;
            s = s.wrapping_add(th);
        }
        h.mirs[i as usize] = h.mirows as u16;
        h.trows = i;
    } else {
        let mut widest = 0u32;
        let (mut i, mut s) = (0u32, 0u32);
        while s < sbcols {
            if i >= AV_MAXTILES {
                return Err(ERR_HDR);
            }
            h.mics[i as usize] = s.wrapping_shl(sbshift) as u16;
            let mw = (sbcols - s).min(max_w_sb);
            let sz = b.ns(mw).wrapping_add(1);
            if sz > widest {
                widest = sz;
            }
            s = s.wrapping_add(sz);
            i += 1;
        }
        h.mics[i as usize] = h.micols as u16;
        h.tcols = i;
        h.tclog2 = tile_log2(1, i);
        let mut area = sbcols.wrapping_mul(sbrows);
        if min_log2_tiles != 0 {
            area = area.wrapping_shr(min_log2_tiles + 1);
        }
        let mut mh = area / widest;
        if mh == 0 {
            mh = 1;
        }
        let (mut i, mut s) = (0u32, 0u32);
        while s < sbrows {
            if i >= AV_MAXTILES {
                return Err(ERR_HDR);
            }
            h.mirs[i as usize] = s.wrapping_shl(sbshift) as u16;
            let m = (sbrows - s).min(mh);
            s = s.wrapping_add(b.ns(m)).wrapping_add(1);
            i += 1;
        }
        h.mirs[i as usize] = h.mirows as u16;
        h.trows = i;
        h.trlog2 = tile_log2(1, i);
    }
    h.tsb = 4;
    let n = h.tclog2 + h.trlog2;
    if n != 0 {
        b.f(n); // context_update_tile_id
        h.tsb = b.f(2) + 1;
    }
    Ok(())
}

/// av_readdq: delta_q (su(7) when delta_coded)
fn readdq(b: &mut Br) -> i32 {
    if b.f(1) == 0 {
        return 0;
    }
    b.su(7)
}

/// av1_qindex: get_qindex( curq < 0, seg ) (CurrentQIndex = curq)
pub(crate) fn qindex(h: &Hdr, seg: u32, curq: i32) -> u32 {
    let seg = (seg & 7) as usize;
    let mut q = h.baseq as i32;
    if h.segen != 0 && h.fen[seg][SEG_LVL_ALT_Q] != 0 {
        let d = h.fdata[seg][SEG_LVL_ALT_Q] as i32;
        if curq >= 0 && h.dqpres != 0 {
            q = curq;
        }
        q = q.wrapping_add(d).clamp(0, 255);
        return q as u32;
    }
    if curq >= 0 && h.dqpres != 0 {
        q = curq;
    }
    q as u32
}

/// av_quantseg: quantization_params, segmentation_params, delta_q / lf params, LosslessArray /
/// SegQMLevel / CodedLossless / AllLossless
fn quantseg(h: &mut Hdr, b: &mut Br) {
    h.baseq = b.f(8);
    h.dqydc = readdq(b);
    h.dqudc = 0;
    h.dquac = 0;
    h.dqvdc = 0;
    h.dqvac = 0;
    if h.np != 1 {
        let diff = if h.sepuv != 0 { b.f(1) } else { 0 }; // diff_uv_delta
        h.dqudc = readdq(b);
        h.dqvdc = h.dqudc;
        h.dquac = readdq(b);
        h.dqvac = h.dquac;
        if diff != 0 {
            h.dqvdc = readdq(b);
            h.dqvac = readdq(b);
        }
    }
    h.useqm = b.f(1);
    let (mut qy, mut qu, mut qv) = (0u32, 0u32, 0u32);
    if h.useqm != 0 {
        qy = b.f(4);
        qu = b.f(4);
        qv = qu;
        if h.sepuv != 0 {
            qv = b.f(4);
        }
    }
    // segmentation_params (primary_ref_frame = PRIMARY_REF_NONE)
    h.fen = [[0; 8]; 8];
    h.fdata = [[0; 8]; 8];
    h.segen = b.f(1);
    if h.segen != 0 {
        for i in 0..8 {
            for j in 0..8 {
                let e = b.f(1);
                h.fen[i][j] = e as u8;
                if e == 0 {
                    continue;
                }
                let bits = SEGMENTATION_FEATURE_BITS[j] as u32;
                let signed = SEGMENTATION_FEATURE_SIGNED[j] != 0;
                let mut v = if signed { b.su(bits + 1) } else { b.f(bits) as i32 };
                let mx = SEGMENTATION_FEATURE_MAX[j] as i32;
                if v > mx {
                    v = mx;
                }
                let lo = if signed { -mx } else { 0 };
                if v < lo {
                    v = lo;
                }
                h.fdata[i][j] = v as i16;
            }
        }
    }
    let (mut preskip, mut last) = (0u32, 0u32);
    for i in 0..8 {
        for j in 0..8 {
            if h.fen[i][j] != 0 {
                last = i as u32;
                if j >= SEG_LVL_REF_FRAME {
                    preskip = 1;
                }
            }
        }
    }
    h.segpreskip = preskip;
    h.lastseg = last;
    // delta_q_params / delta_lf_params
    h.dqpres = 0;
    h.dqres = 0;
    h.dlfpres = 0;
    h.dlfres = 0;
    h.dlfmulti = 0;
    if h.baseq != 0 {
        h.dqpres = b.f(1);
        if h.dqpres != 0 {
            h.dqres = b.f(2);
            h.dlfpres = b.f(1);
            if h.dlfpres != 0 {
                h.dlfres = b.f(2);
                h.dlfmulti = b.f(1);
            }
        }
    }
    h.codedll = 1;
    for seg in 0..8u32 {
        let q = qindex(h, seg, -1);
        let ll = q == 0 && (h.dqydc | h.dquac | h.dqudc | h.dqvac | h.dqvdc) == 0;
        h.llarr[seg as usize] = ll as u8;
        if !ll {
            h.codedll = 0;
        }
        let s = seg as usize;
        if ll {
            h.qmlvl[0][s] = 15;
            h.qmlvl[1][s] = 15;
            h.qmlvl[2][s] = 15;
        } else {
            h.qmlvl[0][s] = qy as u8;
            h.qmlvl[1][s] = qu as u8;
            h.qmlvl[2][s] = qv as u8;
        }
    }
    h.allll = (h.codedll != 0 && h.fw == h.upw) as u32;
}

/// av_filters: loop_filter_params, cdef_params, lr_params, read_tx_mode, reduced_tx_set
fn filters(h: &mut Hdr, b: &mut Br) {
    h.lflvl = [0; 4];
    h.lfsharp = 0;
    h.lfdeltaen = 1; // setup_past_independence
    h.lfrefd = [1, 0, 0, 0, -1, 0, -1, -1];
    h.lfmoded = [0, 0];
    if h.codedll == 0 {
        h.lflvl[0] = b.f(6) as u8;
        h.lflvl[1] = b.f(6) as u8;
        if h.np != 1 && (h.lflvl[0] != 0 || h.lflvl[1] != 0) {
            h.lflvl[2] = b.f(6) as u8;
            h.lflvl[3] = b.f(6) as u8;
        }
        h.lfsharp = b.f(3);
        h.lfdeltaen = b.f(1);
        if h.lfdeltaen != 0 && b.f(1) != 0 {
            // loop_filter_delta_update
            for i in 0..8 {
                if b.f(1) != 0 {
                    h.lfrefd[i] = b.su(7) as i8;
                }
            }
            for i in 0..2 {
                if b.f(1) != 0 {
                    h.lfmoded[i] = b.su(7) as i8;
                }
            }
        }
    }
    // cdef_params
    h.cdefyp = [0; 8];
    h.cdefys = [0; 8];
    h.cdefuvp = [0; 8];
    h.cdefuvs = [0; 8];
    h.cdefbits = 0;
    h.cdefdamp = 3;
    if h.codedll == 0 && h.ecdef != 0 {
        h.cdefdamp = b.f(2) + 3;
        h.cdefbits = b.f(2);
        for i in 0..(1usize << h.cdefbits) {
            h.cdefyp[i] = b.f(4) as u8;
            let s = b.f(2);
            h.cdefys[i] = if s == 3 { 4 } else { s as u8 };
            if h.np != 1 {
                h.cdefuvp[i] = b.f(4) as u8;
                let s = b.f(2);
                h.cdefuvs[i] = if s == 3 { 4 } else { s as u8 };
            }
        }
    }
    // lr_params
    h.frt = [0; 3];
    h.useslr = 0;
    h.lrsize = [256; 3];
    if h.allll == 0 && h.elr != 0 {
        let mut chroma = false;
        for i in 0..h.np as usize {
            let t = REMAP_LR_TYPE[b.f(2) as usize];
            h.frt[i] = t;
            if t != 0 {
                h.useslr = 1;
                if i > 0 {
                    chroma = true;
                }
            }
        }
        if h.useslr != 0 {
            let mut shift = b.f(1); // lr_unit_shift
            if h.sb128 != 0 {
                shift += 1;
            } else if shift != 0 {
                shift += b.f(1);
            }
            let s0 = 64u32 << shift;
            h.lrsize[0] = s0;
            let uvs = if chroma && (h.ssx & h.ssy) != 0 { b.f(1) } else { 0 };
            h.lrsize[1] = s0 >> uvs;
            h.lrsize[2] = s0 >> uvs;
        }
    }
    // read_tx_mode
    h.txmode = ONLY_4X4;
    if h.codedll == 0 {
        h.txmode = b.f(1) + TX_MODE_LARGEST;
    }
    // frame_reference_mode, skip_mode_params, global_motion_params: nothing for intra frames
    h.redtx = b.f(1);
}

/// av_grainparams: film_grain_params (update_grain = 1 for intra frames)
fn grainparams(h: &mut Hdr, b: &mut Br) {
    h.fg = Fg::ZERO;
    h.applygrain = 0;
    if h.fgp == 0 {
        return;
    }
    h.applygrain = b.f(1);
    if h.applygrain == 0 {
        return;
    }
    let mono = h.mono;
    let (ssx, ssy) = (h.ssx, h.ssy);
    let g = &mut h.fg;
    g.seed = b.f(16);
    let ny = b.f(4);
    if ny > 14 {
        h.applygrain = 0; // (invalid point counts: no grain)
        return;
    }
    g.ny = ny;
    for i in 0..ny as usize {
        g.py[i][0] = b.f(8) as u8;
        g.py[i][1] = b.f(8) as u8;
    }
    g.csfl = if mono != 0 { 0 } else { b.f(1) };
    if mono == 0 && g.csfl == 0 && !(ssx == 1 && ssy == 1 && g.ny == 0) {
        let n = b.f(4);
        if n > 10 {
            h.applygrain = 0;
            return;
        }
        g.ncb = n;
        for i in 0..n as usize {
            g.pcb[i][0] = b.f(8) as u8;
            g.pcb[i][1] = b.f(8) as u8;
        }
        let n = b.f(4);
        if n > 10 {
            h.applygrain = 0;
            return;
        }
        g.ncr = n;
        for i in 0..n as usize {
            g.pcr[i][0] = b.f(8) as u8;
            g.pcr[i][1] = b.f(8) as u8;
        }
    }
    g.scaling = b.f(2) + 8;
    g.lag = b.f(2);
    let npl = 2 * g.lag * (g.lag + 1); // numPosLuma
    let mut npc = npl; // numPosChroma
    // (unread coefficients stay 128 = 0)
    g.ary = [128; 24];
    g.arcb = [128; 25];
    g.arcr = [128; 25];
    if g.ny != 0 {
        npc += 1;
        for i in 0..npl as usize {
            g.ary[i] = b.f(8) as u8;
        }
    }
    if g.csfl != 0 || g.ncb != 0 {
        for i in 0..npc as usize {
            g.arcb[i] = b.f(8) as u8;
        }
    }
    if g.csfl != 0 || g.ncr != 0 {
        for i in 0..npc as usize {
            g.arcr[i] = b.f(8) as u8;
        }
    }
    g.arshift = b.f(2) + 6;
    g.gshift = b.f(2);
    if g.ncb != 0 {
        g.cbmult = b.f(8);
        g.cblmult = b.f(8);
        g.cboff = b.f(9);
    }
    if g.ncr != 0 {
        g.crmult = b.f(8);
        g.crlmult = b.f(8);
        g.croff = b.f(9);
    }
    g.overlap = b.f(1);
    g.clip = b.f(1);
}
