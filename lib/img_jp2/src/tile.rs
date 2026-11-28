// tile -- one tile from its packets to its samples (ITU-T T.800 Annex B, E, F, G)
//
// The geometry (B.5 to B.7): each tile-component falls into resolutions, each resolution into
// bands (LL at resolution 0; HL, LH, HH above), each band into precincts and code-blocks.  The
// packets come in the progression order of COD or of the POC volumes (B.12), a packet at most once
// per precinct and layer; a packet header (B.10) gives per code-block its first inclusion (tag
// tree), its missing bit-planes (tag tree), its new passes and their lengths (one length per
// codeword segment the passes reach into).  Packet headers come from the data, or from PPM / PPT.
// Then every code-block is decoded (t1), dequantised (E.1: reversible integers, or the step size
// of its band; ROI max-shift first), the wavelet runs up from resolution 0 (dwt), the components
// get their inverse multiple-component transform (G.2: RCT or ICT), the DC level shift (G.1) and
// are clamped to their precision.
//
// Damaged or truncated data: a packet whose header runs out ends the tile (what is decoded so far
// stays), so does a code-block that is given more passes than its bit-planes hold; a body that
// runs out ends the tile after the contribution it cuts (its arithmetic-coded part is kept, raw
// passes are not, nothing after it).  The position progressions visit the precincts there are
// (not every point of the grid); `MAX_VISITS` bounds the packets looked at.

use crate::codestream::{div_ceil, Codestream, Global, Params, Poc, Quant, Style};
use crate::t1::{raw_pass, seg_max_passes, Seg, MAX_P0, T1};
use crate::{dwt, Error};
use alloc::vec;
use alloc::vec::Vec;

/// limits of one decode against hostile files
#[derive(Default)]
pub(crate) struct Budget {
    pub cblks: u64,
    pub precincts: u64,
    /// the packets the progressions looked at (sent or not) and their layer and resolution passes
    pub visits: u64,
    /// the bytes the current tile holds in its structures (resolutions, precincts, code-blocks, the
    /// coded data, the progression lists), at most `max_bytes`
    pub bytes: u64,
    pub max_bytes: u64,
}

impl Budget {
    /// `n` more bytes held by the tile
    fn hold(&mut self, n: u64) -> Result<(), Error> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.max_bytes {
            return Err(Error::Limit);
        }
        Ok(())
    }
}

pub(crate) const MAX_CBLKS: u64 = 1 << 22;
pub(crate) const MAX_PRECINCTS: u64 = 1 << 22;
pub(crate) const MAX_VISITS: u64 = 1 << 28;

/// ceil(a / 2^s)
pub(crate) fn ceil_shift(a: u32, s: u32) -> u32 {
    if s >= 32 {
        return (a != 0) as u32;
    }
    ((a as u64 + (1u64 << s) - 1) >> s) as u32
}

/// ceil(a / 2^s) for a that may be negative
fn ceil_shift_i(a: i64, s: u32) -> i64 {
    let d = 1i64 << s.min(62);
    if a >= 0 {
        (a + d - 1) / d
    } else {
        -((-a) / d)
    }
}

fn pow2(k: i32) -> f64 {
    let k = k.clamp(-1022, 1023);
    f64::from_bits(((k + 1023) as u64) << 52)
}

// ------------------------------------------------------------------ packet header bits

/// the bits of packet headers (B.10.1): MSB first, after a 0xFF byte only 7 bits of the next
struct Bits<'a> {
    d: &'a [u8],
    p: usize,
    buf: u32,
    ct: u32,
    last_ff: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8], p: usize) -> Bits<'a> {
        Bits { d, p, buf: 0, ct: 0, last_ff: false }
    }

    #[inline]
    fn bit(&mut self) -> Option<u32> {
        if self.ct == 0 {
            let b = *self.d.get(self.p)?;
            self.p += 1;
            self.ct = if self.last_ff { 7 } else { 8 };
            self.last_ff = b == 0xFF;
            self.buf = b as u32;
        }
        self.ct -= 1;
        Some((self.buf >> self.ct) & 1)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    /// the end of a header: the rest of the byte, and the byte after a 0xFF (its stuffed bit)
    fn align(&mut self) {
        self.ct = 0;
        if self.last_ff {
            self.p += 1;
            self.last_ff = false;
        }
    }

    /// skip a marker (EPH) when it is next
    fn skip_marker(&mut self, m: u8) {
        if self.d.get(self.p) == Some(&0xFF) && self.d.get(self.p + 1) == Some(&m) {
            self.p += 2;
        }
    }
}

// ------------------------------------------------------------------ tag trees (B.10.2)

struct TagTree {
    /// per level (leaves first): width, height, offset into the nodes
    levels: Vec<(u32, u32, usize)>,
    low: Vec<u32>,
    val: Vec<u32>,
}

impl TagTree {
    /// the nodes of a tree over w x h leaves (none without leaves)
    fn nodes(w: u32, h: u32) -> u64 {
        if w == 0 || h == 0 {
            return 0;
        }
        let (mut lw, mut lh, mut n) = (w as u64, h as u64, 0u64);
        loop {
            n += lw * lh;
            if lw == 1 && lh == 1 {
                return n;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
    }

    fn new(w: u32, h: u32) -> TagTree {
        let mut levels = Vec::new();
        if w == 0 || h == 0 {
            // a band without code-blocks in this precinct: nothing is ever decoded from it
            return TagTree { levels, low: Vec::new(), val: Vec::new() };
        }
        let (mut lw, mut lh, mut off) = (w, h, 0usize);
        loop {
            levels.push((lw, lh, off));
            off += (lw * lh) as usize;
            if lw == 1 && lh == 1 {
                break;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
        TagTree { levels, low: vec![0; off], val: vec![u32::MAX; off] }
    }

    /// whether the value of leaf (x, y) is below `t`, reading the bits that tell
    fn decode(&mut self, b: &mut Bits, x: u32, y: u32, t: u32) -> Option<bool> {
        let mut low = 0u32;
        let mut leaf = 0usize;
        for l in (0..self.levels.len()).rev() {
            let (lw, _, off) = self.levels[l];
            let i = off + ((x >> l) + (y >> l) * lw) as usize;
            if low > self.low[i] {
                self.low[i] = low;
            } else {
                low = self.low[i];
            }
            while low < t && low < self.val[i] {
                if b.bit()? == 1 {
                    self.val[i] = low;
                } else {
                    low += 1;
                }
            }
            self.low[i] = low;
            leaf = i;
        }
        Some(self.val[leaf] < t)
    }
}

// ------------------------------------------------------------------ the geometry

struct SegBuf {
    first: u32,
    passes: u32,
    data: Vec<u8>,
}

struct Cblk {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    included: bool,
    zbp: u32,
    lblock: u32,
    passes: u32,
    segs: Vec<SegBuf>,
}

/// the code-blocks of one band in one precinct
struct PBand {
    ncx: u32,
    ncy: u32,
    incl: TagTree,
    zbp: TagTree,
    cblks: Vec<Cblk>,
}

enum Coefs {
    Int(Vec<i32>),
    Float(Vec<f32>),
}

struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH
    orient: u8,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    xcb: u32,
    ycb: u32,
    /// the magnitude bit-planes Mb (E.1.1)
    mb: u32,
    /// the step size (irreversible)
    delta: f32,
}

struct Res {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    ppx: u32,
    ppy: u32,
    npx: u32,
    npy: u32,
    bands: Vec<Band>,
    precincts: Vec<Option<Vec<PBand>>>,
}

struct TComp {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    dx: u32,
    dy: u32,
    levels: u32,
    reversible: bool,
    cbstyle: u8,
    roi: u32,
    prec: u32,
    signed: bool,
    res: Vec<Res>,
}

fn band_rect(t: (u32, u32, u32, u32), nb: u32, xo: u32, yo: u32) -> (u32, u32, u32, u32) {
    let off = |v: u32, o: u32| -> u32 {
        let a = v as i64 - if nb == 0 { 0 } else { (o as i64) << (nb - 1) };
        ceil_shift_i(a, nb).max(0) as u32
    };
    (off(t.0, xo), off(t.1, yo), off(t.2, xo), off(t.3, yo))
}

fn build_comp(rect: (u32, u32, u32, u32), c: &crate::codestream::CompSiz, st: &Style, q: &Quant, roi: u8, budget: &mut Budget) -> Result<TComp, Error> {
    let tc = (div_ceil(rect.0, c.dx), div_ceil(rect.1, c.dy), div_ceil(rect.2, c.dx), div_ceil(rect.3, c.dy));
    let nl = st.levels as u32;
    let mut res = Vec::with_capacity(nl as usize + 1);
    for r in 0..=nl {
        let s = nl - r;
        let (x0, y0, x1, y1) = (ceil_shift(tc.0, s), ceil_shift(tc.1, s), ceil_shift(tc.2, s), ceil_shift(tc.3, s));
        let (ppx, ppy) = st.precincts.get(r as usize).copied().unwrap_or((15, 15));
        let (ppx, ppy) = (ppx as u32, ppy as u32);
        let npx = if x1 > x0 { ceil_shift(x1, ppx) - (x0 >> ppx) } else { 0 };
        let npy = if y1 > y0 { ceil_shift(y1, ppy) - (y0 >> ppy) } else { 0 };
        let np = npx as u64 * npy as u64;
        budget.precincts += np;
        if budget.precincts > MAX_PRECINCTS {
            return Err(Error::Limit);
        }
        let kinds: &[(u8, u32, u32)] = if r == 0 { &[(0, 0, 0)] } else { &[(1, 1, 0), (2, 0, 1), (3, 1, 1)] };
        budget.hold((size_of::<Res>() + kinds.len() * size_of::<Band>()) as u64 + np * size_of::<Option<Vec<PBand>>>() as u64)?;
        let nb = if r == 0 { nl } else { nl - r + 1 };
        let sub = (r > 0) as u32;
        let mut bands = Vec::with_capacity(kinds.len());
        for &(orient, xo, yo) in kinds {
            let br = band_rect(tc, nb, xo, yo);
            let bi = if r == 0 { 0 } else { 1 + 3 * (r as usize - 1) + (orient as usize - 1) };
            let (eps, mu) = if q.style == 1 {
                let (e0, m0) = q.steps[0];
                ((e0 as i32 - (nl as i32 - nb as i32)).max(0) as u32, m0)
            } else {
                let (e, m) = *q.steps.get(bi).ok_or(Error::Corrupt)?;
                (e as u32, m)
            };
            let gain = [0, 1, 1, 2][orient as usize];
            let mb = (q.guard as u32 + eps).saturating_sub(1);
            let rb = c.prec as i32 + gain;
            let delta = (pow2(rb - eps as i32) * (1.0 + mu as f64 / 2048.0)) as f32;
            bands.push(Band {
                orient,
                x0: br.0,
                y0: br.1,
                x1: br.2,
                y1: br.3,
                xcb: (st.xcb as u32).min(ppx.saturating_sub(sub)),
                ycb: (st.ycb as u32).min(ppy.saturating_sub(sub)),
                mb,
                delta,
            });
        }
        let mut precincts = Vec::new();
        precincts.resize_with(np as usize, || None);
        res.push(Res { x0, y0, x1, y1, ppx, ppy, npx, npy, bands, precincts });
    }
    Ok(TComp {
        x0: tc.0,
        y0: tc.1,
        x1: tc.2,
        y1: tc.3,
        dx: c.dx,
        dy: c.dy,
        levels: nl,
        reversible: st.reversible,
        cbstyle: st.cbstyle,
        roi: roi as u32,
        prec: c.prec as u32,
        signed: c.signed,
        res,
    })
}

/// the code-blocks of precinct k of resolution r, made the first time a packet reaches it
fn precinct(res: &mut Res, r: u32, k: usize, budget: &mut Budget) -> Result<(), Error> {
    if res.precincts[k].is_some() {
        return Ok(());
    }
    let (px, py) = (k as u32 % res.npx, k as u32 / res.npx);
    let sub = (r > 0) as u32;
    let mut out = Vec::with_capacity(res.bands.len());
    for b in &res.bands {
        let (ex, ey) = (res.ppx.saturating_sub(sub), res.ppy.saturating_sub(sub));
        // the precinct in band coordinates
        let gx = (res.x0 >> res.ppx) + px;
        let gy = (res.y0 >> res.ppy) + py;
        let px0 = ((gx as u64) << ex).max(b.x0 as u64);
        let py0 = ((gy as u64) << ey).max(b.y0 as u64);
        let px1 = (((gx as u64) + 1) << ex).min(b.x1 as u64);
        let py1 = (((gy as u64) + 1) << ey).min(b.y1 as u64);
        let (mut ncx, mut ncy, mut cbx0, mut cby0) = (0u32, 0u32, 0u32, 0u32);
        if px1 > px0 && py1 > py0 {
            cbx0 = (px0 >> b.xcb) as u32;
            cby0 = (py0 >> b.ycb) as u32;
            ncx = (((px1 + (1u64 << b.xcb) - 1) >> b.xcb) as u32) - cbx0;
            ncy = (((py1 + (1u64 << b.ycb) - 1) >> b.ycb) as u32) - cby0;
        }
        let n = ncx as u64 * ncy as u64;
        budget.cblks += n;
        if budget.cblks > MAX_CBLKS {
            return Err(Error::Limit);
        }
        // the band, its code-blocks and its two tag trees (the values and the lowest bounds)
        budget.hold(size_of::<PBand>() as u64 + n * size_of::<Cblk>() as u64 + 2 * TagTree::nodes(ncx, ncy) * 8)?;
        let mut cblks = Vec::with_capacity(n as usize);
        for j in 0..ncy {
            for i in 0..ncx {
                let x0 = (((cbx0 + i) as u64) << b.xcb).max(px0) as u32;
                let y0 = (((cby0 + j) as u64) << b.ycb).max(py0) as u32;
                let x1 = (((cbx0 + i + 1) as u64) << b.xcb).min(px1) as u32;
                let y1 = (((cby0 + j + 1) as u64) << b.ycb).min(py1) as u32;
                cblks.push(Cblk { x0, y0, x1, y1, included: false, zbp: 0, lblock: 3, passes: 0, segs: Vec::new() });
            }
        }
        out.push(PBand { ncx, ncy, incl: TagTree::new(ncx, ncy), zbp: TagTree::new(ncx, ncy), cblks });
    }
    res.precincts[k] = Some(out);
    Ok(())
}

// ------------------------------------------------------------------ packets

/// the number of new coding passes (Table B.4)
fn num_passes(b: &mut Bits) -> Option<u32> {
    if b.bit()? == 0 {
        return Some(1);
    }
    if b.bit()? == 0 {
        return Some(2);
    }
    let v = b.bits(2)?;
    if v < 3 {
        return Some(3 + v);
    }
    let v = b.bits(5)?;
    if v < 31 {
        return Some(6 + v);
    }
    Some(37 + b.bits(7)?)
}

struct Contrib {
    band: usize,
    cb: usize,
    new_seg: Option<u32>,
    take: u32,
    len: u32,
}

/// the state of the packet stream of a tile
struct Stream<'a> {
    body: &'a [u8],
    pos: usize,
    hdr: Option<Bits<'a>>,
    sop: bool,
    eph: bool,
    contribs: Vec<Contrib>,
}

/// read packet (layer, r, k) of component c; false when the data ran out
fn packet(s: &mut Stream, tc: &mut TComp, r: u32, k: usize, layer: u32, budget: &mut Budget) -> Result<bool, Error> {
    let roi = tc.roi;
    let res = &mut tc.res[r as usize];
    if k >= res.precincts.len() {
        return Ok(true);
    }
    precinct(res, r, k, budget)?;
    // the magnitude bit-planes of each band with the ROI shift
    let mut planes = [0u32; 3];
    for (p, b) in planes.iter_mut().zip(&res.bands) {
        *p = b.mb + roi;
    }
    if s.sop && s.body.get(s.pos) == Some(&0xFF) && s.body.get(s.pos + 1) == Some(&0x91) {
        s.pos = (s.pos + 6).min(s.body.len());
    }
    let style = tc.cbstyle;
    let in_band = s.hdr.is_none();
    let mut own = Bits::new(s.body, s.pos);
    let b: &mut Bits = match s.hdr.as_mut() {
        Some(h) => h,
        None => &mut own,
    };
    s.contribs.clear();
    let pbands = res.precincts[k].as_mut().ok_or(Error::Corrupt)?;
    let Some(present) = b.bit() else { return Ok(false) };
    if present == 1 {
        for (bi, pb) in pbands.iter_mut().enumerate() {
            for cy in 0..pb.ncy {
                for cx in 0..pb.ncx {
                    let leaf = (cx + cy * pb.ncx) as usize;
                    let included = pb.cblks[leaf].included;
                    let inc = if included {
                        let Some(v) = b.bit() else { return Ok(false) };
                        v == 1
                    } else {
                        let Some(v) = pb.incl.decode(b, cx, cy, layer + 1) else { return Ok(false) };
                        v
                    };
                    if !inc {
                        continue;
                    }
                    if !included {
                        let mut i = 0u32;
                        loop {
                            let Some(v) = pb.zbp.decode(b, cx, cy, i + 1) else { return Ok(false) };
                            if v {
                                break;
                            }
                            i += 1;
                            if i > 74 {
                                return Err(Error::Corrupt);
                            }
                        }
                        let cb = &mut pb.cblks[leaf];
                        cb.zbp = i;
                        cb.included = true;
                        cb.lblock = 3;
                    }
                    let Some(n) = num_passes(b) else { return Ok(false) };
                    let cb = &mut pb.cblks[leaf];
                    // a cleanup pass in the first bit-plane, three passes in each one below: more
                    // than that is damage (and would make segments without end)
                    let most = (3 * planes[bi].saturating_sub(cb.zbp)).saturating_sub(2);
                    if cb.passes + n > most {
                        return Ok(false);
                    }
                    loop {
                        let Some(v) = b.bit() else { return Ok(false) };
                        if v == 0 {
                            break;
                        }
                        cb.lblock += 1;
                        if cb.lblock > 32 {
                            return Err(Error::Corrupt);
                        }
                    }
                    // the codeword segments the new passes reach into
                    let mut last = cb.segs.last().map(|g| (g.first, g.passes));
                    let mut total = cb.passes;
                    let mut left = n;
                    while left > 0 {
                        let (new_seg, room) = match last {
                            Some((first, passes)) if passes < seg_max_passes(style, first) => (None, seg_max_passes(style, first) - passes),
                            _ => (Some(total), seg_max_passes(style, total)),
                        };
                        let take = left.min(room);
                        let nbits = cb.lblock + (31 - take.leading_zeros());
                        if nbits > 32 {
                            return Err(Error::Corrupt);
                        }
                        let Some(len) = b.bits(nbits) else { return Ok(false) };
                        s.contribs.push(Contrib { band: bi, cb: leaf, new_seg, take, len });
                        last = Some(match new_seg {
                            Some(first) => (first, take),
                            None => {
                                let (f, p) = last.unwrap_or((total, 0));
                                (f, p + take)
                            }
                        });
                        total += take;
                        left -= take;
                    }
                }
            }
        }
    }
    b.align();
    if s.eph {
        b.skip_marker(0x92);
    }
    if in_band {
        s.pos = own.p;
    }
    // the bodies (the header may have ended past the data: a stuffed bit or a marker cut off)
    if s.pos > s.body.len() {
        return Ok(false);
    }
    for c in &s.contribs {
        let cb = &mut pbands[c.band].cblks[c.cb];
        let take = (c.len as usize).min(s.body.len() - s.pos);
        let whole = take == c.len as usize;
        // cut short: an arithmetic-coded part still decodes (its end reads as the padding of
        // C.3.4); raw bits that are not there would be made up
        if whole || take > 0 && !raw_pass(style, cb.passes) {
            // the coded bytes are kept (a copy of the data) and each segment's own record
            budget.hold(take as u64 + if c.new_seg.is_some() { size_of::<SegBuf>() as u64 } else { 0 })?;
            if let Some(first) = c.new_seg {
                cb.segs.push(SegBuf { first, passes: 0, data: Vec::new() });
            }
            if let Some(g) = cb.segs.last_mut() {
                g.passes += c.take;
                g.data.extend_from_slice(&s.body[s.pos..s.pos + take]);
            }
            cb.passes += c.take;
        }
        s.pos += take;
        if !whole {
            return Ok(false);
        }
    }
    Ok(true)
}

/// what the progression needs of a resolution
struct RGeo {
    x0: u32,
    y0: u32,
    ppx: u32,
    ppy: u32,
    npx: u32,
    npy: u32,
}

/// what the progression needs of a tile-component
struct Geo {
    dx: u32,
    dy: u32,
    levels: u32,
    res: Vec<RGeo>,
}

impl Geo {
    fn of(c: &TComp) -> Geo {
        let res = c.res.iter().map(|r| RGeo { x0: r.x0, y0: r.y0, ppx: r.ppx, ppy: r.ppy, npx: r.npx, npy: r.npy }).collect();
        Geo { dx: c.dx, dy: c.dy, levels: c.levels, res }
    }
}

/// a precinct where a position progression reaches it
#[derive(Clone, Copy)]
struct At {
    y: u64,
    x: u64,
    c: u32,
    r: u32,
    k: u32,
}

/// every precinct of the tile with the point of the reference grid where the position orders
/// (B.12.1.3 to B.12.1.5) reach it: they run over the grid (y, then x) and send the precinct of
/// (c, r) at (x, y) where y is a multiple of YRsiz * 2^(PPy + NL - r), or is ty0 when the
/// resolution does not start on such a line (the same for x).  Each precinct has exactly one such
/// point: its row j is reached at ty0 (j = 0, not aligned) or at (y0 / 2^PPy + j) * YRsiz *
/// 2^(PPy + NL - r), below ty1.
fn positions(comps: &[Geo], rect: (u32, u32, u32, u32), budget: &mut Budget) -> Result<Vec<At>, Error> {
    let mut v = Vec::new();
    // the point of row (column) j: tile start t0, resolution start s, precinct exponent pp (at most
    // 15), levels below lv, sub-sampling d
    let at = |j: u32, t0: u32, s: u32, pp: u32, lv: u32, d: u32| -> u64 {
        if j == 0 && s & ((1u32 << pp) - 1) != 0 {
            return t0 as u64;
        }
        let g = ((s >> pp) as u64 + j as u64) as u128;
        // below ty1 < 2^32 for each precinct there is (the shifts stay below 2^100)
        (((g << (pp + lv)) * d as u128).min(u64::MAX as u128)) as u64
    };
    for (c, g) in comps.iter().enumerate() {
        for (r, res) in g.res.iter().enumerate() {
            let lv = g.levels - r as u32;
            let n = res.npx as u64 * res.npy as u64;
            budget.visits += n;
            if budget.visits > MAX_VISITS {
                return Err(Error::Limit);
            }
            budget.hold(n * size_of::<At>() as u64)?;
            for py in 0..res.npy {
                let y = at(py, rect.1, res.y0, res.ppy, lv, g.dy);
                for px in 0..res.npx {
                    let x = at(px, rect.0, res.x0, res.ppx, lv, g.dx);
                    v.push(At { y, x, c: c as u32, r: r as u32, k: py * res.npx + px });
                }
            }
        }
    }
    Ok(v)
}

/// the progression of packets (B.12) with POC volumes: f(layer, r, c, k) for each packet, until it
/// returns false
fn progression(comps: &[Geo], rect: (u32, u32, u32, u32), g: &Global, pocs: &[Poc], budget: &mut Budget, mut f: impl FnMut(u32, u32, usize, usize, &mut Budget) -> Result<bool, Error>) -> Result<(), Error> {
    let nc = comps.len();
    let maxres = comps.iter().map(|c| c.levels + 1).max().unwrap_or(1);
    let layers = g.layers as u32;
    // the next layer of each precinct (a packet is sent once)
    let mut next: Vec<Vec<Vec<u16>>> = comps.iter().map(|c| c.res.iter().map(|r| vec![0u16; r.npx as usize * r.npy as usize]).collect()).collect();
    let default = [Poc { rs: 0, cs: 0, lye: g.layers, re: 33, ce: nc as u16, prog: g.prog }];
    let vols: &[Poc] = if pocs.is_empty() { &default } else { pocs };
    // per resolution the components that have precincts there: the layer-resolution orders pass
    // over these only (a tile of 16384 empty components and 65535 layers was 3.5 * 10^10 passes
    // that sent nothing and cost nothing)
    let live: Vec<Vec<usize>> = (0..maxres as usize).map(|r| (0..nc).filter(|&c| comps[c].res.get(r).is_some_and(|x| x.npx != 0 && x.npy != 0)).collect()).collect();
    // the live components c0 <= c < c1 of resolution r
    fn between(live: &[Vec<usize>], r: u32, c0: usize, c1: usize) -> &[usize] {
        let cs = &live[r as usize];
        &cs[cs.partition_point(|&c| c < c0)..cs.partition_point(|&c| c < c1)]
    }
    // the precincts in the order of each position progression (RPCL, PCRL, CPRL), made once
    let mut orders: [Option<Vec<At>>; 3] = [None, None, None];
    // emit (l, r, c, k) when it is the precinct's next layer
    let mut emit = |l: u32, r: u32, c: usize, k: usize, next: &mut Vec<Vec<Vec<u16>>>, budget: &mut Budget| -> Result<bool, Error> {
        budget.visits += 1;
        if budget.visits > MAX_VISITS {
            return Err(Error::Limit);
        }
        let n = &mut next[c][r as usize][k];
        if *n as u32 != l {
            return Ok(true);
        }
        *n += 1;
        f(l, r, c, k, budget)
    };
    for v in vols {
        let (l1, r0, r1, c0, c1) = ((v.lye as u32).min(layers), v.rs as u32, (v.re as u32).min(maxres), v.cs as usize, (v.ce as usize).min(nc));
        if r0 >= r1 || c0 >= c1 || l1 == 0 {
            continue;
        }
        if v.prog <= 1 {
            // the resolutions of the volume where a component has precincts; each pass over
            // (layer, resolution) counts
            let rs: Vec<u32> = (r0..r1).filter(|&r| !between(&live, r, c0, c1).is_empty()).collect();
            let lrcp = v.prog == 0;
            let (outer, inner) = if lrcp { (l1 as usize, rs.len()) } else { (rs.len(), l1 as usize) };
            for a in 0..outer {
                for bb in 0..inner {
                    let (l, r) = if lrcp { (a as u32, rs[bb]) } else { (bb as u32, rs[a]) };
                    budget.visits += 1;
                    if budget.visits > MAX_VISITS {
                        return Err(Error::Limit);
                    }
                    for &c in between(&live, r, c0, c1) {
                        let res = &comps[c].res[r as usize];
                        for k in 0..res.npx as usize * res.npy as usize {
                            if !emit(l, r, c, k, &mut next, budget)? {
                                return Ok(());
                            }
                        }
                    }
                }
            }
            continue;
        }
        let o = (v.prog as usize - 2).min(2);
        if orders[o].is_none() {
            let mut list = positions(comps, rect, budget)?;
            match o {
                0 => list.sort_unstable_by_key(|a| (a.r, a.y, a.x, a.c)),
                1 => list.sort_unstable_by_key(|a| (a.y, a.x, a.c, a.r)),
                _ => list.sort_unstable_by_key(|a| (a.c, a.y, a.x, a.r)),
            }
            orders[o] = Some(list);
        }
        let list = orders[o].as_ref().map_or(&[][..], |l| &l[..]);
        // (also when every packet of the list was sent before: many volumes cost their passes)
        budget.visits += list.len() as u64;
        if budget.visits > MAX_VISITS {
            return Err(Error::Limit);
        }
        for a in list {
            let (c, r, k) = (a.c as usize, a.r, a.k as usize);
            if c < c0 || c >= c1 || r < r0 || r >= r1 {
                continue;
            }
            for l in next[c][r as usize][k] as u32..l1 {
                if !emit(l, r, c, k, &mut next, budget)? {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ samples

/// a decoded tile-component plane at its resolution
pub(crate) enum Plane {
    Int(Vec<i32>),
    Float(Vec<f32>),
}

/// a component of the whole image at the decoded resolution
pub(crate) struct OutComp {
    pub w: u32,
    pub h: u32,
    pub x0: u32,
    pub y0: u32,
    pub data: Vec<i32>,
}

/// the band's coefficients from its code-blocks; none (an empty buffer) when no code-block of it
/// has data: the band is zero
fn band_coefs(tc: &TComp, r: usize, bi: usize, t1: &mut T1) -> Result<Coefs, Error> {
    let res = &tc.res[r];
    let b = &res.bands[bi];
    let (bw, bh) = ((b.x1 - b.x0) as usize, (b.y1 - b.y0) as usize);
    let coded = res.precincts.iter().flatten().any(|p| p[bi].cblks.iter().any(|cb| cb.passes != 0 && cb.x1 > cb.x0 && cb.y1 > cb.y0));
    let n = if coded { bw * bh } else { 0 };
    let mut out = if tc.reversible { Coefs::Int(vec![0; n]) } else { Coefs::Float(vec![0.0; n]) };
    if !coded {
        return Ok(out);
    }
    let mut segs: Vec<Seg> = Vec::new();
    for pbands in res.precincts.iter().flatten() {
        let pb = &pbands[bi];
        for cb in &pb.cblks {
            if cb.passes == 0 || cb.x1 <= cb.x0 || cb.y1 <= cb.y0 {
                continue;
            }
            let p0 = (b.mb + tc.roi) as i32 - 1 - cb.zbp as i32;
            if p0 > MAX_P0 {
                // more than 60 bit-planes (guard bits and an ROI shift at their largest)
                return Err(Error::Unsupported);
            }
            segs.clear();
            segs.extend(cb.segs.iter().map(|g| Seg { data: &g.data, passes: g.passes }));
            let (w, h) = ((cb.x1 - cb.x0) as usize, (cb.y1 - cb.y0) as usize);
            t1.decode(&segs, w, h, b.orient, tc.cbstyle, p0);
            let (ox, oy) = ((cb.x0 - b.x0) as usize, (cb.y0 - b.y0) as usize);
            let roi = tc.roi;
            for y in 0..h {
                let src = &t1.coef[y * w..][..w];
                let row = (oy + y) * bw + ox;
                for (x, &v) in src.iter().enumerate() {
                    if v == 0 {
                        continue;
                    }
                    let mut m = v.unsigned_abs();
                    if roi > 0 && (m >> 1) >= (1u64 << roi) {
                        m >>= roi;
                    }
                    match &mut out {
                        Coefs::Int(o) => {
                            let a = (m >> 1).min(i32::MAX as u64) as i32;
                            o[row + x] = if v < 0 { -a } else { a };
                        }
                        Coefs::Float(o) => {
                            let a = m as f32 * (b.delta * 0.5);
                            o[row + x] = if v < 0 { -a } else { a };
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// the tile-component's samples at resolution `top` (inverse wavelet from resolution 0); the
/// buffers of zero bands are empty until a level needs them
fn comp_plane(tc: &TComp, top: usize, t1: &mut T1) -> Result<Plane, Error> {
    let mut cur = band_coefs(tc, 0, 0, t1)?;
    if top == 0 {
        // resolution 0 only: its LL band is the plane
        let n = (tc.res[0].x1 - tc.res[0].x0) as usize * (tc.res[0].y1 - tc.res[0].y0) as usize;
        match &mut cur {
            Coefs::Int(v) if v.is_empty() => v.resize(n, 0),
            Coefs::Float(v) if v.is_empty() => v.resize(n, 0.0),
            _ => {}
        }
    }
    for r in 1..=top {
        let res = &tc.res[r];
        let hl = band_coefs(tc, r, 0, t1)?;
        let lh = band_coefs(tc, r, 1, t1)?;
        let hh = band_coefs(tc, r, 2, t1)?;
        cur = match (cur, hl, lh, hh) {
            (Coefs::Int(ll), Coefs::Int(hl), Coefs::Int(lh), Coefs::Int(hh)) => {
                let mut buf = dwt::interleave(&ll, &hl, &lh, &hh, res.x0, res.y0, res.x1, res.y1);
                dwt::level53(&mut buf, res.x0, res.y0, res.x1, res.y1);
                Coefs::Int(buf)
            }
            (Coefs::Float(ll), Coefs::Float(hl), Coefs::Float(lh), Coefs::Float(hh)) => {
                let mut buf = dwt::interleave(&ll, &hl, &lh, &hh, res.x0, res.y0, res.x1, res.y1);
                dwt::level97(&mut buf, res.x0, res.y0, res.x1, res.y1);
                Coefs::Float(buf)
            }
            _ => unreachable!(),
        };
    }
    Ok(match cur {
        Coefs::Int(v) => Plane::Int(v),
        Coefs::Float(v) => Plane::Float(v),
    })
}

/// round half to even: exact in f64 for |v| below 2^51 (every f32 there is), beyond that the
/// conversion saturates; the caller clamps to the precision
#[inline(always)]
fn round_even(v: f32) -> i32 {
    const MAGIC: f64 = 6_755_399_441_055_744.0; // 1.5 * 2^52
    ((v as f64 + MAGIC) - MAGIC) as i32
}

/// the parameters of component c in tile t: COC of the tile, its COD, COC of the main header,
/// its COD (A.6.1); the same for QCC / QCD and RGN
fn comp_params<'p>(main: &'p Params, tile: &'p Params, c: usize) -> Option<(&'p Style, &'p Quant, u8)> {
    let st = tile.coc.get(&c).or(tile.cod.as_ref()).or(main.coc.get(&c)).or(main.cod.as_ref())?;
    let q = tile.qcc.get(&c).or(tile.qcd.as_ref()).or(main.qcc.get(&c)).or(main.qcd.as_ref())?;
    let roi = tile.rgn.get(&c).or(main.rgn.get(&c)).copied().unwrap_or(0);
    Some((st, q, roi))
}

/// decode tile t into the components of the image (at `reduce` levels below full resolution)
pub(crate) fn decode(cs: &Codestream, t: usize, reduce: u32, budget: &mut Budget, out: &mut [OutComp], t1: &mut T1) -> Result<(), Error> {
    let Some(td) = cs.tiles[t].as_ref() else { return Ok(()) };
    let siz = &cs.siz;
    let rect = siz.tile_rect(t as u32);
    let tp = &td.params;
    let g = tp.global.or(cs.main.global).ok_or(Error::Corrupt)?;
    // what the tile holds is freed after it
    budget.bytes = 0;
    let mut comps = Vec::with_capacity(siz.comps.len());
    for (c, cz) in siz.comps.iter().enumerate() {
        let (st, q, roi) = comp_params(&cs.main, tp, c).ok_or(Error::Corrupt)?;
        comps.push(build_comp(rect, cz, st, q, roi, budget)?);
    }
    // the packets
    let body: Vec<u8> = if td.parts.len() == 1 { td.parts[0].to_vec() } else { td.parts.concat() };
    let mut s = Stream { body: &body, pos: 0, hdr: td.headers.as_ref().map(|h| Bits::new(h, 0)), sop: g.sop, eph: g.eph, contribs: Vec::new() };
    let pocs: &[Poc] = if tp.poc.is_empty() { &cs.main.poc } else { &tp.poc };
    let geo: Vec<Geo> = comps.iter().map(Geo::of).collect();
    progression(&geo, rect, &g, pocs, budget, |l, r, c, k, budget| packet(&mut s, &mut comps[c], r, k, l, budget))?;
    // the samples, one component after the other into the image (its code-blocks freed after it);
    // the first three wait for each other when a multiple-component transform joins them
    let mct = g.mct != 0 && comps.len() >= 3;
    let mut held: Vec<TPlane> = Vec::new();
    for c in 0..comps.len() {
        let tc = &comps[c];
        let red = reduce.min(tc.levels);
        let top = (tc.levels - red) as usize;
        let res = &tc.res[top];
        let p = TPlane { plane: comp_plane(tc, top, t1)?, x0: res.x0, y0: res.y0, x1: res.x1, y1: res.y1, red };
        for r in &mut comps[c].res {
            r.precincts = Vec::new();
        }
        if mct && c < 3 {
            held.push(p);
            if c == 2 {
                transform(&mut held);
                for (i, p) in held.drain(..).enumerate() {
                    put(&comps[i], &mut out[i], p, reduce);
                }
            }
        } else {
            put(&comps[c], &mut out[c], p, reduce);
        }
    }
    Ok(())
}

/// a decoded tile-component plane and its rectangle at the resolution it was decoded to (`red`
/// levels below its full one)
struct TPlane {
    plane: Plane,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    red: u32,
}

/// the multiple-component transform of the first three planes (when they cover the same area)
fn transform(planes: &mut [TPlane]) {
    let same = (1..3).all(|i| planes[i].x0 == planes[0].x0 && planes[i].y0 == planes[0].y0 && planes[i].x1 == planes[0].x1 && planes[i].y1 == planes[0].y1);
    if !same {
        return;
    }
    let (a, rest) = planes.split_at_mut(1);
    let (b, c) = rest.split_at_mut(1);
    match (&mut a[0].plane, &mut b[0].plane, &mut c[0].plane) {
        (Plane::Int(y), Plane::Int(u), Plane::Int(v)) => {
            for ((y, u), v) in y.iter_mut().zip(u.iter_mut()).zip(v.iter_mut()) {
                let g = y.wrapping_sub(u.wrapping_add(*v) >> 2);
                let r = v.wrapping_add(g);
                let bb = u.wrapping_add(g);
                *y = r;
                *u = g;
                *v = bb;
            }
        }
        (Plane::Float(y), Plane::Float(u), Plane::Float(v)) => {
            for ((y, u), v) in y.iter_mut().zip(u.iter_mut()).zip(v.iter_mut()) {
                let (yy, cb, cr) = (*y, *u, *v);
                *y = yy + 1.402 * cr;
                *u = yy - 0.344_13 * cb - 0.714_14 * cr;
                *v = yy + 1.772 * cb;
            }
        }
        _ => {}
    }
}

/// a tile-component's plane into the image: the DC level shift, clamped to the precision
fn put(tc: &TComp, o: &mut OutComp, p: TPlane, reduce: u32) {
    let TPlane { plane, x0, y0, x1, y1, red } = p;
    let shift = reduce - red;
    let (pw, ph) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let prec = tc.prec.min(31);
    let (lo, hi, dc) = if tc.signed {
        (-(1i64 << (prec - 1)), (1i64 << (prec - 1)) - 1, 0i64)
    } else {
        (0, (1i64 << prec) - 1, 1i64 << (prec - 1))
    };
    // the target rectangle at `reduce` levels
    let (tx0, ty0, tx1, ty1) = (ceil_shift(tc.x0, reduce.min(31)), ceil_shift(tc.y0, reduce.min(31)), ceil_shift(tc.x1, reduce.min(31)), ceil_shift(tc.y1, reduce.min(31)));
    for ty in ty0..ty1 {
        if ty < o.y0 || ty - o.y0 >= o.h {
            continue;
        }
        let sy = (((ty as u64) << shift) as u32).saturating_sub(y0) as usize;
        if sy >= ph {
            continue;
        }
        let orow = ((ty - o.y0) * o.w) as usize;
        for tx in tx0..tx1 {
            if tx < o.x0 || tx - o.x0 >= o.w {
                continue;
            }
            let sx = (((tx as u64) << shift) as u32).saturating_sub(x0) as usize;
            if sx >= pw {
                continue;
            }
            let v = match &plane {
                Plane::Int(p) => p[sy * pw + sx] as i64,
                Plane::Float(p) => round_even(p[sy * pw + sx]) as i64,
            };
            o.data[orow + (tx - o.x0) as usize] = (v + dc).clamp(lo, hi) as i32;
        }
    }
}
