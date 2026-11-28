// Images (ISO 32000-1, 8.9): the samples of every bit depth through Decode and the colour
// space into RGBA, stencil masks (ImageMask) painted with the fill colour or pattern, explicit
// masks (/Mask as a stencil or as colour-key ranges; a soft mask takes their place), soft masks
// (/SMask, with /Matte), inline images with their abbreviations; DCTDecode through img_jpeg,
// CCITTFaxDecode through fax.rs, JPXDecode through img_jp2 (its own size and bit depth, Decode
// ignored, the colour space from the JP2 header when the dictionary names none, SMaskInData),
// JBIG2Decode through img_jbig2 (with its JBIG2Globals; the page's own size; JBIG2's 1 for black
// is the filter's 0).  Drawing maps the unit square through the CTM:
// shrunk images are averaged over the pixels they cover first (a box filter), enlarged ones
// are sampled bilinearly, from four times their size on (and without /Interpolate) as the
// nearest sample.

use super::super::object::{Dict, Name, Object};
use super::canvas::{self, Put, Shader, Src};
use super::color::Cs;
use super::{apply, inv, mul, Res, Run, M};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use gfx::path::Path;
use gfx::RULE_NONZERO;

pub(super) struct Img {
    pub w: usize,
    pub h: usize,
    /// premultiplied RGBA (a stencil: only the alpha counts)
    pub px: Vec<u8>,
    pub stencil: bool,
    pub interpolate: bool,
    pub oc: Option<Object>,
}

/// the long names of an inline image's abbreviations
fn expand_key(k: &[u8]) -> &[u8] {
    match k {
        b"BPC" => b"BitsPerComponent",
        b"CS" => b"ColorSpace",
        b"D" => b"Decode",
        b"DP" => b"DecodeParms",
        b"F" => b"Filter",
        b"H" => b"Height",
        b"W" => b"Width",
        b"IM" => b"ImageMask",
        b"I" => b"Interpolate",
        b"L" => b"Length",
        k => k,
    }
}

fn expand_value(v: &Object) -> Object {
    let name = |n: &[u8]| -> Option<&'static [u8]> {
        Some(match n {
            b"G" => b"DeviceGray",
            b"RGB" => b"DeviceRGB",
            b"CMYK" => b"DeviceCMYK",
            b"I" => b"Indexed",
            b"AHx" => b"ASCIIHexDecode",
            b"A85" => b"ASCII85Decode",
            b"LZW" => b"LZWDecode",
            b"Fl" => b"FlateDecode",
            b"RL" => b"RunLengthDecode",
            b"CCF" => b"CCITTFaxDecode",
            b"DCT" => b"DCTDecode",
            _ => return None,
        })
    };
    match v {
        Object::Name(n) => name(&n.0).map(|x| Object::Name(Name(x.to_vec()))).unwrap_or_else(|| v.clone()),
        Object::Array(a) => Object::Array(a.iter().map(expand_value).collect()),
        _ => v.clone(),
    }
}

/// the samples of a row-padded image: the value of component c of pixel i
struct Samples<'a> {
    data: &'a [u8],
    n: usize,
    bpc: usize,
    stride: usize,
}

impl Samples<'_> {
    fn new(data: &[u8], w: usize, n: usize, bpc: usize) -> Samples<'_> {
        Samples { data, n, bpc, stride: (w * n * bpc).div_ceil(8) }
    }

    #[inline]
    fn get(&self, x: usize, y: usize, c: usize) -> u32 {
        let bit = (x * self.n + c) * self.bpc;
        let base = y * self.stride;
        match self.bpc {
            8 => self.data.get(base + bit / 8).copied().unwrap_or(0) as u32,
            16 => {
                let o = base + bit / 8;
                (self.data.get(o).copied().unwrap_or(0) as u32) << 8 | self.data.get(o + 1).copied().unwrap_or(0) as u32
            }
            b => {
                let byte = self.data.get(base + bit / 8).copied().unwrap_or(0) as u32;
                let shift = 8 - b - bit % 8;
                (byte >> shift) & ((1 << b) - 1)
            }
        }
    }
}

/// a JPXDecode image, as samples the rest of the image code takes
struct Jpx {
    samples: Vec<u8>,
    w: usize,
    h: usize,
    bpc: usize,
    /// the colour space of the JP2 header (when the dictionary names none)
    cs: Option<Rc<Cs>>,
    /// the opacity channel (8 bits; the colours are straight)
    alpha: Option<Vec<u8>>,
    /// per colour component (empty: none was scaled): its samples went from 0..=.0 to 0..=.1, the
    /// ranges of a colour-key mask follow them
    scaled: Vec<(u32, u32)>,
}

/// v of 0..=max as 0..=to
fn scale_sample(v: u32, max: u32, to: u32) -> u32 {
    ((v.min(max) as u64 * to as u64 + max as u64 / 2) / max.max(1) as u64) as u32
}

/// a colour-key range of samples 0..=max as a range of the samples scaled to 0..=to (the scaling
/// only grows the range of values, so a sample is in the range exactly when its scaled value is)
fn scale_range(lo: i64, hi: i64, max: u32, to: u32) -> (i64, i64) {
    if lo > hi || lo > max as i64 || hi < 0 {
        return (1, 0);
    }
    (scale_sample(lo.max(0) as u32, max, to) as i64, scale_sample(hi.min(max as i64) as u32, max, to) as i64)
}

/// decode a JPEG 2000 image.  `named`: the component count of the dictionary's colour space (None:
/// the JP2 header's colour space, sYCC converted to RGB); `indexed`: the samples are indices;
/// `alpha`: the opacity channel is wanted (true: the dictionary says the colours are premultiplied
/// by it, SMaskInData 2).  The colour components keep their precision when it is 1, 2, 4, 8 or 16
/// bits for each, else they become 8 bits (16 when one has more than 8), scaled (indices are not);
/// sub-sampled ones are brought to the image's size.  Premultiplied colours are made straight on
/// the samples, at 8 or 16 bits (a straight colour needs more steps than the premultiplied one
/// had); indices stay as they are.  A stencil mask comes as 1 bit (a sample at least half the
/// range is 1).
fn jpx(data: &[u8], named: Option<usize>, indexed: bool, alpha: Option<bool>, max_pixels: u64, stencil: bool) -> Option<Jpx> {
    let opt = img_jp2::Options { max_pixels, convert_sycc: named.is_none(), ..Default::default() };
    let img = img_jp2::decode(data, &opt).ok()?;
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let colour: Vec<&img_jp2::Component> = img.components.iter().enumerate().filter(|(i, _)| Some(*i) != img.alpha).map(|(_, c)| c).collect();
    let cs = match named {
        Some(_) => None,
        None => Some(Rc::new(match (&img.color, colour.len()) {
            (img_jp2::Color::Gray, _) => Cs::Gray,
            (img_jp2::Color::Srgb, _) => Cs::Rgb,
            (img_jp2::Color::Cmyk, _) => Cs::Cmyk,
            // the enumerated CIELab of JPX: D50, a and b around the middle of their range
            (img_jp2::Color::Lab, _) => Cs::Lab { wp: [0.9642, 1.0, 0.8249], range: [-128.0, 127.0, -128.0, 127.0] },
            (_, 1 | 2) => Cs::Gray,
            (_, 4) => Cs::Cmyk,
            _ => Cs::Rgb,
        })),
    };
    let n = named.unwrap_or_else(|| cs.as_ref().map_or(1, |c| c.n()));
    if colour.len() < n || n == 0 {
        return None;
    }
    // component c at image pixel (x, y) (unsigned), and its maximum
    let at = |c: &img_jp2::Component, x: usize, y: usize| -> (u32, u32) {
        let (cw, ch) = (c.width as u64, c.height as u64);
        if cw == 0 || ch == 0 {
            return (0, 1);
        }
        let sy = (y as u64 * ch / h as u64).min(ch - 1);
        let sx = (x as u64 * cw / w as u64).min(cw - 1);
        let v = c.data[(sy * cw + sx) as usize] as i64;
        let p = c.precision.clamp(1, 24) as u32;
        let v = if c.signed { v + (1i64 << (p - 1)) } else { v };
        let max = (1i64 << p) - 1;
        (v.clamp(0, max) as u32, max as u32)
    };
    if stencil {
        let stride = w.div_ceil(8);
        let mut s = vec![0u8; stride.checked_mul(h)?];
        for y in 0..h {
            for x in 0..w {
                let (v, max) = at(colour[0], x, y);
                if v as u64 * 2 >= max as u64 {
                    s[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        return Some(Jpx { samples: s, w, h, bpc: 1, cs: None, alpha: None, scaled: Vec::new() });
    }
    let cols = &colour[..n];
    let opacity = alpha.and_then(|pdf_pre| img.alpha.and_then(|i| img.components.get(i)).map(|c| (c, pdf_pre || img.premultiplied)));
    // premultiplied colours (not indices) to be made straight
    let pre = !indexed && opacity.is_some_and(|o| o.1);
    let p0 = cols[0].precision.clamp(1, 24) as usize;
    let native = !pre && [1, 2, 4, 8, 16].contains(&p0) && cols.iter().all(|c| c.precision as usize == p0);
    let wide = cols.iter().any(|c| c.precision > 8);
    let bpc = if native { p0 } else if wide { 16 } else { 8 };
    let scaled: Vec<(u32, u32)> = if native || indexed { Vec::new() } else { cols.iter().map(|c| ((1u32 << c.precision.clamp(1, 24)) - 1, (1u32 << bpc) - 1)).collect() };
    let stride = w.checked_mul(n)?.checked_mul(bpc)?.div_ceil(8);
    let mut s = vec![0u8; stride.checked_mul(h)?];
    let top = (1u32 << bpc) - 1;
    for y in 0..h {
        let row = &mut s[y * stride..][..stride];
        for x in 0..w {
            let a = opacity.filter(|_| pre).map(|o| at(o.0, x, y));
            for (ci, c) in cols.iter().enumerate() {
                let (v, max) = at(c, x, y);
                let v = match a {
                    // premultiplied: straight again on the samples (before any colour conversion),
                    // v / max * amax / a at the precision of the output
                    Some((a, amax)) if a > 0 && a < amax => {
                        let d = max as u64 * a as u64;
                        ((v as u64 * top as u64 * amax as u64 + d / 2) / d).min(top as u64) as u32
                    }
                    _ if scaled.is_empty() => v.min(top),
                    _ => scale_sample(v, max, top),
                };
                let bit = (x * n + ci) * bpc;
                match bpc {
                    16 => row[bit / 8..][..2].copy_from_slice(&(v as u16).to_be_bytes()),
                    8 => row[bit / 8] = v as u8,
                    b => row[bit / 8] |= (v as u8) << (8 - b - bit % 8),
                }
            }
        }
    }
    let alpha = opacity.map(|(c, _)| {
        let mut a = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (v, max) = at(c, x, y);
                a.push(scale_sample(v, max, 255) as u8);
            }
        }
        a
    });
    Some(Jpx { samples: s, w, h, bpc, cs, alpha, scaled })
}

/// a JPEG's own size (it counts, not /Width and /Height) within max_pixels, its pixels counted
/// as work; one whose header does not read fails in the decoder
fn jpeg_allowed(run: &mut Run, data: &[u8]) -> bool {
    let mut m = img_core::ImgMeta::default();
    if !img_jpeg::probe(data, &mut m) {
        return true;
    }
    let px = m.w as u64 * m.h as u64;
    px <= run.opt.max_pixels.min(1 << 28) && run.spend(px)
}

impl Img {
    pub fn load_xobject(run: &mut Run, o: &Object, res: &Rc<Res>) -> Option<Rc<Img>> {
        let Object::Stream(s) = run.doc.resolve(o) else { return None };
        let (data, image_filter) = run.doc.decode_until_image(&s).ok()?;
        Img::load(run, &s.dict, data, image_filter, res, 0).map(Rc::new)
    }

    pub fn load_inline(run: &mut Run, d: &Dict, data: &[u8], res: &Rc<Res>) -> Option<Rc<Img>> {
        let mut full = Dict::new();
        for (k, v) in d.iter() {
            full.map_mut().insert(Name(expand_key(&k.0).to_vec()), expand_value(v));
        }
        // the filters of an inline image run here (its data is in the content stream)
        let filters = super::super::filter::chain(&full);
        let (data, image_filter) = super::super::filter::apply(&filters, data.to_vec(), run.doc.max_stream).ok()?;
        Img::load(run, &full, data, image_filter, res, 0).map(Rc::new)
    }

    fn load(run: &mut Run, d: &Dict, data: Vec<u8>, image_filter: Option<(Name, Dict)>, res: &Rc<Res>, depth: u32) -> Option<Img> {
        let doc = run.doc;
        let w = doc.key(d, "Width").as_int()?;
        let h = doc.key(d, "Height").as_int()?;
        if w <= 0 || h <= 0 || w > 1 << 20 || h > 1 << 20 || (w as u64) * (h as u64) > run.opt.max_pixels.min(1 << 28) {
            return None;
        }
        // decoding and converting it is work of the page (256 images of 16384 x 16384 at 1 bit,
        // 32 KB each, were 256 GB)
        if !run.spend((w as u64) * (h as u64)) {
            return None;
        }
        let (mut w, mut h) = (w as usize, h as usize);
        let stencil = doc.key(d, "ImageMask").as_bool() == Some(true);
        let interpolate = doc.key(d, "Interpolate").as_bool() == Some(true);
        let mut bpc = if stencil { 1 } else { doc.key(d, "BitsPerComponent").as_int().unwrap_or(8) as usize };
        let mut decode_arr: Vec<f64> = super::color::nums(doc, &doc.key(d, "Decode"));
        let mut cs: Rc<Cs> = if stencil {
            Rc::new(Cs::Gray)
        } else {
            match d.get("ColorSpace") {
                Some(c) => {
                    let c = c.clone();
                    run.colorspace(&c, res).unwrap_or_else(|| Rc::new(Cs::Rgb))
                }
                None => Rc::new(Cs::Gray),
            }
        };
        let mut n = cs.n();
        // the samples: decoded by an image filter, else as they are
        let mut rgb_direct: Option<Vec<u8>> = None;
        // a JPEG 2000 image's own opacity channel (SMaskInData), 8 bits, and how its samples were
        // scaled (for a colour-key mask)
        let mut jpx_alpha: Option<Vec<u8>> = None;
        let mut jpx_scaled: Vec<(u32, u32)> = Vec::new();
        let mut samples = data;
        if let Some((f, parms)) = &image_filter {
            match f.0.as_slice() {
                b"DCTDecode" if n == 4 => {
                    // CMYK: the samples as stored, through Decode and the CMYK conversion
                    if !jpeg_allowed(run, &samples) {
                        return None;
                    }
                    let sf = img_jpeg::decode_fl(&samples, img_core::JDF_RAW_CMYK | img_core::JDF_NO_ORIENT).ok()?;
                    let (sw, sh) = (sf.w.max(0) as usize, sf.h.max(0) as usize);
                    let st = sf.stride as usize;
                    let dec: Vec<f64> = if decode_arr.len() == 8 { decode_arr.clone() } else { vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0] };
                    let mut out = Vec::with_capacity(sw * sh * 3);
                    let mut comps = [0f64; 4];
                    for y in 0..sh {
                        for x in 0..sw {
                            let p = &sf.px[y * st + x * 4..][..4];
                            for (c, v) in [p[2], p[1], p[0], p[3]].iter().enumerate() {
                                comps[c] = dec[2 * c] + *v as f64 / 255.0 * (dec[2 * c + 1] - dec[2 * c]);
                            }
                            let c = cs.rgb(&comps);
                            out.extend_from_slice(&[super::to8(c[0]), super::to8(c[1]), super::to8(c[2])]);
                        }
                    }
                    if sw != w || sh != h {
                        let mut img = Img::from_rgb(out, sw, sh);
                        img.interpolate = interpolate;
                        img.oc = d.get("OC").cloned();
                        return Some(img);
                    }
                    rgb_direct = Some(out);
                }
                b"DCTDecode" => {
                    let flags = match doc.key(parms, "ColorTransform").as_int() {
                        Some(0) => img_core::JDF_RGB,
                        Some(1) => img_core::JDF_YCC,
                        _ => 0,
                    } | img_core::JDF_NO_ORIENT;
                    if !jpeg_allowed(run, &samples) {
                        return None;
                    }
                    let sf = img_jpeg::decode_fl(&samples, flags).ok()?;
                    let (sw, sh) = (sf.w.max(0) as usize, sf.h.max(0) as usize);
                    if sw != w || sh != h {
                        // the JPEG's own size counts
                        if sw == 0 || sh == 0 {
                            return None;
                        }
                    }
                    let st = sf.stride as usize;
                    let mut out = Vec::with_capacity(sw * sh * 3);
                    for y in 0..sh {
                        for x in 0..sw {
                            let p = &sf.px[y * st + x * 4..][..4];
                            out.extend_from_slice(&[p[2], p[1], p[0]]);
                        }
                    }
                    if n == 1 || !matches!(&*cs, Cs::Gray | Cs::Rgb | Cs::Cmyk) {
                        // the decoded grey is the sample (Indexed, Separation, ...)
                        samples = out.chunks_exact(3).map(|p| p[0]).collect();
                        bpc = 8;
                        if n != 1 {
                            rgb_direct = Some(out);
                        }
                    } else {
                        rgb_direct = Some(out);
                    }
                    if sw != w || sh != h {
                        let mut img = Img::from_rgb(rgb_direct.unwrap_or_else(|| samples.iter().flat_map(|&v| [v, v, v]).collect()), sw, sh);
                        img.interpolate = interpolate;
                        img.oc = d.get("OC").cloned();
                        return Some(img);
                    }
                }
                b"CCITTFaxDecode" => {
                    samples = super::fax::decode(doc, &samples, parms, w, h)?;
                    bpc = 1;
                }
                b"JPXDecode" => {
                    // with a colour space in the dictionary the JP2 header's counts for nothing
                    let named = (d.get("ColorSpace").is_some() || stencil).then_some(n);
                    let indexed = named.is_some() && matches!(&*cs, Cs::Indexed { .. });
                    let smask_in_data = doc.key(d, "SMaskInData").as_int().unwrap_or(0);
                    let alpha = (smask_in_data > 0 && d.get("SMask").is_none()).then_some(smask_in_data == 2);
                    let j = jpx(&samples, named, indexed, alpha, run.opt.max_pixels.min(1 << 28), stencil)?;
                    samples = j.samples;
                    (w, h, bpc) = (j.w, j.h, j.bpc);
                    jpx_alpha = j.alpha;
                    jpx_scaled = j.scaled;
                    if let Some(c) = j.cs {
                        cs = c;
                        n = cs.n();
                    }
                    // Decode counts only for a stencil mask (ISO 32000-1, 8.9.5.3)
                    if !stencil {
                        decode_arr.clear();
                    }
                }
                b"JBIG2Decode" => {
                    let globals = match doc.resolve(&doc.key(parms, "JBIG2Globals")) {
                        Object::Stream(g) => doc.decode_until_image(&g).ok().map(|x| x.0),
                        _ => None,
                    };
                    let page = img_jbig2::decode_embedded(&samples, globals.as_deref(), &img_jbig2::Options { max_pixels: run.opt.max_pixels.min(1 << 28) }).ok()?;
                    if page.width == 0 || page.height == 0 {
                        return None;
                    }
                    // the page's size counts (as a JPEG's does); its rows are padded to bytes as an
                    // image's are, and JBIG2's 1 for black is the filter's 0
                    (w, h) = (page.width as usize, page.height as usize);
                    samples = page.bits.iter().map(|b| !b).collect();
                    bpc = 1;
                }
                _ => return None,
            }
        }
        if ![1, 2, 4, 8, 16].contains(&bpc) {
            return None;
        }
        let smp = Samples::new(&samples, w, if rgb_direct.is_some() { 3 } else { n }, bpc);
        let maxv = ((1u32 << bpc) - 1) as f64;
        let mut px = canvas::zeroed(w.checked_mul(h)?.checked_mul(4)?)?;
        if stencil {
            // Decode [0 1]: a 0 paints; [1 0]: a 1 paints
            let paint_one = decode_arr.first().is_some_and(|&v| v >= 0.5);
            let smp = Samples::new(&samples, w, 1, 1);
            for y in 0..h {
                for x in 0..w {
                    let s = smp.get(x, y, 0);
                    if (s == 1) == paint_one {
                        px[(y * w + x) * 4 + 3] = 255;
                    }
                }
            }
            return Some(Img { w, h, px, stencil: true, interpolate, oc: d.get("OC").cloned() });
        }
        let dflt = cs.decode_default(bpc as u32);
        let dec: Vec<f64> = if decode_arr.len() == 2 * n { decode_arr.clone() } else { dflt.clone() };
        let plain = dec == dflt;
        if let Some(rgb) = &rgb_direct {
            for i in 0..w * h {
                let p = &mut px[i * 4..i * 4 + 4];
                p[..3].copy_from_slice(&rgb[i * 3..i * 3 + 3]);
                p[3] = 255;
            }
        } else if n == 1 && bpc <= 8 {
            // one component: a table of every sample value
            let lut: Vec<[u8; 3]> = (0..=maxv as u32)
                .map(|s| {
                    let v = dec[0] + s as f64 * (dec[1] - dec[0]) / maxv;
                    let c = cs.rgb(&[v]);
                    [super::to8(c[0]), super::to8(c[1]), super::to8(c[2])]
                })
                .collect();
            for y in 0..h {
                for x in 0..w {
                    let c = lut[smp.get(x, y, 0) as usize];
                    let p = &mut px[(y * w + x) * 4..][..4];
                    p[..3].copy_from_slice(&c);
                    p[3] = 255;
                }
            }
        } else if bpc == 8 && plain && matches!(&*cs, Cs::Rgb | Cs::Cmyk) {
            for i in 0..w * h {
                let y = i / w;
                let x = i % w;
                let o = y * smp.stride + x * n;
                let s = &samples.get(o..o + n).unwrap_or(&[0, 0, 0, 0][..n]);
                let p = &mut px[i * 4..i * 4 + 4];
                if n == 3 {
                    p[..3].copy_from_slice(&s[..3]);
                } else {
                    let f = |v: u8| v as f64 / 255.0;
                    let c = super::color::cmyk_rgb(f(s[0]), f(s[1]), f(s[2]), f(s[3]));
                    p[0] = super::to8(c[0]);
                    p[1] = super::to8(c[1]);
                    p[2] = super::to8(c[2]);
                }
                p[3] = 255;
            }
        } else {
            let mut comps = vec![0f64; n];
            let mut last: Option<(Vec<u32>, [u8; 3])> = None;
            let mut raw = vec![0u32; n];
            for y in 0..h {
                for x in 0..w {
                    for c in 0..n {
                        raw[c] = smp.get(x, y, c);
                    }
                    let col = match &last {
                        Some((r, col)) if *r == raw => *col,
                        _ => {
                            for c in 0..n {
                                comps[c] = dec[2 * c] + raw[c] as f64 * (dec[2 * c + 1] - dec[2 * c]) / maxv;
                            }
                            let v = cs.rgb(&comps);
                            let col = [super::to8(v[0]), super::to8(v[1]), super::to8(v[2])];
                            last = Some((raw.clone(), col));
                            col
                        }
                    };
                    let p = &mut px[(y * w + x) * 4..][..4];
                    p[..3].copy_from_slice(&col);
                    p[3] = 255;
                }
            }
        }
        let mut img = Img { w, h, px, stencil: false, interpolate, oc: d.get("OC").cloned() };
        // a soft mask takes the place of Mask: SMask (ISO 32000-1, 8.9.5, Table 89), and the opacity
        // of a JPEG 2000 image, which stands for SMask (SMaskInData; SMask is not given then)
        let soft = jpx_alpha.is_some() || depth == 0 && matches!(doc.key(d, "SMask"), Object::Stream(_));
        if let Some(a) = jpx_alpha {
            let mut mpx = vec![0u8; w * h * 4];
            for (m, &a) in mpx.chunks_exact_mut(4).zip(&a) {
                m.fill(a);
            }
            img.apply_alpha(&Img { w, h, px: mpx, stencil: false, interpolate: false, oc: None }, true);
        }
        // masks
        match doc.key(d, "Mask") {
            _ if soft => {}
            Object::Array(ranges) if rgb_direct.is_none() => {
                let mut r: Vec<i64> = ranges.iter().filter_map(|v| doc.resolve(v).as_int()).collect();
                if r.len() >= 2 * n {
                    // the ranges name the samples of a JPEG 2000 image before they were scaled
                    for (c, &(max, to)) in jpx_scaled.iter().enumerate().take(n) {
                        (r[2 * c], r[2 * c + 1]) = scale_range(r[2 * c], r[2 * c + 1], max, to);
                    }
                    for y in 0..h {
                        for x in 0..w {
                            if (0..n).all(|c| {
                                let s = smp.get(x, y, c) as i64;
                                s >= r[2 * c] && s <= r[2 * c + 1]
                            }) {
                                img.px[(y * w + x) * 4..][..4].fill(0);
                            }
                        }
                    }
                }
            }
            Object::Stream(ms) if depth == 0 => {
                if let Ok((mdata, mf)) = doc.decode_until_image(&ms) {
                    let mut md = ms.dict.clone();
                    md.insert("ImageMask", Object::Bool(true));
                    if let Some(m) = Img::load(run, &md, mdata, mf, res, 1) {
                        img.apply_alpha(&m, false);
                    }
                }
            }
            _ => {}
        }
        if let Object::Stream(ss) = doc.key(d, "SMask") {
            if depth == 0 {
                if let Ok((sdata, sf)) = doc.decode_until_image(&ss) {
                    let mut sd = ss.dict.clone();
                    sd.insert("ColorSpace", Object::Name(Name(b"DeviceGray".to_vec())));
                    let matte: Vec<f64> = super::color::nums(doc, &doc.key(&ss.dict, "Matte"));
                    if let Some(m) = Img::load(run, &sd, sdata, sf, res, 1) {
                        if matte.len() == n && m.w == img.w && m.h == img.h {
                            let mc = cs.rgb(&matte);
                            img.unmatte([mc[0] * 255.0, mc[1] * 255.0, mc[2] * 255.0], &m);
                        }
                        img.apply_alpha(&m, true);
                    }
                }
            }
        }
        Some(img)
    }

    fn from_rgb(rgb: Vec<u8>, w: usize, h: usize) -> Img {
        let mut px = vec![0u8; w * h * 4];
        for i in 0..w * h {
            px[i * 4..i * 4 + 3].copy_from_slice(&rgb[i * 3..i * 3 + 3]);
            px[i * 4 + 3] = 255;
        }
        Img { w, h, px, stencil: false, interpolate: false, oc: None }
    }

    /// the colours of an image premultiplied with the matte colour back to straight ones
    fn unmatte(&mut self, matte: [f64; 3], m: &Img) {
        for i in 0..self.w * self.h {
            let a = m.px[i * 4] as f64 / 255.0;
            if a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let v = self.px[i * 4 + c] as f64;
                let s = matte[c] + (v - matte[c]) / a;
                self.px[i * 4 + c] = s.clamp(0.0, 255.0) as u8;
            }
        }
    }

    /// multiply the alpha by a mask (soft: its grey, else its stencil alpha), sampled at the
    /// nearest position when the sizes differ; the colours become premultiplied
    fn apply_alpha(&mut self, m: &Img, soft: bool) {
        for y in 0..self.h {
            let my = (y * m.h / self.h).min(m.h - 1);
            for x in 0..self.w {
                let mx = (x * m.w / self.w).min(m.w - 1);
                let mo = (my * m.w + mx) * 4;
                let a = if soft { m.px[mo] } else { m.px[mo + 3] } as u32;
                let p = &mut self.px[(y * self.w + x) * 4..][..4];
                let na = canvas::mul255(p[3] as u32, a);
                p[3] = na as u8;
            }
        }
        self.premultiply();
    }

    fn premultiply(&mut self) {
        for p in self.px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a < 255 {
                for c in &mut p[..3] {
                    *c = canvas::mul255(*c as u32, a) as u8;
                }
            }
        }
    }

    /// the image averaged down to nw x nh (each new pixel the mean of the area it covers)
    fn shrink(&self, nw: usize, nh: usize) -> Img {
        let (w, h) = (self.w, self.h);
        // horizontal, then vertical, in 16.16 fixed point
        let spans = |from: usize, to: usize| -> Vec<(usize, usize, u32, u32)> {
            // for each output pixel: first and last source pixel, their weights (of 65536)
            (0..to)
                .map(|i| {
                    let a = i as u64 * from as u64 * 65536 / to as u64;
                    let b = (i as u64 + 1) * from as u64 * 65536 / to as u64;
                    let s0 = (a >> 16) as usize;
                    let s1 = (((b + 65535) >> 16) as usize).min(from).max(s0 + 1) - 1;
                    let w0 = 65536 - (a & 0xFFFF) as u32;
                    let w1 = if b & 0xFFFF == 0 { 65536 } else { (b & 0xFFFF) as u32 };
                    (s0, s1, w0, w1)
                })
                .collect()
        };
        let xs = spans(w, nw);
        let ys = spans(h, nh);
        // the pass between the two: 8 bytes for each of its pixels (16 before, and only
        // across first: an image narrowed little but made much lower took 16 bytes for each of
        // its own pixels); a large one goes down first when that leaves less
        let first = |v: u64, tot: u64| ((v * 256 + tot / 2) / tot.max(1)) as u16;
        let second = |v: u64, tot: u64| ((v / tot.max(1) + 128) >> 8).min(255) as u8;
        let mut px = vec![0u8; nw * nh * 4];
        if nw * h > w * nh && nw * h > 1 << 22 {
            let mut tmp = vec![0u16; w * nh * 4];
            average(w, &ys, |x, s, c| self.px[(s * w + x) * 4 + c] as u64, |x, j, c, v, t| tmp[(j * w + x) * 4 + c] = first(v, t));
            average(nh, &xs, |j, s, c| tmp[(j * w + s) * 4 + c] as u64, |j, i, c, v, t| px[(j * nw + i) * 4 + c] = second(v, t));
        } else {
            let mut tmp = vec![0u16; nw * h * 4];
            average(h, &xs, |y, s, c| self.px[(y * w + s) * 4 + c] as u64, |y, i, c, v, t| tmp[(y * nw + i) * 4 + c] = first(v, t));
            average(nw, &ys, |x, s, c| tmp[(s * nw + x) * 4 + c] as u64, |x, j, c, v, t| px[(j * nw + x) * 4 + c] = second(v, t));
        }
        Img { w: nw, h: nh, px, stencil: self.stencil, interpolate: self.interpolate, oc: None }
    }
}

/// one axis averaged: for each of `lines` lines and each span (first and last source pixel and
/// their weights of 65536), `put(line, span, channel, weighted sum, total weight)` of the values
/// `get(line, source pixel, channel)`
fn average(lines: usize, spans: &[(usize, usize, u32, u32)], get: impl Fn(usize, usize, usize) -> u64, mut put: impl FnMut(usize, usize, usize, u64, u64)) {
    for line in 0..lines {
        for (i, &(s0, s1, w0, w1)) in spans.iter().enumerate() {
            let mut acc = [0u64; 4];
            let mut tot = 0u64;
            for s in s0..=s1 {
                let wt = if s0 == s1 { 65536 } else if s == s0 { w0 } else if s == s1 { w1 } else { 65536 } as u64;
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += get(line, s, c) * wt;
                }
                tot += wt;
            }
            for (c, &a) in acc.iter().enumerate() {
                put(line, i, c, a, tot);
            }
        }
    }
}

/// an image's pixels on the device: device position -> image position -> premultiplied RGBA
struct ImgShader<'a> {
    img: &'a Img,
    /// device -> image pixel space
    m: M,
    nearest: bool,
    /// a stencil's paint
    paint: Option<&'a Src<'a>>,
}

impl ImgShader<'_> {
    #[inline]
    fn sample(&self, sx: f64, sy: f64) -> [u8; 4] {
        let img = self.img;
        let (w, h) = (img.w as i64, img.h as i64);
        if self.nearest {
            let x = (sx.max(0.0) as i64).min(w - 1) as usize;
            let y = (sy.max(0.0) as i64).min(h - 1) as usize;
            let p = &img.px[(y * img.w + x) * 4..][..4];
            return [p[0], p[1], p[2], p[3]];
        }
        let fx = (sx - 0.5).clamp(0.0, (w - 1) as f64);
        let fy = (sy - 0.5).clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(img.w - 1), (y0 + 1).min(img.h - 1));
        let (tx, ty) = (((fx - x0 as f64) * 256.0) as u32, ((fy - y0 as f64) * 256.0) as u32);
        let px = |x: usize, y: usize, c: usize| img.px[(y * img.w + x) * 4 + c] as u32;
        let mut out = [0u8; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let top = px(x0, y0, c) * (256 - tx) + px(x1, y0, c) * tx;
            let bot = px(x0, y1, c) * (256 - tx) + px(x1, y1, c) * tx;
            *o = ((top * (256 - ty) + bot * ty + 32768) >> 16) as u8;
        }
        out
    }
}

impl Shader for ImgShader<'_> {
    fn row(&self, y: i32, x: i32, out: &mut [[u8; 4]]) {
        let mut col: Vec<[u8; 4]> = Vec::new();
        if let Some(Src::Shade(s)) = self.paint {
            col.resize(out.len(), [0; 4]);
            s.row(y, x, &mut col);
        }
        for (i, o) in out.iter_mut().enumerate() {
            let (sx, sy) = apply(&self.m, x as f64 + i as f64 + 0.5, y as f64 + 0.5);
            let p = self.sample(sx, sy);
            *o = match self.paint {
                Some(Src::Solid(c)) => [c[0], c[1], c[2], p[3]],
                Some(Src::Shade(_)) => {
                    let c = col[i];
                    [c[0], c[1], c[2], canvas::mul255(c[3] as u32, p[3] as u32) as u8]
                }
                None => {
                    // straight colour from the premultiplied sample
                    let a = p[3] as u32;
                    if a == 0 {
                        [0; 4]
                    } else if a == 255 {
                        p
                    } else {
                        let un = |v: u8| ((v as u32 * 255 + a / 2) / a).min(255) as u8;
                        [un(p[0]), un(p[1]), un(p[2]), p[3]]
                    }
                }
            };
        }
    }
}

pub(super) fn draw(run: &mut Run, img: &Img) {
    if run.stopped() || run.hidden > 0 {
        return;
    }
    let ctm = run.gs.ctm;
    // device -> unit square -> image pixels (row 0 at the top: v = 1)
    let Some(ic) = inv(&ctm) else { return };
    let to_px: M = mul(&ic, &[img.w as f64, 0.0, 0.0, -(img.h as f64), 0.0, img.h as f64]);
    // how many device pixels an image pixel spans along each axis
    let lx = super::color::sqrt(ctm[0] * ctm[0] + ctm[1] * ctm[1]);
    let ly = super::color::sqrt(ctm[2] * ctm[2] + ctm[3] * ctm[3]);
    let (fx, fy) = (lx / img.w as f64, ly / img.h as f64);
    let shrunk;
    let mut use_img = img;
    let mut m = to_px;
    if (fx < 1.0 || fy < 1.0) && img.w * img.h > 1 {
        let nw = ((super::color::ceil(lx) as usize).max(1)).min(img.w);
        let nh = ((super::color::ceil(ly) as usize).max(1)).min(img.h);
        if (nw < img.w || nh < img.h) && run.spend((img.w * img.h) as u64) {
            shrunk = img.shrink(nw, nh);
            use_img = &shrunk;
            m = mul(&to_px, &[nw as f64 / img.w as f64, 0.0, 0.0, nh as f64 / img.h as f64, 0.0, 0.0]);
        }
    }
    let nearest = !img.interpolate && (fx >= 4.0 || fy >= 4.0) || use_img.w == 1 && use_img.h == 1;
    // the unit square on the device
    let mut p = Path::default();
    for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().enumerate() {
        let (x, y) = apply(&ctm, u, v);
        if i == 0 {
            p.move_f(x as f32, y as f32);
        } else {
            p.line_f(x as f32, y as f32);
        }
    }
    p.close();
    p.fill_lines(&mut run.ras);
    if img.stencil {
        let use_img: &Img = use_img;
        run.paint(false, &mut |r, src, alpha| {
            let shader = ImgShader { img: use_img, m, nearest, paint: Some(src) };
            let Run { layers, ras, gs, .. } = r;
            let Some(layer) = layers.last_mut() else { return };
            let put = Put { clip: &gs.clip, alpha, blend: gs.blend, smask: gs.smask.as_deref() };
            let n = canvas::fill(layer, ras, RULE_NONZERO, &Src::Shade(&shader), &put);
            r.spend(n);
        });
    } else {
        let shader = ImgShader { img: use_img, m, nearest, paint: None };
        let Run { layers, ras, gs, .. } = run;
        let Some(layer) = layers.last_mut() else { return };
        let put = Put { clip: &gs.clip, alpha: super::to8(gs.fill_alpha), blend: gs.blend, smask: gs.smask.as_deref() };
        let n = canvas::fill(layer, ras, RULE_NONZERO, &Src::Shade(&shader), &put);
        run.spend(n);
    }
}
