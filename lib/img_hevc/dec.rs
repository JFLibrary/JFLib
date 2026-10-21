// dec -- HEVC intra picture decoder for still images: the decoder record and the driver
//                                              [src/img/hevc.s, ASMLIB/inc/hevc_int.inc]
//
//   Hevc::new(flags, maxctu)          decoder record (HVF_* test flags, CTU limit 0 = all)
//   set_cancel(flag)                  a flag polled per CTU (ERR_CANCEL)
//   nal(nal) -> 0 / ERR_*             one NAL unit (2-byte header, emulation prevention still
//                                     in): SPS / PPS / slice segments of the first picture;
//                                     VPS, SEI and the rest are ignored
//   annexb(data) -> 0 / ERR_*         every NAL unit of an Annex B byte stream
//   finish() -> Pic / ERR_*           deblocking, SAO and the picture: u16 planes owned by the
//                                     decoder, conformance window
//   newpic() -> 0 / ERR_*             planes and maps for the active SPS / PPS (encoder)
//
// Supported: I slices of one picture, Main / Main 10 / Main 12 / Main Still Picture, 4:0:0 /
// 4:2:0 / 4:2:2 / 4:4:4 (RExt flags transform_skip_rotation / _context, intra smoothing
// disabled), 8 .. 12 bits, scaling lists, transform skip, transquant bypass, PCM, sign data
// hiding, cu_qp_delta, deblocking, SAO, tiles, wavefront, slices, dependent slice segments.
// ERR_UNSUPPORTED: P / B slices, separate colour planes, cross-component prediction, chroma
// QP offset lists, extended precision, implicit RDPCM, persistent Rice adaptation, bypass
// alignment, long-term reference sets in the header.
//
// The record (HV of hevc_int.inc) is one struct: the parameter sets (Sps, Pps, Slice), the
// picture (planes and the per-4x4 / per-CTB maps), the CABAC state, the CU state and the
// transform buffers.  The parts live in ps.rs (headers), ctu.rs (slice data), rec.rs
// (prediction / reconstruction) and lf.rs (loop filters).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use img_core::{ERR_HDR, ERR_TRUNC};

// ---- flags (hevc_new)
pub const HVF_NODEBLOCK: u32 = 1; // skip the deblocking filter (test hook)
pub const HVF_NOSAO: u32 = 2; // skip SAO (test hook)

// ---- limits
pub(crate) const HV_MAXTILEC: usize = 20; // tile columns / rows (level 6.2: 20 x 22)
pub(crate) const HV_MAXTILER: usize = 22;
pub(crate) const HV_NCTX: usize = 152; // CABAC contexts (see CX_*)
pub(crate) const HV_PAD: usize = 16; // zero bytes after the RBSP (8-byte reads)

// ---- 4x4 map flags (mbyp)
pub(crate) const MB_BYPASS: u8 = 1; // cu_transquant_bypass (no deblocking / SAO of these samples)
pub(crate) const MB_PCMOFF: u8 = 2; // pcm with pcm_loop_filter_disabled
pub(crate) const MB_PCM: u8 = 4; // pcm_flag (MPM derivation: DC)

/// the sequence parameter set (HS_*)
#[derive(Clone)]
pub(crate) struct Sps {
    pub valid: u32,
    pub chroma: u32, // chroma_format_idc
    pub sep: u32,    // separate_colour_plane_flag
    pub w: u32,      // pic_width_in_luma_samples
    pub h: u32,
    pub confl: u32, // conformance window offsets (chroma units)
    pub confr: u32,
    pub conft: u32,
    pub confb: u32,
    pub bdy: u32, // BitDepthY / C
    pub bdc: u32,
    pub log2poc: u32,
    pub log2mincb: u32,
    pub log2ctb: u32,
    pub log2mintb: u32,
    pub log2maxtb: u32,
    pub maxthintra: u32, // max_transform_hierarchy_depth_intra
    pub slen: u32,       // scaling_list_enabled_flag
    pub sao: u32,
    pub pcm: u32,
    pub pcmbdy: u32,
    pub pcmbdc: u32,
    pub log2minpcm: u32,
    pub log2maxpcm: u32,
    pub pcmlfdis: u32,
    pub nstrps: u32, // num_short_term_ref_pic_sets
    pub longterm: u32,
    pub tmvp: u32,
    pub sis: u32,      // strong_intra_smoothing_enabled_flag
    pub imprdpcm: u32, // range extension flags
    pub extprec: u32,
    pub isdis: u32, // intra_smoothing_disabled_flag
    pub persrice: u32,
    pub bypalign: u32,
    pub tsrot: u32,
    pub tsctx: u32,
    pub exprdpcm: u32,
    pub vuicol: u32, // 1 = colour description present
    pub vuiprim: u32,
    pub vuitrc: u32,
    pub vuimc: u32,
    pub vuifull: u32, // video_full_range_flag
    pub cat: u32,     // ChromaArrayType
    pub subw: u32,
    pub subh: u32,
    pub ctbsize: u32,
    pub pwctb: u32,
    pub phctb: u32,
    pub maxsub: u32,        // sps_max_sub_layers_minus1
    pub strpsn: [u8; 68],   // NumDeltaPocs per st_ref_pic_set
    pub sl: [u8; 4 * 6 * 64], // scaling lists (size id, matrix id) in coded order
    pub sldc: [u8; 12],     // DC of size ids 2, 3
}

impl Sps {
    pub const ZERO: Sps = Sps {
        valid: 0,
        chroma: 0,
        sep: 0,
        w: 0,
        h: 0,
        confl: 0,
        confr: 0,
        conft: 0,
        confb: 0,
        bdy: 0,
        bdc: 0,
        log2poc: 0,
        log2mincb: 0,
        log2ctb: 0,
        log2mintb: 0,
        log2maxtb: 0,
        maxthintra: 0,
        slen: 0,
        sao: 0,
        pcm: 0,
        pcmbdy: 0,
        pcmbdc: 0,
        log2minpcm: 0,
        log2maxpcm: 0,
        pcmlfdis: 0,
        nstrps: 0,
        longterm: 0,
        tmvp: 0,
        sis: 0,
        imprdpcm: 0,
        extprec: 0,
        isdis: 0,
        persrice: 0,
        bypalign: 0,
        tsrot: 0,
        tsctx: 0,
        exprdpcm: 0,
        vuicol: 0,
        vuiprim: 0,
        vuitrc: 0,
        vuimc: 0,
        vuifull: 0,
        cat: 0,
        subw: 0,
        subh: 0,
        ctbsize: 0,
        pwctb: 0,
        phctb: 0,
        maxsub: 0,
        strpsn: [0; 68],
        sl: [0; 4 * 6 * 64],
        sldc: [0; 12],
    };
}

/// the picture parameter set (HP_*)
#[derive(Clone)]
pub(crate) struct Pps {
    pub valid: u32,
    pub depsl: u32, // dependent_slice_segments_enabled_flag
    pub outflag: u32,
    pub nextra: u32,
    pub sdh: u32, // sign_data_hiding_enabled_flag
    pub cabacinit: u32,
    pub initqp: i32, // 26 + init_qp_minus26
    pub cip: u32,
    pub tskip: u32,
    pub cuqpd: u32,
    pub cuqpddepth: u32,
    pub cboff: i32,
    pub croff: i32,
    pub slcqppres: u32,
    pub tqbypass: u32,
    pub tiles: u32,
    pub entsync: u32,
    pub ncols: u32,
    pub nrows: u32,
    pub uniform: u32,
    pub lftiles: u32,
    pub lfslices: u32,
    pub dbovr: u32,
    pub dbdis: u32,
    pub beta: i32, // beta_offset (already x2)
    pub tc: i32,
    pub slpres: u32, // pps_scaling_list_data_present_flag
    pub listsmod: u32,
    pub shext: u32,
    pub log2maxts: u32,
    pub ccp: u32,
    pub cqpol: u32,
    pub saoscy: u32,
    pub saoscc: u32,
    pub colbd: [u32; HV_MAXTILEC + 1], // tile column boundaries (CTBs)
    pub rowbd: [u32; HV_MAXTILER + 1],
    pub colw: [u32; HV_MAXTILEC], // explicit column widths (non-uniform)
    pub rowh: [u32; HV_MAXTILER],
    pub sl: [u8; 4 * 6 * 64],
    pub sldc: [u8; 12],
}

impl Pps {
    pub const ZERO: Pps = Pps {
        valid: 0,
        depsl: 0,
        outflag: 0,
        nextra: 0,
        sdh: 0,
        cabacinit: 0,
        initqp: 0,
        cip: 0,
        tskip: 0,
        cuqpd: 0,
        cuqpddepth: 0,
        cboff: 0,
        croff: 0,
        slcqppres: 0,
        tqbypass: 0,
        tiles: 0,
        entsync: 0,
        ncols: 0,
        nrows: 0,
        uniform: 0,
        lftiles: 0,
        lfslices: 0,
        dbovr: 0,
        dbdis: 0,
        beta: 0,
        tc: 0,
        slpres: 0,
        listsmod: 0,
        shext: 0,
        log2maxts: 0,
        ccp: 0,
        cqpol: 0,
        saoscy: 0,
        saoscc: 0,
        colbd: [0; HV_MAXTILEC + 1],
        rowbd: [0; HV_MAXTILER + 1],
        colw: [0; HV_MAXTILEC],
        rowh: [0; HV_MAXTILER],
        sl: [0; 4 * 6 * 64],
        sldc: [0; 12],
    };
}

/// the slice segment header (HL_*)
#[derive(Clone, Copy, Default)]
pub(crate) struct Slice {
    pub first: u32,
    pub dep: u32,
    pub addr: u32,  // slice_segment_address
    pub saddr: u32, // SliceAddrRs (address of the independent slice segment)
    pub ty: u32,
    pub saoy: u32,
    pub saoc: u32,
    pub qp: i32, // SliceQpY
    pub cboff: i32,
    pub croff: i32,
    pub cqpo: u32, // cu_chroma_qp_offset_enabled_flag
    pub dbdis: u32,
    pub beta: i32,
    pub tc: i32,
    pub lfx: u32,  // slice_loop_filter_across_slices_enabled_flag
    pub data: u32, // byte offset of slice_segment_data in buf
    pub naltype: u32,
}

/// SAO parameters of one component of a CTB (12 bytes of HV_CSAO)
#[derive(Clone, Copy, Default)]
pub(crate) struct Sao {
    pub ty: u8,  // 0 off, 1 band, 2 edge
    pub pos: u8, // band position / EO class
    pub off: [i16; 4],
}

/// the CABAC decoder state (HC_*)
pub(crate) struct Cabac {
    pub range: u32,
    pub off: u32,
    pub pos: u64, // bit position in buf
    pub ctx: [u8; HV_NCTX],
    pub wctx: [u8; HV_NCTX], // WPP storage
    pub wvalid: u32,
    pub wrow: u32, // CTB row / tile column of the storage
    pub wtc: u32,
    pub dctx: [u8; HV_NCTX], // end-of-slice-segment storage (dependent slices)
    pub dvalid: u32,
    pub dqp: i32, // qPY_PREV at the end of the slice segment
}

/// the CTU / CU state (HU_*)
#[derive(Clone, Copy, Default)]
pub(crate) struct Cu {
    pub rx: u32,
    pub ry: u32,
    pub ts: u32,
    pub tile: u32, // current tile id
    pub cux: u32,
    pub cuy: u32,
    pub culog2: u32,
    pub byp: u32, // cu_transquant_bypass_flag
    pub pcm: u32,
    pub nxn: u32,
    pub modes: [u8; 4],  // luma modes of the PUs
    pub cmodes: [u8; 4], // chroma modes
    pub qpy: i32,        // QpY of the CU
    pub qppred: i32,
    pub qpprev: i32, // qPY_PREV
    pub qpdc: u32,   // IsCuQpDeltaCoded
    pub qpdval: i32, // CuQpDeltaVal
    pub qgx: u32,    // current quantization group
    pub qgy: u32,
    pub qgvalid: u32, // 1 = qgpred computed for the group
    pub qgpred: i32,
    pub sdepth: u32,    // MaxTrafoDepth of the CU
    pub sliceaddr: i32, // SliceAddrRs of the current slice (copy)
    pub ncomp: u32,     // 1 or 3
}

/// residual coding work (HR_*)
#[derive(Clone, Copy)]
pub(crate) struct ResW {
    pub sig: [u8; 16],  // significant positions of the sub-block (scan order)
    pub abs: [i32; 16], // levels
    pub sign: [u8; 16],
    pub csbf: [u8; 64], // coded_sub_block_flag (8 x 8 sub-blocks)
    pub nsig: u32,
    pub c1: i32, // greater1 state carried between sub-blocks (-1 = first)
    pub lastx: u32,
    pub lasty: u32,
    pub scan: u32,
    pub ts: u32,
    pub log2: u32,
    pub cidx: u32,
    pub mode: u32,
    pub xs: u32, // current sub-block
    pub ys: u32,
    pub prev: u32, // prevCsbf (right | below << 1)
    pub nsb: u32,  // sub-blocks per row
    pub n: u32,    // TB size
    pub ptab: u32, // position scan table: offset into ctu::SCAN4 (x | y << 4)
    pub rlog2: u32, // recon_tb: TB size (scaling factors)
    pub rdc: i32,   // recon_tb: DC scaling factor of 16x16 / 32x32 (-1 = none)
}

impl Default for ResW {
    fn default() -> Self {
        ResW {
            sig: [0; 16],
            abs: [0; 16],
            sign: [0; 16],
            csbf: [0; 64],
            nsig: 0,
            c1: 0,
            lastx: 0,
            lasty: 0,
            scan: 0,
            ts: 0,
            log2: 0,
            cidx: 0,
            mode: 0,
            xs: 0,
            ys: 0,
            prev: 0,
            nsb: 0,
            n: 0,
            ptab: 0,
            rlog2: 0,
            rdc: 0,
        }
    }
}

/// the decoded picture (HEVCPIC): planes owned by the decoder
pub struct Pic<'p> {
    pub w: u32, // output size (conformance window applied)
    pub h: u32,
    pub x0: u32, // luma offset of the window in the planes
    pub y0: u32,
    pub bd: u32, // BitDepthY / C
    pub bdc: u32,
    pub chroma: u32, // chroma_format_idc
    pub subw: u32,
    pub subh: u32,
    pub vuicol: u32, // colour description present (VUI)
    pub prim: u32,
    pub trc: u32,
    pub mc: u32,
    pub full: u32, // video_full_range_flag
    pub planes: [&'p [u16]; 3],
    pub pw: [u32; 3], // plane width (= stride in samples) / height
    pub ph: [u32; 3],
}

/// the decoder record (HV)
pub struct Hevc<'a> {
    // general
    pub(crate) buf: Vec<u8>, // RBSP of the current NAL (emulation prevention removed) + HV_PAD zeros
    pub(crate) blen: usize,  // RBSP length (bytes)
    pub(crate) pos: u64,     // bit position (header parsing)
    pub(crate) err: i32,     // sticky error
    pub(crate) flags: u32,
    pub(crate) maxctu: u32, // decode at most this many CTUs (0 = all; test hook)
    pub(crate) nctu: u32,   // CTUs decoded
    pub(crate) picok: u32,  // 1 = a picture was started
    pub(crate) cancel: Option<&'a AtomicU32>,
    pub(crate) s: Sps,
    pub(crate) p: Pps,
    pub(crate) l: Slice,
    // picture
    pub(crate) pl: [Vec<u16>; 3],
    pub(crate) pw: [u32; 3],
    pub(crate) ph: [u32; 3],
    pub(crate) mw: u32, // 4x4 map width / height
    pub(crate) mh: u32,
    pub(crate) mdec: Vec<i32>,   // per 4x4: slice address, -1 = not decoded
    pub(crate) mtid: Vec<u16>,   // per 4x4: tile id
    pub(crate) mdepth: Vec<u8>,  // per 4x4: CtDepth
    pub(crate) mipm: Vec<u8>,    // per 4x4: IntraPredModeY
    pub(crate) mqp: Vec<i8>,     // per 4x4: QpY
    pub(crate) mbyp: Vec<u8>,    // per 4x4: MB_*
    pub(crate) medge: Vec<u8>,   // per 4x4: bit 0 vertical edge (left), bit 1 horizontal (top)
    pub(crate) cslice: Vec<i32>, // per CTB: SliceAddrRs
    pub(crate) cdb: Vec<u8>,     // per CTB: bit 0 deblocking disabled, bit 1 lf across slices
    pub(crate) cbeta: Vec<[i8; 2]>, // per CTB: beta / tc offsets
    pub(crate) csao: Vec<[Sao; 3]>, // per CTB: SAO parameters
    pub(crate) rs2ts: Vec<u32>,
    pub(crate) ts2rs: Vec<u32>,
    pub(crate) tid: Vec<u16>, // per CTB (tile scan)
    pub(crate) nctb: u32,     // PicSizeInCtbsY
    pub(crate) saoused: u32,  // some slice enabled SAO
    // CABAC, CU
    pub(crate) c: Cabac,
    pub(crate) u: Cu,
    // buffers
    pub(crate) coef: [i32; 1024], // coefficients (raster)
    pub(crate) tmp: [i32; 1024],  // transform scratch
    pub(crate) res: [i32; 1024],  // residual / prediction
    pub(crate) refs: [i32; 264],  // intra reference samples (left 2n, corner, top 2n)
    pub(crate) ref2: [i32; 264],  // filtered references
    pub(crate) rw: ResW,
}

/// a zeroed vector (mem_alloc returns zeroed blocks)
pub(crate) fn zvec<T: Clone + Default>(n: usize) -> Vec<T> {
    vec![T::default(); n]
}

impl<'a> Hevc<'a> {
    /// hevc_new: a fresh (zeroed) record
    pub fn new(flags: u32, maxctu: u32) -> Hevc<'a> {
        Hevc {
            buf: Vec::new(),
            blen: 0,
            pos: 0,
            err: 0,
            flags,
            maxctu,
            nctu: 0,
            picok: 0,
            cancel: None,
            s: Sps::ZERO,
            p: Pps::ZERO,
            l: Slice::default(),
            pl: [Vec::new(), Vec::new(), Vec::new()],
            pw: [0; 3],
            ph: [0; 3],
            mw: 0,
            mh: 0,
            mdec: Vec::new(),
            mtid: Vec::new(),
            mdepth: Vec::new(),
            mipm: Vec::new(),
            mqp: Vec::new(),
            mbyp: Vec::new(),
            medge: Vec::new(),
            cslice: Vec::new(),
            cdb: Vec::new(),
            cbeta: Vec::new(),
            csao: Vec::new(),
            rs2ts: Vec::new(),
            ts2rs: Vec::new(),
            tid: Vec::new(),
            nctb: 0,
            saoused: 0,
            c: Cabac {
                range: 0,
                off: 0,
                pos: 0,
                ctx: [0; HV_NCTX],
                wctx: [0; HV_NCTX],
                wvalid: 0,
                wrow: 0,
                wtc: 0,
                dctx: [0; HV_NCTX],
                dvalid: 0,
                dqp: 0,
            },
            u: Cu::default(),
            coef: [0; 1024],
            tmp: [0; 1024],
            res: [0; 1024],
            refs: [0; 264],
            ref2: [0; 264],
            rw: ResW::default(),
        }
    }

    /// hevc_set_cancel: a flag polled per CTU (None = none)
    pub fn set_cancel(&mut self, flag: Option<&'a AtomicU32>) {
        self.cancel = flag;
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }

    /// picfree: the picture buffers and per-picture arrays
    fn picfree(&mut self) {
        self.pl = [Vec::new(), Vec::new(), Vec::new()];
        self.mdec = Vec::new();
        self.mtid = Vec::new();
        self.mdepth = Vec::new();
        self.mipm = Vec::new();
        self.mqp = Vec::new();
        self.mbyp = Vec::new();
        self.medge = Vec::new();
        self.cslice = Vec::new();
        self.cdb = Vec::new();
        self.cbeta = Vec::new();
        self.csao = Vec::new();
    }

    /// hevc_newpic: allocate the planes and maps of a picture (SPS / PPS active; the
    /// encoder uses the record for prediction / reconstruction)
    pub fn newpic(&mut self) -> i32 {
        self.picfree();
        let r = self.tiles();
        if r != 0 {
            return r;
        }
        self.u.ncomp = if self.s.cat != 0 { 3 } else { 1 };
        for c in 0..self.u.ncomp as usize {
            let (mut w, mut h) = (self.s.w, self.s.h);
            if c != 0 {
                w /= self.s.subw;
                h /= self.s.subh;
            }
            self.pw[c] = w;
            self.ph[c] = h;
            self.pl[c] = zvec(w as usize * h as usize);
        }
        self.mw = (self.s.w + 3) >> 2;
        self.mh = (self.s.h + 3) >> 2;
        let n = self.mw as usize * self.mh as usize; // 4x4 blocks
        self.mdec = vec![-1; n];
        self.mtid = zvec(n);
        self.mdepth = zvec(n);
        self.mipm = zvec(n);
        self.mqp = zvec(n);
        self.mbyp = zvec(n);
        self.medge = zvec(n);
        let nc = self.nctb as usize;
        self.cslice = zvec(nc);
        self.cdb = zvec(nc);
        self.cbeta = zvec(nc);
        self.csao = zvec(nc);
        self.c.wvalid = 0;
        self.c.dvalid = 0;
        self.nctu = 0;
        self.saoused = 0;
        self.picok = 1;
        0
    }

    /// hevc_nal: one NAL unit (see the file header)
    pub fn nal(&mut self, nal: &[u8]) -> i32 {
        if nal.len() < 2 {
            return 0;
        }
        let b0 = nal[0] as u32;
        if b0 & 0x80 != 0 {
            return ERR_HDR; // forbidden_zero_bit
        }
        let ty = (b0 >> 1) & 63; // nal_unit_type
        let layer = (b0 & 1) << 5 | (nal[1] as u32) >> 3; // nuh_layer_id
        if layer != 0 {
            return 0; // other layers: ignored
        }
        match ty {
            33 => {
                if self.picok != 0 {
                    return 0; // parameter sets after the picture started
                }
                let r = self.load(nal);
                if r != 0 {
                    return r;
                }
                self.sps()
            }
            34 => {
                if self.picok != 0 {
                    return 0;
                }
                let r = self.load(nal);
                if r != 0 {
                    return r;
                }
                self.pps()
            }
            0..=9 | 16..=21 => {
                // a second picture is not decoded
                if self.picok != 0 && self.nctu >= self.nctb {
                    return 0;
                }
                let r = self.load(nal);
                if r != 0 {
                    return r;
                }
                let r = self.slice(ty);
                if r != 0 {
                    return r;
                }
                if self.l.first != 0 {
                    if self.picok != 0 {
                        return 0; // first slice of another picture
                    }
                    let r = self.newpic();
                    if r != 0 {
                        return r;
                    }
                } else if self.picok == 0 {
                    return ERR_HDR;
                }
                self.saoused |= self.l.saoy | self.l.saoc;
                if self.maxctu != 0 && self.nctu >= self.maxctu {
                    return 0;
                }
                self.slice_data()
            }
            _ => 0,
        }
    }

    /// hevc_annexb: split at 00 00 01 start codes (trailing zero bytes of a NAL dropped) and
    /// feed every NAL unit to nal
    pub fn annexb(&mut self, data: &[u8]) -> i32 {
        let len = data.len();
        let mut i = 0usize;
        let mut start: Option<usize> = None; // start of the current NAL
        while i + 3 <= len {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                if let Some(s) = start {
                    let r = self.abfeed(data, s, i);
                    if r != 0 {
                        return r;
                    }
                }
                start = Some(i + 3);
                i += 3;
            } else {
                i += 1;
            }
        }
        match start {
            Some(s) => self.abfeed(data, s, len),
            None => 0,
        }
    }

    /// abfeed: NAL data[start, end) without trailing zero bytes -> nal
    fn abfeed(&mut self, data: &[u8], start: usize, mut end: usize) -> i32 {
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        self.nal(&data[start..end])
    }

    /// hevc_finish: deblocking, SAO and the picture (see the file header)
    pub fn finish(&mut self) -> Result<Pic<'_>, i32> {
        if self.picok == 0 {
            return Err(ERR_HDR);
        }
        if self.maxctu == 0 && self.nctu < self.nctb {
            return Err(ERR_TRUNC); // slices missing
        }
        if self.flags & HVF_NODEBLOCK == 0 {
            self.deblock();
        }
        if self.flags & HVF_NOSAO == 0 && self.saoused != 0 {
            let r = self.sao();
            if r != 0 {
                return Err(r);
            }
        }
        let s = &self.s;
        let x0 = s.confl.wrapping_mul(s.subw);
        let y0 = s.conft.wrapping_mul(s.subh);
        Ok(Pic {
            w: s.w.wrapping_sub(s.confr.wrapping_mul(s.subw).wrapping_add(x0)),
            h: s.h.wrapping_sub(s.confb.wrapping_mul(s.subh).wrapping_add(y0)),
            x0,
            y0,
            bd: s.bdy,
            bdc: s.bdc,
            chroma: s.chroma,
            subw: s.subw,
            subh: s.subh,
            vuicol: s.vuicol,
            prim: s.vuiprim,
            trc: s.vuitrc,
            mc: s.vuimc,
            full: s.vuifull,
            planes: [&self.pl[0], &self.pl[1], &self.pl[2]],
            pw: self.pw,
            ph: self.ph,
        })
    }
}
