// reflect -- what a pipeline needs to know of a SPIR-V module: its entry points, its
// specialization constants (the pipeline constants: SpecId, type, default), the value every
// constant takes once a pipeline sets some of them (composites and OpSpecConstantOp
// evaluated), and the workgroup size of a compute entry point.  lib/gpu checks pipeline
// constants and workgroup limits with it; shadertest's interpreter runs specialized modules.
//
//   entry_points(words) -> Result<Vec<Entry>, String>        model, name, function id
//   spec_constants(words) -> Result<Vec<SpecConst>, String>  SpecId, result id, Kind, default
//   constant_values(words, set) -> Result<BTreeMap<u32, Vec<u32>>, String>
//                    the words of every module-level constant (OpConstant*, OpSpecConstant*,
//                    composites, OpSpecConstantOp) once the specialization constants of `set`
//                    ((SpecId, value bits) pairs) are replaced; Err for what SPIR-V leaves
//                    undefined (division by 0, a shift by the width or more) and for an
//                    operation SPIR-V does not allow in OpSpecConstantOp under Shader
//   workgroup_size(words, entry, set) -> Result<Option<[u32; 3]>, String>
//                    of the GLCompute entry point `entry`: the WorkgroupSize built-in (it wins),
//                    else LocalSizeId, else LocalSize (None: no such compute entry point or no
//                    size)
//   uses_local_size_id(words) -> bool                        (Vulkan: needs maintenance4)
//   f16_from_f32 / f16_to_f32                                IEEE half conversions
//
// Value words: a scalar of up to 32 bits takes one word (narrower ones in its low bits, signed
// ones sign-extended, as SPIR-V literals), a 64-bit one two (low word first), bool one (0 / 1);
// a composite is its constituents' words in order.  Value bits in `set` and in SpecConst
// default: the scalar's bits in the low bits of the u64 (bool 0 / 1).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::module::{Id, Ty};
use crate::parse::{self, Inst};
use crate::spv::{self, op, Bi, Dec, Mode, Model};

/// the type of a scalar specialization constant
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int { width: u32, signed: bool },
    Float { width: u32 },
}

/// a scalar specialization constant with its SpecId (OpSpecConstant / True / False)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecConst {
    pub spec_id: u32,
    /// the constant's result id
    pub id: u32,
    pub kind: Kind,
    /// the default value's bits
    pub default: u64,
}

/// an entry point (OpEntryPoint)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// spv::Model as u32
    pub model: u32,
    pub name: String,
    pub func: u32,
}

fn load(words: &[u32]) -> Result<Vec<Inst<'_>>, String> {
    if words.len() < 5 || words[0] != spv::MAGIC {
        return Err(String::from("not a SPIR-V module (magic number)"));
    }
    parse::parse(words)
}

pub fn entry_points(words: &[u32]) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for i in load(words)? {
        if i.op == op::EntryPoint && i.ops.len() >= 3 {
            out.push(Entry { model: i.ops[0], name: parse::string(&i.ops[2..]).0, func: i.ops[1] });
        }
    }
    Ok(out)
}

/// the scalar types of a module (bool, int, float, vector, matrix, array, struct)
fn types(ins: &[Inst]) -> BTreeMap<u32, Ty> {
    let mut t = BTreeMap::new();
    for i in ins {
        let o = i.ops;
        let g = |n: usize| o.get(n).copied().unwrap_or(0);
        let ty = match i.op {
            op::TypeBool => Ty::Bool,
            op::TypeInt => Ty::Int { width: g(0), signed: g(1) != 0 },
            op::TypeFloat => Ty::Float { width: g(0) },
            op::TypeVector => Ty::Vector { elem: Id(g(0)), n: g(1) },
            op::TypeMatrix => Ty::Matrix { col: Id(g(0)), cols: g(1) },
            op::TypeArray => Ty::Array { elem: Id(g(0)), len: Id(g(1)) },
            op::TypeStruct => Ty::Struct { members: o.iter().map(|&m| Id(m)).collect() },
            _ => continue,
        };
        t.insert(i.res, ty);
    }
    t
}

fn kind_of(tys: &BTreeMap<u32, Ty>, t: u32) -> Option<Kind> {
    match tys.get(&t)? {
        Ty::Bool => Some(Kind::Bool),
        Ty::Int { width, signed } => Some(Kind::Int { width: *width, signed: *signed }),
        Ty::Float { width } => Some(Kind::Float { width: *width }),
        _ => None,
    }
}

fn width_of(k: Kind) -> u32 {
    match k {
        Kind::Bool => 1,
        Kind::Int { width, .. } | Kind::Float { width } => width,
    }
}

fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn sext(v: u64, w: u32) -> i64 {
    if w >= 64 || w == 0 { v as i64 } else { ((v << (64 - w)) as i64) >> (64 - w) }
}

/// the words of a scalar value
fn put(out: &mut Vec<u32>, k: Kind, v: u64) {
    match k {
        Kind::Bool => out.push((v != 0) as u32),
        Kind::Int { width: 64, .. } | Kind::Float { width: 64 } => {
            out.push(v as u32);
            out.push((v >> 32) as u32);
        }
        Kind::Int { width, signed: true } => out.push(sext(v & mask(width), width) as u32),
        _ => out.push((v & mask(width_of(k))) as u32),
    }
}

/// the bits of a scalar at word w
fn get(words: &[u32], k: Kind) -> u64 {
    let lo = words.first().copied().unwrap_or(0) as u64;
    match k {
        Kind::Bool => (lo != 0) as u64,
        Kind::Int { width: 64, .. } | Kind::Float { width: 64 } => lo | (words.get(1).copied().unwrap_or(0) as u64) << 32,
        _ => lo & mask(width_of(k)),
    }
}

fn scalar_words(k: Kind) -> usize {
    if width_of(k) == 64 { 2 } else { 1 }
}

/// a 32-bit float as the bits of an IEEE half (round to nearest even, overflow to infinity)
pub fn f16_from_f32(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 | (man >> 13) as u16 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let r = if rem > mid || (rem == mid && half & 1 == 1) { half + 1 } else { half };
        return sign | r as u16;
    }
    let half = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1fff;
    let r = if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) { half + 1 } else { half };
    sign | r as u16
}

/// the bits of an IEEE half as a 32-bit float (exact)
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h as u32) & 0x8000) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign
        } else {
            let (mut e, mut m) = (113u32, man);
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (man << 13)
    } else {
        sign | ((exp + 112) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}

/// a float of width w (16, 32, 64) as f64
fn float_of(v: u64, w: u32) -> f64 {
    match w {
        16 => f16_to_f32(v as u16) as f64,
        32 => f32::from_bits(v as u32) as f64,
        _ => f64::from_bits(v),
    }
}

/// f64 as the bits of a float of width w (rounded to nearest)
fn float_bits(x: f64, w: u32) -> u64 {
    match w {
        16 => f16_from_f32(x as f32) as u64,
        32 => (x as f32).to_bits() as u64,
        _ => x.to_bits(),
    }
}

pub fn spec_constants(words: &[u32]) -> Result<Vec<SpecConst>, String> {
    let ins = load(words)?;
    let tys = types(&ins);
    let mut spec_ids: BTreeMap<u32, u32> = BTreeMap::new();
    for i in &ins {
        if i.op == op::Decorate && i.ops.len() >= 3 && i.ops[1] == Dec::SpecId as u32 {
            spec_ids.insert(i.ops[0], i.ops[2]);
        }
    }
    let mut out = Vec::new();
    for i in &ins {
        let Some(&spec_id) = spec_ids.get(&i.res) else { continue };
        let (kind, default) = match i.op {
            op::SpecConstantTrue | op::SpecConstantFalse => (Kind::Bool, (i.op == op::SpecConstantTrue) as u64),
            op::SpecConstant => {
                let k = kind_of(&tys, i.ty).ok_or_else(|| format!("SpecId {}: not a scalar specialization constant", spec_id))?;
                (k, get(i.ops, k))
            }
            _ => return Err(format!("SpecId {} on %{}, which is no scalar specialization constant", spec_id, i.res)),
        };
        out.push(SpecConst { spec_id, id: i.res, kind, default });
    }
    Ok(out)
}

struct Eval {
    tys: BTreeMap<u32, Ty>,
    vty: BTreeMap<u32, u32>,
    vals: BTreeMap<u32, Vec<u32>>,
}

impl Eval {
    /// words of a value of type t
    fn size(&self, t: u32) -> Result<usize, String> {
        Ok(match self.tys.get(&t) {
            Some(Ty::Bool) => 1,
            Some(Ty::Int { width, .. } | Ty::Float { width }) => if *width == 64 { 2 } else { 1 },
            Some(Ty::Vector { elem, n }) => *n as usize * self.size(elem.0)?,
            Some(Ty::Matrix { col, cols }) => *cols as usize * self.size(col.0)?,
            Some(Ty::Array { elem, len }) => {
                let l = self.vals.get(&len.0).and_then(|w| w.first()).copied().ok_or("an array length without a value")?;
                l as usize * self.size(elem.0)?
            }
            Some(Ty::Struct { members }) => {
                let mut s = 0;
                for m in members {
                    s += self.size(m.0)?;
                }
                s
            }
            _ => return Err(format!("%{} is no type of a constant", t)),
        })
    }

    /// (word offset, type) of the element `idx` selects inside type t
    fn locate(&self, mut t: u32, idx: &[u32]) -> Result<(usize, u32), String> {
        let mut at = 0;
        for &k in idx {
            let (next, off) = match self.tys.get(&t) {
                Some(Ty::Struct { members }) => {
                    let m = members.get(k as usize).ok_or("a member index out of range")?;
                    let mut off = 0;
                    for x in &members[..k as usize] {
                        off += self.size(x.0)?;
                    }
                    (m.0, off)
                }
                Some(Ty::Vector { elem, n }) if k < *n => (elem.0, k as usize * self.size(elem.0)?),
                Some(Ty::Matrix { col, cols }) if k < *cols => (col.0, k as usize * self.size(col.0)?),
                Some(Ty::Array { elem, .. }) => (elem.0, k as usize * self.size(elem.0)?),
                _ => return Err(String::from("an index out of range")),
            };
            at += off;
            t = next;
        }
        Ok((at, t))
    }

    /// (component kind, count) of a scalar or vector type
    fn comps(&self, t: u32) -> Result<(Kind, usize), String> {
        match self.tys.get(&t) {
            Some(Ty::Vector { elem, n }) => Ok((kind_of(&self.tys, elem.0).ok_or("a vector of no scalar")?, *n as usize)),
            _ => Ok((kind_of(&self.tys, t).ok_or_else(|| format!("%{} is no scalar or vector type", t))?, 1)),
        }
    }

    fn value(&self, id: u32) -> Result<&Vec<u32>, String> {
        self.vals.get(&id).ok_or_else(|| format!("%{} is no constant defined before its use", id))
    }

    /// the components of a scalar / vector value
    fn components(&self, id: u32) -> Result<(Kind, Vec<u64>), String> {
        let t = *self.vty.get(&id).ok_or_else(|| format!("%{} has no type", id))?;
        let (k, n) = self.comps(t)?;
        let w = self.value(id)?;
        let sw = scalar_words(k);
        if w.len() < n * sw {
            return Err(format!("%{}: too few words", id));
        }
        Ok((k, (0..n).map(|c| get(&w[c * sw..], k)).collect()))
    }

    fn spec_op(&self, ty: u32, o: &[u32]) -> Result<Vec<u32>, String> {
        let opc = *o.first().ok_or("OpSpecConstantOp without an operation")? as u16;
        let a = &o[1..];
        let arg = |n: usize| a.get(n).copied().ok_or_else(|| String::from("OpSpecConstantOp: an operand is missing"));
        let mut out = Vec::new();
        match opc {
            op::CompositeExtract => {
                let c = arg(0)?;
                let ct = *self.vty.get(&c).ok_or("no type")?;
                let (at, t) = self.locate(ct, &a[1..])?;
                let n = self.size(t)?;
                out.extend_from_slice(self.value(c)?.get(at..at + n).ok_or("a constant too short")?);
            }
            op::CompositeInsert => {
                let (obj, c) = (arg(0)?, arg(1)?);
                let ct = *self.vty.get(&c).ok_or("no type")?;
                let (at, t) = self.locate(ct, &a[2..])?;
                let n = self.size(t)?;
                out = self.value(c)?.clone();
                let ov = self.value(obj)?;
                if out.len() < at + n || ov.len() != n {
                    return Err(String::from("CompositeInsert: sizes do not fit"));
                }
                out[at..at + n].copy_from_slice(ov);
            }
            op::VectorShuffle => {
                let (x, y) = (self.components(arg(0)?)?, self.components(arg(1)?)?);
                let k = x.0;
                for &c in &a[2..] {
                    let c = c as usize;
                    let v = if c < x.1.len() { x.1[c] } else { *y.1.get(c - x.1.len()).ok_or("a shuffle component out of range")? };
                    put(&mut out, k, v);
                }
            }
            op::Select => {
                let (c, x, y) = (arg(0)?, arg(1)?, arg(2)?);
                let ct = *self.vty.get(&c).ok_or("no type")?;
                let (_, cn) = self.comps(ct)?;
                let (cw, xw, yw) = (self.value(c)?, self.value(x)?, self.value(y)?);
                if cn == 1 {
                    out = if cw[0] != 0 { xw.clone() } else { yw.clone() };
                } else {
                    let (k, n) = self.comps(ty)?;
                    let sw = scalar_words(k);
                    for i in 0..n {
                        let src = if cw[i] != 0 { xw } else { yw };
                        out.extend_from_slice(&src[i * sw..i * sw + sw]);
                    }
                }
            }
            _ => {
                let (rk, rn) = self.comps(ty)?;
                let (ak, av) = self.components(arg(0)?)?;
                let bv = if a.len() > 1 { Some(self.components(arg(1)?)?) } else { None };
                for i in 0..rn {
                    let x = *av.get(i).or(av.first()).ok_or("an empty operand")?;
                    let (bk, y) = match &bv {
                        Some((k, v)) => (*k, *v.get(i).or(v.first()).ok_or("an empty operand")?),
                        None => (ak, 0),
                    };
                    let r = scalar_op(opc, rk, ak, bk, x, y)?;
                    put(&mut out, rk, r);
                }
            }
        }
        Ok(out)
    }
}

/// one component of an OpSpecConstantOp operation (rk: the result's kind, ak / bk the
/// operands')
fn scalar_op(opc: u16, rk: Kind, ak: Kind, bk: Kind, x: u64, y: u64) -> Result<u64, String> {
    let rw = width_of(rk);
    let (aw, bw) = (width_of(ak), width_of(bk));
    let m = mask(rw);
    let (sx, sy) = (sext(x, aw), sext(y, bw));
    let undefined = |what: &str| Err(format!("OpSpecConstantOp: {} is undefined", what));
    let b = |c: bool| Ok(c as u64);
    match opc {
        op::SConvert => Ok(sx as u64 & m),
        op::UConvert => Ok(x & m),
        op::FConvert => Ok(float_bits(float_of(x, aw), rw)),
        op::QuantizeToF16 => {
            let h = f16_from_f32(f32::from_bits(x as u32));
            // too small for a normal half: +-0 (SPIR-V allows either sign of 0)
            let h = if h & 0x7c00 == 0 { h & 0x8000 } else { h };
            Ok(f16_to_f32(h).to_bits() as u64)
        }
        op::SNegate => Ok((sx.wrapping_neg() as u64) & m),
        op::Not => Ok(!x & m),
        op::IAdd => Ok(x.wrapping_add(y) & m),
        op::ISub => Ok(x.wrapping_sub(y) & m),
        op::IMul => Ok(x.wrapping_mul(y) & m),
        op::UDiv | op::UMod => {
            if y == 0 {
                return undefined("division by 0");
            }
            Ok(if opc == op::UDiv { x / y } else { x % y })
        }
        op::SDiv | op::SRem | op::SMod => {
            if sy == 0 {
                return undefined("division by 0");
            }
            let r = match opc {
                op::SDiv => sx.wrapping_div(sy),
                op::SRem => sx.wrapping_rem(sy),
                _ => {
                    let r = sx.wrapping_rem(sy);
                    if r != 0 && (r < 0) != (sy < 0) { r + sy } else { r }
                }
            };
            Ok(r as u64 & m)
        }
        op::ShiftLeftLogical | op::ShiftRightLogical | op::ShiftRightArithmetic => {
            if y >= rw as u64 {
                return undefined("a shift by the bit width or more");
            }
            Ok(match opc {
                op::ShiftLeftLogical => (x << y) & m,
                op::ShiftRightLogical => (x & m) >> y,
                _ => ((sx >> y) as u64) & m,
            })
        }
        op::BitwiseOr => Ok(x | y),
        op::BitwiseXor => Ok(x ^ y),
        op::BitwiseAnd => Ok(x & y),
        op::LogicalOr => b(x != 0 || y != 0),
        op::LogicalAnd => b(x != 0 && y != 0),
        op::LogicalNot => b(x == 0),
        op::LogicalEqual => b((x != 0) == (y != 0)),
        op::LogicalNotEqual => b((x != 0) != (y != 0)),
        op::IEqual => b(x == y),
        op::INotEqual => b(x != y),
        op::ULessThan => b(x < y),
        op::UGreaterThan => b(x > y),
        op::ULessThanEqual => b(x <= y),
        op::UGreaterThanEqual => b(x >= y),
        op::SLessThan => b(sx < sy),
        op::SGreaterThan => b(sx > sy),
        op::SLessThanEqual => b(sx <= sy),
        op::SGreaterThanEqual => b(sx >= sy),
        _ => Err(format!("Op{} is no operation of OpSpecConstantOp under Shader",
                         spv::op_info(opc).map_or("?", |i| i.name))),
    }
}

pub fn constant_values(words: &[u32], set: &[(u32, u64)]) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    let ins = load(words)?;
    let mut spec_ids: BTreeMap<u32, u32> = BTreeMap::new();
    for i in &ins {
        if i.op == op::Decorate && i.ops.len() >= 3 && i.ops[1] == Dec::SpecId as u32 {
            spec_ids.insert(i.ops[0], i.ops[2]);
        }
    }
    let given = |id: u32| spec_ids.get(&id).and_then(|s| set.iter().find(|x| x.0 == *s)).map(|x| x.1);
    let mut e = Eval { tys: types(&ins), vty: BTreeMap::new(), vals: BTreeMap::new() };
    for i in &ins {
        if i.op == op::Function {
            break;
        }
        if i.ty != 0 {
            e.vty.insert(i.res, i.ty);
        }
        let v = match i.op {
            op::Constant => i.ops.to_vec(),
            op::ConstantTrue | op::ConstantFalse => alloc::vec![(i.op == op::ConstantTrue) as u32],
            op::ConstantNull => alloc::vec![0; e.size(i.ty)?],
            op::ConstantComposite | op::SpecConstantComposite => {
                let mut w = Vec::new();
                for p in i.ops {
                    w.extend_from_slice(e.value(*p)?);
                }
                w
            }
            op::SpecConstantTrue | op::SpecConstantFalse => {
                alloc::vec![given(i.res).map_or(i.op == op::SpecConstantTrue, |v| v != 0) as u32]
            }
            op::SpecConstant => match (given(i.res), kind_of(&e.tys, i.ty)) {
                (Some(v), Some(k)) => {
                    let mut w = Vec::new();
                    put(&mut w, k, v);
                    w
                }
                _ => i.ops.to_vec(),
            },
            op::SpecConstantOp => e.spec_op(i.ty, i.ops)?,
            _ => continue,
        };
        e.vals.insert(i.res, v);
    }
    Ok(e.vals)
}

pub fn workgroup_size(words: &[u32], entry: &str, set: &[(u32, u64)]) -> Result<Option<[u32; 3]>, String> {
    let ins = load(words)?;
    let Some(func) = ins.iter()
        .find(|i| i.op == op::EntryPoint && i.ops.len() >= 3 && i.ops[0] == Model::GLCompute as u32
              && parse::string(&i.ops[2..]).0 == entry)
        .map(|i| i.ops[1])
    else {
        return Ok(None);
    };
    let builtin = ins.iter().find(|i| i.op == op::Decorate && i.ops.len() >= 3 && i.ops[1] == Dec::BuiltIn as u32
                                  && i.ops[2] == Bi::WorkgroupSize as u32).map(|i| i.ops[0]);
    let id_mode = ins.iter().find(|i| i.op == op::ExecutionModeId && i.ops.len() >= 5 && i.ops[0] == func
                                  && i.ops[1] == Mode::LocalSizeId as u32);
    if builtin.is_some() || id_mode.is_some() {
        let vals = constant_values(words, set)?;
        let word = |id: u32| vals.get(&id).and_then(|w| w.first()).copied().ok_or_else(|| format!("%{} has no constant value", id));
        if let Some(c) = builtin {
            let w = vals.get(&c).ok_or("the WorkgroupSize built-in is no constant")?;
            if w.len() != 3 {
                return Err(String::from("the WorkgroupSize built-in is no uvec3"));
            }
            return Ok(Some([w[0], w[1], w[2]]));
        }
        if let Some(i) = id_mode {
            return Ok(Some([word(i.ops[2])?, word(i.ops[3])?, word(i.ops[4])?]));
        }
    }
    Ok(ins.iter().find(|i| i.op == op::ExecutionMode && i.ops.len() >= 5 && i.ops[0] == func && i.ops[1] == Mode::LocalSize as u32)
        .map(|i| [i.ops[2], i.ops[3], i.ops[4]]))
}

pub fn uses_local_size_id(words: &[u32]) -> bool {
    load(words).is_ok_and(|ins| ins.iter().any(|i| i.op == op::ExecutionModeId && i.ops.get(1) == Some(&(Mode::LocalSizeId as u32))))
}
