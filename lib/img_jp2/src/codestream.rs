// codestream -- the markers of a JPEG 2000 codestream (ITU-T T.800 Annex A)
//
// The main header (SIZ, COD, COC, QCD, QCC, RGN, POC, PPM; TLM, PLM, CRG, COM and unknown marker
// segments are skipped), then the tile-parts: SOT, their headers (COD, COC, QCD, QCC, RGN, POC,
// PPT; PLT, COM skipped), SOD and the data up to the next tile-part (Psot 0: up to EOC).  The
// data of a tile's parts is kept in order of appearance; packet headers given in PPM (one series
// per tile-part, in codestream order) or PPT (per tile, in Zppt order) are joined per tile.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

pub(crate) const MAX_COMPONENTS: usize = 16384;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CompSiz {
    pub prec: u8,
    pub signed: bool,
    pub dx: u32,
    pub dy: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Siz {
    pub x1: u32,
    pub y1: u32,
    pub x0: u32,
    pub y0: u32,
    pub tw: u32,
    pub th: u32,
    pub tx0: u32,
    pub ty0: u32,
    pub comps: Vec<CompSiz>,
}

impl Siz {
    pub fn tiles_x(&self) -> u32 {
        div_ceil(self.x1 - self.tx0, self.tw)
    }
    pub fn tiles_y(&self) -> u32 {
        div_ceil(self.y1 - self.ty0, self.th)
    }
    /// the tile's area on the reference grid
    pub fn tile_rect(&self, t: u32) -> (u32, u32, u32, u32) {
        let (p, q) = (t % self.tiles_x(), t / self.tiles_x());
        let x0 = (self.tx0 as u64 + p as u64 * self.tw as u64).max(self.x0 as u64) as u32;
        let y0 = (self.ty0 as u64 + q as u64 * self.th as u64).max(self.y0 as u64) as u32;
        let x1 = (self.tx0 as u64 + (p as u64 + 1) * self.tw as u64).min(self.x1 as u64) as u32;
        let y1 = (self.ty0 as u64 + (q as u64 + 1) * self.th as u64).min(self.y1 as u64) as u32;
        (x0, y0, x1, y1)
    }
}

pub(crate) fn div_ceil(a: u32, b: u32) -> u32 {
    ((a as u64 + b as u64 - 1) / b as u64) as u32
}

/// the coding style of a tile-component (SPcod / SPcoc and the precinct bit of Scod / Scoc)
#[derive(Clone, Debug)]
pub(crate) struct Style {
    pub levels: u8,
    /// code-block width and height exponents
    pub xcb: u8,
    pub ycb: u8,
    pub cbstyle: u8,
    pub reversible: bool,
    /// per resolution level (0 = lowest) the precinct exponents PPx, PPy
    pub precincts: Vec<(u8, u8)>,
}

/// the parts of COD that hold for all components
#[derive(Clone, Copy, Debug)]
pub(crate) struct Global {
    pub sop: bool,
    pub eph: bool,
    pub prog: u8,
    pub layers: u16,
    pub mct: u8,
}

#[derive(Clone, Debug)]
pub(crate) struct Quant {
    /// 0 none, 1 scalar derived, 2 scalar expounded
    pub style: u8,
    pub guard: u8,
    /// per band (one for scalar derived): exponent, mantissa
    pub steps: Vec<(u8, u16)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Poc {
    pub rs: u8,
    pub cs: u16,
    pub lye: u16,
    pub re: u8,
    pub ce: u16,
    pub prog: u8,
}

/// the parameters a header sets (the main header's, or a tile's)
#[derive(Clone, Debug, Default)]
pub(crate) struct Params {
    pub global: Option<Global>,
    pub cod: Option<Style>,
    pub coc: Vec<Option<Style>>,
    pub qcd: Option<Quant>,
    pub qcc: Vec<Option<Quant>>,
    pub rgn: Vec<Option<u8>>,
    pub poc: Vec<Poc>,
}

impl Params {
    fn new(n: usize) -> Params {
        Params { coc: vec![None; n], qcc: vec![None; n], rgn: vec![None; n], ..Params::default() }
    }
}

#[derive(Default)]
pub(crate) struct TileData<'a> {
    pub params: Params,
    /// the data of the tile-parts, in order
    pub parts: Vec<&'a [u8]>,
    /// the packet headers from PPM or PPT, joined (None: they are in the data)
    pub headers: Option<Vec<u8>>,
    ppt: Vec<(u8, Vec<u8>)>,
}

pub(crate) struct Codestream<'a> {
    pub siz: Siz,
    pub main: Params,
    pub tiles: Vec<Option<TileData<'a>>>,
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Result<u8, Error> {
        let v = *self.d.get(self.p).ok_or(Error::Truncated)?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(((self.u8()? as u16) << 8) | self.u8()? as u16)
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(((self.u16()? as u32) << 16) | self.u16()? as u32)
    }
    fn left(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
}

/// a component index (one byte when there are fewer than 257 components)
fn comp_index(r: &mut Rd, n: usize) -> Result<usize, Error> {
    Ok(if n < 257 { r.u8()? as usize } else { r.u16()? as usize })
}

fn read_siz(r: &mut Rd) -> Result<Siz, Error> {
    let _rsiz = r.u16()?;
    let x1 = r.u32()?;
    let y1 = r.u32()?;
    let x0 = r.u32()?;
    let y0 = r.u32()?;
    let tw = r.u32()?;
    let th = r.u32()?;
    let tx0 = r.u32()?;
    let ty0 = r.u32()?;
    let n = r.u16()? as usize;
    if x1 <= x0 || y1 <= y0 || tw == 0 || th == 0 || tx0 > x0 || ty0 > y0 || tx0 as u64 + tw as u64 <= x0 as u64 || ty0 as u64 + th as u64 <= y0 as u64 {
        return Err(Error::Corrupt);
    }
    if n == 0 || n > MAX_COMPONENTS {
        return Err(Error::Corrupt);
    }
    let mut comps = Vec::with_capacity(n);
    for _ in 0..n {
        let s = r.u8()?;
        let dx = r.u8()? as u32;
        let dy = r.u8()? as u32;
        let prec = (s & 0x7F) + 1;
        if dx == 0 || dy == 0 || prec > 38 {
            return Err(Error::Corrupt);
        }
        comps.push(CompSiz { prec, signed: s & 0x80 != 0, dx, dy });
    }
    let siz = Siz { x1, y1, x0, y0, tw, th, tx0, ty0, comps };
    if siz.tiles_x() as u64 * siz.tiles_y() as u64 > 65535 {
        return Err(Error::Corrupt);
    }
    Ok(siz)
}

fn read_style(r: &mut Rd, precincts: bool) -> Result<Style, Error> {
    let levels = r.u8()?;
    let xcb = r.u8()? + 2;
    let ycb = r.u8()? + 2;
    let cbstyle = r.u8()?;
    let transform = r.u8()?;
    if levels > 32 || xcb > 10 || ycb > 10 || xcb + ycb > 12 {
        return Err(Error::Corrupt);
    }
    if transform > 1 {
        // a wavelet of Part 2
        return Err(Error::Unsupported);
    }
    let mut pp = Vec::with_capacity(levels as usize + 1);
    for _ in 0..=levels {
        if precincts {
            let b = r.u8()?;
            pp.push((b & 0xF, b >> 4));
        } else {
            pp.push((15, 15));
        }
    }
    Ok(Style { levels, xcb, ycb, cbstyle, reversible: transform == 1, precincts: pp })
}

fn read_quant(r: &mut Rd, end: usize) -> Result<Quant, Error> {
    let s = r.u8()?;
    let style = s & 0x1F;
    let guard = s >> 5;
    let mut steps = Vec::new();
    match style {
        0 => {
            while r.p < end {
                steps.push((r.u8()? >> 3, 0));
            }
        }
        1 | 2 => {
            while r.p + 1 < end {
                let v = r.u16()?;
                steps.push(((v >> 11) as u8, v & 0x7FF));
            }
        }
        _ => return Err(Error::Unsupported),
    }
    if steps.is_empty() {
        return Err(Error::Corrupt);
    }
    Ok(Quant { style, guard, steps })
}

/// one marker segment of a header (main or tile-part) into `p`; false when the marker does not
/// belong into a header
fn header_segment<'a>(m: u16, r: &mut Rd<'a>, end: usize, p: &mut Params, n: usize, ppm: &mut Vec<(u8, &'a [u8])>, ppt: Option<&mut Vec<(u8, Vec<u8>)>>) -> Result<(), Error> {
    match m {
        0xFF52 => {
            let scod = r.u8()?;
            let prog = r.u8()?;
            let layers = r.u16()?;
            let mct = r.u8()?;
            if layers == 0 || prog > 4 {
                return Err(Error::Corrupt);
            }
            p.global = Some(Global { sop: scod & 2 != 0, eph: scod & 4 != 0, prog, layers, mct });
            p.cod = Some(read_style(r, scod & 1 != 0)?);
        }
        0xFF53 => {
            let c = comp_index(r, n)?;
            let s = r.u8()?;
            let st = read_style(r, s & 1 != 0)?;
            if c < n {
                p.coc[c] = Some(st);
            }
        }
        0xFF5C => p.qcd = Some(read_quant(r, end)?),
        0xFF5D => {
            let c = comp_index(r, n)?;
            let q = read_quant(r, end)?;
            if c < n {
                p.qcc[c] = Some(q);
            }
        }
        0xFF5E => {
            let c = comp_index(r, n)?;
            let srgn = r.u8()?;
            let shift = r.u8()?;
            if srgn == 0 && c < n {
                p.rgn[c] = Some(shift.min(37));
            }
        }
        0xFF5F => {
            let mut v = Vec::new();
            let wide = n >= 257;
            while r.left() > 0 && r.p + if wide { 9 } else { 7 } <= end {
                let rs = r.u8()?;
                let cs = if wide { r.u16()? } else { r.u8()? as u16 };
                let lye = r.u16()?;
                let re = r.u8()?;
                let ce = if wide { r.u16()? } else { r.u8()? as u16 };
                let prog = r.u8()?;
                let ce = if ce == 0 { if wide { 16384 } else { 256 } } else { ce };
                v.push(Poc { rs, cs, lye, re, ce, prog: prog.min(4) });
            }
            p.poc = v;
        }
        0xFF60 => {
            let z = r.u8()?;
            ppm.push((z, &r.d[r.p..end]));
        }
        0xFF61 => {
            let z = r.u8()?;
            if let Some(ppt) = ppt {
                ppt.push((z, r.d[r.p..end].to_vec()));
            }
        }
        _ => {}
    }
    Ok(())
}

/// the codestream's headers and its tiles' data
pub(crate) fn parse(d: &[u8]) -> Result<Codestream<'_>, Error> {
    let mut r = Rd { d, p: 0 };
    if r.u16()? != 0xFF4F || r.u16()? != 0xFF51 {
        return Err(Error::NotJpeg2000);
    }
    let lsiz = r.u16()? as usize;
    let siz_end = r.p + lsiz - 2.min(lsiz);
    let siz = read_siz(&mut r)?;
    r.p = siz_end.max(r.p);
    let n = siz.comps.len();
    let mut main = Params::new(n);
    let mut ppm: Vec<(u8, &[u8])> = Vec::new();
    // the main header
    loop {
        let m = r.u16()?;
        if m == 0xFF90 {
            r.p -= 2;
            break;
        }
        if m == 0xFFD9 {
            break;
        }
        if m >> 8 != 0xFF {
            return Err(Error::Corrupt);
        }
        if (0xFF30..=0xFF3F).contains(&m) {
            continue;
        }
        let len = r.u16()? as usize;
        if len < 2 || r.left() < len - 2 {
            return Err(Error::Truncated);
        }
        let end = r.p + len - 2;
        header_segment(m, &mut r, end, &mut main, n, &mut ppm, None)?;
        r.p = end;
    }
    if main.cod.is_none() || main.qcd.is_none() && main.qcc.iter().any(|q| q.is_none()) {
        return Err(Error::Corrupt);
    }
    let ntiles = (siz.tiles_x() * siz.tiles_y()) as usize;
    let mut tiles: Vec<Option<TileData>> = Vec::new();
    tiles.resize_with(ntiles, || None);
    // PPM: the joined segments are a series of (length, packet headers), one per tile-part
    let use_ppm = !ppm.is_empty();
    ppm.sort_by_key(|x| x.0);
    let ppm_all: Vec<u8> = ppm.iter().flat_map(|x| x.1.iter().copied()).collect();
    let mut ppm_spans = Vec::new();
    let mut q = 0usize;
    while q + 4 <= ppm_all.len() {
        let l = u32::from_be_bytes([ppm_all[q], ppm_all[q + 1], ppm_all[q + 2], ppm_all[q + 3]]) as usize;
        q += 4;
        let e = (q + l).min(ppm_all.len());
        ppm_spans.push((q, e));
        q = e;
    }
    // the tile-parts
    let mut part_no = 0usize;
    loop {
        if r.left() < 2 {
            break;
        }
        let sot = r.p;
        let m = r.u16()?;
        if m != 0xFF90 {
            break;
        }
        let lsot = r.u16()? as usize;
        let isot = r.u16()? as usize;
        let psot = r.u32()? as usize;
        let _tpsot = r.u8()?;
        let _tnsot = r.u8()?;
        if lsot != 10 {
            return Err(Error::Corrupt);
        }
        let end = if psot == 0 {
            if d.len() >= 2 && d[d.len() - 2] == 0xFF && d[d.len() - 1] == 0xD9 {
                d.len() - 2
            } else {
                d.len()
            }
        } else {
            (sot + psot).min(d.len())
        };
        if isot >= ntiles {
            // a tile that is not there: skip its part
            r.p = end.max(r.p);
            part_no += 1;
            if psot == 0 {
                break;
            }
            continue;
        }
        let tile = tiles[isot].get_or_insert_with(|| TileData { params: Params::new(n), ..TileData::default() });
        // the tile-part header
        let mut body = end;
        while r.p + 2 <= end {
            let m = r.u16()?;
            if m == 0xFF93 {
                body = r.p;
                break;
            }
            if m >> 8 != 0xFF {
                return Err(Error::Corrupt);
            }
            let len = r.u16()? as usize;
            if len < 2 || r.p + len - 2 > end {
                return Err(Error::Truncated);
            }
            let e = r.p + len - 2;
            let mut no_ppm = Vec::new();
            header_segment(m, &mut r, e, &mut tile.params, n, &mut no_ppm, Some(&mut tile.ppt))?;
            r.p = e;
        }
        tile.parts.push(&d[body.min(end)..end]);
        if use_ppm {
            let h = tile.headers.get_or_insert_with(Vec::new);
            if let Some(&(a, b)) = ppm_spans.get(part_no) {
                h.extend_from_slice(&ppm_all[a..b]);
            }
        }
        part_no += 1;
        r.p = end;
        if psot == 0 {
            break;
        }
    }
    for t in tiles.iter_mut().flatten() {
        if !use_ppm && !t.ppt.is_empty() {
            t.ppt.sort_by_key(|x| x.0);
            t.headers = Some(t.ppt.iter().flat_map(|x| x.1.iter().copied()).collect());
        }
    }
    Ok(Codestream { siz, main, tiles })
}
