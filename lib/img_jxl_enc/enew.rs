// enew -- the new JPEG XL writer (jxl-plus): options, effort mapping, the lossless modular
// frame (libjxl 0.11.1 enc_modular.cc ComputeEncodingData / PrepareStreamParams, enc_frame.cc;
// the speed tiers of enc_params.h)                                       [jxl-plus: no asm]
//
//   Opts                       the parsed ImgOpt: effort 1..9 (0 = 7), quality, flags, blobs
//                              (img_jxl::writer, re-exported; jxl-module)
//   encode(sf, opts) -> file   lossless (modular) or lossy (VarDCT, evdct.rs)
//   encode_px(px, opts) -> file   the same on the samples of image_of (the module's entry)
//   tier(effort)               libjxl SpeedTier: 10 - effort (effort 1 behaves like 2)
//   read_image(sf) -> Px       the 8-bit samples: gray (R = G = B everywhere), alpha (a pixel
//                              not opaque) (img_jxl::writer, re-exported; jxl-module)
//
// Lossless, as libjxl at the same effort: global palettes (full / channel), YCoCg at efforts
// 1..4, per-group local palettes and an RCT search at 5+, the weighted predictor and a learned
// MA tree (one tree for all groups) at 4+ (fixed WP / gradient trees below), WP parameter
// search at 8+, LZ77 at 8+.  Groups are prepared, sampled and tokenized on the worker pool.

use alloc::vec;
use alloc::vec::Vec;

use img_core::{ImgOpt, Surface, ERR_CANCEL, IOF_EMBED_EXIF, IOF_EMBED_XMP};

use crate::eans::HParams;
use crate::efrm::{self, Colour, Dims, FrameHdr, ImgHdr};
use crate::ema::{Kind, PRED_GRADIENT, PRED_WEIGHTED};
use crate::emod::{self, MOpts, Stream, Streams, WP_MODES};
use crate::enc::{je_container, EncParams};
use crate::ewr::Jw;
use crate::modular::{Mc, Mi};

// jxl-module: the options, the samples and the flags live in img_jxl::writer (the host reads
// the samples and hands them to the module); the names stay here
pub use img_jxl::writer::{
    image_of, read_image, Opts, Px, IOF_JXL_1THREAD, IOF_JXL_COMPAT, IOF_JXL_NOISE_MASK, IOF_JXL_NOISE_SHIFT,
    IOF_JXL_NOPATCHES, IOF_JXL_PROGRESSIVE,
};

/// lossless: one tree per group (false: one global tree)
const LOCAL_TREES: bool = false;

/// libjxl SpeedTier of an effort
pub fn tier(effort: u32) -> i32 {
    (10 - effort.clamp(2, 9)) as i32
}

/// the colour encoding: ICC (RGB or gray profile) or sRGB / gray; whether the image is coded
/// as one gray channel
pub fn colour_of<'a>(px: &Px, icc: Option<&'a [u8]>) -> (Colour, bool, Option<&'a [u8]>, bool) {
    if let Some(p) = icc
        && p.len() >= 132
        && &p[36..40] == b"acsp"
    {
        let gray_profile = &p[16..20] == b"GRAY";
        if gray_profile && px.gray {
            return (Colour::Icc, true, Some(p), true);
        }
        if !gray_profile && &p[16..20] == b"RGB " {
            return (Colour::Icc, false, Some(p), false);
        }
    }
    if px.gray { (Colour::Gray, false, None, true) } else { (Colour::Srgb, false, None, false) }
}

/// encode (see the module header)
pub fn encode(sf: &Surface, o: &Opts) -> Result<Vec<u8>, i32> {
    let px = image_of(sf)?;
    encode_px(&px, o)
}

/// encode the samples of image_of (jxl-module: what the module runs; the linked writer too)
pub fn encode_px(px: &Px, o: &Opts) -> Result<Vec<u8>, i32> {
    let mut cs = if o.lossless() { encode_lossless(px, o)? } else { crate::evdct::encode_lossy(px, o)? };
    let p = EncParams { flags: o.flags & (IOF_EMBED_EXIF | IOF_EMBED_XMP), quality: 0, exif: o.exif, xmp: o.xmp, cancel: None };
    je_container(&p, &mut cs)?;
    Ok(cs)
}

/// the writer with the test-only spline bundle q in the frame (jxl-tools: libjxl's encoder
/// cannot write splines; the decoder's checks use this)
pub fn encode_with_splines(sf: &Surface, opt: &ImgOpt, q: &crate::spline::Quantized) -> Result<Vec<u8>, i32> {
    let mut o = Opts::from_imgopt(opt);
    o.splines = Some(q);
    encode(sf, &o)
}

/// the number of MA tree properties and the property value limit of a speed tier
fn tier_props(tier: i32) -> (usize, usize) {
    match tier {
        5 => (4, 24),
        4 => (5, 32),
        3 => (7, 48),
        2 => (10, 96),
        0 | 1 => (16, 256),
        _ => (3, 16),
    }
}

/// the modular options of a speed tier (lossless, not responsive)
pub fn lossless_opts(tier: i32, nstreams: usize, gdim: u32) -> MOpts {
    let mut order: Vec<u32> = vec![0, 1, 15, 9, 10, 11, 12, 13, 14, 2, 3, 4, 5, 6, 7, 8];
    if nstreams < 30 && tier > 1 {
        order.remove(1);
    }
    let (n, mv) = tier_props(tier);
    let props: Vec<u32> = order.iter().copied().take(n.min(order.len())).collect();
    let predictor = if tier < 7 {
        14
    } else if tier == 7 {
        PRED_WEIGHTED
    } else {
        PRED_GRADIENT
    };
    let kind = if tier < 7 {
        Kind::Learn
    } else if tier == 7 {
        Kind::WpFixedDc
    } else {
        Kind::GradientFixedDc
    };
    MOpts {
        max_chan: gdim,
        nb_repeats: 0.5,
        props,
        threshold: (82 + 14 * tier) as f32,
        max_vals: mv,
        predictor,
        wp_mode: 0,
        fast_mult: 1.01,
        wp_tree: 0,
        kind,
        hp: HParams::for_modular(tier, true, false),
        max_props: 0,
    }
}

/// the colour / alpha channels of the image as a modular image
fn modular_image(px: &Px, gray: bool) -> Mi {
    let n = (px.w * px.h) as usize;
    let mut mi = Mi::new(px.w, px.h, 8);
    let nc = if gray { 1 } else { 3 };
    for c in 0..nc {
        let d: Vec<i32> = (0..n).map(|i| px.rgba[i * 4 + c] as i32).collect();
        mi.ch.push(Mc { w: px.w, h: px.h, hs: 0, vs: 0, d });
    }
    if px.alpha {
        let d: Vec<i32> = (0..n).map(|i| px.rgba[i * 4 + 3] as i32).collect();
        mi.ch.push(Mc { w: px.w, h: px.h, hs: 0, vs: 0, d });
    }
    mi
}

/// a group's crop of the full image: the channels from the first one larger than a group,
/// within the shift bracket
fn crop(full: &Mi, rect: [u32; 4], gdim: u32, mins: i32, maxs: i32) -> Mi {
    let mut gi = Mi::new(rect[2], rect[3], full.bd);
    let mut first = full.nbmeta as usize;
    while first < full.ch.len() {
        let c = &full.ch[first];
        if c.w > gdim || c.h > gdim {
            break;
        }
        first += 1;
    }
    for c in &full.ch[first..] {
        let m = c.hs.min(c.vs);
        if m > maxs || m < mins {
            continue;
        }
        let hs = c.hs.max(0) as u32;
        let vs = c.vs.max(0) as u32;
        let x0 = rect[0] >> hs;
        let y0 = rect[1] >> vs;
        let xs = rect[2] >> hs;
        let ys = rect[3] >> vs;
        let w = if c.w > x0 { (c.w - x0).min(xs) } else { 0 };
        let hh = if c.h > y0 { (c.h - y0).min(ys) } else { 0 };
        if w == 0 || hh == 0 {
            continue;
        }
        let mut d = Vec::with_capacity((w * hh) as usize);
        for y in 0..hh {
            let o = ((y0 + y) * c.w + x0) as usize;
            d.extend_from_slice(&c.d[o..o + w as usize]);
        }
        gi.ch.push(Mc { w, h: hh, hs: c.hs, vs: c.vs, d });
    }
    gi
}

/// the number of RCTs tried at a speed tier (5 and below)
fn rct_count(tier: i32) -> usize {
    match tier {
        5 => 4,
        4 => 5,
        3 => 7,
        2 => 9,
        _ => 19,
    }
}

/// PrepareStreamParams for one stream image: local palettes, RCT search, WP preset
fn prepare(img: &mut Mi, tier: i32, local: bool, colour_chans: usize, threads: bool) -> usize {
    if local && tier < 6 {
        emod::try_palettes(img, tier, 80.0, 1024);
    }
    if tier <= 5 && colour_chans >= 3 && img.ch.len() - img.nbmeta as usize >= 3 {
        emod::rct_search(img, rct_count(tier), threads);
    }
    let modes = if tier <= 1 {
        5
    } else if tier <= 2 {
        2
    } else {
        1
    };
    let mut best = 0usize;
    if modes > 1 {
        let mut bc = f32::MAX;
        for m in 0..modes {
            let c = emod::est_wp_cost(img, m);
            if c < bc {
                bc = c;
                best = m;
            }
        }
    }
    best
}

/// the lossless modular codestream
pub fn encode_lossless(px: &Px, o: &Opts) -> Result<Vec<u8>, i32> {
    let t = tier(o.effort());
    let threads = o.threads();
    let responsive = o.flags & IOF_JXL_PROGRESSIVE != 0;
    let (colour, gray_icc, icc, gray) = colour_of(px, o.icc);
    let gshift = if px.w <= 400 && px.h <= 400 { 2 } else { 1 };
    let dims = Dims::new(px.w, px.h, gshift);
    let nstreams = dims.num_streams(1);
    let mut opts = lossless_opts(t, nstreams, dims.gdim);
    let colour_chans = if gray { 1 } else { 3 };
    // the global image: palettes, YCoCg at low efforts (always when responsive), squeeze
    let mut full = modular_image(px, gray);
    // jxl-tools: patches out of the image into a reference frame
    let patches = if o.patches(true) { lossless_patches(px, &mut full, gray, t, threads, o)? } else { None };
    let pal_percent = if t <= 8 { 95.0 } else { 0.0 };
    emod::try_palettes(&mut full, t, pal_percent, 1024);
    if t > 5 && colour_chans >= 3 && full.ch.len() - full.nbmeta as usize >= 3 {
        let b = full.nbmeta as usize;
        emod::fwd_rct(&mut full, b, 6);
    }
    if responsive && t <= 5 && colour_chans >= 3 && full.ch.len() - full.nbmeta as usize >= 3 {
        // jxl-plus: squeeze is global, so is the colour transform: the RCT search of the groups
        // on the whole image (libjxl: none)
        emod::rct_search_squeeze(&mut full, rct_count(t), threads);
    }
    if responsive {
        // Responsive (libjxl): squeeze, the zero predictor, properties for squeeze residuals
        emod::fwd_squeeze(&mut full);
        let order: [u32; 16] = [0, 1, 4, 5, 6, 7, 8, 15, 9, 10, 11, 12, 13, 14, 2, 3];
        opts.props = order.iter().copied().take(tier_props(t).0).collect();
        opts.predictor = crate::ema::PRED_ZERO;
        opts.kind = Kind::Learn;
    }
    if o.cancelled() {
        return Err(ERR_CANCEL);
    }
    let feat = crate::efeat::Feat { patches: patches.as_ref().map(|p| &p.1), splines: o.splines, ..Default::default() };
    let mut lf = Jw::new();
    feat.write(&mut lf); // jxl-tools
    lf.bits(1, 1); // default DC quantization
    let local_prep = !responsive;
    let sections = modular_sections(full, &dims, t, threads, &opts, if local_prep { colour_chans } else { 0 }, lf, o)?;
    // codestream
    let mut out = Jw::new();
    let ih = ImgHdr { w: px.w, h: px.h, bits: 8, alpha: px.alpha, xyb: false, colour, gray_icc, icc };
    efrm::write_image(&mut out, &ih);
    if let Some((rf, _)) = patches.as_ref() {
        out.append(rf);
    }
    let fh = FrameHdr {
        modular: true,
        xyb: false,
        flags: feat.flags(),
        nec: px.alpha as u32,
        gshift,
        gab: false,
        epf: 0,
        ..FrameHdr::default()
    };
    efrm::assemble(&mut out, &fh, &sections);
    out.pad();
    if patches.is_some() && o.effort() == 9 {
        // jxl-tools: effort 9 keeps the smaller of the files with and without patches (exact
        // repetitions can code better through LZ77 than as patches)
        let alt = encode_lossless(px, &Opts { flags: o.flags | IOF_JXL_NOPATCHES, ..*o })?;
        if alt.len() < out.buf.len() {
            return Ok(alt);
        }
    }
    Ok(out.buf)
}

/// patches of a lossless image (jxl-tools): found on the colour values / 255 as libjxl does,
/// subtracted from the modular image's colour channels (exactly: integers); the reference
/// frame (lossless modular, gradient predictor as libjxl's patch frame) and the dictionary
fn lossless_patches(px: &Px, full: &mut Mi, gray: bool, t: i32, threads: bool, o: &Opts) -> Result<Option<(Jw, crate::epatch::Dict)>, i32> {
    let (w, h) = (px.w as usize, px.h as usize);
    let n = w * h;
    let k = 1.0f32 / 255.0;
    let pl: [Vec<f32>; 3] = core::array::from_fn(|c| (0..n).map(|i| px.rgba[i * 4 + c] as f32 * k).collect());
    let found = crate::epatch::find([&pl[0], &pl[1], &pl[2]], w, w, h, false);
    if found.is_empty() {
        return Ok(None);
    }
    let pk = crate::epatch::pack(found, px.alpha as u32);
    let (rw, rh) = (pk.w, pk.h);
    let nc = if gray { 1 } else { 3 };
    // the patches as integers (colour minus background, exact multiples of 1 / 255)
    let ri: Vec<Vec<i32>> = (0..nc).map(|c| pk.planes[c].iter().map(|&v| crate::emath::roundf(v * 255.0) as i32).collect()).collect();
    for (rp, ps) in pk.dict.refs.iter().zip(&pk.dict.pos) {
        for &(x, y) in ps {
            for iy in 0..rp.h as usize {
                for ix in 0..rp.w as usize {
                    let ro = (rp.y0 as usize + iy) * rw + rp.x0 as usize + ix;
                    let io = (y as usize + iy) * w + x as usize + ix;
                    for c in 0..nc {
                        full.ch[c].d[io] -= ri[c][ro];
                    }
                }
            }
        }
    }
    // the reference frame
    let mut mi = Mi::new(rw as u32, rh as u32, 8);
    for d in ri {
        mi.ch.push(Mc { w: rw as u32, h: rh as u32, hs: 0, vs: 0, d });
    }
    if px.alpha {
        mi.ch.push(Mc { w: rw as u32, h: rh as u32, hs: 0, vs: 0, d: vec![0; rw * rh] });
    }
    // the image's lossless tools (palettes, colour transforms), the gradient predictor
    let pal_percent = if t <= 8 { 95.0 } else { 0.0 };
    emod::try_palettes(&mut mi, t, pal_percent, 1024);
    if t > 5 && nc >= 3 && mi.ch.len() - mi.nbmeta as usize >= 3 {
        let b = mi.nbmeta as usize;
        emod::fwd_rct(&mut mi, b, 6);
    }
    let gshift = if rw <= 400 && rh <= 400 { 2 } else { 1 };
    let dims = Dims::new(rw as u32, rh as u32, gshift);
    let mut opts = lossless_opts(t, dims.num_streams(1), dims.gdim);
    opts.predictor = PRED_GRADIENT;
    opts.kind = Kind::Learn;
    let mut lf = Jw::new();
    lf.bits(1, 1); // default DC quantization
    let sections = modular_sections(mi, &dims, t, threads, &opts, nc, lf, o)?;
    let fh = FrameHdr {
        typ: 2,
        size: Some((rw as u32, rh as u32)),
        modular: true,
        xyb: false,
        nec: px.alpha as u32,
        gshift,
        gab: false,
        epf: 0,
        islast: false,
        saveref: crate::epatch::PATCH_REF,
        ..FrameHdr::default()
    };
    let mut rf = Jw::new();
    efrm::assemble(&mut rf, &fh, &sections);
    Ok(Some((rf, pk.dict)))
}

/// the sections of a modular frame (one pass) for the image `full` (global transforms done):
/// LfGlobal = lf (the bits before the global tree) + tree / histograms + the global stream,
/// the DC groups (channels squeezed by 8 and more), an empty AC global, the AC groups.
/// colour_chans: 3 = RCT search per group (0: none); groups prepared and coded on the pool
pub fn modular_sections(full: Mi, dims: &Dims, t: i32, threads: bool, opts: &MOpts, colour_chans: usize, mut lf: Jw, o: &Opts) -> Result<Vec<Jw>, i32> {
    let nstreams = dims.num_streams(1);
    let mut ss = Streams::new(nstreams, t);
    if dims.ng == 1 {
        let mut full = full;
        let wpm = prepare(&mut full, t, false, colour_chans, threads);
        let mut s = Stream::new(full, opts.clone());
        s.wp = WP_MODES[wpm];
        ss.st[0] = Some(s);
    } else {
        let gd = dims.gdim;
        let full_ref = &full;
        let groups: Vec<(Mi, usize)> = emod::par_map(dims.ng as usize, threads, |g| {
            let gx = g as u32 % dims.xg;
            let gy = g as u32 / dims.xg;
            let rect = [gx * gd, gy * gd, gd, gd];
            let mut gi = crop(full_ref, rect, gd, 0, 2);
            let wpm = if gi.ch.is_empty() { 0 } else { prepare(&mut gi, t, true, colour_chans, threads) };
            (gi, wpm)
        });
        for (g, (gi, wpm)) in groups.into_iter().enumerate() {
            let id = dims.sid_modular_ac(g as u32, 0);
            let mut s = Stream::new(gi, opts.clone());
            s.wp = WP_MODES[wpm];
            ss.st[id] = Some(s);
        }
        // channels squeezed by 8 or more go to the DC groups
        let dd = gd * 8;
        for g in 0..dims.ndg {
            let gx = g % dims.xdg;
            let gy = g / dims.xdg;
            let gi = crop(full_ref, [gx * dd, gy * dd, dd, dd], gd, 3, 1000);
            if !gi.ch.is_empty() {
                ss.st[dims.sid_modular_dc(g)] = Some(Stream::new(gi, opts.clone()));
            }
        }
        // the global stream keeps the transforms, the meta channels and the small channels
        let mut s = Stream::new(full, opts.clone());
        s.wp = crate::modular::WH_DEF;
        ss.st[0] = Some(s);
    }
    if o.cancelled() {
        return Err(ERR_CANCEL);
    }
    let mut parts: Vec<Option<Jw>> = (0..nstreams).map(|_| None).collect();
    if LOCAL_TREES {
        // local trees (libjxl below effort 10): every stream learns and codes its own
        let ids: Vec<usize> = (0..nstreams).filter(|&i| ss.st[i].is_some()).collect();
        let cells: Vec<rt::sync::SpinLock<Option<Stream>>> = ids.iter().map(|&i| rt::sync::SpinLock::new(ss.st[i].take())).collect();
        let coded: Vec<Jw> = emod::par_map(ids.len(), threads, |k| {
            let mut g = cells[k].lock();
            match g.as_mut() {
                Some(s) => emod::encode_local(s, ids[k] as u32, threads),
                None => Jw::new(),
            }
        });
        for (k, jw) in coded.into_iter().enumerate() {
            parts[ids[k]] = Some(jw);
        }
        lf.bits(0, 1); // no global tree
        if let Some(p) = parts[0].take() {
            lf.append(&p);
        }
    } else {
        // one global tree and histogram set for all groups (libjxl effort 10); groups gathered
        // and tokenized in parallel
        ss.learn_tree(threads);
        ss.tokens(threads);
        if o.cancelled() {
            return Err(ERR_CANCEL);
        }
        let mut hp = opts.hp.clone();
        hp.par = threads;
        ss.write_global(&mut lf, &hp);
        ss.write_stream(0, &mut lf);
        let ssr = &ss;
        let w: Vec<Jw> = emod::par_map(nstreams, threads, |i| {
            let mut s = Jw::new();
            if i != 0 {
                ssr.write_stream(i, &mut s);
            }
            s
        });
        for (i, jw) in w.into_iter().enumerate() {
            parts[i] = Some(jw);
        }
    }
    if o.cancelled() {
        return Err(ERR_CANCEL);
    }
    Ok(if dims.ng == 1 {
        vec![lf]
    } else {
        let mut v = Vec::with_capacity(dims.ntoc(1));
        v.push(lf);
        for g in 0..dims.ndg {
            v.push(parts[dims.sid_modular_dc(g)].take().unwrap_or_default());
        }
        v.push(Jw::new()); // AC global
        for g in 0..dims.ng {
            v.push(parts[dims.sid_modular_ac(g, 0)].take().unwrap_or_default());
        }
        v
    })
}
