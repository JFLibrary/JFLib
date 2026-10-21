// frm -- JPEG XL frame header, loop filter, TOC (frame_header.cc, loop_filter.cc, toc.cc;
// model: tools/jxl_ref.py Blending, LoopFilter, FrameHeader, read_toc)   [src/img/jxl_frm.s]
//
//   Jf                              the frame record (JF): header, loop filter, derived sizes,
//                                   TOC, and the decoding state (dec.rs and the stages)
//   jf_header(jx, br, jf, preview) -> 0 / ERR_*   FrameHeader into jf (a fresh Jf)
//   jf_toc(br, jf) -> 0 / ERR_*     TOC (with permutation) after the header: ntoc, tocsz,
//                                   tocoff, base, end
//   jf.hshift(c) / jf.vshift(c)     chroma subsampling shift of channel c

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_NOMEM};

use crate::bits::Jb;
use crate::blend::Patch;
use crate::rnd::Jrn;
use crate::ent::{je_read, Jr};
use crate::hdr::{unpack_signed, Jx, JX_MAXDIM};
use crate::modular::{Ma, Mi, WH_DEF};
use crate::tab;
use crate::vdc::Jv;

pub const JFT_REGULAR: u32 = 0;
pub const JFT_DC: u32 = 1;
pub const JFT_REFONLY: u32 = 2;
pub const JFT_SKIPPROG: u32 = 3;
pub const JFF_NOISE: u64 = 1;
pub const JFF_PATCHES: u64 = 2;
pub const JFF_SPLINES: u64 = 16;
pub const JFF_USEDC: u64 = 32;
pub const JFF_SKIPDCSMOOTH: u64 = 128;
pub const JCT_NONE: u32 = 0;
pub const JCT_YCBCR: u32 = 1;
pub const JCT_XYB: u32 = 2;
pub const JBM_REPLACE: u32 = 0;
pub const JBM_ADD: u32 = 1;
pub const JBM_BLEND: u32 = 2;
pub const JBM_AWADD: u32 = 3;
pub const JBM_MUL: u32 = 4;

static D_0123: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 0];
static D_1234: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 0];
static D_1248: [u32; 8] = [1, 0, 2, 0, 4, 0, 8, 0];
static D_BMODE: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 2];
static D_BALPHA: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 3];
static D_PASSES: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 3];
static D_NDS: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 1];
static D_LASTPASS: [u32; 8] = [0, 0, 1, 0, 2, 0, 0, 3];
static D_ORIGIN: [u32; 8] = [0, 8, 256, 11, 2304, 14, 18688, 30];
static D_DUR: [u32; 8] = [0, 0, 1, 0, 0, 8, 0, 32];
static D_NAME: [u32; 8] = [0, 0, 0, 4, 16, 5, 48, 10];
static D_TOC: [u32; 8] = [0, 10, 1024, 14, 17408, 22, 4211712, 30];
static CSHX: [u8; 4] = [0, 1, 1, 0];
static CSHY: [u8; 4] = [0, 1, 0, 1];

/// the modular group header (JMH): weighted predictor header + use_global_tree
#[derive(Clone, Copy)]
pub struct Jmh {
    pub wp: [u32; 11],
    pub global: u32,
}

impl Default for Jmh {
    fn default() -> Self {
        Jmh { wp: [0; 11], global: 0 }
    }
}

/// the frame record (JF)
#[derive(Default)]
pub struct Jf {
    pub typ: u32,
    pub modular: u32,
    pub flags: u64,
    pub ct: u32,
    pub chroma: [u8; 3],
    pub ups: u32,
    pub gshift: u32,
    pub xqm: u32,
    pub bqm: u32,
    pub passes: u32,
    pub nds: u32,
    pub shift: [u32; 11],
    pub ds: [u32; 4],
    pub lastpass: [u32; 4],
    pub dclevel: u32,
    pub x0: i32,
    pub y0: i32,
    pub xsize: u32,
    pub ysize: u32,
    pub custom: u32,
    pub bmode: u32,
    pub balpha: u32,
    pub bclamp: u32,
    pub bsrc: u32,
    pub duration: u32,
    pub timecode: u32,
    pub islast: u32,
    pub saveref: u32,
    pub savebct: u32,
    pub partial: u32,
    pub preview: u32,
    pub ecups: Vec<u32>,
    pub ecbl: Vec<[u32; 4]>, // mode, alpha, clamp, source
    // loop filter (defaults from LFDEF)
    pub gab: u32,
    pub gabw: [f32; 6],
    pub epfit: u32,
    pub sharp: [f32; 8],
    pub epfcs: [f32; 3],
    pub p1zf: f32,
    pub p2zf: f32,
    pub qmul: f32,
    pub p0ss: f32,
    pub p2ss: f32,
    pub bsad: f32,
    pub sigmod: f32,
    // derived sizes
    pub maxhs: u32,
    pub maxvs: u32,
    pub gdim: u32,
    pub wup: u32,
    pub hup: u32,
    pub w: u32,
    pub h: u32,
    pub xb: u32,
    pub yb: u32,
    pub xg: u32,
    pub yg: u32,
    pub xdg: u32,
    pub ydg: u32,
    pub ng: u32,
    pub ndg: u32,
    // TOC
    pub ntoc: u32,
    pub tocsz: Vec<u32>,
    pub tocoff: Vec<u64>,
    pub base: u64,
    pub end: u64,
    // decoding state (dec.rs and the stages)
    pub nbcol: u32,
    pub dcq: [f32; 3],
    pub full: Option<Box<Mi>>,
    pub gcode: Option<Box<crate::ent::Jc>>,
    pub gtree: Ma,
    pub ghdr: Jmh,
    pub dbg: u32,
    pub vd: Option<Box<Jv>>,
    pub rnd: Option<Box<Jrn>>,
    pub patch: Vec<Patch>,
    pub splines: Option<Box<crate::spline::Splines>>, // jxl-tools: the draw cache of the frame's splines
    pub nlut: [f32; 8],
    pub vis: u32,
    pub nonvis: u32,
    pub newref: Option<Box<Jrn>>,
    pub newdc: Option<Box<Jrn>>, // jxl-plus: the output of a DC frame (before the colour transform)
}

impl Jf {
    /// jf_hshift
    pub fn hshift(&self, c: usize) -> u32 {
        self.maxhs.wrapping_sub(CSHX[self.chroma[c] as usize & 3] as u32)
    }
    /// jf_vshift
    pub fn vshift(&self, c: usize) -> u32 {
        self.maxvs.wrapping_sub(CSHY[self.chroma[c] as usize & 3] as u32)
    }

    /// loop filter defaults (JF_GAB .. JF_SIGMOD from jxt_lfdef)
    fn lf_defaults(&mut self) {
        let d = &tab::LFDEF;
        let f = |i: usize| f32::from_bits(d[i]);
        self.gab = d[0];
        for k in 0..6 {
            self.gabw[k] = f(1 + k);
        }
        self.epfit = d[7];
        for k in 0..8 {
            self.sharp[k] = f(8 + k);
        }
        for k in 0..3 {
            self.epfcs[k] = f(16 + k);
        }
        self.p1zf = f(19);
        self.p2zf = f(20);
        self.qmul = f(21);
        self.p0ss = f(22);
        self.p2ss = f(23);
        self.bsad = f(24);
        self.sigmod = f(25);
    }

    /// fh_finish: derived sizes
    fn finish(&mut self) {
        let mut mh = 0u32;
        let mut mv = 0u32;
        for c in 0..3 {
            let m = self.chroma[c] as usize & 3;
            mh = mh.max(CSHX[m] as u32);
            mv = mv.max(CSHY[m] as u32);
        }
        self.maxhs = mh;
        self.maxvs = mv;
        let (mut w, mut h) = (self.xsize, self.ysize);
        if self.dclevel != 0 {
            let s = (self.dclevel * 3) & 31;
            let m = (1u32 << s).wrapping_sub(1);
            w = w.wrapping_add(m) >> s;
            h = h.wrapping_add(m) >> s;
        }
        self.wup = w;
        self.hup = h;
        let s = self.ups.trailing_zeros() & 31;
        let m = (1u32 << s) - 1;
        w = w.wrapping_add(m) >> s;
        h = h.wrapping_add(m) >> s;
        self.w = w;
        self.h = h;
        let gd = 128u32.wrapping_shl(self.gshift);
        self.gdim = gd;
        let s = mh + 3;
        self.xb = (w.wrapping_add((1 << s) - 1) >> s) << mh;
        let s = mv + 3;
        self.yb = (h.wrapping_add((1 << s) - 1) >> s) << mv;
        let gs = (self.gshift + 7) & 31;
        let m = gd.wrapping_sub(1);
        self.xg = w.wrapping_add(m) >> gs;
        self.yg = h.wrapping_add(m) >> gs;
        self.xdg = self.xb.wrapping_add(m) >> gs;
        self.ydg = self.yb.wrapping_add(m) >> gs;
        self.ng = self.xg.wrapping_mul(self.yg);
        self.ndg = self.xdg.wrapping_mul(self.ydg);
    }
}

/// fh_blending: BlendingInfo -> (mode, alpha, clamp, source)
fn blending(br: &mut Jb, nec: u32, partial: u32) -> [u32; 4] {
    let mode = br.u32f(&D_BMODE);
    let mut o = [mode, 0, 0, 0];
    if mode > 4 {
        br.err = ERR_HDR;
        return o;
    }
    if nec != 0 && (mode == JBM_BLEND || mode == JBM_AWADD) {
        o[1] = br.u32f(&D_BALPHA);
        o[2] = br.f(1);
    } else if mode == JBM_MUL {
        o[2] = br.f(1);
    }
    if mode != 0 || partial != 0 {
        o[3] = br.u32f(&D_0123);
    }
    o
}

/// fh_loopfilter: LoopFilter (defaults already in jf)
fn loopfilter(br: &mut Jb, jf: &mut Jf) {
    let modular = jf.modular;
    if br.f(1) != 0 {
        return; // all_default
    }
    jf.gab = br.f(1);
    if jf.gab != 0 && br.f(1) != 0 {
        for k in 0..6 {
            jf.gabw[k] = br.f16f();
        }
    }
    jf.epfit = br.f(2);
    if jf.epfit != 0 {
        if modular == 0 && br.f(1) != 0 {
            for k in 0..8 {
                jf.sharp[k] = br.f16f();
            }
        }
        if br.f(1) != 0 {
            jf.epfcs[0] = br.f16f();
            jf.epfcs[1] = br.f16f();
            jf.epfcs[2] = br.f16f();
            jf.p1zf = br.f16f();
            jf.p2zf = br.f16f();
        }
        if br.f(1) != 0 {
            if modular == 0 {
                jf.qmul = br.f16f();
            }
            jf.p0ss = br.f16f();
            jf.p2ss = br.f16f();
            jf.bsad = br.f16f();
        }
        if modular != 0 {
            jf.sigmod = br.f16f();
        }
    }
    br.ext();
}

/// jf_header: FrameHeader from br into jf (a fresh record)
pub fn jf_header(jx: &Jx, br: &mut Jb, jf: &mut Jf, preview: bool) -> i32 {
    jf.preview = preview as u32;
    jf.typ = JFT_REGULAR;
    jf.ups = 1;
    jf.gshift = 1;
    jf.xqm = 3;
    jf.bqm = 2;
    jf.passes = 1;
    jf.islast = 1;
    if preview {
        jf.xsize = jx.pw;
        jf.ysize = jx.ph;
    } else {
        jf.xsize = jx.w;
        jf.ysize = jx.h;
    }
    jf.lf_defaults();
    let nec = jx.nec as usize;
    jf.ecups = vec![0; nec];
    jf.ecbl = vec![[0; 4]; nec];
    for e in 0..nec {
        jf.ecups[e] = 1u32 << (jx.ec[e].shift & 31);
    }
    if br.f(1) != 0 {
        // all_default
        jf.ct = (jx.xyb as u32) * 2;
    } else {
        jf.typ = br.u32f(&D_0123);
        jf.modular = br.f(1);
        jf.flags = br.u64f();
        jf.ct = JCT_XYB;
        if jx.xyb == 0 {
            jf.ct = br.f(1);
        }
        if jf.flags & JFF_USEDC == 0 {
            if jf.ct == JCT_YCBCR {
                jf.chroma[0] = br.f(2) as u8;
                jf.chroma[1] = br.f(2) as u8;
                jf.chroma[2] = br.f(2) as u8;
            }
            jf.ups = br.u32f(&D_1248);
            for e in 0..nec {
                let u = br.u32f(&D_1248) << (jx.ec[e].shift & 31);
                jf.ecups[e] = u;
                if u < jf.ups || u > 8 {
                    return ERR_HDR;
                }
            }
        }
        if jf.modular != 0 {
            jf.gshift = br.f(2);
        }
        jf.xqm = 2;
        jf.bqm = 2;
        if jf.modular == 0 && jf.ct == JCT_XYB {
            jf.xqm = br.f(3);
            jf.bqm = br.f(3);
        }
        if jf.typ != JFT_REFONLY {
            jf.passes = br.u32f(&D_PASSES);
            if jf.passes != 1 {
                if jf.passes > 11 {
                    return ERR_HDR;
                }
                jf.nds = br.u32f(&D_NDS);
                if jf.nds > 4 {
                    return ERR_HDR;
                }
                for i in 0..(jf.passes - 1) as usize {
                    jf.shift[i] = br.f(2);
                }
                for i in 0..jf.nds as usize {
                    jf.ds[i] = br.u32f(&D_1248);
                }
                for i in 0..jf.nds as usize {
                    jf.lastpass[i] = br.u32f(&D_LASTPASS);
                }
            }
        }
        if jf.typ == JFT_DC {
            jf.dclevel = br.u32f(&D_1234);
        } else {
            jf.custom = br.f(1);
            if jf.custom != 0 {
                if jf.typ != JFT_REFONLY {
                    jf.x0 = unpack_signed(br.u32f(&D_ORIGIN));
                    jf.y0 = unpack_signed(br.u32f(&D_ORIGIN));
                }
                let (iw, ih) = (jf.xsize, jf.ysize);
                jf.xsize = br.u32f(&D_ORIGIN);
                if jf.xsize == 0 || jf.xsize > JX_MAXDIM {
                    return ERR_HDR;
                }
                jf.ysize = br.u32f(&D_ORIGIN);
                if jf.ysize == 0 || jf.ysize > JX_MAXDIM {
                    return ERR_HDR;
                }
                if jf.typ != JFT_REFONLY {
                    let part = jf.x0 > 0
                        || jf.y0 > 0
                        || (jf.x0 as i64 + jf.xsize as i64) < iw as i64
                        || (jf.y0 as i64 + jf.ysize as i64) < ih as i64;
                    jf.partial = part as u32;
                }
            }
        }
        if jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG {
            let b = blending(br, jx.nec, jf.partial);
            jf.bmode = b[0];
            jf.balpha = b[1];
            jf.bclamp = b[2];
            jf.bsrc = b[3];
            for e in 0..nec {
                jf.ecbl[e] = blending(br, jx.nec, jf.partial);
            }
            if jx.haveanim != 0 {
                jf.duration = br.u32f(&D_DUR);
                if jx.timecodes != 0 {
                    jf.timecode = br.f(32);
                }
            }
            jf.islast = br.f(1);
        } else {
            jf.islast = 0;
        }
        if jf.typ == JFT_DC {
            jf.savebct = 1;
        } else {
            if jf.islast == 0 {
                jf.saveref = br.u32f(&D_0123);
            }
            // can_ref = not last and (duration == 0 or save_as_reference != 0)
            let canref = jf.islast == 0 && (jf.duration == 0 || jf.saveref != 0);
            let rd = (canref
                && jf.bmode == JBM_REPLACE
                && jf.partial == 0
                && (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG))
                || jf.typ == JFT_REFONLY;
            if rd {
                jf.savebct = br.f(1);
            }
        }
        let mut n = br.u32f(&D_NAME);
        while n != 0 {
            br.f(8);
            n -= 1;
        }
        loopfilter(br, jf);
        br.ext();
    }
    jf.finish();
    br.check()
}

/// tc_ctx: min(7, CoeffOrderContext token of v)
#[inline]
pub fn tc_ctx(v: u32) -> u32 {
    if v == 0 { 0 } else { (32 - v.leading_zeros()).min(7) }
}

/// jf_toc: TOC (with permutation) after the header
pub fn jf_toc(br: &mut Jb, jf: &mut Jf) -> i32 {
    let n = if jf.ng == 1 && jf.passes == 1 {
        1
    } else {
        jf.ng.wrapping_mul(jf.passes).wrapping_add(jf.ndg).wrapping_add(2)
    };
    jf.ntoc = n;
    let nn = n as usize;
    let mut sz: Vec<u32> = Vec::new();
    let mut off: Vec<u64> = Vec::new();
    if sz.try_reserve_exact(nn).is_err() || off.try_reserve_exact(nn).is_err() {
        return ERR_NOMEM;
    }
    sz.resize(nn, 0);
    off.resize(nn, 0);
    jf.tocsz = sz;
    jf.tocoff = off;
    let mut perm: Vec<u32> = Vec::new();
    if br.f(1) != 0 {
        let jc = match je_read(br, 8, false) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let mut jr = match Jr::new(&jc, br, 0) {
            Ok(r) => r,
            Err(e) => return e,
        };
        if perm.try_reserve_exact(nn).is_err() {
            return ERR_NOMEM;
        }
        perm.resize(nn, 0);
        let end = jr.read(br, tc_ctx(n));
        if end > n {
            return ERR_HDR;
        }
        let mut last = 0u32;
        for i in 0..end {
            let v = jr.read(br, tc_ctx(last));
            last = v;
            if v >= n - i {
                return ERR_HDR;
            }
            perm[i as usize] = v;
        }
        if !jr.fin() || br.err != 0 {
            return ERR_HDR;
        }
        // decode the Lehmer code: perm[i] = remaining.pop(lehmer[i])
        let mut rem: Vec<u32> = (0..n).collect();
        for p in perm.iter_mut() {
            let k = *p as usize;
            *p = rem.remove(k);
        }
    }
    br.align();
    let mut run: u64 = 0;
    for i in 0..nn {
        let s = br.u32f(&D_TOC);
        jf.tocsz[i] = s;
        jf.tocoff[i] = run;
        run = run.wrapping_add(s as u64);
    }
    br.align();
    jf.base = br.pos >> 3;
    jf.end = jf.base.wrapping_add(run);
    if !perm.is_empty() {
        // section i is stored at position perm[i]
        let sz = jf.tocsz.clone();
        let off = jf.tocoff.clone();
        for i in 0..nn {
            let p = perm[i] as usize;
            jf.tocsz[i] = sz[p];
            jf.tocoff[i] = off[p];
        }
    }
    br.check()
}

/// the default weighted predictor header of a frame (no global channels)  [xd_wpdef]
pub fn ghdr_default() -> Jmh {
    Jmh { wp: WH_DEF, global: 0 }
}
