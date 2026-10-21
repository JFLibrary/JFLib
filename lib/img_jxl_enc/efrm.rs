// efrm -- headers of the new encoder: image header (size, metadata, colour encoding or ICC,
// transform data), frame header (all frame kinds the new encoder writes), TOC and section
// assembly (libjxl 0.11.1 headers.cc, image_metadata.cc, frame_header.cc, loop_filter.cc,
// enc_toc.cc; the field order is the one frm.rs / hdr.rs read)          [jxl-plus: no asm]
//
//   ImgHdr / write_image(jw, h)      signature .. ICC stream, byte aligned
//   FrameHdr / write_frame(jw, f)    FrameHeader
//   Frame::layout(w, h, gshift, passes, dclevel)  group / DC group counts (FrameDimensions)
//   write_toc(jw, sizes)             TOC without permutation (byte aligned before and after)
//   assemble(out, header, sections)  frame = header + TOC + sections

use alloc::vec::Vec;

use crate::enc::je_sizehdr;
use crate::ewr::Jw;

static D_TOC: [u32; 8] = [0, 10, 1024, 14, 17408, 22, 4211712, 30];
static D_BITS: [u32; 8] = [8, 0, 10, 0, 12, 0, 1, 6];
static D_NEC: [u32; 8] = [0, 0, 1, 0, 2, 4, 1, 12];
static D_0123: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 0];
static D_1248: [u32; 8] = [1, 0, 2, 0, 4, 0, 8, 0];
static D_1234: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 0];
static D_PASSES: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 3];
static D_NDS: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 1];
static D_LASTPASS: [u32; 8] = [0, 0, 1, 0, 2, 0, 0, 3];
static D_ORIGIN: [u32; 8] = [0, 8, 256, 11, 2304, 14, 18688, 30];

/// colour encoding of the image header
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Srgb,
    Gray,
    Icc,
}

/// the image header
pub struct ImgHdr<'a> {
    pub w: u32,
    pub h: u32,
    pub bits: u32,
    pub alpha: bool,
    pub xyb: bool,
    pub colour: Colour,
    pub gray_icc: bool, // ICC profile of a gray image (colour space Gray)
    pub icc: Option<&'a [u8]>,
}

/// the image header: signature, SizeHeader, ImageMetadata, CustomTransformData, ICC
pub fn write_image(o: &mut Jw, h: &ImgHdr) {
    o.bits(0x0AFF, 16);
    je_sizehdr(o, h.w, h.h);
    let all_default = h.bits == 8 && !h.alpha && h.xyb && h.colour == Colour::Srgb;
    if all_default {
        o.bits(1, 1);
    } else {
        o.bits(0, 1); // all_default
        o.bits(0, 1); // extra_fields
        o.bits(0, 1); // integer samples
        o.u32(h.bits, &D_BITS);
        o.bits(1, 1); // modular_16_bit_buffer_sufficient
        o.u32(h.alpha as u32, &D_NEC);
        if h.alpha {
            o.bits(1, 1); // ExtraChannelInfo all_default: 8-bit alpha
        }
        o.bits(h.xyb as u64, 1);
        match h.colour {
            Colour::Srgb => o.bits(1, 1),
            Colour::Gray => {
                o.bits(0, 2); // not all_default, no ICC
                o.enum_(1); // gray
                o.enum_(1); // D65
                o.bits(0, 1); // no gamma
                o.enum_(13); // sRGB transfer
                o.enum_(1); // relative intent
            }
            Colour::Icc => {
                o.bits(0, 1);
                o.bits(1, 1); // want_icc
                o.enum_(if h.gray_icc { 1 } else { 0 });
            }
        }
        o.bits(0, 2); // extensions 0
    }
    o.bits(1, 1); // transform data all_default
    if h.colour == Colour::Icc {
        if let Some(p) = h.icc {
            crate::eicc::write_icc(o, p);
        }
    }
    o.pad();
}

/// passes of a frame (Passes)
#[derive(Clone, Default)]
pub struct Passes {
    pub n: u32,
    pub shift: Vec<u32>,     // n - 1 entries
    pub ds: Vec<u32>,        // downsample factors
    pub last_pass: Vec<u32>, // per downsample
}

/// the frame header fields the new encoder sets
#[derive(Clone)]
pub struct FrameHdr {
    pub typ: u32, // 0 regular, 1 DC, 2 reference only (jxl-tools: the patch frame)
    pub size: Option<(u32, u32)>, // jxl-tools: custom size (reference-only frames: no origin)
    pub modular: bool,
    pub flags: u64,
    pub xyb: bool, // image metadata xyb_encoded
    pub ycc: bool, // not xyb: YCbCr colour transform
    pub nec: u32,
    pub gshift: u32,
    pub xqm: u32,
    pub bqm: u32,
    pub passes: Passes,
    pub dclevel: u32,
    pub islast: bool,
    pub saveref: u32,
    pub gab: bool,
    pub epf: u32,
    pub sigmod: Option<f32>, // EPF sigma for modular
}

impl Default for FrameHdr {
    fn default() -> Self {
        FrameHdr {
            typ: 0,
            size: None,
            modular: false,
            flags: 0,
            xyb: true,
            ycc: false,
            nec: 0,
            gshift: 1,
            xqm: 3,
            bqm: 2,
            passes: Passes { n: 1, ..Default::default() },
            dclevel: 0,
            islast: true,
            saveref: 0,
            gab: true,
            epf: 1,
            sigmod: None,
        }
    }
}

/// FrameHeader
pub fn write_frame(o: &mut Jw, f: &FrameHdr) {
    o.bits(0, 1); // not all_default
    o.u32(f.typ, &D_0123);
    o.bits(f.modular as u64, 1);
    o.u64(f.flags as u32);
    if !f.xyb {
        o.bits(f.ycc as u64, 1);
    }
    if f.flags & 32 == 0 {
        if f.ycc {
            for _ in 0..3 {
                o.bits(0, 2); // 4:4:4
            }
        }
        o.u32(1, &D_1248); // upsampling 1
        for _ in 0..f.nec {
            o.u32(1, &D_1248);
        }
    }
    if f.modular {
        o.bits(f.gshift as u64, 2);
    }
    if !f.modular && f.xyb {
        o.bits(f.xqm as u64, 3);
        o.bits(f.bqm as u64, 3);
    }
    // passes (not a reference-only frame)
    if f.typ != 2 {
        o.u32(f.passes.n, &D_PASSES);
    }
    if f.typ != 2 && f.passes.n != 1 {
        o.u32(f.passes.ds.len() as u32, &D_NDS);
        for i in 0..(f.passes.n - 1) as usize {
            o.bits(f.passes.shift.get(i).copied().unwrap_or(0) as u64, 2);
        }
        for &d in &f.passes.ds {
            o.u32(d, &D_1248);
        }
        for &l in &f.passes.last_pass {
            o.u32(l, &D_LASTPASS);
        }
    }
    if f.typ == 1 {
        o.u32(f.dclevel, &D_1234);
    } else if f.typ == 2 {
        // jxl-tools: reference only: its size, saved before the colour transform
        let (w, h) = f.size.unwrap_or((1, 1));
        o.bits(1, 1);
        o.u32(w, &D_ORIGIN);
        o.u32(h, &D_ORIGIN);
        o.u32(f.saveref, &D_0123);
        o.bits(1, 1);
    } else {
        o.bits(0, 1); // no custom size / origin
        o.bits(0, 2); // blending replace
        for _ in 0..f.nec {
            o.bits(0, 2);
        }
        o.bits(f.islast as u64, 1);
        if !f.islast {
            o.u32(f.saveref, &D_0123);
            // can_reference (duration 0) + replace + full frame: save_before_ct
            o.bits(0, 1);
        }
    }
    o.bits(0, 2); // name length 0
    // loop filter
    let default_lf = f.gab && f.epf == 1 && f.sigmod.is_none();
    if default_lf {
        o.bits(1, 1);
    } else {
        o.bits(0, 1);
        o.bits(f.gab as u64, 1);
        if f.gab {
            o.bits(0, 1); // default weights
        }
        o.bits(f.epf as u64, 2);
        if f.epf != 0 {
            if !f.modular {
                o.bits(0, 1); // default sharpness LUT
            }
            o.bits(0, 1); // default channel scales
            o.bits(0, 1); // default sigma parameters
            if f.modular {
                o.f16(f.sigmod.unwrap_or(1.0));
            }
        }
        o.bits(0, 2); // loop filter extensions
    }
    o.bits(0, 2); // frame extensions
}

/// FrameDimensions of a frame
#[derive(Clone, Copy, Default)]
pub struct Dims {
    pub w: u32,
    pub h: u32,
    pub xb: u32,
    pub yb: u32,
    pub gdim: u32,
    pub xg: u32,
    pub yg: u32,
    pub ng: u32,
    pub xdg: u32,
    pub ydg: u32,
    pub ndg: u32,
}

impl Dims {
    pub fn new(w: u32, h: u32, gshift: u32) -> Dims {
        let gdim = 128u32 << gshift;
        let xb = (w + 7) >> 3;
        let yb = (h + 7) >> 3;
        let xg = (w + gdim - 1) / gdim;
        let yg = (h + gdim - 1) / gdim;
        let xdg = (xb + gdim - 1) / gdim;
        let ydg = (yb + gdim - 1) / gdim;
        Dims { w, h, xb, yb, gdim, xg, yg, ng: xg * yg, xdg, ydg, ndg: xdg * ydg }
    }
    /// number of TOC entries
    pub fn ntoc(&self, passes: u32) -> usize {
        if self.ng == 1 && passes == 1 { 1 } else { (self.ng * passes + self.ndg + 2) as usize }
    }
    /// ModularStreamId::ID values
    pub fn sid_vardct_dc(&self, g: u32) -> usize {
        1 + g as usize
    }
    pub fn sid_modular_dc(&self, g: u32) -> usize {
        1 + (self.ndg + g) as usize
    }
    pub fn sid_ac_meta(&self, g: u32) -> usize {
        1 + (2 * self.ndg + g) as usize
    }
    pub fn sid_quant(&self, i: u32) -> usize {
        1 + (3 * self.ndg + i) as usize
    }
    pub fn sid_modular_ac(&self, g: u32, pass: u32) -> usize {
        1 + (3 * self.ndg + 17 + self.ng * pass + g) as usize
    }
    pub fn num_streams(&self, passes: u32) -> usize {
        1 + (3 * self.ndg + 17 + self.ng * passes) as usize
    }
}

/// TOC: no permutation, sizes
pub fn write_toc(o: &mut Jw, sizes: &[usize]) {
    o.bits(0, 1);
    o.pad();
    for &s in sizes {
        o.u32(s as u32, &D_TOC);
    }
    o.pad();
}

/// the frame: header, TOC, then the byte-aligned sections; one section when ntoc == 1 (the
/// sections are concatenated bit-wise into one)
pub fn assemble(out: &mut Jw, fh: &FrameHdr, sections: &[Jw]) {
    write_frame(out, fh);
    if sections.len() == 1 {
        let mut s = Jw::new();
        s.append(&sections[0]);
        s.pad();
        write_toc(out, &[s.len()]);
        out.data(&s.buf);
        return;
    }
    let mut padded: Vec<Jw> = Vec::with_capacity(sections.len());
    for s in sections {
        let mut p = Jw::new();
        p.append(s);
        p.pad();
        padded.push(p);
    }
    let sizes: Vec<usize> = padded.iter().map(|p| p.len()).collect();
    write_toc(out, &sizes);
    for p in &padded {
        out.data(&p.buf);
    }
}
