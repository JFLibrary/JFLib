// var -- OpenType font variations: fvar (axes, named instances), avar (segment maps), gvar
// (TrueType point deltas), HVAR (advance deltas), MVAR (font-wide metrics) and the item
// variation store HVAR, MVAR and GDEF share
//
//   VarAxis { tag, min, def, max, hidden, name }, VarInstance { name, coords }   what fvar
//        says, in user units; names from the name table (an axis without one: its tag)
//   Var::parse(tables) -> Option<Var>  a font's fvar, avar, gvar, HVAR, MVAR, GDEF store
//   normalize(user) -> coords          F2Dot14 per axis: clamped to the axis, -1 .. 0 .. 1
//        around the default in 16.16, avar in 16.16, then 2.14 (rounded like HarfBuzz and
//        FreeType: floor(x + 0.5))
//   gvar_deltas(gvar, glyph, coords, orig, ends, out, budget)   the deltas of a glyph's points
//        and of its 4 phantom points, added to out: embedded and shared peak tuples,
//        intermediate regions, shared and private packed point numbers, packed deltas (zero
//        runs, bytes, words, longs), IUP for the outline points a tuple leaves out
//   hvar_delta(hvar, glyph, coords)    the advance delta (HVAR, with or without its mapping)
//   mvar_delta(mvar, tag, coords)      a font-wide metric's delta (MVAR)
//   gdef_delta(gdef, outer, inner, coords)   a GPOS VariationIndex delta (GDEF's store)
//
// Every offset is checked against its table (gvar data against the glyph's own range).
// Broken gvar data of a glyph: false (the glyph comes out empty, like broken glyf data);
// broken HVAR / MVAR / store data: no delta.  Bounded work: a glyph whose tuples x points
// exceed VAR_BUDGET counts as broken; a store row whose regions x axes exceed IVS_WORK has
// no delta.  Not supported: avar version 2 beyond its segment maps, CFF2 outlines (such
// fonts do not load), VVAR (no vertical layout).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rt::fx::floor_i;

use crate::ttf::{sfnt_name, Tb, NAME_CAP};

/// axes and named instances read at most
pub const AXES_MAX: usize = 64;
pub const INSTANCES_MAX: usize = 1024;
/// tuple x point steps of one glyph (composites: of the whole glyph)
pub(crate) const VAR_BUDGET: u32 = 1 << 22;
const IVS_WORK: u32 = 1 << 16;

const EMBEDDED_PEAK: u32 = 0x8000;
const INTERMEDIATE: u32 = 0x4000;
const PRIVATE_POINTS: u32 = 0x2000;
const SHARED_POINTS: u32 = 0x8000;

/// an axis of a variable font (fvar) in user units (min <= def <= max)
#[derive(Clone, Debug, PartialEq)]
pub struct VarAxis {
    /// `wght`, `wdth`, `slnt`, `ital`, `opsz` or one of the font's own
    pub tag: [u8; 4],
    /// the smallest value
    pub min: f32,
    /// the default (the font's own outlines)
    pub def: f32,
    /// the largest value
    pub max: f32,
    /// the font asks user interfaces not to show it (HIDDEN_AXIS)
    pub hidden: bool,
    /// the name table's name (UTF-8; the tag when there is none)
    pub name: String,
}

/// a named instance of a variable font (fvar)
#[derive(Clone, Debug, PartialEq)]
pub struct VarInstance {
    /// its subfamily name ("SemiLight Condensed"; "" when the name table has none)
    pub name: String,
    /// user coordinates, one per axis
    pub coords: Vec<f32>,
}

/// a 4-byte tag as the u32 of the font file
pub const fn tag_u32(t: [u8; 4]) -> u32 {
    u32::from_be_bytes(t)
}

// gvar header values (offsets into the gvar table)
struct Gvar {
    shared: u32,
    nshared: u32,
    nglyphs: u32,
    long: bool,
    data: u32,
}

/// the variation data of a font (TtFont::var)
pub(crate) struct Var {
    pub axes: Vec<VarAxis>,
    pub instances: Vec<VarInstance>,
    /// avar segment maps per axis {from, to} (F2Dot14); none for an axis: identity
    avar: Vec<Vec<(i32, i32)>>,
    gvar: Option<Gvar>,
    /// HVAR item variation store offset (0: none) and advance mapping offset (0: implicit)
    hvar: (u32, u32),
    /// MVAR records {tag, outer, inner} and its store offset
    mvar: Vec<(u32, u16, u16)>,
    mvar_store: u32,
    /// GDEF's item variation store offset (0: none)
    gdef_store: u32,
}

// ------------------------------------------------------------------ readers

fn r16(t: Tb, o: u32) -> Option<u32> {
    let v = t.u16(o);
    if v < 0 { None } else { Some(v as u32) }
}

fn s16(t: Tb, o: u32) -> Option<i32> {
    r16(t, o).map(|v| v as u16 as i16 as i32)
}

fn r32(t: Tb, o: u32) -> Option<u32> {
    let v = t.u32(o);
    if v < 0 { None } else { Some(v as u32) }
}

fn fixed(v: u32) -> f32 {
    v as i32 as f32 * (1.0 / 65536.0)
}

fn name_of(name: Tb, id: u32) -> String {
    String::from_utf8_lossy(&sfnt_name(name, id, NAME_CAP)).into_owned()
}

// a + b * c as u32 (None on overflow)
fn at(a: u32, b: u32, c: u32) -> Option<u32> {
    a.checked_add(b.checked_mul(c)?)
}

impl Var {
    /// fvar (with its names), avar, gvar, HVAR, MVAR and GDEF's store; None when fvar is
    /// missing or unusable
    pub(crate) fn parse(fvar: Tb, avar: Tb, gvar: Tb, hvar: Tb, mvar: Tb, gdef: Tb, name: Tb) -> Option<Var> {
        // ---- fvar: {u16 major 1, minor, axesArrayOffset, reserved, axisCount, axisSize,
        // instanceCount, instanceSize}
        let t = fvar;
        if r16(t, 0)? != 1 {
            return None;
        }
        let ao = r16(t, 4)?;
        let n = r16(t, 8)?;
        let asz = r16(t, 10)?;
        let ni = r16(t, 12)?;
        let isz = r16(t, 14)?;
        if n == 0 || n as usize > AXES_MAX || asz < 20 {
            return None;
        }
        let mut axes = Vec::with_capacity(n as usize);
        for i in 0..n {
            let r = at(ao, i, asz)?;
            let tag = r32(t, r)?;
            let (mn, df, mx) = (fixed(r32(t, r + 4)?), fixed(r32(t, r + 8)?), fixed(r32(t, r + 12)?));
            let flags = r16(t, r + 16)?;
            let mut nm = name_of(name, r16(t, r + 18)?);
            if nm.is_empty() {
                nm = String::from_utf8_lossy(&tag.to_be_bytes()).into_owned();
            }
            // min and max never cross the default (HarfBuzz's get_coordinates)
            axes.push(VarAxis { tag: tag.to_be_bytes(), min: mn.min(df), def: df, max: mx.max(df), hidden: flags & 1 != 0, name: nm });
        }
        // instances {u16 subfamilyNameID, flags, Fixed coords[axisCount], [u16 psNameID]}
        let mut instances = Vec::new();
        if isz >= 4 + 4 * n {
            let io = at(ao, n, asz)?;
            for j in 0..ni.min(INSTANCES_MAX as u32) {
                let Some(r) = at(io, j, isz) else { break };
                let mut coords = Vec::with_capacity(n as usize);
                for k in 0..n {
                    let Some(v) = r32(t, r + 4 + 4 * k) else { break };
                    coords.push(fixed(v));
                }
                if coords.len() != n as usize {
                    break;
                }
                let Some(id) = r16(t, r) else { break };
                instances.push(VarInstance { name: name_of(name, id), coords });
            }
        }
        let mut v = Var {
            axes,
            instances,
            avar: Vec::new(),
            gvar: None,
            hvar: (0, 0),
            mvar: Vec::new(),
            mvar_store: 0,
            gdef_store: 0,
        };
        v.avar = Self::parse_avar(avar, n);
        v.gvar = Self::parse_gvar(gvar, n);
        // ---- HVAR {u16 major 1, minor, Offset32 store, advanceMap, lsbMap, rsbMap}
        if r16(hvar, 0) == Some(1) {
            let store = r32(hvar, 4).unwrap_or(0);
            if store != 0 && store < hvar.len() {
                v.hvar = (store, r32(hvar, 8).unwrap_or(0));
            }
        }
        // ---- MVAR {u16 major 1, minor, reserved, valueRecordSize, count, Offset16 store}
        if r16(mvar, 0) == Some(1) {
            let rsz = r16(mvar, 6).unwrap_or(0);
            let cnt = r16(mvar, 8).unwrap_or(0);
            let store = r16(mvar, 10).unwrap_or(0);
            if rsz >= 8 && store != 0 {
                for i in 0..cnt {
                    let r = 12 + i * rsz;
                    let (Some(tag), Some(o), Some(inn)) = (r32(mvar, r), r16(mvar, r + 4), r16(mvar, r + 6)) else { break };
                    v.mvar.push((tag, o as u16, inn as u16));
                }
                v.mvar_store = store;
            }
        }
        // ---- GDEF 1.3: Offset32 itemVarStore at 14
        if r16(gdef, 0) == Some(1) && r16(gdef, 2).is_some_and(|m| m >= 3) {
            v.gdef_store = r32(gdef, 14).unwrap_or(0);
        }
        Some(v)
    }

    // avar {u16 major, minor, reserved, axisCount, segment maps {u16 n, {F2Dot14 from, to}[n]}}:
    // the maps of the first min(axisCount, n) axes (avar 2: only these)
    fn parse_avar(t: Tb, n: u32) -> Vec<Vec<(i32, i32)>> {
        let mut maps = Vec::new();
        let (Some(major), Some(cnt)) = (r16(t, 0), r16(t, 6)) else { return maps };
        if major != 1 && major != 2 {
            return maps;
        }
        let mut o = 8u32;
        for _ in 0..cnt.min(n) {
            let Some(m) = r16(t, o) else { return Vec::new() };
            let mut seg = Vec::with_capacity(m as usize);
            for k in 0..m {
                let p = o + 2 + k * 4;
                let (Some(f), Some(to)) = (s16(t, p), s16(t, p + 2)) else { return Vec::new() };
                seg.push((f, to));
            }
            maps.push(seg);
            o += 2 + m * 4;
        }
        maps
    }

    // gvar {u16 major 1, minor, axisCount, sharedTupleCount, Offset32 sharedTuples, u16
    // glyphCount, flags, Offset32 data, offsets[glyphCount + 1]}; unusable: None
    fn parse_gvar(t: Tb, n: u32) -> Option<Gvar> {
        if r16(t, 0)? != 1 || r16(t, 4)? != n {
            return None;
        }
        let nshared = r16(t, 6)?;
        let shared = r32(t, 8)?;
        let nglyphs = r16(t, 12)?;
        let long = r16(t, 14)? & 1 != 0;
        let data = r32(t, 16)?;
        let osz = if long { 4u64 } else { 2 };
        if 20 + (nglyphs as u64 + 1) * osz > t.len() as u64 || shared as u64 + nshared as u64 * n as u64 * 2 > t.len() as u64 {
            return None;
        }
        Some(Gvar { shared, nshared, nglyphs, long, data })
    }

    pub(crate) fn has_gvar(&self) -> bool {
        self.gvar.is_some()
    }

    pub(crate) fn has_hvar(&self) -> bool {
        self.hvar.0 != 0
    }

    // ------------------------------------------------------------------ coordinates

    /// user values (one per axis; missing: the default) -> normalized F2Dot14 coordinates
    /// with avar applied
    pub(crate) fn normalize(&self, user: &[f32]) -> Vec<i16> {
        let mut c = Vec::with_capacity(self.axes.len());
        for (i, a) in self.axes.iter().enumerate() {
            let mut v = user.get(i).copied().unwrap_or(a.def);
            if v.is_nan() {
                v = a.def;
            }
            v = v.clamp(a.min, a.max);
            // 16.16 first, avar in 16.16, then 2.14 (HarfBuzz, FreeType)
            let n = if v == a.def {
                0
            } else if v < a.def {
                floor_i((v - a.def) / (a.def - a.min) * 65536.0 + 0.5)
            } else {
                floor_i((v - a.def) / (a.max - a.def) * 65536.0 + 0.5)
            };
            let m = match self.avar.get(i) {
                Some(seg) => avar_map(seg, n),
                None => n,
            };
            c.push(((m.clamp(-65536, 65536) + 2) >> 2) as i16);
        }
        c
    }

    // ------------------------------------------------------------------ gvar

    /// the deltas of glyph's points at coords, added to out (out[..n]: the outline points or
    /// components, out[n..n + 4]: the phantom points).  orig / ends: the outline points and
    /// contour ends for IUP (empty for a composite: its untouched components do not move).
    /// false: broken data or the budget is spent
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gvar_deltas(&self, t: Tb, glyph: u32, coords: &[i16], orig: &[(f32, f32)], ends: &[usize], out: &mut [(f32, f32)], budget: &mut u32) -> bool {
        let Some(gv) = &self.gvar else { return true };
        let npts = out.len();
        if glyph >= gv.nglyphs || coords.iter().all(|&c| c == 0) {
            return true;
        }
        // the glyph's data range
        let (a, b) = if gv.long {
            (r32(t, 20 + glyph * 4), r32(t, 24 + glyph * 4))
        } else {
            (r16(t, 20 + glyph * 2).map(|v| v * 2), r16(t, 22 + glyph * 2).map(|v| v * 2))
        };
        let (Some(a), Some(b)) = (a, b) else { return false };
        if b <= a {
            return true; // no variation data
        }
        let (Some(a), Some(b)) = (gv.data.checked_add(a), gv.data.checked_add(b)) else { return false };
        if b > t.len() {
            return false;
        }
        let g = Tb(&t.0[a as usize..b as usize]);
        self.glyph_deltas(t, gv, g, coords, orig, ends, out, npts, budget).is_some()
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph_deltas(&self, t: Tb, gv: &Gvar, g: Tb, coords: &[i16], orig: &[(f32, f32)], ends: &[usize], out: &mut [(f32, f32)], npts: usize, budget: &mut u32) -> Option<()> {
        let naxes = self.axes.len() as u32;
        let tvc = r16(g, 0)?;
        let count = tvc & 0x0FFF;
        let mut p = r16(g, 2)?; // serialized data
        let end = g.len();
        let mut shared: Vec<u32> = Vec::new();
        if tvc & SHARED_POINTS != 0 {
            packed_points(g, &mut p, end, &mut shared)?;
        }
        let mut h = 4u32; // tuple variation headers
        let mut peak = vec![0i32; naxes as usize];
        let mut inter = vec![(0i32, 0i32); naxes as usize];
        let mut pts: Vec<u32> = Vec::new();
        let mut dx: Vec<i32> = Vec::new();
        let mut dy: Vec<i32> = Vec::new();
        let mut tmp: Vec<(f32, f32)> = Vec::new();
        let mut touched: Vec<bool> = Vec::new();
        for _ in 0..count {
            let size = r16(g, h)?;
            let idx = r16(g, h + 2)?;
            h += 4;
            if idx & EMBEDDED_PEAK != 0 {
                for (k, v) in peak.iter_mut().enumerate() {
                    *v = s16(g, h + k as u32 * 2)?;
                }
                h += naxes * 2;
            } else {
                let si = idx & 0x0FFF;
                if si >= gv.nshared {
                    return None;
                }
                for (k, v) in peak.iter_mut().enumerate() {
                    *v = s16(t, gv.shared + (si * naxes + k as u32) * 2)?;
                }
            }
            let has_inter = idx & INTERMEDIATE != 0;
            if has_inter {
                for (k, v) in inter.iter_mut().enumerate() {
                    *v = (s16(g, h + k as u32 * 2)?, s16(g, h + (naxes + k as u32) * 2)?);
                }
                h += naxes * 4;
            }
            let q0 = p;
            let qend = p.checked_add(size)?;
            if qend > end {
                return None;
            }
            p = qend;
            let scalar = tuple_scalar(coords, &peak, if has_inter { Some(&inter) } else { None });
            if scalar == 0.0 {
                continue;
            }
            // budget: every tuple that applies costs the glyph's points
            let cost = (npts as u32).max(1);
            if *budget < cost {
                return None;
            }
            *budget -= cost;
            let mut q = q0;
            let all = if idx & PRIVATE_POINTS != 0 {
                packed_points(g, &mut q, qend, &mut pts)?;
                pts.is_empty()
            } else {
                pts.clear();
                pts.extend_from_slice(&shared);
                pts.is_empty()
            };
            let num = if all { npts } else { pts.len() };
            packed_deltas(g, &mut q, qend, num, &mut dx)?;
            packed_deltas(g, &mut q, qend, num, &mut dy)?;
            let s = scalar as f32;
            if all {
                for (i, o) in out.iter_mut().enumerate() {
                    o.0 += dx[i] as f32 * s;
                    o.1 += dy[i] as f32 * s;
                }
                continue;
            }
            // explicit points: the tuple's own deltas, IUP for the untouched outline points
            tmp.clear();
            tmp.resize(npts, (0.0, 0.0));
            touched.clear();
            touched.resize(npts, false);
            for (k, &pi) in pts.iter().enumerate() {
                let pi = pi as usize;
                if pi >= npts {
                    continue;
                }
                tmp[pi].0 += dx[k] as f32 * s;
                tmp[pi].1 += dy[k] as f32 * s;
                touched[pi] = true;
            }
            if !ends.is_empty() {
                iup(orig, ends, &touched, &mut tmp);
            }
            for (o, d) in out.iter_mut().zip(&tmp) {
                o.0 += d.0;
                o.1 += d.1;
            }
        }
        Some(())
    }

    // ------------------------------------------------------------------ item variation stores

    /// the advance delta of glyph (HVAR); None without HVAR
    pub(crate) fn hvar_delta(&self, t: Tb, glyph: u32, coords: &[i16]) -> Option<f32> {
        let (store, map) = self.hvar;
        if store == 0 {
            return None;
        }
        let (outer, inner) = if map == 0 { (0, glyph) } else { dsim(t, map, glyph).unwrap_or((0, glyph)) };
        Some(ivs(t, store, outer, inner, coords).unwrap_or(0.0))
    }

    /// the delta of a font-wide metric (MVAR tag); 0 without one
    pub(crate) fn mvar_delta(&self, t: Tb, tag: u32, coords: &[i16]) -> f32 {
        match self.mvar.iter().find(|r| r.0 == tag) {
            Some(&(_, o, i)) => ivs(t, self.mvar_store, o as u32, i as u32, coords).unwrap_or(0.0),
            None => 0.0,
        }
    }

    /// a delta of GDEF's store (a GPOS VariationIndex)
    pub(crate) fn gdef_delta(&self, t: Tb, outer: u32, inner: u32, coords: &[i16]) -> f32 {
        if self.gdef_store == 0 {
            return 0.0;
        }
        ivs(t, self.gdef_store, outer, inner, coords).unwrap_or(0.0)
    }
}

// avar segment map of one axis on a 16.16 value (the map's F2Dot14 values x 4; HarfBuzz's
// SegmentMaps::map, also for broken maps)
fn avar_map(seg: &[(i32, i32)], v: i32) -> i32 {
    let fr = |k: usize| seg[k].0 << 2;
    let to = |k: usize| seg[k].1 << 2;
    match seg.len() {
        0 => return v,
        1 => return v - fr(0) + to(0),
        _ => {}
    }
    if v <= fr(0) {
        return v - fr(0) + to(0);
    }
    let count = seg.len() - 1;
    let mut i = 1;
    while i < count && v > fr(i) {
        i += 1;
    }
    if v >= fr(i) {
        return v - fr(i) + to(i);
    }
    if fr(i - 1) == fr(i) {
        return to(i - 1);
    }
    let denom = (fr(i) - fr(i - 1)) as f32;
    floor_i(to(i - 1) as f32 + ((to(i) - to(i - 1)) as f32 * (v - fr(i - 1)) as f32) / denom + 0.5)
}

// the scalar of a tuple at coords (HarfBuzz's calculate_scalar)
fn tuple_scalar(coords: &[i16], peak: &[i32], inter: Option<&Vec<(i32, i32)>>) -> f64 {
    let mut s = 1.0f64;
    for (i, &pk) in peak.iter().enumerate() {
        if pk == 0 {
            continue;
        }
        let v = coords.get(i).copied().unwrap_or(0) as i32;
        if v == 0 {
            return 0.0;
        }
        if v == pk {
            continue;
        }
        match inter {
            Some(r) => {
                let (st, en) = r[i];
                if st > pk || pk > en || (st < 0 && en > 0) {
                    continue; // an invalid region: the axis is ignored
                }
                if v < st || v > en {
                    return 0.0;
                }
                if v < pk {
                    if pk != st {
                        s *= (v - st) as f64 / (pk - st) as f64;
                    }
                } else if pk != en {
                    s *= (en - v) as f64 / (en - pk) as f64;
                }
            }
            None => {
                if v < pk.min(0) || v > pk.max(0) {
                    return 0.0;
                }
                s *= v as f64 / pk as f64;
            }
        }
    }
    s
}

// packed point numbers at *p (< end), moved past them: out empty = every point
fn packed_points(t: Tb, p: &mut u32, end: u32, out: &mut Vec<u32>) -> Option<()> {
    out.clear();
    let rd = |p: &mut u32| -> Option<u32> {
        if *p >= end {
            return None;
        }
        let v = t.u8(*p);
        *p += 1;
        if v < 0 { None } else { Some(v as u32) }
    };
    let mut count = rd(p)?;
    if count & 0x80 != 0 {
        count = ((count & 0x7F) << 8) | rd(p)?;
    }
    let mut n = 0u32;
    while (out.len() as u32) < count {
        let ctrl = rd(p)?;
        let run = (ctrl & 0x7F) + 1;
        if out.len() as u32 + run > count {
            return None;
        }
        for _ in 0..run {
            let d = if ctrl & 0x80 != 0 { (rd(p)? << 8) | rd(p)? } else { rd(p)? };
            n = n.wrapping_add(d);
            out.push(n);
        }
    }
    Some(())
}

// n packed deltas at *p (< end) into out
fn packed_deltas(t: Tb, p: &mut u32, end: u32, n: usize, out: &mut Vec<i32>) -> Option<()> {
    out.clear();
    while out.len() < n {
        if *p >= end {
            return None;
        }
        let ctrl = t.u8(*p);
        if ctrl < 0 {
            return None;
        }
        *p += 1;
        let ctrl = ctrl as u32;
        let run = (ctrl & 0x3F) as usize + 1;
        if out.len() + run > n {
            return None;
        }
        let size = match ctrl & 0xC0 {
            0x80 => 0,
            0x40 => 2,
            0xC0 => 4,
            _ => 1,
        };
        if *p as u64 + (run * size) as u64 > end as u64 {
            return None;
        }
        for _ in 0..run {
            let v = match size {
                0 => 0,
                1 => t.u8(*p) as u8 as i8 as i32,
                2 => t.u16(*p) as u16 as i16 as i32,
                _ => t.u32(*p) as u32 as i32,
            };
            *p += size as u32;
            out.push(v);
        }
    }
    Some(())
}

// infer the deltas of the untouched points of each contour from the nearest touched ones
// before and after (IUP): between their coordinates interpolated, outside them the nearer
// one's delta; equal coordinates with different deltas give 0
fn iup(orig: &[(f32, f32)], ends: &[usize], touched: &[bool], d: &mut [(f32, f32)]) {
    let mut start = 0usize;
    for &e in ends {
        if e >= orig.len() || e < start {
            break;
        }
        let range = start..e + 1;
        start = e + 1;
        let first = match range.clone().find(|&i| touched[i]) {
            Some(f) => f,
            None => continue, // nothing touched: no deltas
        };
        // walk the touched points cyclically; fill each gap
        let n = range.end - range.start;
        let mut cur = first;
        loop {
            // the next touched point after cur (cyclic)
            let mut nx = cur;
            let mut gap = 0usize;
            loop {
                nx = if nx + 1 == range.end { range.start } else { nx + 1 };
                if touched[nx] {
                    break;
                }
                gap += 1;
            }
            if gap > 0 {
                let mut i = if cur + 1 == range.end { range.start } else { cur + 1 };
                for _ in 0..gap {
                    let (x, y) = orig[i];
                    let px = iup_one(x, orig[cur].0, d[cur].0, orig[nx].0, d[nx].0);
                    let py = iup_one(y, orig[cur].1, d[cur].1, orig[nx].1, d[nx].1);
                    d[i] = (px, py);
                    i = if i + 1 == range.end { range.start } else { i + 1 };
                }
            }
            if nx == first || n == 1 {
                break;
            }
            cur = nx;
        }
    }
}

// one coordinate: c between the reference coordinates (c1, d1) and (c2, d2)
fn iup_one(c: f32, c1: f32, d1: f32, c2: f32, d2: f32) -> f32 {
    let (c1, d1, c2, d2) = if c1 <= c2 { (c1, d1, c2, d2) } else { (c2, d2, c1, d1) };
    if c1 == c2 {
        return if d1 == d2 { d1 } else { 0.0 };
    }
    if c <= c1 {
        d1
    } else if c >= c2 {
        d2
    } else {
        d1 + (c - c1) * (d2 - d1) / (c2 - c1)
    }
}

// DeltaSetIndexMap entry i -> (outer, inner); the last entry for i beyond the map
fn dsim(t: Tb, m: u32, i: u32) -> Option<(u32, u32)> {
    let fmt = t.u8(m);
    let ef = t.u8(m.checked_add(1)?);
    if ef < 0 {
        return None;
    }
    let ef = ef as u32;
    let (count, data) = match fmt {
        0 => (r16(t, m + 2)?, m + 4),
        1 => (r32(t, m + 2)?, m + 6),
        _ => return None,
    };
    if count == 0 {
        return Some((i >> 16, i & 0xFFFF));
    }
    let i = i.min(count - 1);
    let size = ((ef >> 4) & 3) + 1;
    let bits = (ef & 0xF) + 1;
    let o = at(data, i, size)?;
    let mut v = 0u32;
    for k in 0..size {
        let b = t.u8(o.checked_add(k)?);
        if b < 0 {
            return None;
        }
        v = (v << 8) | b as u32;
    }
    Some((v >> bits, v & ((1u32 << bits) - 1)))
}

// one region axis' factor (HarfBuzz's VarRegionAxis::evaluate)
fn region_factor(v: i32, start: i32, peak: i32, end: i32) -> f32 {
    if peak == 0 || v == peak {
        return 1.0;
    }
    if v == 0 {
        return 0.0;
    }
    if start > peak || peak > end || (start < 0 && end > 0) {
        return 1.0;
    }
    if v <= start || end <= v {
        return 0.0;
    }
    if v < peak { (v - start) as f32 / (peak - start) as f32 } else { (end - v) as f32 / (end - peak) as f32 }
}

// the delta (outer, inner) of the item variation store at s
fn ivs(t: Tb, s: u32, outer: u32, inner: u32, coords: &[i16]) -> Option<f32> {
    if s == 0 || r16(t, s)? != 1 {
        return None;
    }
    let rl = s.checked_add(r32(t, s + 2)?)?;
    if outer >= r16(t, s + 6)? {
        return None;
    }
    let d = s.checked_add(r32(t, at(s + 8, outer, 4)?)?)?;
    let items = r16(t, d)?;
    let wc = r16(t, d + 2)?;
    let rc = r16(t, d + 4)?;
    if inner >= items {
        return None;
    }
    let long = wc & 0x8000 != 0;
    let words = wc & 0x7FFF;
    if words > rc {
        return None;
    }
    let (ws, bs) = if long { (4u32, 2u32) } else { (2, 1) };
    let row = words * ws + (rc - words) * bs;
    let base = at(d.checked_add(6 + rc * 2)?, inner, row)?;
    let axes = r16(t, rl)?;
    let nreg = r16(t, rl + 2)?;
    if rc.saturating_mul(axes.max(1)) > IVS_WORK {
        return None;
    }
    let mut sum = 0.0f32;
    for k in 0..rc {
        let ri = r16(t, d + 6 + k * 2)?;
        if ri >= nreg {
            return None;
        }
        let mut sc = 1.0f32;
        for a in 0..axes {
            let rec = at(rl + 4, ri * axes + a, 6)?;
            let (st, pk, en) = (s16(t, rec)?, s16(t, rec + 2)?, s16(t, rec + 4)?);
            let v = coords.get(a as usize).copied().unwrap_or(0) as i32;
            let f = region_factor(v, st, pk, en);
            if f == 0.0 {
                sc = 0.0;
                break;
            }
            sc *= f;
        }
        if sc == 0.0 {
            continue;
        }
        let dv = if k < words {
            let o = at(base, k, ws)?;
            if long { r32(t, o)? as i32 } else { s16(t, o)? }
        } else {
            let o = at(base.checked_add(words * ws)?, k - words, bs)?;
            if long {
                s16(t, o)?
            } else {
                let b = t.u8(o);
                if b < 0 {
                    return None;
                }
                b as u8 as i8 as i32
            }
        };
        sum += sc * dv as f32;
    }
    Some(sum)
}
