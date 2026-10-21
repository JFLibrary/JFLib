// ps -- HEVC intra decoder: NAL / RBSP reader, parameter sets, slice header, tiles
//                                                                 [src/img/hevc_ps.s]
//
//   load(nal) -> 0 / ERR_HDR          RBSP of a NAL unit (without the 2-byte NAL header)
//                                     into buf, emulation prevention removed  [hvp_load]
//   sps() -> 0 / ERR_*                seq_parameter_set_rbsp (one SPS; ids ignored)  [hvp_sps]
//   pps() -> 0 / ERR_*                pic_parameter_set_rbsp  [hvp_pps]
//   slice(nal_type) -> 0 / ERR_*      slice_segment_header; l.data = byte offset of the slice
//                                     data  [hvp_slice]
//   tiles() -> 0 / ERR_HDR            tile boundaries, CtbAddrRsToTs / TsToRs, TileId (PPS +
//                                     SPS)  [hvp_tiles]
//   rd(n) / rue() / rse()             header reads (n 0..32)  [hvp_u / hvp_ue / hvp_se]
//
// The bit reader returns 0 past the end of the RBSP; ue(v) with 32 leading zeros or more
// gives 0xFFFFFFFF and sets the sticky error ERR_HDR (never cleared, as in the assembly).

use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_UNSUPPORTED};

use crate::dec::{Hevc, Pps, Sps, HV_MAXTILEC, HV_MAXTILER, HV_PAD};

// Table 7-6, intra 8x8 default (up-right diagonal order)
static SLINTRA: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, //
    17, 18, 18, 17, 18, 21, 19, 20, 21, 20, 19, 21, 24, 22, 22, 24, //
    24, 22, 22, 24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31, //
    29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115,
];
// inter 8x8 default
static SLINTER: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, //
    18, 18, 18, 18, 18, 20, 20, 20, 20, 20, 20, 20, 24, 24, 24, 24, //
    24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28, //
    28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91,
];
// 32x32 chroma matrices copied from 16x16
static CHR32: [usize; 4] = [1, 2, 4, 5];

/// deflist: Table 7-5 / 7-6 defaults of (size id, matrix id) into list
fn deflist(list: &mut [u8], size: u32, matrix: u32) {
    if size == 0 {
        list[..16].fill(16);
        return;
    }
    list[..64].copy_from_slice(if matrix < 3 { &SLINTRA } else { &SLINTER });
}

/// sldefault: all default lists
fn sldefault(sl: &mut [u8; 1536], dc: &mut [u8; 12]) {
    for k in 0..24u32 {
        deflist(&mut sl[k as usize * 64..], k / 6, k % 6);
    }
    *dc = [16; 12];
}

impl Hevc<'_> {
    // ---------------------------------------------------------------- bit reader
    /// rd: the next n bits (0 past the end)
    #[inline]
    pub(crate) fn rd(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let p = self.pos;
        let mut v = 0;
        if p >> 3 < self.blen as u64 {
            let i = (p >> 3) as usize;
            let w = u64::from_be_bytes([
                self.buf[i],
                self.buf[i + 1],
                self.buf[i + 2],
                self.buf[i + 3],
                self.buf[i + 4],
                self.buf[i + 5],
                self.buf[i + 6],
                self.buf[i + 7],
            ]);
            v = ((w << (p & 7)) >> (64 - n)) as u32;
        }
        self.pos = p.wrapping_add(n as u64);
        v
    }

    #[inline]
    pub(crate) fn rd1(&mut self) -> u32 {
        self.rd(1)
    }

    /// rue: ue(v) (32 leading zeros or more: 0xFFFFFFFF, err = ERR_HDR)
    pub(crate) fn rue(&mut self) -> u32 {
        let mut lz = 0u32;
        while self.rd1() == 0 {
            lz += 1;
            if lz >= 32 {
                self.err = ERR_HDR;
                return u32::MAX;
            }
        }
        self.rd(lz).wrapping_add((1u32 << lz) - 1)
    }

    /// rse: se(v)
    pub(crate) fn rse(&mut self) -> i32 {
        let v = self.rue();
        let h = v >> 1;
        if v & 1 != 0 { h.wrapping_add(1) as i32 } else { (h as i32).wrapping_neg() }
    }

    /// skip: drop n bits
    #[inline]
    fn skip(&mut self, n: u32) {
        self.pos = self.pos.wrapping_add(n as u64);
    }

    // ---------------------------------------------------------------- NAL loading
    /// hvp_load: the RBSP (after the 2-byte header) into buf without
    /// emulation_prevention_three_bytes; pos = 0
    pub(crate) fn load(&mut self, nal: &[u8]) -> i32 {
        if nal.len() < 2 {
            return ERR_HDR;
        }
        let mut buf = core::mem::take(&mut self.buf);
        buf.clear();
        buf.reserve(nal.len() + HV_PAD);
        let mut z = 0u32; // zero run
        for &b in &nal[2..] {
            if z >= 2 && b == 3 {
                z = 0; // 00 00 03: drop the 03
                continue;
            }
            buf.push(b);
            z = if b == 0 { z + 1 } else { 0 };
        }
        self.blen = buf.len();
        buf.extend_from_slice(&[0u8; HV_PAD]);
        self.buf = buf;
        self.pos = 0;
        0
    }

    // ---------------------------------------------------------------- profile / scaling lists
    /// ptl: skip profile_tier_level( 1, maxNumSubLayersMinus1 )
    fn ptl(&mut self, maxsub: u32) {
        self.skip(96); // general profile (88 bits) + level (8)
        let mut prof = 0u32; // sub-layer profile present bits
        let mut lev = 0u32; // sub-layer level present bits
        for i in 0..maxsub {
            if self.rd1() != 0 {
                prof |= 1 << (i & 31);
            } else {
                prof &= !(1 << (i & 31));
            }
            if self.rd1() != 0 {
                lev |= 1 << (i & 31);
            } else {
                lev &= !(1 << (i & 31));
            }
        }
        if maxsub != 0 {
            self.skip((8u32.wrapping_sub(maxsub)).wrapping_add(8u32.wrapping_sub(maxsub)));
        }
        for i in 0..maxsub {
            if prof >> (i & 31) & 1 != 0 {
                self.skip(88);
            }
            if lev >> (i & 31) & 1 != 0 {
                self.skip(8);
            }
        }
    }

    /// sldata: scaling_list_data( ) -> 0 / ERR_HDR
    fn sldata(&mut self, sl: &mut [u8; 1536], dc: &mut [u8; 12]) -> i32 {
        for size in 0..4u32 {
            let mut m = 0u32;
            while m < 6 {
                let pred = self.rd1(); // scaling_list_pred_mode_flag
                let li = ((size * 6 + m) << 6) as usize;
                if pred == 0 {
                    let d = self.rue(); // scaling_list_pred_matrix_id_delta
                    if d > 5 {
                        return ERR_HDR;
                    }
                    if d == 0 {
                        deflist(&mut sl[li..], size, m);
                        if size >= 2 {
                            dc[((size - 2) * 6 + m) as usize] = 16;
                        }
                    } else {
                        let d = if size == 3 { d * 3 } else { d };
                        let rm = m as i32 - d as i32; // refMatrixId
                        if rm < 0 {
                            return ERR_HDR;
                        }
                        let ri = ((size * 6 + rm as u32) << 6) as usize;
                        sl.copy_within(ri..ri + 64, li);
                        if size >= 2 {
                            let b = ((size - 2) * 6) as usize;
                            dc[b + m as usize] = dc[b + rm as usize];
                        }
                    }
                } else {
                    let mut next = 8i32; // nextCoef
                    let num = if size == 0 { 16 } else { 64 };
                    if size >= 2 {
                        next = self.rse().wrapping_add(8); // scaling_list_dc_coef_minus8
                        dc[((size - 2) * 6 + m) as usize] = next as u8;
                    }
                    for i in 0..num {
                        next = self.rse().wrapping_add(next).wrapping_add(256) & 255;
                        sl[li + (i & 63)] = next as u8;
                    }
                }
                if size == 3 {
                    m += 2; // size 3: matrices 0 and 3
                }
                m += 1;
            }
        }
        // 32x32 chroma lists (4:4:4) from 16x16
        for &k in &CHR32 {
            let s = (12 + k) << 6;
            let d = (18 + k) << 6;
            sl.copy_within(s..s + 64, d);
            dc[k + 6] = dc[k];
        }
        if self.err != 0 { ERR_HDR } else { 0 }
    }

    /// strps: st_ref_pic_set( idx ): only NumDeltaPocs is kept (s.strpsn[idx])
    fn strps(&mut self, idx: u32) {
        let mut cnt;
        let mut inter = 0;
        if idx != 0 {
            inter = self.rd1(); // inter_ref_pic_set_prediction_flag
        }
        if inter != 0 {
            let mut delta = 1u32; // delta_idx
            if idx == self.s.nstrps {
                delta = self.rue().wrapping_add(1);
            }
            self.rd1(); // delta_rps_sign
            self.rue(); // abs_delta_rps_minus1
            let r = idx.wrapping_sub(delta) as i32;
            if r < 0 || r > 64 {
                self.err = ERR_HDR;
                return;
            }
            let n = self.s.strpsn[r as usize] as u32;
            cnt = 0u32;
            let mut j = 0u32;
            while j <= n {
                if self.rd1() != 0 || self.rd1() != 0 {
                    // used_by_curr_pic_flag, use_delta_flag
                    cnt += 1;
                }
                j += 1;
            }
        } else {
            cnt = self.rue(); // num_negative_pics
            cnt = cnt.wrapping_add(self.rue()); // num_positive_pics
            if cnt > 32 {
                self.err = ERR_HDR;
                return;
            }
            for _ in 0..cnt {
                self.rue();
                self.rd1();
            }
        }
        if idx <= 64 {
            self.s.strpsn[idx as usize] = cnt as u8;
        }
    }

    /// hrd: skip hrd_parameters( )
    fn hrd(&mut self, common: u32, maxsub: u32) {
        let mut nal = 0u32;
        let mut vcl = 0u32;
        let mut subpic = 0u32;
        if common != 0 {
            nal = self.rd1();
            vcl = self.rd1();
            if nal | vcl != 0 {
                subpic = self.rd1();
                if subpic != 0 {
                    self.skip(19); // tick_divisor 8, du_cpb_removal_delay_increment 5, flag 1, dpb_output_delay_du 5
                }
                self.skip(8); // bit_rate_scale, cpb_size_scale
                if subpic != 0 {
                    self.skip(4);
                }
                self.skip(15);
            }
        }
        let mut i = 0u32;
        while i <= maxsub {
            let fixed_gen = self.rd1(); // fixed_pic_rate_general_flag
            let fixed = if fixed_gen != 0 { 1 } else { self.rd1() };
            let mut lowdelay = 0;
            if fixed != 0 {
                self.rue(); // elemental_duration_in_tc_minus1
            } else {
                lowdelay = self.rd1();
            }
            let mut cpb = 1u32;
            if lowdelay == 0 {
                cpb = self.rue().wrapping_add(1);
                if cpb > 32 {
                    self.err = ERR_HDR;
                    return;
                }
            }
            for _ in 0..nal + vcl {
                for _ in 0..cpb {
                    self.rue();
                    self.rue();
                    if subpic != 0 {
                        self.rue();
                        self.rue();
                    }
                    self.rd1();
                }
            }
            i += 1;
        }
    }

    /// vui: vui_parameters( ): colour description into s.vui*, the rest skipped
    fn vui(&mut self) {
        if self.rd1() != 0 {
            // aspect_ratio_info_present_flag
            if self.rd(8) == 255 {
                self.skip(32);
            }
        }
        if self.rd1() != 0 {
            // overscan_info_present_flag
            self.rd1();
        }
        if self.rd1() != 0 {
            // video_signal_type_present_flag
            self.skip(3);
            self.s.vuifull = self.rd1();
            if self.rd1() != 0 {
                // colour_description_present_flag
                self.s.vuicol = 1;
                self.s.vuiprim = self.rd(8);
                self.s.vuitrc = self.rd(8);
                self.s.vuimc = self.rd(8);
            }
        }
        if self.rd1() != 0 {
            // chroma_loc_info_present_flag
            self.rue();
            self.rue();
        }
        self.skip(3); // neutral_chroma, field_seq, frame_field_info
        if self.rd1() != 0 {
            // default_display_window_flag
            self.rue();
            self.rue();
            self.rue();
            self.rue();
        }
        if self.rd1() != 0 {
            // vui_timing_info_present_flag
            self.skip(64);
            if self.rd1() != 0 {
                self.rue();
            }
            if self.rd1() != 0 {
                // vui_hrd_parameters_present_flag
                let m = self.s.maxsub;
                self.hrd(1, m);
            }
        }
        if self.rd1() != 0 {
            // bitstream_restriction_flag
            self.skip(3);
            for _ in 0..5 {
                self.rue();
            }
        }
    }

    // ---------------------------------------------------------------- SPS
    /// hvp_sps -> 0 / ERR_HDR / ERR_UNSUPPORTED
    pub(crate) fn sps(&mut self) -> i32 {
        self.s = Sps::ZERO;
        self.skip(4); // sps_video_parameter_set_id
        self.s.maxsub = self.rd(3);
        self.rd1(); // temporal_id_nesting
        let m = self.s.maxsub;
        self.ptl(m);
        self.rue(); // sps_seq_parameter_set_id
        self.s.chroma = self.rue();
        if self.s.chroma > 3 {
            return ERR_HDR;
        }
        if self.s.chroma == 3 {
            self.s.sep = self.rd1();
        }
        self.s.w = self.rue();
        self.s.h = self.rue();
        if self.rd1() != 0 {
            // conformance_window_flag
            self.s.confl = self.rue();
            self.s.confr = self.rue();
            self.s.conft = self.rue();
            self.s.confb = self.rue();
        }
        self.s.bdy = self.rue().wrapping_add(8);
        self.s.bdc = self.rue().wrapping_add(8);
        self.s.log2poc = self.rue().wrapping_add(4);
        // sps_sub_layer_ordering_info_present_flag
        let mut i = if self.rd1() != 0 { 0 } else { self.s.maxsub };
        while i <= self.s.maxsub {
            self.rue();
            self.rue();
            self.rue();
            i += 1;
        }
        self.s.log2mincb = self.rue().wrapping_add(3);
        self.s.log2ctb = self.rue().wrapping_add(self.s.log2mincb);
        self.s.log2mintb = self.rue().wrapping_add(2);
        self.s.log2maxtb = self.rue().wrapping_add(self.s.log2mintb);
        self.rue(); // max_transform_hierarchy_depth_inter
        self.s.maxthintra = self.rue();
        self.s.slen = self.rd1();
        if self.s.slen != 0 {
            let present = self.rd1(); // sps_scaling_list_data_present_flag
            let mut sl = self.s.sl;
            let mut dc = self.s.sldc;
            if present != 0 {
                let r = self.sldata(&mut sl, &mut dc);
                self.s.sl = sl;
                self.s.sldc = dc;
                if r != 0 {
                    return r;
                }
            } else {
                sldefault(&mut sl, &mut dc);
                self.s.sl = sl;
                self.s.sldc = dc;
            }
        }
        self.rd1(); // amp_enabled_flag
        self.s.sao = self.rd1();
        self.s.pcm = self.rd1();
        if self.s.pcm != 0 {
            self.s.pcmbdy = self.rd(4) + 1;
            self.s.pcmbdc = self.rd(4) + 1;
            self.s.log2minpcm = self.rue().wrapping_add(3);
            self.s.log2maxpcm = self.rue().wrapping_add(self.s.log2minpcm);
            self.s.pcmlfdis = self.rd1();
        }
        let n = self.rue();
        if n > 64 {
            return ERR_HDR;
        }
        self.s.nstrps = n;
        for i in 0..n {
            self.strps(i);
        }
        self.s.longterm = self.rd1();
        if self.s.longterm != 0 {
            let n = self.rue();
            if n > 32 {
                return ERR_HDR;
            }
            for _ in 0..n {
                let b = self.s.log2poc;
                self.skip(b);
                self.rd1();
            }
        }
        self.s.tmvp = self.rd1();
        self.s.sis = self.rd1();
        if self.rd1() != 0 {
            // vui_parameters_present_flag
            self.vui();
        }
        if self.rd1() != 0 {
            // sps_extension_present_flag
            let range = self.rd1(); // sps_range_extension_flag
            self.skip(7); // multilayer, 3d, scc, 4bits
            if range != 0 {
                self.s.tsrot = self.rd1();
                self.s.tsctx = self.rd1();
                self.s.imprdpcm = self.rd1();
                self.s.exprdpcm = self.rd1();
                self.s.extprec = self.rd1();
                self.s.isdis = self.rd1();
                self.rd1(); // high_precision_offsets_enabled_flag
                self.s.persrice = self.rd1();
                self.s.bypalign = self.rd1();
            }
        }
        // derive
        if self.err != 0 {
            return ERR_HDR;
        }
        let s = &mut self.s;
        // validity: sizes, bit depths, block sizes, unsupported tools
        if s.sep != 0 || s.extprec != 0 || s.persrice != 0 || s.bypalign != 0 || s.imprdpcm != 0 || s.bdy > 12 || s.bdc > 12 {
            return ERR_UNSUPPORTED;
        }
        if s.log2ctb < 4
            || s.log2ctb > 6
            || s.log2mincb > s.log2ctb
            || s.log2mintb >= s.log2mincb
            || s.log2maxtb > 5
            || s.log2maxtb > s.log2ctb
            || s.maxthintra > 4
            || s.w.wrapping_sub(1) > 16383
            || s.h.wrapping_sub(1) > 16383
        {
            return ERR_HDR;
        }
        // minimum CB size divides the picture size (log2mincb 3..6 here)
        let m = (1u32 << s.log2mincb) - 1;
        if s.w & m != 0 || s.h & m != 0 {
            return ERR_HDR;
        }
        if s.pcm != 0 && (s.log2maxpcm > 5 || s.pcmbdy > s.bdy || s.pcmbdc > s.bdc) {
            return ERR_HDR;
        }
        // derived
        s.cat = s.chroma;
        let (sw, sh) = match s.chroma {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        s.subw = sw;
        s.subh = sh;
        s.ctbsize = 1 << s.log2ctb;
        s.pwctb = (s.w + s.ctbsize - 1) >> s.log2ctb;
        s.phctb = (s.h + s.ctbsize - 1) >> s.log2ctb;
        // conformance window inside the picture
        if s.confl.wrapping_add(s.confr).wrapping_mul(s.subw) >= s.w || s.conft.wrapping_add(s.confb).wrapping_mul(s.subh) >= s.h {
            return ERR_HDR;
        }
        s.valid = 1;
        0
    }

    // ---------------------------------------------------------------- PPS
    /// hvp_pps -> 0 / ERR_HDR / ERR_UNSUPPORTED
    pub(crate) fn pps(&mut self) -> i32 {
        self.p = Pps::ZERO;
        self.rue(); // pps_pic_parameter_set_id
        self.rue(); // pps_seq_parameter_set_id
        self.p.depsl = self.rd1();
        self.p.outflag = self.rd1();
        self.p.nextra = self.rd(3);
        self.p.sdh = self.rd1();
        self.p.cabacinit = self.rd1();
        self.rue();
        self.rue(); // num_ref_idx_l0 / l1_default_active_minus1
        self.p.initqp = self.rse().wrapping_add(26);
        self.p.cip = self.rd1();
        self.p.tskip = self.rd1();
        self.p.cuqpd = self.rd1();
        if self.p.cuqpd != 0 {
            let d = self.rue();
            if d > 3 {
                return ERR_HDR;
            }
            self.p.cuqpddepth = d;
        }
        self.p.cboff = self.rse();
        self.p.croff = self.rse();
        self.p.slcqppres = self.rd1();
        self.skip(2); // weighted pred / bipred
        self.p.tqbypass = self.rd1();
        self.p.tiles = self.rd1();
        self.p.entsync = self.rd1();
        self.p.ncols = 1;
        self.p.nrows = 1;
        self.p.uniform = 1;
        self.p.lftiles = 1;
        if self.p.tiles != 0 {
            let c = self.rue().wrapping_add(1);
            if c > HV_MAXTILEC as u32 {
                return ERR_HDR;
            }
            self.p.ncols = c;
            let r = self.rue().wrapping_add(1);
            if r > HV_MAXTILER as u32 {
                return ERR_HDR;
            }
            self.p.nrows = r;
            self.p.uniform = self.rd1();
            if self.p.uniform == 0 {
                // (a count of 0 after a failed ue(v): the assembly loops on past the arrays;
                // here the loop ends with them -- the PPS is rejected either way)
                let n = (self.p.ncols.wrapping_sub(1) as usize).min(HV_MAXTILEC);
                for i in 0..n {
                    self.p.colw[i] = self.rue().wrapping_add(1);
                }
                let n = (self.p.nrows.wrapping_sub(1) as usize).min(HV_MAXTILER);
                for i in 0..n {
                    self.p.rowh[i] = self.rue().wrapping_add(1);
                }
            }
            self.p.lftiles = self.rd1();
        }
        self.p.lfslices = self.rd1();
        if self.rd1() != 0 {
            // deblocking_filter_control_present_flag
            self.p.dbovr = self.rd1();
            self.p.dbdis = self.rd1();
            if self.p.dbdis == 0 {
                self.p.beta = self.rse().wrapping_mul(2);
                self.p.tc = self.rse().wrapping_mul(2);
            }
        }
        self.p.slpres = self.rd1(); // pps_scaling_list_data_present_flag
        if self.p.slpres != 0 {
            let mut sl = self.p.sl;
            let mut dc = self.p.sldc;
            let r = self.sldata(&mut sl, &mut dc);
            self.p.sl = sl;
            self.p.sldc = dc;
            if r != 0 {
                return r;
            }
        }
        self.p.listsmod = self.rd1();
        self.rue(); // log2_parallel_merge_level_minus2
        self.p.shext = self.rd1();
        self.p.log2maxts = 2;
        if self.rd1() != 0 {
            // pps_extension_present_flag
            let range = self.rd1(); // pps_range_extension_flag
            self.skip(7);
            if range != 0 {
                if self.p.tskip != 0 {
                    let v = self.rue().wrapping_add(2);
                    if v > 5 {
                        return ERR_HDR;
                    }
                    self.p.log2maxts = v;
                }
                self.p.ccp = self.rd1();
                self.p.cqpol = self.rd1();
                if self.p.cqpol != 0 {
                    return ERR_UNSUPPORTED; // chroma QP offset lists: not supported
                }
                self.p.saoscy = self.rue();
                self.p.saoscc = self.rue();
            }
        }
        if self.err != 0 {
            return ERR_HDR;
        }
        if self.p.ccp != 0 {
            return ERR_UNSUPPORTED;
        }
        self.p.valid = 1;
        0
    }

    // ---------------------------------------------------------------- slice header
    /// hvp_slice -> 0 / ERR_HDR / ERR_UNSUPPORTED: slice_segment_header( ) (I slices)
    pub(crate) fn slice(&mut self, naltype: u32) -> i32 {
        self.l.naltype = naltype;
        if self.s.valid == 0 || self.p.valid == 0 {
            return ERR_HDR;
        }
        self.l.first = self.rd1();
        if (16..=23).contains(&naltype) {
            self.rd1(); // no_output_of_prior_pics_flag
        }
        self.rue(); // slice_pic_parameter_set_id
        self.l.dep = 0;
        self.l.addr = 0;
        let mut dep = false;
        if self.l.first == 0 {
            if self.p.depsl != 0 {
                self.l.dep = self.rd1();
            }
            let n = self.s.pwctb.wrapping_mul(self.s.phctb);
            let m = n.wrapping_sub(1);
            let bits = if m == 0 { 0 } else { 32 - m.leading_zeros() }; // Ceil( Log2( PicSizeInCtbsY ) )
            self.l.addr = self.rd(bits);
            if self.l.addr >= n {
                return ERR_HDR;
            }
            dep = self.l.dep != 0;
        }
        if !dep {
            self.l.saddr = self.l.addr;
            let n = self.p.nextra;
            self.skip(n);
            self.l.ty = self.rue();
            if self.l.ty != 2 {
                return ERR_UNSUPPORTED; // only I slices
            }
            if self.p.outflag != 0 {
                self.rd1();
            }
            if naltype != 19 && naltype != 20 {
                let b = self.s.log2poc;
                self.skip(b); // slice_pic_order_cnt_lsb
                if self.rd1() == 0 {
                    // short_term_ref_pic_set_sps_flag
                    let n = self.s.nstrps;
                    self.strps(n);
                } else {
                    let n = self.s.nstrps;
                    if n > 1 {
                        let b = 32 - (n - 1).leading_zeros();
                        self.skip(b); // short_term_ref_pic_set_idx
                    }
                }
                if self.s.longterm != 0 {
                    return ERR_UNSUPPORTED;
                }
                if self.s.tmvp != 0 {
                    self.rd1();
                }
            }
            self.l.saoy = 0;
            self.l.saoc = 0;
            if self.s.sao != 0 {
                self.l.saoy = self.rd1();
                if self.s.cat != 0 {
                    self.l.saoc = self.rd1();
                }
            }
            let qp = self.rse().wrapping_add(self.p.initqp); // slice_qp_delta
            self.l.qp = qp;
            let lo = (self.s.bdy as i32).wrapping_sub(8).wrapping_mul(-6);
            if qp < lo || qp > 51 {
                return ERR_HDR;
            }
            self.l.cboff = 0;
            self.l.croff = 0;
            if self.p.slcqppres != 0 {
                self.l.cboff = self.rse();
                self.l.croff = self.rse();
            }
            let mut ovr = 0; // deblocking_filter_override_flag
            if self.p.dbovr != 0 {
                ovr = self.rd1();
            }
            self.l.dbdis = self.p.dbdis;
            self.l.beta = self.p.beta;
            self.l.tc = self.p.tc;
            if ovr != 0 {
                self.l.dbdis = self.rd1();
                if self.l.dbdis == 0 {
                    self.l.beta = self.rse().wrapping_mul(2);
                    self.l.tc = self.rse().wrapping_mul(2);
                }
            }
            self.l.lfx = self.p.lfslices;
            if self.l.lfx != 0 && (self.l.saoy | self.l.saoc != 0 || self.l.dbdis == 0) {
                self.l.lfx = self.rd1();
            }
        }
        // entry points
        if self.p.tiles | self.p.entsync != 0 {
            let n = self.rue(); // num_entry_point_offsets
            if n > 1 << 20 {
                return ERR_HDR;
            }
            if n != 0 {
                let b = self.rue().wrapping_add(1);
                if b > 32 {
                    return ERR_HDR;
                }
                for _ in 0..n {
                    self.skip(b);
                }
            }
        }
        if self.p.shext != 0 {
            let v = self.rue();
            if v > 256 {
                return ERR_HDR;
            }
            self.skip(v * 8);
        }
        self.rd1(); // alignment_bit_equal_to_one
        while self.pos & 7 != 0 {
            self.rd1();
        }
        let d = self.pos >> 3;
        if d >= self.blen as u64 {
            return ERR_HDR;
        }
        self.l.data = d as u32;
        if self.err != 0 {
            return ERR_HDR;
        }
        0
    }

    // ---------------------------------------------------------------- tiles
    /// hvp_tiles -> 0 / ERR_HDR: column / row boundaries, scan conversion tables rs2ts /
    /// ts2rs and tid (tile scan), (re)allocated for PicSizeInCtbsY
    pub(crate) fn tiles(&mut self) -> i32 {
        let pw = self.s.pwctb;
        let ph = self.s.phctb;
        // column boundaries
        let n = self.p.ncols as usize;
        self.p.colbd[0] = 0;
        for i in 0..n {
            let v = if self.p.uniform != 0 {
                ((i as u32 + 1).wrapping_mul(pw)) / n as u32
            } else if i < n - 1 {
                self.p.colbd[i].wrapping_add(self.p.colw[i])
            } else {
                pw
            };
            self.p.colbd[i + 1] = v;
        }
        if self.p.colbd[n] != pw {
            return ERR_HDR;
        }
        // row boundaries
        let n = self.p.nrows as usize;
        self.p.rowbd[0] = 0;
        for i in 0..n {
            let v = if self.p.uniform != 0 {
                ((i as u32 + 1).wrapping_mul(ph)) / n as u32
            } else if i < n - 1 {
                self.p.rowbd[i].wrapping_add(self.p.rowh[i])
            } else {
                ph
            };
            self.p.rowbd[i + 1] = v;
        }
        if self.p.rowbd[n] != ph {
            return ERR_HDR;
        }
        // boundaries strictly increasing
        for i in 0..self.p.ncols as usize {
            if self.p.colbd[i] >= self.p.colbd[i + 1] {
                return ERR_HDR;
            }
        }
        for i in 0..self.p.nrows as usize {
            if self.p.rowbd[i] >= self.p.rowbd[i + 1] {
                return ERR_HDR;
            }
        }
        let nctb = pw.wrapping_mul(ph);
        self.nctb = nctb;
        let mut rs2ts: Vec<u32> = crate::dec::zvec(nctb as usize);
        let mut ts2rs: Vec<u32> = crate::dec::zvec(nctb as usize);
        let mut tid: Vec<u16> = crate::dec::zvec(nctb as usize);
        // walk tiles in order: ts increments; rs from (tile, y, x)
        let mut ts = 0u32;
        let mut id = 0u32;
        for tr in 0..self.p.nrows as usize {
            for tc in 0..self.p.ncols as usize {
                for y in self.p.rowbd[tr]..self.p.rowbd[tr + 1] {
                    for x in self.p.colbd[tc]..self.p.colbd[tc + 1] {
                        let rs = y.wrapping_mul(pw).wrapping_add(x);
                        if ts >= nctb || rs >= nctb {
                            self.rs2ts = rs2ts;
                            self.ts2rs = ts2rs;
                            self.tid = tid;
                            return ERR_HDR;
                        }
                        rs2ts[rs as usize] = ts;
                        ts2rs[ts as usize] = rs;
                        tid[ts as usize] = id as u16;
                        ts += 1;
                    }
                }
                id += 1;
            }
        }
        self.rs2ts = rs2ts;
        self.ts2rs = ts2rs;
        self.tid = tid;
        0
    }
}
