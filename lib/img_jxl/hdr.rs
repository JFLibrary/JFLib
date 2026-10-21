// hdr -- JPEG XL image headers (headers.cc, image_metadata.cc, color_encoding_internal.cc;
// model: tools/jxl_ref.py SizeHeader .. TransformData, read_headers)    [src/img/jxl_hdr.s]
//
//   Jx                      the decoder record (JX) without the codestream copy
//   Jx::headers(br) -> 0 / ERR_*   signature, size header, image metadata, transform data,
//                                  ICC profile (icc.rs); hdrend = byte aligned position
//   size(br, preview) -> (w, h) / ERR_HDR   SizeHeader / PreviewHeader

use alloc::boxed::Box;
use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_NOMEM, ERR_UNSUPPORTED};

use crate::bits::Jb;
use crate::rnd::Jrn;
use crate::tab;

/// extra channels accepted (JX_MAXEC)
pub const JX_MAXEC: u32 = 256;
/// largest width / height of the size header (JX_MAXDIM)
pub const JX_MAXDIM: u32 = 1 << 30;

pub const JCS_RGB: u8 = 0;
pub const JCS_GRAY: u8 = 1;
pub const JCS_XYB: u8 = 2;
pub const JTF_709: u8 = 1;
pub const JTF_LINEAR: u8 = 8;
pub const JTF_SRGB: u8 = 13;
pub const JTF_PQ: u8 = 16;
pub const JTF_DCI: u8 = 17;
pub const JTF_HLG: u8 = 18;

/// extra channel info (JXE)
#[derive(Clone, Copy, Default)]
pub struct Jxe {
    pub typ: u32,       // 0 alpha, 1 depth, 2 spot colour, 3 mask, 4 black, 5 CFA, 6 thermal, 15 optional
    pub bits: u32,      // bits per sample
    pub flt: u8,        // float samples
    pub expb: u8,       // exponent bits
    pub shift: u8,      // dim_shift
    pub assoc: u8,      // alpha_associated
    pub cfa: u32,       // cfa channel
    pub spot: [u32; 4], // spot colour (f32 bits)
}

/// the decoder record (JX); the codestream copy is kept apart (the readers borrow it)
pub struct Jx {
    pub cslen: u64, // codestream bytes (without padding)
    pub w: u32,
    pub h: u32,
    pub pw: u32, // preview size
    pub ph: u32,
    pub haveprev: u8,
    pub orient: u8,
    pub haveanim: u8,
    pub timecodes: u8,
    pub xyb: u8,
    pub flt: u8,
    pub expb: u8,
    pub m16: u8,
    pub bits: u32,
    pub nec: u32,
    pub ec: Vec<Jxe>,
    pub tpsnum: u32,
    pub tpsden: u32,
    pub loops: u32,
    pub gamma: u32, // gamma * 10^7 (havegamma)
    pub wanticc: u8,
    pub cspace: u8,
    pub white: u8,
    pub prim: u8,
    pub havegamma: u8,
    pub tf: u8,
    pub intent: u8,
    pub relmax: u8,
    pub customxy: [i32; 8],
    pub itarget: u32, // f32 bits
    pub minnits: u32,
    pub linbelow: u32,
    pub upflags: u32,
    pub invop: [f32; 9],
    pub opbias: [f32; 3],
    pub qbias: [f32; 4],
    pub up: [f32; 280], // upsampling weights 2x (15), 4x (55), 8x (210)
    pub icc: Vec<u8>,
    pub hdrend: u64, // bit position after the image headers
    pub refs: [Option<Box<Jrn>>; 4],
    pub dcf: [Option<Box<Jrn>>; 4], // jxl-plus: output of the last DC frame of each level (XYB)
    pub vis: u32,
    pub nonvis: u32,
    pub exif: Option<(usize, usize)>, // range of the EXIF (TIFF) data in the input
    pub xmp: Option<(usize, usize)>,
    pub jbrd: Option<(usize, usize)>,    // jxl-plus: the JPEG reconstruction box (contents)
    pub exifbox: Option<(usize, usize, bool)>, // jxl-plus: contents of the first Exif / xml box
    pub xmlbox: Option<(usize, usize, bool)>,  // (true: a brob box, the range is Brotli data)
}

impl Default for Jx {
    fn default() -> Self {
        Jx {
            cslen: 0,
            w: 0,
            h: 0,
            pw: 0,
            ph: 0,
            haveprev: 0,
            orient: 0,
            haveanim: 0,
            timecodes: 0,
            xyb: 0,
            flt: 0,
            expb: 0,
            m16: 0,
            bits: 0,
            nec: 0,
            ec: Vec::new(),
            tpsnum: 0,
            tpsden: 0,
            loops: 0,
            gamma: 0,
            wanticc: 0,
            cspace: 0,
            white: 0,
            prim: 0,
            havegamma: 0,
            tf: 0,
            intent: 0,
            relmax: 0,
            customxy: [0; 8],
            itarget: 0,
            minnits: 0,
            linbelow: 0,
            upflags: 0,
            invop: [0.0; 9],
            opbias: [0.0; 3],
            qbias: [0.0; 4],
            up: [0.0; 280],
            icc: Vec::new(),
            hdrend: 0,
            refs: [None, None, None, None],
            dcf: [None, None, None, None],
            vis: 0,
            nonvis: 0,
            exif: None,
            xmp: None,
            jbrd: None,
            exifbox: None,
            xmlbox: None,
        }
    }
}

static D_SIZE: [u32; 8] = [1, 9, 1, 13, 1, 18, 1, 30];
static D_PDIV8: [u32; 8] = [16, 0, 32, 0, 1, 5, 33, 9];
static D_PSIZE: [u32; 8] = [1, 6, 65, 8, 321, 10, 1345, 12];
static D_BITS: [u32; 8] = [8, 0, 10, 0, 12, 0, 1, 6];
static D_FBITS: [u32; 8] = [32, 0, 16, 0, 24, 0, 1, 6];
static D_NEC: [u32; 8] = [0, 0, 1, 0, 2, 4, 1, 12];
static D_SHIFT: [u32; 8] = [0, 0, 3, 0, 4, 0, 1, 3];
static D_NAME: [u32; 8] = [0, 0, 0, 4, 16, 5, 48, 10];
static D_CFA: [u32; 8] = [1, 0, 0, 2, 3, 4, 19, 8];
static D_XY: [u32; 8] = [0, 19, 524288, 19, 1048576, 20, 2097152, 21];
static D_TPSN: [u32; 8] = [100, 0, 1000, 0, 1, 10, 1, 30];
static D_TPSD: [u32; 8] = [1, 0, 1001, 0, 1, 8, 1, 10];
static D_LOOPS: [u32; 8] = [0, 0, 0, 3, 0, 16, 0, 32];
static D_ASPECT: [(u32, u32); 8] = [(1, 1), (1, 1), (12, 10), (4, 3), (3, 2), (16, 9), (5, 4), (2, 1)];

/// UnpackSigned
#[inline]
pub fn unpack_signed(v: u32) -> i32 {
    ((v >> 1) ^ (v & 1).wrapping_neg()) as i32
}

/// jx_size: SizeHeader / PreviewHeader -> (w, h)
pub fn size(br: &mut Jb, preview: bool) -> Result<(u32, u32), i32> {
    let div8 = br.f(1);
    let read = |br: &mut Jb| -> u32 {
        if !preview {
            if div8 != 0 { (br.f(5) + 1) << 3 } else { br.u32f(&D_SIZE) }
        } else if div8 != 0 {
            br.u32f(&D_PDIV8) << 3
        } else {
            br.u32f(&D_PSIZE)
        }
    };
    let ys = read(br);
    let ratio = br.f(3);
    let xs = if ratio != 0 {
        let (n, d) = D_ASPECT[ratio as usize];
        ((ys as u64 * n as u64) / d as u64) as u32
    } else {
        read(br)
    };
    if xs == 0 || xs > JX_MAXDIM || ys == 0 || ys > JX_MAXDIM {
        return Err(ERR_HDR);
    }
    Ok((xs, ys))
}

/// hd_bitdepth -> (bits, float, exponent bits)
fn bitdepth(br: &mut Jb) -> (u32, u8, u8) {
    let flt = br.f(1) as u8;
    if flt == 0 {
        (br.u32f(&D_BITS), 0, 0)
    } else {
        let b = br.u32f(&D_FBITS);
        let e = br.f(4) + 1;
        (b, flt, e as u8)
    }
}

/// hd_customxy: two signed coordinates
fn customxy(br: &mut Jb, out: &mut [i32]) {
    for o in out.iter_mut().take(2) {
        *o = unpack_signed(br.u32f(&D_XY));
    }
}

impl Jx {
    /// hd_color: ColorEncoding
    fn color(&mut self, br: &mut Jb) {
        self.wanticc = 0;
        self.cspace = JCS_RGB;
        self.white = 1;
        self.prim = 1;
        self.havegamma = 0;
        self.gamma = 0;
        self.tf = JTF_SRGB;
        self.intent = 1;
        if br.f(1) != 0 {
            return; // all_default: sRGB
        }
        self.wanticc = br.f(1) as u8;
        let cs = br.enum_();
        if cs > 3 {
            br.err = ERR_HDR;
            return;
        }
        self.cspace = cs as u8;
        if self.wanticc != 0 {
            return;
        }
        if self.cspace != JCS_XYB {
            let w = br.enum_();
            self.white = w as u8;
            if w == 2 {
                customxy(br, &mut self.customxy[0..2]);
            }
        }
        if self.cspace != JCS_GRAY && self.cspace != JCS_XYB {
            let p = br.enum_();
            self.prim = p as u8;
            if p == 2 {
                customxy(br, &mut self.customxy[2..4]);
                customxy(br, &mut self.customxy[4..6]);
                customxy(br, &mut self.customxy[6..8]);
            }
        }
        if self.cspace != JCS_XYB {
            self.havegamma = br.f(1) as u8;
            if self.havegamma != 0 {
                self.gamma = br.f(24);
            } else {
                self.tf = br.enum_() as u8;
            }
        } else {
            self.havegamma = 1;
            self.gamma = 3333333;
        }
        self.intent = br.enum_() as u8;
    }

    /// hd_ecinfo: ExtraChannelInfo
    fn ecinfo(br: &mut Jb, e: &mut Jxe) {
        e.typ = 0;
        e.bits = 8;
        e.flt = 0;
        e.expb = 0;
        e.shift = 0;
        e.assoc = 0;
        if br.f(1) != 0 {
            return;
        }
        e.typ = br.enum_();
        if e.typ > 6 && e.typ != 15 {
            br.err = ERR_HDR;
            return;
        }
        let (b, f, x) = bitdepth(br);
        e.bits = b;
        e.flt = f;
        e.expb = x;
        let sh = br.u32f(&D_SHIFT);
        if sh > 31 {
            br.err = ERR_HDR;
            return;
        }
        e.shift = sh as u8;
        let mut n = br.u32f(&D_NAME);
        while n != 0 {
            br.f(8);
            n -= 1;
        }
        if e.typ == 0 {
            e.assoc = br.f(1) as u8;
        } else if e.typ == 2 {
            for k in 0..4 {
                e.spot[k] = br.f16();
            }
        } else if e.typ == 5 {
            e.cfa = br.u32f(&D_CFA);
        }
    }

    /// hd_meta: ImageMetadata
    fn meta(&mut self, br: &mut Jb) -> i32 {
        self.orient = 1;
        self.bits = 8;
        self.m16 = 1;
        self.xyb = 1;
        self.itarget = 0x437f_0000; // 255.0
        self.minnits = 0;
        self.relmax = 0;
        self.linbelow = 0;
        self.cspace = JCS_RGB;
        self.white = 1;
        self.prim = 1;
        self.tf = JTF_SRGB;
        self.intent = 1;
        if br.f(1) != 0 {
            return 0; // all_default
        }
        let extra = br.f(1);
        if extra != 0 {
            self.orient = (br.f(3) + 1) as u8;
            if br.f(1) != 0 {
                // have_intrinsic_size
                if let Err(e) = size(br, false) {
                    return e;
                }
            }
            self.haveprev = br.f(1) as u8;
            if self.haveprev != 0 {
                match size(br, true) {
                    Ok((w, h)) => {
                        self.pw = w;
                        self.ph = h;
                    }
                    Err(e) => return e,
                }
            }
            self.haveanim = br.f(1) as u8;
            if self.haveanim != 0 {
                self.tpsnum = br.u32f(&D_TPSN);
                self.tpsden = br.u32f(&D_TPSD);
                self.loops = br.u32f(&D_LOOPS);
                self.timecodes = br.f(1) as u8;
            }
        }
        let (b, f, x) = bitdepth(br);
        if b == 0 || b > 31 {
            return ERR_HDR;
        }
        self.bits = b;
        self.flt = f;
        self.expb = x;
        self.m16 = br.f(1) as u8;
        let nec = br.u32f(&D_NEC);
        if nec > JX_MAXEC {
            return ERR_UNSUPPORTED;
        }
        self.nec = nec;
        if nec != 0 {
            let mut v = Vec::new();
            if v.try_reserve_exact(nec as usize).is_err() {
                return ERR_NOMEM;
            }
            v.resize(nec as usize, Jxe::default());
            for e in v.iter_mut() {
                Self::ecinfo(br, e);
            }
            self.ec = v;
        }
        self.xyb = br.f(1) as u8;
        self.color(br);
        if extra != 0 && br.f(1) == 0 {
            // tone mapping
            self.itarget = br.f16();
            self.minnits = br.f16();
            self.relmax = br.f(1) as u8;
            self.linbelow = br.f16();
        }
        br.ext();
        0
    }

    /// hd_transform: TransformData (opsin inverse matrix, upsampling weights)
    fn transform(&mut self, br: &mut Jb) {
        self.invop.copy_from_slice(&tab::OPSIN_DEF[0..9]);
        self.opbias.copy_from_slice(&tab::OPSIN_DEF[9..12]);
        self.qbias.copy_from_slice(&tab::OPSIN_DEF[12..16]);
        self.up.copy_from_slice(&tab::UP2);
        self.upflags = 0;
        if br.f(1) != 0 {
            return;
        }
        if self.xyb != 0 && br.f(1) == 0 {
            for k in 0..16 {
                let v = br.f16f();
                match k {
                    0..=8 => self.invop[k] = v,
                    9..=11 => self.opbias[k - 9] = v,
                    _ => self.qbias[k - 12] = v,
                }
            }
        }
        self.upflags = br.f(3);
        let sets = [(0usize, 15usize), (15, 55), (70, 210)];
        for (k, &(o, n)) in sets.iter().enumerate() {
            if self.upflags >> k & 1 != 0 {
                for i in 0..n {
                    self.up[o + i] = br.f16f();
                }
            }
        }
    }

    /// jx_headers: the image headers from br (over the codestream copy)
    pub fn headers(&mut self, br: &mut Jb) -> i32 {
        if br.f(16) != 0x0AFF {
            return ERR_HDR;
        }
        match size(br, false) {
            Ok((w, h)) => {
                self.w = w;
                self.h = h;
            }
            Err(e) => return e,
        }
        let r = self.meta(br);
        if r != 0 {
            return r;
        }
        self.transform(br);
        if self.wanticc != 0 {
            let r = crate::icc::jx_icc(self, br);
            if r != 0 {
                return r;
            }
        }
        br.align();
        self.hdrend = br.pos;
        br.check()
    }
}
