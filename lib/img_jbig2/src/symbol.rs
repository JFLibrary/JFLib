// symbol -- symbol dictionaries (ITU-T T.88 6.5, 7.4.2) and text regions (6.4, 7.4.3)
//
// A symbol dictionary decodes its new symbols height class by height class: with arithmetic
// coding each symbol as a generic region, or (refinement / aggregate coding) as a refinement of
// one earlier symbol or as a small text region of several; with Huffman coding a height class is
// one collective bitmap (uncompressed or MMR) cut at the symbols' widths.  Then the export flags
// (run lengths) say which input and new symbols it exports.
//
// A text region places symbol instances strip by strip: the strip's T, the first instance's S,
// then S deltas until out-of-band; each instance's symbol id, an optional refinement, and its
// reference corner (the four corners, transposed or not) decide where it lands; it is combined
// with the region's operator.

use crate::bitmap::Bmp;
use crate::generic::{self, Gb, Gr};
use crate::huff::{self, Bits, Table};
use crate::mmr;
use crate::mq::{decode_id, decode_int, Cx, IntCx, Mq};
use crate::Error;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

/// the arithmetic contexts of the generic and refinement decoders (kept by a dictionary that
/// retains them)
#[derive(Clone)]
pub(crate) struct Stats {
    pub gb: Vec<Cx>,
    pub gr: Vec<Cx>,
}

impl Stats {
    pub fn new() -> Stats {
        Stats { gb: vec![Cx::default(); 1 << 16], gr: vec![Cx::default(); 1 << 13] }
    }
}

/// the integer contexts of a text region (and of a dictionary's aggregate coding)
pub(crate) struct TextCx {
    pub iadt: IntCx,
    pub iafs: IntCx,
    pub iads: IntCx,
    pub iait: IntCx,
    pub iari: IntCx,
    pub iardw: IntCx,
    pub iardh: IntCx,
    pub iardx: IntCx,
    pub iardy: IntCx,
    pub iaid: Vec<Cx>,
}

impl TextCx {
    pub fn new(codelen: u32) -> TextCx {
        TextCx {
            iadt: IntCx::new(),
            iafs: IntCx::new(),
            iads: IntCx::new(),
            iait: IntCx::new(),
            iari: IntCx::new(),
            iardw: IntCx::new(),
            iardh: IntCx::new(),
            iardx: IntCx::new(),
            iardy: IntCx::new(),
            iaid: vec![Cx::default(); 2usize << codelen.min(24)],
        }
    }
}

/// the Huffman tables of a text region
pub(crate) struct TextHuff {
    pub fs: Table,
    pub ds: Table,
    pub dt: Table,
    pub rdw: Table,
    pub rdh: Table,
    pub rdx: Table,
    pub rdy: Table,
    pub rsize: Table,
    /// the symbol ids (None: SBSYMCODELEN bits, in a dictionary's aggregate coding)
    pub id: Option<Table>,
}

/// where a text region's numbers come from
pub(crate) enum Src<'a, 'b> {
    Arith { mq: &'b mut Mq<'a>, cx: &'b mut TextCx },
    Huff { b: &'b mut Bits<'a>, t: &'b TextHuff },
}

/// the parameters of a text region (Table 9)
pub(crate) struct Text<'s> {
    pub w: usize,
    pub h: usize,
    pub instances: u32,
    pub strips: u32,
    pub refine: bool,
    pub refcorner: u8,
    pub transposed: bool,
    pub combop: u8,
    pub defpixel: u8,
    pub dsoffset: i32,
    pub rtemplate: u8,
    pub rat: [(i8, i8); 2],
    pub syms: &'s [Rc<Bmp>],
    pub codelen: u32,
}

#[derive(Clone, Copy)]
enum Which {
    Dt,
    Fs,
    Ds,
    It,
    Ri,
    Rdw,
    Rdh,
    Rdx,
    Rdy,
}

fn int(src: &mut Src, w: Which) -> Result<Option<i32>, Error> {
    match src {
        Src::Arith { mq, cx } => {
            let c = match w {
                Which::Dt => &mut cx.iadt,
                Which::Fs => &mut cx.iafs,
                Which::Ds => &mut cx.iads,
                Which::It => &mut cx.iait,
                Which::Ri => &mut cx.iari,
                Which::Rdw => &mut cx.iardw,
                Which::Rdh => &mut cx.iardh,
                Which::Rdx => &mut cx.iardx,
                Which::Rdy => &mut cx.iardy,
            };
            Ok(decode_int(mq, c))
        }
        Src::Huff { b, t } => {
            let tab = match w {
                Which::Dt => &t.dt,
                Which::Fs => &t.fs,
                Which::Ds => &t.ds,
                Which::Rdw => &t.rdw,
                Which::Rdh => &t.rdh,
                Which::Rdx => &t.rdx,
                Which::Rdy => &t.rdy,
                // CURT and RI are plain bits with Huffman coding (callers read them)
                Which::It | Which::Ri => return Err(Error::Corrupt),
            };
            tab.decode(b)
        }
    }
}

fn need(v: Option<i32>) -> Result<i32, Error> {
    v.ok_or(Error::Corrupt)
}

/// 6.4.5: the text region
pub(crate) fn text_region(src: &mut Src, p: &Text, gr: &mut [Cx], work: &mut u64, limit: u64, max: u64) -> Result<Bmp, Error> {
    let mut out = Bmp::new(p.w, p.h, p.defpixel, max)?;
    let strips = p.strips.max(1) as i64;
    let logstrips = strips.trailing_zeros();
    let mut stript = -(need(int(src, Which::Dt)?)? as i64) * strips;
    let mut firsts = 0i64;
    let mut n = 0u32;
    while n < p.instances {
        if matches!(src, Src::Arith { mq, .. } if mq.exhausted()) {
            return Err(Error::Truncated);
        }
        stript += need(int(src, Which::Dt)?)? as i64 * strips;
        firsts += need(int(src, Which::Fs)?)? as i64;
        let mut curs = firsts;
        let mut first = true;
        loop {
            if !first {
                match int(src, Which::Ds)? {
                    Some(ds) => curs += ds as i64 + p.dsoffset as i64,
                    None => break,
                }
            }
            first = false;
            if matches!(src, Src::Arith { mq, .. } if mq.exhausted()) {
                return Err(Error::Truncated);
            }
            let curt = if strips == 1 {
                0
            } else {
                match src {
                    Src::Huff { b, .. } => b.bits(logstrips)? as i64,
                    _ => need(int(src, Which::It)?)? as i64,
                }
            };
            let t = stript + curt;
            let id = match src {
                Src::Arith { mq, cx } => decode_id(mq, &mut cx.iaid, p.codelen) as usize,
                Src::Huff { b, t } => match &t.id {
                    Some(tab) => need(tab.decode(b)?)? as usize,
                    None => b.bits(p.codelen)? as usize,
                },
            };
            let sym = p.syms.get(id).ok_or(Error::Corrupt)?;
            let ri = if p.refine {
                match src {
                    Src::Huff { b, .. } => b.bit()? as i32,
                    _ => need(int(src, Which::Ri)?)?,
                }
            } else {
                0
            };
            let refined;
            let ib: &Bmp = if ri == 0 {
                sym
            } else {
                let rdw = need(int(src, Which::Rdw)?)?;
                let rdh = need(int(src, Which::Rdh)?)?;
                let rdx = need(int(src, Which::Rdx)?)?;
                let rdy = need(int(src, Which::Rdy)?)?;
                let w = sym.w as i64 + rdw as i64;
                let h = sym.h as i64 + rdh as i64;
                if w < 0 || h < 0 || w > 1 << 20 || h > 1 << 20 {
                    return Err(Error::Corrupt);
                }
                let g = Gr { w: w as usize, h: h as usize, template: p.rtemplate, reference: sym, dx: (rdw >> 1) + rdx, dy: (rdh >> 1) + rdy, tpgron: false, at: p.rat };
                *work = work.saturating_add((w * h) as u64);
                if *work > limit {
                    return Err(Error::Limit);
                }
                refined = match src {
                    Src::Arith { mq, .. } => generic::refine(mq, gr, &g, max)?,
                    Src::Huff { b, t } => {
                        let size = need(t.rsize.decode(b)?)? as usize;
                        b.align();
                        let start = b.byte_pos();
                        let data = b.d.get(start..start.saturating_add(size).min(b.d.len())).ok_or(Error::Truncated)?;
                        let mut mq = Mq::new(data);
                        let r = generic::refine(&mut mq, gr, &g, max)?;
                        b.pos = (start + size) * 8;
                        r
                    }
                };
                &refined
            };
            let (wi, hi) = (ib.w as i64, ib.h as i64);
            let right = p.refcorner & 2 != 0;
            let top = p.refcorner & 1 != 0;
            if !p.transposed && right {
                curs += wi - 1;
            } else if p.transposed && !top {
                curs += hi - 1;
            }
            let s = curs;
            let (x, y) = if !p.transposed {
                (if right { s - wi + 1 } else { s }, if top { t } else { t - hi + 1 })
            } else {
                (if right { t - wi + 1 } else { t }, if top { s } else { s - hi + 1 })
            };
            out.compose(ib, x, y, p.combop);
            if !p.transposed && !right {
                curs += wi - 1;
            } else if p.transposed && top {
                curs += hi - 1;
            }
            n += 1;
            *work = work.saturating_add(16 + (wi * hi).max(0) as u64);
            // a strip that never ends (hostile data): far more instances than the region has
            if *work > limit || n as u64 > p.instances as u64 * 4 + 4096 {
                return Err(Error::Limit);
            }
        }
    }
    Ok(out)
}

fn ceil_log2(n: u64) -> u32 {
    if n <= 1 {
        0
    } else {
        64 - (n - 1).leading_zeros()
    }
}

/// a symbol dictionary segment -> (the exported symbols, the contexts when it retains them)
pub(crate) fn dictionary(d: &[u8], input: &[Rc<Bmp>], custom: &[&Table], retained: Option<&Stats>, work: &mut u64, limit: u64, max: u64) -> Result<(Vec<Rc<Bmp>>, Option<Stats>), Error> {
    if d.len() < 2 {
        return Err(Error::Truncated);
    }
    let flags = u16::from_be_bytes([d[0], d[1]]);
    let sdhuff = flags & 1 != 0;
    let refagg = flags & 2 != 0;
    let (sel_dh, sel_dw) = ((flags >> 2) & 3, (flags >> 4) & 3);
    let (sel_bm, sel_agg) = ((flags >> 6) & 1, (flags >> 7) & 1);
    let ctx_used = flags & 0x100 != 0;
    let ctx_retained = flags & 0x200 != 0;
    let template = ((flags >> 10) & 3) as u8;
    let rtemplate = ((flags >> 12) & 1) as u8;
    let mut p = 2usize;
    let byte = |i: usize| -> Result<i8, Error> { d.get(i).map(|&b| b as i8).ok_or(Error::Truncated) };
    let mut at = [(0i8, 0i8); 4];
    if !sdhuff {
        let n = if template == 0 { 4 } else { 1 };
        for a in at.iter_mut().take(n) {
            *a = (byte(p)?, byte(p + 1)?);
            p += 2;
        }
    }
    let mut rat = [(0i8, 0i8); 2];
    if refagg && rtemplate == 0 {
        for a in rat.iter_mut() {
            *a = (byte(p)?, byte(p + 1)?);
            p += 2;
        }
    }
    let rd = |i: usize| -> Result<u32, Error> { d.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or(Error::Truncated) };
    let num_ex = rd(p)?;
    let num_new = rd(p + 4)? as usize;
    p += 8;
    if num_new > 1 << 20 || input.len() + num_new > 1 << 21 {
        return Err(Error::Limit);
    }
    let data = &d[p..];
    // the Huffman tables (custom ones in order)
    let mut ci = custom.iter();
    let mut pick = |sel: u16, std: &[u8]| -> Result<Table, Error> {
        match std.get(sel as usize) {
            Some(&k) if k != 0 => Ok(huff::standard(k)),
            _ => ci.next().map(|t| (*t).clone()).ok_or(Error::Corrupt),
        }
    };
    let huff = if sdhuff {
        Some((pick(sel_dh, &[4, 5, 0, 0])?, pick(sel_dw, &[2, 3, 0, 0])?, pick(sel_bm, &[1, 0])?, pick(sel_agg, &[1, 0])?))
    } else {
        None
    };
    let mut stats = match retained {
        Some(s) if ctx_used => s.clone(),
        _ => Stats::new(),
    };
    let total = input.len() + num_new;
    let codelen = ceil_log2(total as u64);
    let mut tcx = TextCx::new(codelen);
    let (mut iadh, mut iadw, mut iaex, mut iaai) = (IntCx::new(), IntCx::new(), IntCx::new(), IntCx::new());
    let mut mq = Mq::new(data);
    let mut bits = Bits::new(data);
    let mut new: Vec<Rc<Bmp>> = Vec::with_capacity(num_new);
    // the symbols a refinement or aggregate may refer to: the input, then the new ones so far
    let mut all: Vec<Rc<Bmp>> = if refagg { input.to_vec() } else { Vec::new() };
    let mut hcheight = 0i64;
    let text_huff = || TextHuff {
        fs: huff::standard(6),
        ds: huff::standard(8),
        dt: huff::standard(11),
        rdw: huff::standard(15),
        rdh: huff::standard(15),
        rdx: huff::standard(15),
        rdy: huff::standard(15),
        rsize: huff::standard(1),
        id: None,
    };
    while new.len() < num_new {
        if huff.is_none() && mq.exhausted() {
            return Err(Error::Truncated);
        }
        let dh = match &huff {
            Some(h) => need(h.0.decode(&mut bits)?)?,
            None => need(decode_int(&mut mq, &mut iadh))?,
        };
        hcheight += dh as i64;
        if !(0..=1 << 20).contains(&hcheight) {
            return Err(Error::Corrupt);
        }
        // a height class costs something even when it is empty (data that ran out repeats them)
        *work = work.saturating_add(64);
        if *work > limit {
            return Err(Error::Limit);
        }
        let mut symwidth = 0i64;
        let mut totwidth = 0i64;
        let first = new.len();
        let mut widths: Vec<usize> = Vec::new();
        loop {
            let dw = match &huff {
                Some(h) => h.1.decode(&mut bits)?,
                None => decode_int(&mut mq, &mut iadw),
            };
            let Some(dw) = dw else { break };
            if new.len() >= num_new {
                return Err(Error::Corrupt);
            }
            if huff.is_none() && mq.exhausted() {
                return Err(Error::Truncated);
            }
            symwidth += dw as i64;
            totwidth += symwidth;
            if !(0..=1 << 20).contains(&symwidth) || totwidth > 1 << 24 {
                return Err(Error::Corrupt);
            }
            let (w, h) = (symwidth as usize, hcheight as usize);
            *work = work.saturating_add((w * h) as u64 + 16);
            if *work > limit {
                return Err(Error::Limit);
            }
            if huff.is_some() && !refagg {
                widths.push(w);
                new.push(Rc::new(Bmp::default()));
                continue;
            }
            let bmp = if !refagg {
                generic::generic(&mut mq, &mut stats.gb, &Gb { w, h, template, tpgdon: false, at, skip: None }, max)?
            } else {
                let ninst = match &huff {
                    Some(hh) => need(hh.3.decode(&mut bits)?)?,
                    None => need(decode_int(&mut mq, &mut iaai))?,
                };
                let syms = &all;
                if ninst > 1 {
                    let tp = Text {
                        w,
                        h,
                        instances: ninst as u32,
                        strips: 1,
                        refine: true,
                        refcorner: 1,
                        transposed: false,
                        combop: 0,
                        defpixel: 0,
                        dsoffset: 0,
                        rtemplate,
                        rat,
                        syms,
                        codelen,
                    };
                    match &huff {
                        Some(_) => {
                            let th = text_huff();
                            text_region(&mut Src::Huff { b: &mut bits, t: &th }, &tp, &mut stats.gr, work, limit, max)?
                        }
                        None => text_region(&mut Src::Arith { mq: &mut mq, cx: &mut tcx }, &tp, &mut stats.gr, work, limit, max)?,
                    }
                } else {
                    // one refined symbol
                    let (id, rdx, rdy) = match &huff {
                        Some(_) => {
                            let id = bits.bits(codelen)? as usize;
                            let t15 = huff::standard(15);
                            let rdx = need(t15.decode(&mut bits)?)?;
                            let rdy = need(t15.decode(&mut bits)?)?;
                            (id, rdx, rdy)
                        }
                        None => (decode_id(&mut mq, &mut tcx.iaid, codelen) as usize, need(decode_int(&mut mq, &mut tcx.iardx))?, need(decode_int(&mut mq, &mut tcx.iardy))?),
                    };
                    let reference = syms.get(id).ok_or(Error::Corrupt)?;
                    let g = Gr { w, h, template: rtemplate, reference, dx: rdx, dy: rdy, tpgron: false, at: rat };
                    match &huff {
                        Some(_) => {
                            let size = need(huff::standard(1).decode(&mut bits)?)? as usize;
                            bits.align();
                            let start = bits.byte_pos();
                            let part = data.get(start..start.saturating_add(size).min(data.len())).ok_or(Error::Truncated)?;
                            let mut m2 = Mq::new(part);
                            let r = generic::refine(&mut m2, &mut stats.gr, &g, max)?;
                            bits.pos = (start + size) * 8;
                            r
                        }
                        None => generic::refine(&mut mq, &mut stats.gr, &g, max)?,
                    }
                }
            };
            let bmp = Rc::new(bmp);
            if refagg {
                all.push(bmp.clone());
            }
            new.push(bmp);
        }
        // the collective bitmap of a Huffman-coded height class
        if let (Some(h), false) = (&huff, refagg) {
            let size = need(h.2.decode(&mut bits)?)? as usize;
            bits.align();
            let start = bits.byte_pos();
            let (tw, hh) = (totwidth as usize, hcheight as usize);
            let collective = if size == 0 {
                let stride = tw.div_ceil(8);
                let mut b = Bmp::new(tw, hh, 0, max)?;
                for y in 0..hh {
                    for x in 0..tw {
                        let byte = *data.get(start + y * stride + x / 8).ok_or(Error::Truncated)?;
                        b.d[y * tw + x] = (byte >> (7 - x % 8)) & 1;
                    }
                }
                bits.pos = (start + stride * hh) * 8;
                b
            } else {
                let part = data.get(start..start.saturating_add(size).min(data.len())).ok_or(Error::Truncated)?;
                let (b, _) = mmr::decode(part, tw, hh, max)?;
                bits.pos = (start + size) * 8;
                b
            };
            let mut x = 0usize;
            for (k, &w) in widths.iter().enumerate() {
                new[first + k] = Rc::new(collective.sub(x as i64, 0, w, hh)?);
                x += w;
            }
        }
    }
    // the export flags
    let mut exported = Vec::with_capacity(num_ex.min(1 << 20) as usize);
    let mut index = 0usize;
    let mut flag = false;
    let mut runs = 0usize;
    while index < total {
        runs += 1;
        if runs > 2 * total + 4 {
            return Err(Error::Corrupt);
        }
        let run = match &huff {
            Some(_) => need(huff::standard(1).decode(&mut bits)?)?,
            None => need(decode_int(&mut mq, &mut iaex))?,
        };
        if run < 0 || index + run as usize > total {
            return Err(Error::Corrupt);
        }
        if flag {
            for i in index..index + run as usize {
                exported.push(if i < input.len() { input[i].clone() } else { new[i - input.len()].clone() });
            }
        }
        index += run as usize;
        flag = !flag;
    }
    Ok((exported, if ctx_retained { Some(stats) } else { None }))
}
