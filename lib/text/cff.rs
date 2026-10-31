// cff -- CFF and CFF2 ('CFF ' / 'CFF2' tables of OpenType fonts): Type 2 charstrings -> cubic
// outlines                                                               [src/text/cff.s]
//
//   Cff::open(table, upem) -> context / ERR_*   CFF: the header, Top DICT, CharStrings, global
//        and local subrs, CID-keyed FDArray / FDSelect (formats 0 and 3), FontMatrix
//   Cff::open2(table, upem) -> context / ERR_*  CFF2: the header and the Top DICT right after
//        it (no Name / String INDEX), INDEXes with 32-bit counts, CharStrings, global subrs,
//        FDArray (each font DICT's Private DICT: Subrs and vsindex; blend in its values),
//        FDSelect (formats 0, 3 and 4; none: font DICT 0), the VariationStore (vstore),
//        maxstack, FontMatrix
//   outline(table, glyph, ol) -> false on error   run the charstring, points into the outline
//        buffer (OLT_ON / OLT_CUBIC, font units)
//   outline_var(table, glyph, coords, ol)   the same at a CFF2 instance's normalized
//        coordinates (F2Dot14 per axis, avar applied: var.rs normalize; empty = the default)
//
// Type 2 operators: all path operators (r/h/v moveto, lineto, rrcurveto, hh/vv/hv/vh curveto,
// rcurveline, rlinecurve, flex, hflex, hflex1, flex1), hint operators (stems are counted for
// hintmask / cntrmask and skipped), callsubr / callgsubr (bias 107 / 1131 / 32768, nesting
// <= 10), return, endchar (a seac accent is ignored), the arithmetic / storage operators
// (abs add sub div neg eq drop put get ifelse random mul sqrt dup exch index roll and or
// not).  Argument stack 48, transient array 32, at most 200000 operators per glyph.  The
// width argument is dropped (advances come from hmtx).  Like the assembly, the transient
// array and the random generator are process-wide.
//
// CFF2 charstrings: no width argument, no endchar (the charstring ends with its data, a subr
// too; return and endchar are still accepted, as are the arithmetic operators, like FreeType),
// vsindex (before the first blend; else the Private DICT's, default 0) and blend: n values,
// n x k deltas, n -> the n values default + sum(delta x scalar), k = the regions of that item
// variation data, their scalars at coords (var.rs ivs_scalars: HarfBuzz's region rules, f32).
// Argument stack maxstack (default 193, here 193 ..= 513), at most 512 regions per item
// variation data, a vsindex after a blend is an error.  Advances come from hmtx + HVAR
// (ttf.rs); hints are skipped (CFF2 outlines are not hinted, like CFF ones).

use alloc::vec;
use alloc::vec::Vec;

use rt::fx::{cvtt, cvttd, sqrt};

use crate::outline::{Outline, OLT_CUBIC, OLT_ON};
use crate::ttf::{Tb, ERR_HDR, ERR_UNSUPPORTED};
use crate::var;

const FD_MAX: u32 = 256;
const CS_STACK: usize = 48;
const CS2_STACK: usize = 513; // CFF2: the largest maxstack
const CS2_DEFSTACK: i64 = 193; // CFF2: maxstack without the operator
const CS2_REGIONS: usize = 512; // regions of one item variation data a blend may use
const DICT2_STACK: usize = 513; // CFF2 DICT operands
const CS_DEPTH: u32 = 10;
const CS_BUDGET: i32 = 200_000;
const OPS_MAX: usize = 48; // DICT operands kept
const F_INV65536: f32 = 0.000_015_258_789_062_5;
const F_INV8M: f32 = 0.000_000_119_209_29; // 1 / 2^23
const F_EPS: f32 = 0.000_001;

/// a CFF INDEX: count, offSize, offsets position, data position (table offsets)
#[derive(Clone, Copy, Default)]
struct Index {
    count: u32,
    offsize: u32,
    offs: u32,
    data: u32,
}

pub struct Cff {
    /// a CFF2 table (open2)
    v2: bool,
    cid: bool, // font DICTs: CID-keyed CFF, every CFF2
    cs: Index,
    gs: Index,
    ls: Index,
    fdsel: u32, // FDSelect offset (0: none, CFF2 only: font DICT 0)
    nfd: u32,
    mat: [f32; 6],
    hasmat: bool,
    fdls: Vec<Index>,
    /// CFF2: the vsindex of each font DICT's Private DICT
    fdvs: Vec<u16>,
    /// CFF2: the item variation store (table offset, past vstore's length; 0 = none)
    vstore: u32,
    /// CFF2: the argument stack's size
    maxstack: u32,
}

// ------------------------------------------------------------------ byte readers

fn c8(t: Tb, off: u32) -> i32 {
    t.u8(off)
}

fn c16(t: Tb, off: u32) -> i32 {
    t.u16(off)
}

// c_off(off, size 1..4) -> big-endian unsigned value, -1 when out of bounds
fn c_off(t: Tb, off: u32, size: u32) -> i64 {
    if off as u64 + size as u64 > t.0.len() as u64 {
        return -1;
    }
    let mut v = 0u32;
    for i in 0..size {
        v = (v << 8) | t.0[(off + i) as usize] as u32;
    }
    v as i64
}

// ------------------------------------------------------------------ INDEX

// ix_parse(pos, index) -> position after the INDEX, or -1 (malformed; count = 0)
fn ix_parse(t: Tb, pos: u32, ix: &mut Index) -> i64 {
    let count = c16(t, pos);
    if count < 0 {
        ix.count = 0;
        return -1;
    }
    ix.count = count as u32;
    if count == 0 {
        ix.offsize = 1;
        ix.offs = pos.wrapping_add(2);
        ix.data = ix.offs;
        return ix.offs as i64;
    }
    let os = c8(t, pos.wrapping_add(2));
    if !(1..=4).contains(&os) {
        ix.count = 0;
        return -1;
    }
    let os = os as u32;
    ix.offsize = os;
    ix.offs = pos.wrapping_add(3);
    ix.data = ix.offs.wrapping_add((count as u32 + 1).wrapping_mul(os));
    // last offset: the end of the data
    let last = c_off(t, (count as u32).wrapping_mul(os).wrapping_add(ix.offs), os);
    if last <= 0 {
        ix.count = 0;
        return -1;
    }
    let end = ix.data as u64 + last as u64 - 1;
    if end > t.0.len() as u64 {
        ix.count = 0;
        return -1;
    }
    end as u32 as i64
}

// ix_get(index, i) -> (start, length) or None
fn ix_get(t: Tb, ix: &Index, i: u32) -> Option<(u32, u32)> {
    if i >= ix.count {
        return None;
    }
    let a = c_off(t, i.wrapping_mul(ix.offsize).wrapping_add(ix.offs), ix.offsize);
    if a <= 0 {
        return None;
    }
    let b = c_off(t, (i + 1).wrapping_mul(ix.offsize).wrapping_add(ix.offs), ix.offsize);
    if b <= 0 || b < a {
        return None;
    }
    let len = (b - a) as u64;
    let start = ix.data as u64 + a as u64 - 1;
    if start + len > t.0.len() as u64 {
        return None;
    }
    if start as u32 as i32 >= 0 { Some((start as u32, len as u32)) } else { None }
}

// ix2_parse(pos, index) -> position after a CFF2 INDEX {u32 count, offSize, offsets[count + 1],
// data} (an empty INDEX is its count alone), or -1 (malformed; count = 0)
fn ix2_parse(t: Tb, pos: u32, ix: &mut Index) -> i64 {
    ix.count = 0;
    let count = c_off(t, pos, 4);
    if count < 0 {
        return -1;
    }
    let base = pos as u64 + 4; // <= the table's length
    if count == 0 {
        ix.offsize = 1;
        ix.offs = base as u32;
        ix.data = base as u32;
        return base as i64;
    }
    let os = c8(t, base as u32);
    if !(1..=4).contains(&os) {
        return -1;
    }
    let os = os as u64;
    let offs = base + 1;
    let data = offs + (count as u64 + 1) * os;
    if data > t.len() as u64 {
        return -1;
    }
    let last = c_off(t, (offs + count as u64 * os) as u32, os as u32);
    if last <= 0 {
        return -1;
    }
    let end = data + last as u64 - 1;
    if end > t.len() as u64 {
        return -1;
    }
    ix.count = count as u32;
    ix.offsize = os as u32;
    ix.offs = offs as u32;
    ix.data = data as u32;
    end as i64
}

// ------------------------------------------------------------------ CFF2 DICT

// walk a CFF2 DICT: f(operator, operands) for every operator (0..=27 but 12 / 23, or 1200 +
// the escaped byte); blend (23) leaves the n default values in place of its operands (the
// values that vary are hint values, which are not used) with k = regions(vsindex) deltas per
// value, vsindex (22) as the DICT's own sets it -> false: malformed (the walk stopped)
fn dict2_walk(t: Tb, start: u32, end: u32, regions: &dyn Fn(u32) -> Option<u32>, f: &mut dyn FnMut(u32, &[f64])) -> bool {
    let mut st: Vec<f64> = Vec::new();
    let mut vsi = 0u32;
    let mut p = start;
    while p < end {
        let b = c8(t, p);
        if b < 0 {
            return false;
        }
        let v: f64;
        if b <= 27 {
            p += 1;
            let mut o = b as u32;
            if o == 12 {
                let b2 = c8(t, p);
                if b2 < 0 {
                    return false;
                }
                p += 1;
                o = 1200 + b2 as u32;
            }
            if o == 23 {
                // blend: n defaults, n x k deltas, n -> the n defaults
                let Some(nv) = st.pop() else { return false };
                let n = cvttd(nv);
                let Some(k) = regions(vsi) else { return false };
                if n < 0 {
                    return false;
                }
                let need = n as u64 * (k as u64 + 1);
                if need > st.len() as u64 {
                    return false;
                }
                let keep = st.len() - need as usize + n as usize;
                st.truncate(keep);
                continue;
            }
            if o == 22 {
                match st.last().map(|&x| cvttd(x)) {
                    Some(i) if i >= 0 => vsi = i as u32,
                    _ => {}
                }
            }
            f(o, &st);
            st.clear();
            continue;
        } else if b == 28 {
            let w = c16(t, p.wrapping_add(1));
            if w < 0 {
                return false;
            }
            v = (w as u16 as i16) as f64;
            p += 3;
        } else if b == 29 {
            let w = c_off(t, p.wrapping_add(1), 4);
            if w < 0 {
                return false;
            }
            v = (w as u32 as i32) as f64;
            p += 5;
        } else if b == 30 {
            p += 1;
            v = dict_real(t, &mut p);
        } else if b == 31 || b == 255 {
            p += 1; // reserved
            st.clear();
            continue;
        } else if b <= 246 {
            v = (b - 139) as f64;
            p += 1;
        } else {
            let b2 = c8(t, p.wrapping_add(1));
            if b2 < 0 {
                return false;
            }
            p += 2;
            v = if b <= 250 { ((b - 247) * 256 + b2 + 108) as f64 } else { (-((b - 251) * 256 + b2 + 108)) as f64 };
        }
        if st.len() >= DICT2_STACK {
            return false;
        }
        st.push(v);
    }
    true
}

// dict2_find(start, end, op) -> the operand count of op's first occurrence (its last OPS_MAX
// operands in ops), -1 = absent
fn dict2_find(t: Tb, start: u32, end: u32, op: u32, regions: &dyn Fn(u32) -> Option<u32>, ops: &mut [f64; OPS_MAX]) -> i32 {
    let mut found = -1i32;
    dict2_walk(t, start, end, regions, &mut |o, a| {
        if found < 0 && o == op {
            let a = &a[a.len().saturating_sub(OPS_MAX)..];
            ops[..a.len()].copy_from_slice(a);
            found = a.len() as i32;
        }
    });
    found
}

// CFF2: the local subrs (into ix) and the vsindex of the Private DICT that the font DICT
// [start, end) names; blend in its values with the regions of the store at vstore
fn cf2_private(t: Tb, start: u32, end: u32, vstore: u32, ix: &mut Index, ops: &mut [f64; OPS_MAX]) -> u16 {
    ix.count = 0;
    let n = dict2_find(t, start, end, 18, &|_| None, ops);
    if n < 2 {
        return 0;
    }
    let size = cvttd(ops[n as usize - 2]);
    let off = cvttd(ops[n as usize - 1]);
    if size <= 0 || off <= 0 || off as u32 >= t.len() {
        return 0;
    }
    let off = off as u32;
    let e = off.saturating_add(size as u32).min(t.len());
    let regions = |vsi: u32| if vstore == 0 { None } else { var::ivs_region_count(t, vstore, vsi) };
    let (mut subrs, mut vsi) = (None, 0u16);
    dict2_walk(t, off, e, &regions, &mut |o, a| {
        let last = a.last().map(|&v| cvttd(v));
        match (o, last) {
            (19, Some(s)) if subrs.is_none() => subrs = Some(s),
            (22, Some(v)) if (0..=0xFFFF).contains(&v) => vsi = v as u16,
            _ => {}
        }
    });
    if let Some(s) = subrs
        && s > 0
        && let Some(pos) = (s as u32).checked_add(off)
    {
        ix2_parse(t, pos, ix); // Subrs: relative to the Private DICT
    }
    vsi
}

// ------------------------------------------------------------------ DICT

// dict_find(start, end, op) -> operand count (operands in ops as f64), -1 = absent
// op: 0..21, or 1200 + second byte for escaped operators
fn dict_find(t: Tb, start: u32, end: u32, op: u32, ops: &mut [f64; OPS_MAX]) -> i32 {
    let mut p = start;
    let mut n = 0usize;
    while p < end {
        let b = c8(t, p);
        if b < 0 {
            return -1;
        }
        let v: f64;
        if b <= 21 {
            p += 1;
            let mut o = b as u32;
            if o == 12 {
                let b2 = c8(t, p);
                if b2 < 0 {
                    return -1;
                }
                p += 1;
                o = 1200 + b2 as u32;
            }
            if o == op {
                return n as i32;
            }
            n = 0;
            continue;
        } else if b == 28 {
            let w = c16(t, p.wrapping_add(1));
            if w < 0 {
                return -1;
            }
            v = (w as u16 as i16) as f64;
            p = p.wrapping_add(3);
        } else if b == 29 {
            let w = c_off(t, p.wrapping_add(1), 4);
            if w < 0 {
                return -1;
            }
            v = (w as u32 as i32) as f64;
            p = p.wrapping_add(5);
        } else if b == 30 {
            p += 1;
            v = dict_real(t, &mut p);
        } else if b < 32 {
            p += 1;
            n = 0;
            continue;
        } else if b <= 246 {
            v = (b - 139) as f64;
            p += 1;
        } else if b <= 254 {
            let b2 = c8(t, p.wrapping_add(1));
            if b2 < 0 {
                return -1;
            }
            p = p.wrapping_add(2);
            v = if b <= 250 {
                ((b - 247) * 256 + b2 + 108) as f64
            } else {
                (-((b - 251) * 256 + b2 + 108)) as f64
            };
        } else {
            p += 1;
            n = 0;
            continue;
        }
        if n < OPS_MAX {
            ops[n] = v;
            n += 1;
        }
    }
    -1
}

// a nibble-coded real at p (advanced past it)  [dict_real]
fn dict_real(t: Tb, p: &mut u32) -> f64 {
    let mut m = 0u64; // mantissa
    let mut digits = 0i32; // digits after the point
    let mut fl = 0u32; // 1 negative, 2 point seen, 4 exponent, 8 exponent negative
    let mut exp = 0i32;
    'bytes: while (*p as usize) < t.0.len() {
        let b = t.0[*p as usize] as u32;
        *p += 1;
        for nib in [b >> 4, b & 15] {
            if nib <= 9 {
                if fl & 4 != 0 {
                    if (exp as u32) < 1000 {
                        exp = exp * 10 + nib as i32;
                    }
                } else if m >= 100_000_000_000_000_000 {
                    if fl & 2 == 0 {
                        digits -= 1; // integer digit dropped: scale up
                    }
                } else {
                    m = m * 10 + nib as u64;
                    if fl & 2 != 0 {
                        digits += 1;
                    }
                }
                continue;
            }
            match nib {
                0xA => fl |= 2,
                0xB => fl |= 4,
                0xC => fl |= 4 | 8,
                0xE => fl |= 1,
                0xF => break 'bytes,
                _ => {}
            }
        }
    }
    // value = mantissa * 10^(exp - digits)
    let mut v = m as i64 as f64;
    if fl & 8 != 0 {
        exp = exp.wrapping_neg();
    }
    let e = exp.wrapping_sub(digits);
    if e > 0 {
        if e as u32 > 400 {
            return 0.0;
        }
        for _ in 0..e {
            v = v * 10.0;
        }
    } else if e < 0 {
        let mut c = e.wrapping_neg() as u32;
        loop {
            v = v / 10.0;
            c -= 1;
            if c == 0 {
                break;
            }
            if c > 400 {
                return 0.0;
            }
        }
    }
    if fl & 1 != 0 {
        v = 0.0 - v;
    }
    v
}

// the local subrs of the Private DICT named by the dict's Private operator  [cf_private]
fn cf_private(t: Tb, start: u32, end: u32, ix: &mut Index, ops: &mut [f64; OPS_MAX]) {
    ix.count = 0;
    let n = dict_find(t, start, end, 18, ops);
    if n < 2 {
        return;
    }
    let size = rt::fx::cvttd(ops[n as usize - 2]);
    let off = rt::fx::cvttd(ops[n as usize - 1]);
    if size <= 0 || off <= 0 || off as u32 >= t.len() {
        return;
    }
    let off = off as u32;
    let e = off.wrapping_add(size as u32).min(t.len());
    let n = dict_find(t, off, e, 19, ops); // Subrs (relative to the Private DICT)
    if n < 1 {
        return;
    }
    let s = rt::fx::cvttd(ops[n as usize - 1]);
    if s <= 0 {
        return;
    }
    let (pos, carry) = (s as u32).overflowing_add(off);
    if carry {
        return;
    }
    ix_parse(t, pos, ix);
}

impl Cff {
    fn empty(v2: bool) -> Cff {
        Cff {
            v2,
            cid: false,
            cs: Index::default(),
            gs: Index::default(),
            ls: Index::default(),
            fdsel: 0,
            nfd: 0,
            mat: [0.0; 6],
            hasmat: false,
            fdls: Vec::new(),
            fdvs: Vec::new(),
            vstore: 0,
            maxstack: CS2_DEFSTACK as u32,
        }
    }

    /// a CFF2 table (its outline() takes normalized coordinates)
    pub fn is_v2(&self) -> bool {
        self.v2
    }

    /// a CFF2 table with a variation store: its outlines vary
    pub fn varies(&self) -> bool {
        self.v2 && self.vstore != 0
    }

    // FontMatrix operands (a b c d e f) for upem: kept unless the identity (within 1e-6)
    fn set_matrix(&mut self, m: &[f64], upem: u32) {
        let u = upem as i32 as f64;
        for (i, &v) in m.iter().take(6).enumerate() {
            self.mat[i] = (v * u) as f32;
        }
        let abs = |v: f32| f32::from_bits(v.to_bits() & 0x7FFF_FFFF);
        if abs(self.mat[0] - 1.0) > F_EPS || abs(self.mat[3] - 1.0) > F_EPS {
            self.hasmat = true;
        } else {
            for i in [1, 2, 4, 5] {
                if abs(self.mat[i]) > F_EPS {
                    self.hasmat = true;
                    break;
                }
            }
        }
    }

    /// cff_open: ERR_HDR (malformed) / ERR_UNSUPPORTED (not CFF 1 / not Type 2 charstrings)
    pub fn open(t: Tb, upem: u32) -> Result<Cff, i32> {
        let mut ops = [0f64; OPS_MAX];
        let mut c = Cff::empty(false);
        if c8(t, 0) != 1 {
            return Err(ERR_UNSUPPORTED);
        }
        let hs = c8(t, 2);
        if hs < 0 {
            return Err(ERR_HDR);
        }
        // Name INDEX, Top DICT INDEX, String INDEX, Global Subr INDEX
        let mut top = Index::default();
        let mut strs = Index::default();
        let p = ix_parse(t, hs as u32, &mut top); // name (parsed into the top slot, reused)
        if p < 0 {
            return Err(ERR_HDR);
        }
        let p = ix_parse(t, p as u32, &mut top);
        if p < 0 {
            return Err(ERR_HDR);
        }
        let p = ix_parse(t, p as u32, &mut strs);
        if p < 0 {
            return Err(ERR_HDR);
        }
        if ix_parse(t, p as u32, &mut c.gs) < 0 {
            return Err(ERR_HDR);
        }
        let Some((ds, dl)) = ix_get(t, &top, 0) else { return Err(ERR_HDR) };
        let de = ds.wrapping_add(dl);
        // CharstringType must be 2
        let n = dict_find(t, ds, de, 1206, &mut ops);
        if n >= 1 && rt::fx::cvttd(ops[0]) != 2 {
            return Err(ERR_UNSUPPORTED);
        }
        // CharStrings
        let n = dict_find(t, ds, de, 17, &mut ops);
        if n < 1 {
            return Err(ERR_HDR);
        }
        let o = rt::fx::cvttd(ops[n as usize - 1]);
        if o <= 0 || ix_parse(t, o as u32, &mut c.cs) < 0 || c.cs.count == 0 {
            return Err(ERR_HDR);
        }
        // FontMatrix (default 0.001 0 0 0.001 0 0 for upem 1000)
        let n = dict_find(t, ds, de, 1207, &mut ops);
        if n >= 6 {
            let m = ops;
            c.set_matrix(&m[..6], upem);
        }
        // CID-keyed?
        if dict_find(t, ds, de, 1230, &mut ops) >= 0 {
            c.cid = true;
            let n = dict_find(t, ds, de, 1236, &mut ops); // FDArray
            if n < 1 {
                return Err(ERR_HDR);
            }
            let o = rt::fx::cvttd(ops[n as usize - 1]);
            if o <= 0 {
                return Err(ERR_HDR);
            }
            let mut fda = Index::default();
            if ix_parse(t, o as u32, &mut fda) < 0 {
                return Err(ERR_HDR);
            }
            c.nfd = fda.count.min(FD_MAX);
            let n = dict_find(t, ds, de, 1237, &mut ops); // FDSelect
            if n < 1 {
                return Err(ERR_HDR);
            }
            let o = rt::fx::cvttd(ops[n as usize - 1]);
            if o <= 0 || o as u32 >= t.len() {
                return Err(ERR_HDR);
            }
            c.fdsel = o as u32;
            // every font DICT: its Private DICT and local subrs
            c.fdls = vec![Index::default(); c.nfd as usize];
            for fd in 0..c.nfd {
                if let Some((s, l)) = ix_get(t, &fda, fd) {
                    cf_private(t, s, s.wrapping_add(l), &mut c.fdls[fd as usize], &mut ops);
                }
            }
        } else {
            cf_private(t, ds, de, &mut c.ls, &mut ops);
        }
        Ok(c)
    }

    /// cff2_open: ERR_HDR (malformed) / ERR_UNSUPPORTED (not CFF2)
    pub fn open2(t: Tb, upem: u32) -> Result<Cff, i32> {
        let mut ops = [0f64; OPS_MAX];
        let mut c = Cff::empty(true);
        if c8(t, 0) != 2 {
            return Err(ERR_UNSUPPORTED);
        }
        // header {u8 major 2, minor, headerSize, u16 topDictLength}, the Top DICT after it
        let hs = c8(t, 2);
        let tdl = c16(t, 3);
        if hs < 5 || tdl < 0 {
            return Err(ERR_HDR);
        }
        let ds = hs as u32;
        let de = ds + tdl as u32;
        if de > t.len() {
            return Err(ERR_HDR);
        }
        // CharStrings, FDArray, FDSelect, vstore, maxstack, FontMatrix (the first of each)
        let (mut cso, mut fda, mut fds, mut vst, mut maxst) = (None, None, None, None, None);
        let mut mat: Option<[f64; 6]> = None;
        dict2_walk(t, ds, de, &|_| None, &mut |op, a| {
            let last = a.last().map(|&v| cvttd(v) as i64);
            let slot = match op {
                17 => &mut cso,
                1236 => &mut fda,
                1237 => &mut fds,
                24 => &mut vst,
                25 => &mut maxst,
                1207 => {
                    if mat.is_none() && a.len() >= 6 {
                        mat = Some([a[0], a[1], a[2], a[3], a[4], a[5]]);
                    }
                    return;
                }
                _ => return,
            };
            if slot.is_none() {
                *slot = last;
            }
        });
        let len = t.len() as i64;
        let pos = |o: Option<i64>| o.filter(|&o| o > 0 && o < len).map(|o| o as u32);
        // global subrs right after the Top DICT; CharStrings
        if ix2_parse(t, de, &mut c.gs) < 0 {
            return Err(ERR_HDR);
        }
        match pos(cso) {
            Some(o) if ix2_parse(t, o, &mut c.cs) >= 0 && c.cs.count != 0 => {}
            _ => return Err(ERR_HDR),
        }
        if let Some(m) = mat {
            c.set_matrix(&m, upem);
        }
        c.maxstack = maxst.unwrap_or(CS2_DEFSTACK).clamp(CS2_DEFSTACK, CS2_STACK as i64) as u32;
        // VariationStore {u16 length, item variation store}
        if let Some(o) = pos(vst)
            && o as u64 + 2 < t.len() as u64
        {
            c.vstore = o + 2;
        }
        // FDArray (every CFF2 font has one), FDSelect (none: every glyph uses font DICT 0)
        let mut fdix = Index::default();
        match pos(fda) {
            Some(o) if ix2_parse(t, o, &mut fdix) >= 0 && fdix.count != 0 => {}
            _ => return Err(ERR_HDR),
        }
        c.cid = true;
        c.nfd = fdix.count.min(FD_MAX);
        c.fdsel = pos(fds).unwrap_or(0);
        c.fdls = vec![Index::default(); c.nfd as usize];
        c.fdvs = vec![0; c.nfd as usize];
        for fd in 0..c.nfd as usize {
            if let Some((s, l)) = ix_get(t, &fdix, fd as u32) {
                c.fdvs[fd] = cf2_private(t, s, s.wrapping_add(l), c.vstore, &mut c.fdls[fd], &mut ops);
            }
        }
        Ok(c)
    }

    // font DICT index of glyph (-1 on error)  [cf_fdselect]
    fn fdselect(&self, t: Tb, glyph: u32) -> i32 {
        let fs = self.fdsel;
        if fs == 0 {
            return 0; // CFF2 without FDSelect (a CID-keyed CFF always has one)
        }
        match c8(t, fs) {
            0 => c8(t, fs.wrapping_add(glyph).wrapping_add(1)),
            3 => {
                let n = c16(t, fs.wrapping_add(1));
                if n <= 0 {
                    return -1;
                }
                // ranges {u16 first, u8 fd}, sentinel u16
                // binary search: the last range whose first glyph is <= glyph
                let (mut lo, mut hi) = (0u32, n as u32);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let first = c16(t, mid.wrapping_mul(3).wrapping_add(fs).wrapping_add(3));
                    if first < 0 {
                        return -1;
                    }
                    if first as u32 > glyph {
                        hi = mid;
                    } else {
                        lo = mid + 1;
                    }
                }
                if lo == 0 {
                    return -1;
                }
                c8(t, (lo - 1).wrapping_mul(3).wrapping_add(fs).wrapping_add(5))
            }
            4 if self.v2 => {
                // CFF2: {u32 nRanges, {u32 first, u16 fd}[nRanges], u32 sentinel}
                let n = c_off(t, fs.wrapping_add(1), 4);
                if n <= 0 {
                    return -1;
                }
                let first = |k: u64| -> i64 {
                    let at = fs as u64 + 5 + k * 6;
                    if at + 6 > t.len() as u64 { -1 } else { c_off(t, at as u32, 4) }
                };
                let (mut lo, mut hi) = (0u64, n as u64);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let f = first(mid);
                    if f < 0 {
                        return -1;
                    }
                    if f as u64 > glyph as u64 {
                        hi = mid;
                    } else {
                        lo = mid + 1;
                    }
                }
                if lo == 0 {
                    return -1;
                }
                c16(t, (fs as u64 + 5 + (lo - 1) * 6 + 4) as u32)
            }
            _ => -1,
        }
    }

    /// run the charstring of glyph into the outline buffer: false = malformed  [cff_outline]
    pub fn outline(&self, t: Tb, glyph: u32, ol: &mut Outline) -> bool {
        self.outline_var(t, glyph, &[], ol)
    }

    /// outline at a CFF2 instance's normalized coordinates (empty: the default instance; CFF
    /// ignores them)
    pub fn outline_var(&self, t: Tb, glyph: u32, coords: &[i16], ol: &mut Outline) -> bool {
        if glyph >= self.cs.count {
            return true;
        }
        let (ls, vsi) = if self.cid {
            let fd = self.fdselect(t, glyph);
            if fd as u32 >= self.nfd {
                return false;
            }
            (self.fdls[fd as usize], self.fdvs.get(fd as usize).copied().unwrap_or(0))
        } else {
            (self.ls, 0)
        };
        let Some((s, l)) = ix_get(t, &self.cs, glyph) else { return false };
        let mut cs = Cs {
            t,
            gs: self.gs,
            ls,
            st: [0.0; CS2_STACK],
            sp: 0,
            maxst: if self.v2 { self.maxstack as usize } else { CS_STACK },
            x: 0.0,
            y: 0.0,
            px: 0.0,
            py: 0.0,
            d: [0.0; 6],
            nstems: 0,
            wd: self.v2, // CFF2 charstrings carry no width
            open: false,
            budget: CS_BUDGET,
            trans: [0.0; 32],
            rand: 0,
            v2: self.v2,
            vstore: self.vstore,
            vsi: vsi as u32,
            coords,
            sc: Vec::new(),
            nreg: None,
            blended: false,
            ol,
        };
        if cs.run(s, s.wrapping_add(l), 0) < 0 {
            return false;
        }
        let ol = cs.ol;
        ol.close();
        if self.hasmat {
            // FontMatrix: (x, y) -> (a x + c y + e, b x + d y + f)
            let m = self.mat;
            ol.transform(0, m[0], m[1], m[2], m[3]);
            ol.translate(0, m[4], m[5]);
        }
        true
    }
}

// subr bias for an INDEX with `count` entries
fn cs_bias(count: u32) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

fn fabs(v: f32) -> f32 {
    f32::from_bits(v.to_bits() & 0x7FFF_FFFF)
}

// ucomiss + je: equal or unordered
fn zf(a: f32, b: f32) -> bool {
    !(a < b || a > b)
}

// the Type 2 interpreter state (g_cs_*); CFF2: the store, the item variation data (vsindex),
// the coordinates, the scalars of its regions (computed at the first blend), blend seen
struct Cs<'a, 'o> {
    t: Tb<'a>,
    gs: Index,
    ls: Index,
    st: [f32; CS2_STACK],
    sp: usize,
    maxst: usize, // the argument stack's size (CFF: 48)
    x: f32,
    y: f32,
    px: f32,
    py: f32,
    d: [f32; 6],
    nstems: u32,
    wd: bool,
    open: bool,
    budget: i32,
    trans: [f32; 32], // transient array and random state live per glyph
    rand: u32,
    v2: bool,
    vstore: u32,
    vsi: u32,
    coords: &'a [i16],
    sc: Vec<f32>,
    nreg: Option<usize>,
    blended: bool,
    ol: &'o mut Outline,
}

enum Flow {
    Clear,
    Loop,
    Err,
    Ret1,
}

impl Cs<'_, '_> {
    // stack entry i (0.0 when outside the stack)
    fn arg(&self, i: usize) -> f32 {
        if i < self.sp { self.st[i] } else { 0.0 }
    }

    // append (x, y) with tag; opens a contour when needed  [cs_point]
    fn point(&mut self, tag: u8) {
        if !self.open {
            // a drawing operator without moveto: start at the current point
            self.open = true;
            self.ol.point(self.px, self.py, OLT_ON);
        }
        self.ol.point(self.x, self.y, tag);
    }

    fn line(&mut self, dx: f32, dy: f32) {
        self.px = self.x;
        self.py = self.y;
        self.x = dx + self.x;
        self.y = dy + self.y;
        self.point(OLT_ON);
    }

    // curve by the 6 deltas in d (dxa dya dxb dyb dxc dyc)
    fn curve(&mut self) {
        self.px = self.x;
        self.py = self.y;
        for i in 0..3 {
            self.x = self.d[i * 2] + self.x;
            self.y = self.d[i * 2 + 1] + self.y;
            self.point(if i == 2 { OLT_ON } else { OLT_CUBIC });
        }
    }

    fn d6(&mut self, i: usize) {
        for k in 0..6 {
            self.d[k] = self.arg(i + k);
        }
    }

    fn curve6(&mut self, i: usize) {
        self.d6(i);
        self.curve();
    }

    // 1 when the first stack-clearing operator carries an extra (width) argument
    fn width(&mut self, expected: usize) -> usize {
        if self.wd {
            return 0;
        }
        self.wd = true;
        (self.sp > expected) as usize
    }

    // 1 when the argument count is odd (stems: pairs + width)
    fn width_even(&mut self) -> u32 {
        if self.wd {
            return 0;
        }
        self.wd = true;
        (self.sp & 1) as u32
    }

    fn moveto_begin(&mut self) {
        if self.open {
            self.ol.close();
            self.open = false;
        }
    }

    fn moveto_end(&mut self) {
        self.open = true;
        self.ol.point(self.x, self.y, OLT_ON);
    }

    fn push(&mut self, v: f32) -> Flow {
        if self.sp >= self.maxst {
            return Flow::Err;
        }
        self.st[self.sp] = v;
        self.sp += 1;
        Flow::Loop
    }

    fn top(&self) -> Option<f32> {
        if self.sp == 0 { None } else { Some(self.st[self.sp - 1]) }
    }

    fn settop(&mut self, v: f32) -> Flow {
        self.st[self.sp - 1] = v;
        Flow::Loop
    }

    // (second, top), both removed
    fn pop2(&mut self) -> Option<(f32, f32)> {
        if self.sp < 2 {
            return None;
        }
        self.sp -= 2;
        Some((self.st[self.sp], self.st[self.sp + 1]))
    }

    // rotate the top num stack entries by j (positive: towards the top)  [cs_roll]
    fn roll(&mut self, num: i32, j: i32) {
        let mut r = j % num;
        if r < 0 {
            r += num;
        }
        let first = self.sp - num as usize;
        for _ in 0..r {
            let top = self.st[self.sp - 1];
            let mut k = self.sp - 1;
            while k > first {
                self.st[k] = self.st[k - 1];
                k -= 1;
            }
            self.st[first] = top;
        }
    }

    // CFF2 blend: n defaults, n x k deltas, n -> the n values at the instance (k: the regions of
    // item variation data vsi, their scalars at coords, computed once per glyph and vsindex);
    // false: malformed (no store, too few operands)
    fn blend(&mut self) -> bool {
        let Some(nv) = self.top() else { return false };
        self.sp -= 1;
        let n = cvtt(nv);
        if n < 0 {
            return false;
        }
        let k = match self.nreg {
            Some(k) => k,
            None => {
                if var::ivs_scalars(self.t, self.vstore, self.vsi, self.coords, CS2_REGIONS, &mut self.sc).is_none() {
                    return false;
                }
                self.nreg = Some(self.sc.len());
                self.sc.len()
            }
        };
        let n = n as usize;
        let Some(need) = n.checked_mul(k + 1) else { return false };
        if need > self.sp {
            return false;
        }
        let base = self.sp - need;
        for i in 0..n {
            let mut v = self.st[base + i];
            let d = base + n + i * k;
            for (r, &s) in self.sc[..k].iter().enumerate() {
                if s != 0.0 {
                    v = v + self.st[d + r] * s;
                }
            }
            self.st[base + i] = v;
        }
        self.sp = base + n;
        self.blended = true;
        true
    }

    /// cs_run(start, end, depth) -> 0 (return / end of data), 1 (endchar), -1 error
    fn run(&mut self, start: u32, end: u32, depth: u32) -> i32 {
        if depth > CS_DEPTH {
            return -1;
        }
        let t = self.t;
        let mut p = start;
        loop {
            if p >= end {
                return 0;
            }
            self.budget -= 1;
            if self.budget < 0 {
                return -1;
            }
            let b = t.0[p as usize] as u32;
            p += 1;
            let flow = if b >= 32 || b == 28 {
                // ---- numbers
                let v: f32;
                if b == 28 {
                    if p + 2 > end {
                        return -1;
                    }
                    v = (u16::from_be_bytes([t.0[p as usize], t.0[p as usize + 1]]) as i16) as f32;
                    p += 2;
                } else if b <= 246 {
                    v = (b as i32 - 139) as f32;
                } else if b == 255 {
                    if p + 4 > end {
                        return -1;
                    }
                    let w = u32::from_be_bytes([
                        t.0[p as usize],
                        t.0[p as usize + 1],
                        t.0[p as usize + 2],
                        t.0[p as usize + 3],
                    ]) as i32;
                    p += 4;
                    v = w as f32 * F_INV65536;
                } else {
                    if p >= end {
                        return -1;
                    }
                    let b2 = t.0[p as usize] as i32;
                    p += 1;
                    v = if b <= 250 {
                        (((b as i32 - 247) << 8) + b2 + 108) as f32
                    } else {
                        (-(((b as i32 - 251) << 8) + b2 + 108)) as f32
                    };
                }
                self.push(v)
            } else {
                match b {
                    1 | 3 | 18 | 23 => {
                        // hstem vstem hstemhm vstemhm
                        let n = self.width_even();
                        self.nstems = self.nstems.wrapping_add((self.sp as u32 - n) >> 1);
                        Flow::Clear
                    }
                    19 | 20 => {
                        // hintmask cntrmask
                        if self.sp != 0 {
                            let n = self.width_even(); // implicit vstem
                            self.nstems = self.nstems.wrapping_add((self.sp as u32 - n) >> 1);
                        }
                        self.wd = true;
                        p = p.wrapping_add(self.nstems.wrapping_add(7) >> 3);
                        Flow::Clear
                    }
                    21 => {
                        // rmoveto
                        let i = self.width(2);
                        self.moveto_begin();
                        self.x = self.arg(i) + self.x;
                        self.y = self.arg(i + 1) + self.y;
                        self.moveto_end();
                        Flow::Clear
                    }
                    22 => {
                        // hmoveto
                        let i = self.width(1);
                        self.moveto_begin();
                        self.x = self.arg(i) + self.x;
                        self.moveto_end();
                        Flow::Clear
                    }
                    4 => {
                        // vmoveto
                        let i = self.width(1);
                        self.moveto_begin();
                        self.y = self.arg(i) + self.y;
                        self.moveto_end();
                        Flow::Clear
                    }
                    5 => {
                        // rlineto
                        let mut i = 0;
                        while i + 2 <= self.sp {
                            let (dx, dy) = (self.arg(i), self.arg(i + 1));
                            self.line(dx, dy);
                            i += 2;
                        }
                        Flow::Clear
                    }
                    6 | 7 => {
                        // hlineto / vlineto: alternating
                        let mut vert = b == 7;
                        let mut i = 0;
                        while i < self.sp {
                            let v = self.arg(i);
                            if vert { self.line(0.0, v) } else { self.line(v, 0.0) }
                            vert = !vert;
                            i += 1;
                        }
                        Flow::Clear
                    }
                    8 => {
                        // rrcurveto
                        let mut i = 0;
                        while i + 6 <= self.sp {
                            self.curve6(i);
                            i += 6;
                        }
                        Flow::Clear
                    }
                    24 => {
                        // rcurveline
                        let mut i = 0;
                        while i + 8 <= self.sp {
                            self.curve6(i);
                            i += 6;
                        }
                        if i + 2 <= self.sp {
                            let (dx, dy) = (self.arg(i), self.arg(i + 1));
                            self.line(dx, dy);
                        }
                        Flow::Clear
                    }
                    25 => {
                        // rlinecurve
                        let mut i = 0;
                        while i + 8 <= self.sp {
                            let (dx, dy) = (self.arg(i), self.arg(i + 1));
                            self.line(dx, dy);
                            i += 2;
                        }
                        if i + 6 <= self.sp {
                            self.curve6(i);
                        }
                        Flow::Clear
                    }
                    27 => {
                        // hhcurveto: dy1? {dxa dxb dyb dxc}+
                        let mut i = 0;
                        self.d[1] = 0.0;
                        if self.sp & 1 != 0 {
                            self.d[1] = self.arg(0);
                            i = 1;
                        }
                        while i + 4 <= self.sp {
                            self.d[0] = self.arg(i);
                            self.d[2] = self.arg(i + 1);
                            self.d[3] = self.arg(i + 2);
                            self.d[4] = self.arg(i + 3);
                            self.d[5] = 0.0;
                            self.curve();
                            self.d[1] = 0.0;
                            i += 4;
                        }
                        Flow::Clear
                    }
                    26 => {
                        // vvcurveto: dx1? {dya dxb dyb dyc}+
                        let mut i = 0;
                        self.d[0] = 0.0;
                        if self.sp & 1 != 0 {
                            self.d[0] = self.arg(0);
                            i = 1;
                        }
                        while i + 4 <= self.sp {
                            self.d[1] = self.arg(i);
                            self.d[2] = self.arg(i + 1);
                            self.d[3] = self.arg(i + 2);
                            self.d[4] = 0.0;
                            self.d[5] = self.arg(i + 3);
                            self.curve();
                            self.d[0] = 0.0;
                            i += 4;
                        }
                        Flow::Clear
                    }
                    30 | 31 => {
                        // vhcurveto / hvcurveto: alternating
                        let mut h = b == 31;
                        let mut i = 0;
                        while i + 4 <= self.sp {
                            // the last curve takes a fifth argument when exactly 5 remain
                            let last = (self.sp - i == 5) as usize;
                            if h {
                                self.d[0] = self.arg(i);
                                self.d[1] = 0.0;
                                self.d[2] = self.arg(i + 1);
                                self.d[3] = self.arg(i + 2);
                                self.d[5] = self.arg(i + 3);
                                self.d[4] = 0.0;
                                if last != 0 {
                                    self.d[4] = self.arg(i + 4);
                                }
                            } else {
                                self.d[0] = 0.0;
                                self.d[1] = self.arg(i);
                                self.d[2] = self.arg(i + 1);
                                self.d[3] = self.arg(i + 2);
                                self.d[4] = self.arg(i + 3);
                                self.d[5] = 0.0;
                                if last != 0 {
                                    self.d[5] = self.arg(i + 4);
                                }
                            }
                            self.curve();
                            i += 4 + last;
                            h = !h;
                        }
                        Flow::Clear
                    }
                    10 | 29 => {
                        // callsubr / callgsubr
                        let ix = if b == 10 { self.ls } else { self.gs };
                        if self.sp == 0 {
                            return -1;
                        }
                        self.sp -= 1;
                        let idx = cvtt(self.st[self.sp]);
                        let i = cs_bias(ix.count).wrapping_add(idx);
                        if i < 0 {
                            return -1;
                        }
                        let Some((s, l)) = ix_get(t, &ix, i as u32) else { return -1 };
                        let r = self.run(s, s.wrapping_add(l), depth + 1);
                        if r < 0 {
                            return -1;
                        }
                        if r != 0 {
                            return 1; // endchar inside the subroutine
                        }
                        Flow::Loop
                    }
                    11 => return 0, // return
                    15 if self.v2 => {
                        // vsindex: the item variation data of the following blends
                        let Some(v) = self.top() else { return -1 };
                        let i = cvtt(v);
                        if self.blended || i < 0 {
                            return -1;
                        }
                        self.vsi = i as u32;
                        self.nreg = None;
                        Flow::Clear
                    }
                    16 if self.v2 => {
                        if !self.blend() {
                            return -1;
                        }
                        Flow::Loop // the values stay on the stack
                    }
                    14 => {
                        // endchar (a seac accent is ignored)
                        self.wd = true;
                        self.moveto_begin();
                        Flow::Ret1
                    }
                    12 => {
                        if p >= end {
                            return -1;
                        }
                        let e = t.0[p as usize] as u32;
                        p += 1;
                        self.escape(e)
                    }
                    _ => Flow::Clear, // reserved
                }
            };
            match flow {
                Flow::Clear => self.sp = 0,
                Flow::Loop => {}
                Flow::Err => return -1,
                Flow::Ret1 => return 1,
            }
        }
    }

    // escaped operators 12 x
    fn escape(&mut self, e: u32) -> Flow {
        match e {
            35 => {
                // flex: 12 deltas + fd: two curves
                if self.sp < 12 {
                    return Flow::Clear;
                }
                self.d6(0);
                self.curve();
                self.d6(6);
                self.curve();
                Flow::Clear
            }
            34 => {
                // hflex: dx1 dx2 dy2 dx3 dx4 dx5 dx6
                if self.sp < 7 {
                    return Flow::Clear;
                }
                let y0 = self.y;
                self.d = [self.arg(0), 0.0, self.arg(1), self.arg(2), self.arg(3), 0.0];
                self.curve();
                self.d[0] = self.arg(4);
                self.d[1] = 0.0;
                self.d[2] = self.arg(5);
                self.d[3] = y0 - self.y; // dy5 brings y back to the start
                self.d[4] = self.arg(6);
                self.d[5] = 0.0;
                self.curve();
                Flow::Clear
            }
            36 => {
                // hflex1: dx1 dy1 dx2 dy2 dx3 dx4 dx5 dy5 dx6
                if self.sp < 9 {
                    return Flow::Clear;
                }
                let y0 = self.y;
                self.d = [self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4), 0.0];
                self.curve();
                self.d[0] = self.arg(5);
                self.d[1] = 0.0;
                self.d[2] = self.arg(6);
                self.d[3] = self.arg(7);
                self.d[4] = self.arg(8);
                self.d[5] = (y0 - self.y) - self.d[3]; // back to the start y
                self.curve();
                Flow::Clear
            }
            37 => {
                // flex1: dx1 dy1 .. dx5 dy5 d6
                if self.sp < 11 {
                    return Flow::Clear;
                }
                let (x0, y0) = (self.x, self.y);
                let (mut sx, mut sy) = (0.0f32, 0.0f32);
                for k in 0..5 {
                    sx = sx + self.st[k * 2];
                    sy = sy + self.st[k * 2 + 1];
                }
                let horiz = fabs(sx) > fabs(sy); // the last delta is horizontal
                self.d6(0);
                self.curve();
                self.d[0] = self.arg(6);
                self.d[1] = self.arg(7);
                self.d[2] = self.arg(8);
                self.d[3] = self.arg(9);
                // the end point: d6 along the main direction, back to the start in the other
                let xx = (self.x + self.d[0]) + self.d[2];
                let yy = (self.y + self.d[1]) + self.d[3];
                let d6 = self.arg(10);
                if horiz {
                    self.d[4] = d6;
                    self.d[5] = y0 - yy;
                } else {
                    self.d[5] = d6;
                    self.d[4] = x0 - xx;
                }
                self.curve();
                Flow::Clear
            }
            9 => match self.top() {
                Some(v) => self.settop(fabs(v)),
                None => Flow::Err,
            },
            14 => match self.top() {
                Some(v) => self.settop(f32::from_bits(v.to_bits() ^ 0x8000_0000)),
                None => Flow::Err,
            },
            26 => match self.top() {
                Some(v) => self.settop(sqrt(fabs(v))),
                None => Flow::Err,
            },
            5 => match self.top() {
                Some(v) => self.settop(if zf(v, 0.0) { 1.0 } else { 0.0 }),
                None => Flow::Err,
            },
            10 | 11 | 24 | 12 | 3 | 4 | 15 => {
                let Some((a, b)) = self.pop2() else { return Flow::Err };
                let r = match e {
                    10 => a + b,
                    11 => a - b,
                    24 => a * b,
                    12 => {
                        if zf(b, 0.0) {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    3 => {
                        if zf(a, 0.0) || zf(b, 0.0) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                    4 => {
                        if (a < 0.0 || a > 0.0) || (b < 0.0 || b > 0.0) {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => {
                        if zf(a, b) {
                            1.0
                        } else {
                            0.0
                        }
                    }
                };
                self.push(r)
            }
            18 => {
                // drop
                if self.sp == 0 {
                    return Flow::Err;
                }
                self.sp -= 1;
                Flow::Loop
            }
            27 => match self.top() {
                Some(v) => self.push(v),
                None => Flow::Err,
            },
            28 => {
                // exch
                let Some((a, b)) = self.pop2() else { return Flow::Err };
                self.st[self.sp] = b;
                self.st[self.sp + 1] = a;
                self.sp += 2;
                Flow::Loop
            }
            29 => {
                // index
                let Some(v) = self.top() else { return Flow::Err };
                let mut i = cvtt(v);
                if i < 0 {
                    i = 0;
                }
                let k = (self.sp as i32).wrapping_sub(2).wrapping_sub(i);
                if k < 0 {
                    return Flow::Err;
                }
                let v = self.arg(k as usize);
                self.settop(v)
            }
            30 => {
                // roll: num j roll
                let Some((a, b)) = self.pop2() else { return Flow::Err };
                let num = cvtt(a);
                let j = cvtt(b);
                if num <= 0 {
                    return Flow::Loop;
                }
                if num as usize > self.sp {
                    return Flow::Err;
                }
                self.roll(num, j);
                Flow::Loop
            }
            20 => {
                // put
                let Some((a, b)) = self.pop2() else { return Flow::Err };
                let i = cvtt(b) as u32;
                if i < 32 {
                    self.trans[i as usize] = a;
                }
                Flow::Loop
            }
            21 => {
                // get
                let Some(v) = self.top() else { return Flow::Err };
                let i = cvtt(v) as u32;
                if i >= 32 {
                    return Flow::Err;
                }
                self.settop(self.trans[i as usize])
            }
            22 => {
                // s1 s2 v1 v2 ifelse -> v1 <= v2 ? s1 : s2
                if self.sp < 4 {
                    return Flow::Err;
                }
                self.sp -= 4;
                let s = self.sp;
                let v1 = self.st[s + 2];
                let v2 = self.st[s + 3];
                let r = if !(v1 > v2) { self.st[s] } else { self.st[s + 1] };
                self.push(r)
            }
            23 => {
                // random
                let r = self.rand.wrapping_mul(1_103_515_245).wrapping_add(12345);
                self.rand = r;
                let v = ((r >> 9) | 1) as i32 as f32 * F_INV8M; // never 0
                self.push(v)
            }
            _ => Flow::Clear, // reserved / deprecated
        }
    }
}
