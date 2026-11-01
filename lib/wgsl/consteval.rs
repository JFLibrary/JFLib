// consteval -- arithmetic on folded constants (ir::Value) with WGSL's error rules
//
//   comps(v, ty)                   the components of a composite value (Zero expanded)
//   convert(v, from, to, explicit) a scalar conversion applied component-wise: automatic
//                                  conversions (AbstractInt -> i32 must fit, ...) and the
//                                  value constructors (f32 -> i32 truncates and clamps, i32 ->
//                                  u32 reinterprets, bool -> 0 / 1)
//   unary / binary                 the operators of spec section 8 on equal shapes (the
//                                  checker broadcasts scalars); matrix products separately
//   check_float(x, s)              round an f64 result to s: None when it is NaN, infinite or
//                                  outside the finite range (spec 15.7.3: an error)
//   fmt(v)                         a value for messages
//
// AbstractInt is i64 with every overflow an error; AbstractFloat is f64; f32 and f16
// results are computed in f64 and rounded once (exact for + - * / and sqrt: 53 >= 2p + 2);
// concrete integers wrap, except division by zero, MIN / -1 and the shift rules (8.10).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::BinOp;
use crate::ir::*;
use crate::lex::{round_bits, round_f64, Fmt};
use crate::math;
use crate::types::Types;

pub type CR<T> = Result<T, String>;

/// the largest composite a constant may have (elements, all levels together)
pub const MAX_ELEMS: usize = 1 << 20;

pub fn zero(s: Scalar) -> Value {
    match s {
        Scalar::Bool => Value::Bool(false),
        Scalar::AbstractInt => Value::AInt(0),
        Scalar::AbstractFloat => Value::AFloat(0.0),
        Scalar::I32 => Value::I32(0),
        Scalar::U32 => Value::U32(0),
        Scalar::F32 => Value::F32(0.0),
        Scalar::F16 => Value::F16(0.0),
    }
}

/// the component count and type of a composite
pub fn shape(types: &Types, ty: TypeId) -> Option<(usize, Vec<TypeId>)> {
    match types.get(ty) {
        Type::Vector(n, s) => Some((n as usize, alloc::vec![s.id(); n as usize])),
        Type::Matrix(c, r, s) => {
            let col = types.vec(r, s);
            Some((c as usize, alloc::vec![col; c as usize]))
        }
        Type::Array { elem, count: ArrayCount::Fixed(n), .. } => {
            let n = (n as usize).min(MAX_ELEMS);
            Some((n, alloc::vec![elem; n]))
        }
        Type::Struct(i) => {
            let s = types.structs.get(i as usize)?;
            Some((s.members.len(), s.members.iter().map(|m| m.ty).collect()))
        }
        _ => None,
    }
}

/// the type of component i
pub fn comp_type(types: &Types, ty: TypeId, i: usize) -> TypeId {
    match types.get(ty) {
        Type::Vector(_, s) => s.id(),
        Type::Matrix(_, r, s) => types.vec(r, s),
        Type::Array { elem, .. } => elem,
        Type::Struct(k) => types.structs.get(k as usize).and_then(|s| s.members.get(i)).map(|m| m.ty).unwrap_or(BOOL),
        _ => ty,
    }
}

/// the components of a composite value (scalars: the value itself as the one component)
pub fn comps(types: &Types, v: &Value, ty: TypeId) -> Vec<Value> {
    match v {
        Value::Comp(c) => c.clone(),
        Value::Zero => match shape(types, ty) {
            Some((n, tys)) => (0..n).map(|i| zero_of(types, tys[i])).collect(),
            None => alloc::vec![zero_of(types, ty)],
        },
        _ => alloc::vec![v.clone()],
    }
}

/// component i without expanding the whole value
pub fn comp(types: &Types, v: &Value, ty: TypeId, i: usize) -> Value {
    match v {
        Value::Comp(c) => c.get(i).cloned().unwrap_or(Value::Zero),
        Value::Zero => {
            let t = comp_type(types, ty, i);
            zero_of(types, t)
        }
        _ => v.clone(),
    }
}

/// the zero value of a type: scalars directly, composites as Zero
pub fn zero_of(types: &Types, ty: TypeId) -> Value {
    match types.get(ty) {
        Type::Scalar(s) => zero(s),
        _ => Value::Zero,
    }
}

/// a scalar as f64
pub fn as_f64(v: &Value) -> f64 {
    match *v {
        Value::AInt(x) => x as f64,
        Value::AFloat(x) => x,
        Value::I32(x) => x as f64,
        Value::U32(x) => x as f64,
        Value::F32(x) | Value::F16(x) => x as f64,
        Value::Bool(b) => b as u8 as f64,
        _ => 0.0,
    }
}

/// a scalar integer as i128
pub fn as_int(v: &Value) -> Option<i128> {
    match *v {
        Value::AInt(x) => Some(x as i128),
        Value::I32(x) => Some(x as i128),
        Value::U32(x) => Some(x as i128),
        _ => None,
    }
}

/// an f64 result rounded to a float scalar type; None for NaN, infinity or out of range
pub fn check_float(x: f64, s: Scalar) -> Option<Value> {
    if !x.is_finite() {
        return None;
    }
    match s {
        Scalar::AbstractFloat => Some(Value::AFloat(x)),
        Scalar::F32 => {
            if x.abs() > f32::MAX as f64 {
                return None;
            }
            let r = x as f32;
            if r.is_finite() { Some(Value::F32(r)) } else { None }
        }
        Scalar::F16 => {
            if x.abs() > 65504.0 {
                return None;
            }
            round_f64(x, Fmt::F16).map(|v| Value::F16(v as f32))
        }
        _ => None,
    }
}

fn int_to_f16(x: i128) -> Option<Value> {
    let (m, neg) = (x.unsigned_abs(), x < 0);
    let (v, _) = round_bits(m, 0, false, Fmt::F16)?;
    Some(Value::F16(if neg { -v as f32 } else { v as f32 }))
}

/// convert one scalar (see the module comment)
pub fn convert_scalar(v: &Value, to: Scalar, explicit: bool) -> CR<Value> {
    use Value::*;
    let range = |what: String| Err(format!("der Wert {} lässt sich nicht als {} darstellen", what, to.name()));
    Ok(match (v, to) {
        (Zero, _) => zero(to),
        (Bool(b), Scalar::Bool) => Bool(*b),
        (Bool(b), _) if explicit => match to {
            Scalar::I32 => I32(*b as i32),
            Scalar::U32 => U32(*b as u32),
            Scalar::F32 => F32(*b as u8 as f32),
            Scalar::F16 => F16(*b as u8 as f32),
            Scalar::AbstractInt => AInt(*b as i64),
            _ => AFloat(*b as u8 as f64),
        },
        (_, Scalar::Bool) => Bool(match v {
            AInt(x) => *x != 0,
            I32(x) => *x != 0,
            U32(x) => *x != 0,
            AFloat(x) => *x != 0.0,
            F32(x) | F16(x) => *x != 0.0,
            _ => false,
        }),
        (AInt(x), Scalar::AbstractInt) => AInt(*x),
        (AInt(x), Scalar::I32) => match i32::try_from(*x) {
            Ok(y) => I32(y),
            Err(_) => return range(format!("{}", x)),
        },
        (AInt(x), Scalar::U32) => match u32::try_from(*x) {
            Ok(y) => U32(y),
            Err(_) => return range(format!("{}", x)),
        },
        (AInt(x), Scalar::AbstractFloat) => AFloat(*x as f64),
        (AInt(x), Scalar::F32) => F32(*x as f32),
        (AInt(x), Scalar::F16) => match int_to_f16(*x as i128) {
            Some(v) if as_f64(&v).abs() <= 65504.0 => v,
            _ => return range(format!("{}", x)),
        },
        (I32(x), Scalar::I32) => I32(*x),
        (I32(x), Scalar::U32) => U32(*x as u32),
        (I32(x), Scalar::F32) => F32(*x as f32),
        (I32(x), Scalar::F16) => match int_to_f16(*x as i128) {
            Some(v) if (*x as f64).abs() <= 65504.0 => v,
            _ => return range(format!("{}", x)),
        },
        (U32(x), Scalar::U32) => U32(*x),
        (U32(x), Scalar::I32) => I32(*x as i32),
        (U32(x), Scalar::F32) => F32(*x as f32),
        (U32(x), Scalar::F16) => match int_to_f16(*x as i128) {
            Some(v) if (*x as f64) <= 65504.0 => v,
            _ => return range(format!("{}", x)),
        },
        (AFloat(x), Scalar::AbstractFloat) => AFloat(*x),
        (AFloat(x), Scalar::F32) | (AFloat(x), Scalar::F16) => match check_float(*x, to) {
            Some(v) => v,
            None => return range(fmt_f64(*x)),
        },
        (F32(x), Scalar::F32) => F32(*x),
        (F32(x), Scalar::F16) => match check_float(*x as f64, Scalar::F16) {
            Some(v) => v,
            None => return range(fmt_f64(*x as f64)),
        },
        (F16(x), Scalar::F16) => F16(*x),
        (F16(x), Scalar::F32) => F32(*x),
        (AFloat(_), Scalar::I32) | (F16(_), Scalar::I32) | (F32(_), Scalar::I32) if explicit => I32(float_to_i32(v)),
        (AFloat(_), Scalar::U32) | (F32(_), Scalar::U32) | (F16(_), Scalar::U32) if explicit => U32(float_to_u32(v)),
        _ => return Err(format!("keine Umwandlung von {} nach {}", fmt(v), to.name())),
    })
}

fn float_to_i32(v: &Value) -> i32 {
    let x = math::trunc(as_f64(v));
    let (lo, hi) = match v {
        Value::F32(_) => (-2147483648.0, 2147483520.0),
        _ => (-2147483648.0, 2147483647.0),
    };
    x.clamp(lo, hi) as i32
}

fn float_to_u32(v: &Value) -> u32 {
    let x = math::trunc(as_f64(v));
    let hi = match v {
        Value::F32(_) => 4294967040.0,
        _ => 4294967295.0,
    };
    x.clamp(0.0, hi) as u32
}

/// convert a value of type `from` to `to` (same shape; component-wise)
pub fn convert(types: &mut Types, v: &Value, from: TypeId, to: TypeId, explicit: bool) -> CR<Value> {
    if from == to {
        return Ok(v.clone());
    }
    if let Value::Zero = v {
        return Ok(zero_of(types, to));
    }
    match (types.get(from), types.get(to)) {
        (Type::Scalar(_), Type::Scalar(s)) => convert_scalar(v, s, explicit),
        (_, Type::Struct(_)) => {
            // frexp / modf results: member-wise
            let n = shape(types, to).map(|s| s.0).unwrap_or(0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let (f, t) = (comp_type(types, from, i), comp_type(types, to, i));
                let c = comp(types, v, from, i);
                out.push(convert(types, &c, f, t, explicit)?);
            }
            Ok(Value::Comp(out))
        }
        _ => {
            let n = shape(types, from).map(|s| s.0).unwrap_or(0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let (f, t) = (comp_type(types, from, i), comp_type(types, to, i));
                let c = comp(types, v, from, i);
                out.push(convert(types, &c, f, t, explicit)?);
            }
            Ok(Value::Comp(out))
        }
    }
}

/// a float scalar's type
fn fscalar(v: &Value) -> Scalar {
    match v {
        Value::F32(_) => Scalar::F32,
        Value::F16(_) => Scalar::F16,
        _ => Scalar::AbstractFloat,
    }
}

pub fn float_result(x: f64, like: &Value) -> CR<Value> {
    let s = fscalar(like);
    check_float(x, s).ok_or_else(|| format!("das Ergebnis {} lässt sich nicht als {} darstellen", fmt_f64(x), s.name()))
}

fn overflow(what: &str) -> String {
    format!("Überlauf: das Ergebnis von {} passt nicht in AbstractInt (64 Bit)", what)
}

/// a unary operator on a scalar
pub fn unary_scalar(op: UnaryOp, v: &Value) -> CR<Value> {
    use Value::*;
    Ok(match (op, v) {
        (UnaryOp::Neg, AInt(x)) => AInt(x.checked_neg().ok_or_else(|| overflow(&format!("-({})", x)))?),
        (UnaryOp::Neg, I32(x)) => I32(x.wrapping_neg()),
        (UnaryOp::Neg, AFloat(x)) => AFloat(-x),
        (UnaryOp::Neg, F32(x)) => F32(-x),
        (UnaryOp::Neg, F16(x)) => F16(-x),
        (UnaryOp::Not, Bool(b)) => Bool(!b),
        (UnaryOp::BitNot, AInt(x)) => AInt(!x),
        (UnaryOp::BitNot, I32(x)) => I32(!x),
        (UnaryOp::BitNot, U32(x)) => U32(!x),
        _ => return Err(String::from("ungültiger Operand")),
    })
}

/// a binary operator on two scalars of the same type (shifts: the right one is u32)
pub fn binary_scalar(op: BinOp, a: &Value, b: &Value) -> CR<Value> {
    use Value::*;
    let txt = || format!("{} {} {}", fmt(a), op.text(), fmt(b));
    if op.is_comparison() {
        let r = match (a, b) {
            (Bool(x), Bool(y)) => match op {
                BinOp::Eq => x == y,
                BinOp::Ne => x != y,
                _ => return Err(String::from("bool lässt sich nicht ordnen")),
            },
            _ => {
                if let (Some(x), Some(y)) = (as_int(a), as_int(b)) {
                    cmp(op, x.cmp(&y))
                } else {
                    let (x, y) = (as_f64(a), as_f64(b));
                    match x.partial_cmp(&y) {
                        Some(o) => cmp(op, o),
                        None => op == BinOp::Ne,
                    }
                }
            }
        };
        return Ok(Bool(r));
    }
    Ok(match (a, b) {
        (Bool(x), Bool(y)) => match op {
            BinOp::And | BinOp::LogAnd => Bool(*x && *y),
            BinOp::Or | BinOp::LogOr => Bool(*x || *y),
            _ => return Err(String::from("ungültiger Operator für bool")),
        },
        (AInt(x), AInt(y)) => {
            let (x, y) = (*x, *y);
            AInt(match op {
                BinOp::Add => x.checked_add(y).ok_or_else(|| overflow(&txt()))?,
                BinOp::Sub => x.checked_sub(y).ok_or_else(|| overflow(&txt()))?,
                BinOp::Mul => x.checked_mul(y).ok_or_else(|| overflow(&txt()))?,
                BinOp::Div | BinOp::Rem => {
                    if y == 0 {
                        return Err(format!("{}: Division durch null", txt()));
                    }
                    let r = if op == BinOp::Div { x.checked_div(y) } else { x.checked_rem(y) };
                    r.ok_or_else(|| overflow(&txt()))?
                }
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                _ => return Err(String::from("ungültiger Operator")),
            })
        }
        (AInt(x), U32(s)) => {
            let (x, s) = (*x, *s);
            match op {
                BinOp::Shl => {
                    if s >= 64 {
                        if x != 0 {
                            return Err(overflow(&txt()));
                        }
                        AInt(0)
                    } else {
                        let top = x >> (63 - s);
                        if top != 0 && top != -1 {
                            return Err(overflow(&txt()));
                        }
                        AInt(x << s)
                    }
                }
                BinOp::Shr => AInt(if s >= 64 { x >> 63 } else { x >> s }),
                _ => return Err(String::from("ungültiger Operator")),
            }
        }
        (I32(x), I32(y)) => {
            let (x, y) = (*x, *y);
            I32(match op {
                BinOp::Add => x.wrapping_add(y),
                BinOp::Sub => x.wrapping_sub(y),
                BinOp::Mul => x.wrapping_mul(y),
                BinOp::Div | BinOp::Rem => {
                    if y == 0 {
                        return Err(format!("{}: ganzzahlige Division durch null", txt()));
                    }
                    if x == i32::MIN && y == -1 {
                        return Err(format!("{}: das Ergebnis passt nicht in i32", txt()));
                    }
                    if op == BinOp::Div { x / y } else { x % y }
                }
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                _ => return Err(String::from("ungültiger Operator")),
            })
        }
        (U32(x), U32(y)) if !matches!(op, BinOp::Shl | BinOp::Shr) => {
            let (x, y) = (*x, *y);
            U32(match op {
                BinOp::Add => x.wrapping_add(y),
                BinOp::Sub => x.wrapping_sub(y),
                BinOp::Mul => x.wrapping_mul(y),
                BinOp::Div | BinOp::Rem => {
                    if y == 0 {
                        return Err(format!("{}: ganzzahlige Division durch null", txt()));
                    }
                    if op == BinOp::Div { x / y } else { x % y }
                }
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                _ => return Err(String::from("ungültiger Operator")),
            })
        }
        (I32(_), U32(s)) | (U32(_), U32(s)) => {
            let s = *s;
            if s >= 32 {
                return Err(format!("{}: die Schiebeweite muss kleiner als 32 sein", txt()));
            }
            match (a, op) {
                (I32(x), BinOp::Shl) => {
                    let top = *x >> (31 - s);
                    if top != 0 && top != -1 {
                        return Err(format!("{}: Überlauf beim Linksschieben (das Vorzeichen ändert sich)", txt()));
                    }
                    I32(x.wrapping_shl(s))
                }
                (I32(x), _) => I32(x >> s),
                (U32(x), BinOp::Shl) => {
                    if s > 0 && (x >> (32 - s)) != 0 {
                        return Err(format!("{}: Überlauf beim Linksschieben", txt()));
                    }
                    U32(x.wrapping_shl(s))
                }
                (U32(x), _) => U32(x >> s),
                _ => return Err(String::from("ungültiger Operator")),
            }
        }
        (AFloat(_), AFloat(_)) | (F32(_), F32(_)) | (F16(_), F16(_)) => {
            let (x, y) = (as_f64(a), as_f64(b));
            let r = match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => x / y,
                BinOp::Rem => {
                    if y == 0.0 {
                        f64::NAN
                    } else {
                        x - y * math::trunc(x / y)
                    }
                }
                _ => return Err(String::from("ungültiger Operator")),
            };
            float_result(r, a).map_err(|_| format!("`{}` lässt sich nicht als {} darstellen", txt(), fscalar(a).name()))?
        }
        _ => return Err(format!("ungültige Operanden für `{}`", op.text())),
    })
}

fn cmp(op: BinOp, o: core::cmp::Ordering) -> bool {
    use core::cmp::Ordering::*;
    match op {
        BinOp::Eq => o == Equal,
        BinOp::Ne => o != Equal,
        BinOp::Lt => o == Less,
        BinOp::Le => o != Greater,
        BinOp::Gt => o == Greater,
        _ => o != Less,
    }
}

/// component-wise unary operator
pub fn unary(types: &Types, op: UnaryOp, v: &Value, ty: TypeId) -> CR<Value> {
    if types.is_scalar(ty) {
        return unary_scalar(op, v);
    }
    let n = shape(types, ty).map(|s| s.0).unwrap_or(0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = comp_type(types, ty, i);
        out.push(unary(types, op, &comp(types, v, ty, i), t)?);
    }
    Ok(Value::Comp(out))
}

/// component-wise binary operator on equal shapes (matrices: + and -)
pub fn binary(types: &Types, op: BinOp, a: &Value, b: &Value, ty: TypeId, ty_b: TypeId) -> CR<Value> {
    if types.is_scalar(ty) {
        return binary_scalar(op, a, b);
    }
    let n = shape(types, ty).map(|s| s.0).unwrap_or(0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (t, tb) = (comp_type(types, ty, i), comp_type(types, ty_b, i));
        out.push(binary(types, op, &comp(types, a, ty, i), &comp(types, b, ty_b, i), t, tb)?);
    }
    Ok(Value::Comp(out))
}

/// a sum of products in the precision of `like` (each step rounded)
pub fn dot(xs: &[Value], ys: &[Value]) -> CR<Value> {
    let mut acc: Option<Value> = None;
    for (x, y) in xs.iter().zip(ys) {
        let p = binary_scalar(BinOp::Mul, x, y)?;
        acc = Some(match acc {
            None => p,
            Some(a) => binary_scalar(BinOp::Add, &a, &p)?,
        });
    }
    acc.ok_or_else(|| String::from("leeres Produkt"))
}

/// the columns of a matrix value as component vectors
pub fn mat_cols(types: &Types, v: &Value, ty: TypeId) -> Vec<Vec<Value>> {
    let (c, r, s) = match types.get(ty) {
        Type::Matrix(c, r, s) => (c, r, s),
        _ => return Vec::new(),
    };
    let col = types.vec(r, s);
    (0..c as usize).map(|i| comps(types, &comp(types, v, ty, i), col)).collect()
}

/// matrix * vector, vector * matrix, matrix * matrix
pub fn mat_mul(types: &Types, a: &Value, ta: TypeId, b: &Value, tb: TypeId) -> CR<Value> {
    match (types.get(ta), types.get(tb)) {
        (Type::Matrix(c, r, s), Type::Vector(..)) => {
            let cols = mat_cols(types, a, ta);
            let v = comps(types, b, tb);
            let _ = (c, s);
            let mut out = Vec::new();
            for i in 0..r as usize {
                let row: Vec<Value> = cols.iter().map(|col| col[i].clone()).collect();
                out.push(dot(&row, &v)?);
            }
            Ok(Value::Comp(out))
        }
        (Type::Vector(..), Type::Matrix(..)) => {
            let cols = mat_cols(types, b, tb);
            let v = comps(types, a, ta);
            let mut out = Vec::new();
            for col in &cols {
                out.push(dot(&v, col)?);
            }
            Ok(Value::Comp(out))
        }
        (Type::Matrix(_, r, _), Type::Matrix(c2, _, _)) => {
            let ca = mat_cols(types, a, ta);
            let cb = mat_cols(types, b, tb);
            let mut out = Vec::new();
            for j in 0..c2 as usize {
                let mut col = Vec::new();
                for i in 0..r as usize {
                    let row: Vec<Value> = ca.iter().map(|c| c[i].clone()).collect();
                    col.push(dot(&row, &cb[j])?);
                }
                out.push(Value::Comp(col));
            }
            Ok(Value::Comp(out))
        }
        _ => Err(String::from("ungültige Operanden für die Matrixmultiplikation")),
    }
}

pub fn fmt_f64(x: f64) -> String {
    if x == math::trunc(x) && x.abs() < 1e16 {
        format!("{}.0", x as i64)
    } else {
        format!("{}", x)
    }
}

/// a value for messages ("2147483648", "1.5", "vec(1, 2)")
pub fn fmt(v: &Value) -> String {
    match v {
        Value::Bool(b) => format!("{}", b),
        Value::AInt(x) => format!("{}", x),
        Value::AFloat(x) => fmt_f64(*x),
        Value::I32(x) => format!("{}i", x),
        Value::U32(x) => format!("{}u", x),
        Value::F32(x) => format!("{}f", fmt_f64(*x as f64)),
        Value::F16(x) => format!("{}h", fmt_f64(*x as f64)),
        Value::Comp(c) => {
            let mut s = String::from("(");
            for (i, x) in c.iter().take(8).enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&fmt(x));
            }
            if c.len() > 8 {
                s.push_str(", …");
            }
            s.push(')');
            s
        }
        Value::Zero => String::from("0"),
    }
}

/// the number of scalar components of a type (arrays multiply), capped
pub fn elem_count(types: &Types, ty: TypeId) -> usize {
    match types.get(ty) {
        Type::Vector(n, _) => n as usize,
        Type::Matrix(c, r, _) => c as usize * r as usize,
        Type::Array { elem, count: ArrayCount::Fixed(n), .. } => (n as usize).saturating_mul(elem_count(types, elem)).min(usize::MAX / 2),
        Type::Struct(i) => types.structs.get(i as usize).map(|s| s.members.iter().map(|m| elem_count(types, m.ty)).fold(0usize, |a, b| a.saturating_add(b))).unwrap_or(1),
        _ => 1,
    }
}
