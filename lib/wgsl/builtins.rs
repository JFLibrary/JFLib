// builtins -- the built-in functions of spec section 17: the name table with properties,
// the overload candidates, texture overloads, and the constant evaluation of every @const
// built-in
//
//   lookup(name) -> Option<(Builtin, flags)>   flags: CONST, MUST_USE, FRAG (fragment stage
//                                     only), COMPUTE (compute stage only), NOT_VERTEX and the
//                                     feature bit the function needs (feature())
//   candidates(b, types, f16) -> Vec<Sig>   the fully elaborated overloads of the numeric,
//                                     logical, packing, derivative and barrier functions;
//                                     overload resolution (check_expr.rs) ranks them
//   TEXTURE                           the texture overloads as patterns (one entry per
//                                     overload of the spec's tables)
//   eval(b, args) -> Result<Value, String>   const evaluation (args already converted
//                                     to the overload's parameter types)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::consteval::{self, as_f64, as_int, comp, comps, float_result, CR};
use crate::feature;
use crate::ir::*;
use crate::math;
use crate::types::Types;

pub const CONST: u8 = 1;
pub const MUST_USE: u8 = 2;
pub const FRAG: u8 = 4;
pub const COMPUTE: u8 = 8;
/// fragment and compute stage (the subgroup and quad functions)
pub const NOT_VERTEX: u8 = 16;

const CM: u8 = CONST | MUST_USE;
const SG: u8 = MUST_USE | NOT_VERTEX;

pub const TABLE: &[(&str, Builtin, u8)] = {
    use Builtin::*;
    &[
        ("all", All, CM), ("any", Any, CM), ("select", Select, CM), ("arrayLength", ArrayLength, MUST_USE),
        ("abs", Abs, CM), ("acos", Acos, CM), ("acosh", Acosh, CM), ("asin", Asin, CM), ("asinh", Asinh, CM),
        ("atan", Atan, CM), ("atanh", Atanh, CM), ("atan2", Atan2, CM), ("ceil", Ceil, CM), ("clamp", Clamp, CM),
        ("cos", Cos, CM), ("cosh", Cosh, CM), ("countLeadingZeros", CountLeadingZeros, CM),
        ("countOneBits", CountOneBits, CM), ("countTrailingZeros", CountTrailingZeros, CM), ("cross", Cross, CM),
        ("degrees", Degrees, CM), ("determinant", Determinant, CM), ("distance", Distance, CM), ("dot", Dot, CM),
        ("dot4U8Packed", Dot4U8Packed, CM), ("dot4I8Packed", Dot4I8Packed, CM), ("exp", Exp, CM), ("exp2", Exp2, CM),
        ("extractBits", ExtractBits, CM), ("faceForward", FaceForward, CM), ("firstLeadingBit", FirstLeadingBit, CM),
        ("firstTrailingBit", FirstTrailingBit, CM), ("floor", Floor, CM), ("fma", Fma, CM), ("fract", Fract, CM),
        ("frexp", Frexp, CM), ("insertBits", InsertBits, CM), ("inverseSqrt", InverseSqrt, CM), ("ldexp", Ldexp, CM),
        ("length", Length, CM), ("log", Log, CM), ("log2", Log2, CM), ("max", Max, CM), ("min", Min, CM),
        ("mix", Mix, CM), ("modf", Modf, CM), ("normalize", Normalize, CM), ("pow", Pow, CM),
        ("quantizeToF16", QuantizeToF16, CM), ("radians", Radians, CM), ("reflect", Reflect, CM),
        ("refract", Refract, CM), ("reverseBits", ReverseBits, CM), ("round", Round, CM), ("saturate", Saturate, CM),
        ("sign", Sign, CM), ("sin", Sin, CM), ("sinh", Sinh, CM), ("smoothstep", Smoothstep, CM), ("sqrt", Sqrt, CM),
        ("step", Step, CM), ("tan", Tan, CM), ("tanh", Tanh, CM), ("transpose", Transpose, CM), ("trunc", Trunc, CM),
        ("dpdx", Dpdx, MUST_USE | FRAG), ("dpdxCoarse", DpdxCoarse, MUST_USE | FRAG), ("dpdxFine", DpdxFine, MUST_USE | FRAG),
        ("dpdy", Dpdy, MUST_USE | FRAG), ("dpdyCoarse", DpdyCoarse, MUST_USE | FRAG), ("dpdyFine", DpdyFine, MUST_USE | FRAG),
        ("fwidth", Fwidth, MUST_USE | FRAG), ("fwidthCoarse", FwidthCoarse, MUST_USE | FRAG),
        ("fwidthFine", FwidthFine, MUST_USE | FRAG),
        ("textureDimensions", TextureDimensions, MUST_USE), ("textureGather", TextureGather, MUST_USE),
        ("textureGatherCompare", TextureGatherCompare, MUST_USE), ("textureLoad", TextureLoad, MUST_USE),
        ("textureNumLayers", TextureNumLayers, MUST_USE), ("textureNumLevels", TextureNumLevels, MUST_USE),
        ("textureNumSamples", TextureNumSamples, MUST_USE), ("textureSample", TextureSample, MUST_USE | FRAG),
        ("textureSampleBias", TextureSampleBias, MUST_USE | FRAG), ("textureSampleCompare", TextureSampleCompare, MUST_USE | FRAG),
        ("textureSampleCompareLevel", TextureSampleCompareLevel, MUST_USE), ("textureSampleGrad", TextureSampleGrad, MUST_USE),
        ("textureSampleLevel", TextureSampleLevel, MUST_USE),
        ("textureSampleBaseClampToEdge", TextureSampleBaseClampToEdge, MUST_USE), ("textureStore", TextureStore, 0),
        ("atomicLoad", AtomicLoad, 0), ("atomicStore", AtomicStore, 0), ("atomicAdd", AtomicAdd, 0),
        ("atomicSub", AtomicSub, 0), ("atomicMax", AtomicMax, 0), ("atomicMin", AtomicMin, 0), ("atomicAnd", AtomicAnd, 0),
        ("atomicOr", AtomicOr, 0), ("atomicXor", AtomicXor, 0), ("atomicExchange", AtomicExchange, 0),
        ("atomicCompareExchangeWeak", AtomicCompareExchangeWeak, 0), ("atomicStoreMin", AtomicStoreMin, 0),
        ("atomicStoreMax", AtomicStoreMax, 0),
        ("pack4x8snorm", Pack4x8snorm, CM), ("pack4x8unorm", Pack4x8unorm, CM), ("pack4xI8", Pack4xI8, CM),
        ("pack4xU8", Pack4xU8, CM), ("pack4xI8Clamp", Pack4xI8Clamp, CM), ("pack4xU8Clamp", Pack4xU8Clamp, CM),
        ("pack2x16snorm", Pack2x16snorm, CM), ("pack2x16unorm", Pack2x16unorm, CM), ("pack2x16float", Pack2x16float, CM),
        ("unpack4x8snorm", Unpack4x8snorm, CM), ("unpack4x8unorm", Unpack4x8unorm, CM), ("unpack4xI8", Unpack4xI8, CM),
        ("unpack4xU8", Unpack4xU8, CM), ("unpack2x16snorm", Unpack2x16snorm, CM), ("unpack2x16unorm", Unpack2x16unorm, CM),
        ("unpack2x16float", Unpack2x16float, CM),
        ("storageBarrier", StorageBarrier, COMPUTE), ("textureBarrier", TextureBarrier, COMPUTE),
        ("workgroupBarrier", WorkgroupBarrier, COMPUTE), ("workgroupUniformLoad", WorkgroupUniformLoad, MUST_USE | COMPUTE),
        ("subgroupAdd", SubgroupAdd, SG), ("subgroupExclusiveAdd", SubgroupExclusiveAdd, SG), ("subgroupInclusiveAdd", SubgroupInclusiveAdd, SG),
        ("subgroupAll", SubgroupAll, SG), ("subgroupAnd", SubgroupAnd, SG), ("subgroupAny", SubgroupAny, SG), ("subgroupBallot", SubgroupBallot, SG),
        ("subgroupBroadcast", SubgroupBroadcast, SG), ("subgroupBroadcastFirst", SubgroupBroadcastFirst, SG), ("subgroupElect", SubgroupElect, SG),
        ("subgroupMax", SubgroupMax, SG), ("subgroupMin", SubgroupMin, SG), ("subgroupMul", SubgroupMul, SG),
        ("subgroupExclusiveMul", SubgroupExclusiveMul, SG), ("subgroupInclusiveMul", SubgroupInclusiveMul, SG), ("subgroupOr", SubgroupOr, SG),
        ("subgroupShuffle", SubgroupShuffle, SG), ("subgroupShuffleDown", SubgroupShuffleDown, SG), ("subgroupShuffleUp", SubgroupShuffleUp, SG),
        ("subgroupShuffleXor", SubgroupShuffleXor, SG), ("subgroupXor", SubgroupXor, SG),
        ("quadBroadcast", QuadBroadcast, SG), ("quadSwapDiagonal", QuadSwapDiagonal, SG), ("quadSwapX", QuadSwapX, SG), ("quadSwapY", QuadSwapY, SG),
    ]
};

/// the names of the buffer view built-ins (not implemented: buffer_view is a gap)
pub const BUFFER_VIEW_NAMES: &[&str] = &["bufferView", "bufferArrayView", "bufferLength"];

pub fn lookup(name: &str) -> Option<(Builtin, u8)> {
    TABLE.iter().find(|e| e.0 == name).map(|e| (e.1, e.2))
}

pub fn name(b: Builtin) -> &'static str {
    TABLE.iter().find(|e| e.1 == b).map(|e| e.0).unwrap_or("?")
}

/// the feature a built-in needs (0: none)
pub fn feature(b: Builtin) -> u32 {
    use Builtin::*;
    match b {
        Dot4U8Packed | Dot4I8Packed | Pack4xI8 | Pack4xU8 | Pack4xI8Clamp | Pack4xU8Clamp | Unpack4xI8 | Unpack4xU8 => {
            feature::PACKED_4X8_INTEGER_DOT_PRODUCT
        }
        TextureBarrier => feature::READONLY_AND_READWRITE_STORAGE_TEXTURES,
        AtomicStoreMin | AtomicStoreMax => feature::ATOMIC_VEC2U_MIN_MAX,
        SubgroupAdd | SubgroupExclusiveAdd | SubgroupInclusiveAdd | SubgroupAll | SubgroupAnd | SubgroupAny | SubgroupBallot | SubgroupBroadcast
        | SubgroupBroadcastFirst | SubgroupElect | SubgroupMax | SubgroupMin | SubgroupMul | SubgroupExclusiveMul | SubgroupInclusiveMul
        | SubgroupOr | SubgroupShuffle | SubgroupShuffleDown | SubgroupShuffleUp | SubgroupShuffleXor | SubgroupXor | QuadBroadcast
        | QuadSwapDiagonal | QuadSwapX | QuadSwapY => feature::SUBGROUPS,
        _ => 0,
    }
}

/// a fully elaborated overload: parameter types and the result (NONE: no result)
#[derive(Clone, Copy, Debug)]
pub struct Sig {
    pub p: [TypeId; 8],
    pub n: u8,
    pub ret: TypeId,
}

pub const NONE: TypeId = VOID;

impl Sig {
    pub fn new(ps: &[TypeId], ret: TypeId) -> Sig {
        let mut p = [0; 8];
        for (i, &t) in ps.iter().take(8).enumerate() {
            p[i] = t;
        }
        Sig { p, n: ps.len().min(8) as u8, ret }
    }
    pub fn params(&self) -> &[TypeId] {
        &self.p[..self.n as usize]
    }
}

const FLOATS: [Scalar; 3] = [Scalar::AbstractFloat, Scalar::F32, Scalar::F16];
const NUMS: [Scalar; 6] = [Scalar::AbstractInt, Scalar::AbstractFloat, Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16];
const SIGNED: [Scalar; 5] = [Scalar::AbstractInt, Scalar::AbstractFloat, Scalar::I32, Scalar::F32, Scalar::F16];
const INTS: [Scalar; 2] = [Scalar::I32, Scalar::U32];
const CONCRETE: [Scalar; 4] = [Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16];

fn sets(f16: bool, s: &[Scalar]) -> impl Iterator<Item = Scalar> + '_ {
    s.iter().copied().filter(move |&x| f16 || x != Scalar::F16)
}

/// T = S or vecN<S>: every width, `arity` parameters of type T; ret: T (same) or S
fn cw(out: &mut Vec<Sig>, t: &Types, f16: bool, set: &[Scalar], arity: usize, ret_scalar: bool, vec_only: bool) {
    for s in sets(f16, set) {
        for n in if vec_only { 2..=4u8 } else { 1..=4u8 } {
            let ty = t.vec_or_scalar(n, s);
            let ps = [ty; 4];
            out.push(Sig::new(&ps[..arity], if ret_scalar { s.id() } else { ty }));
        }
    }
}

/// the overloads of a built-in that are not texture, atomic, pointer or struct-result
/// functions
pub fn candidates(b: Builtin, t: &mut Types, f16: bool) -> Vec<Sig> {
    use Builtin::*;
    let mut v = Vec::new();
    match b {
        All | Any => {
            v.push(Sig::new(&[BOOL], BOOL));
            for n in 2..=4 {
                v.push(Sig::new(&[t.vec(n, Scalar::Bool)], BOOL));
            }
        }
        Select => {
            for s in sets(f16, &Scalar::ALL) {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    v.push(Sig::new(&[ty, ty, BOOL], ty));
                    if n > 1 {
                        v.push(Sig::new(&[ty, ty, t.vec(n, Scalar::Bool)], ty));
                    }
                }
            }
        }
        Abs | Clamp | Max | Min => {
            let ar = match b {
                Abs => 1,
                Clamp => 3,
                _ => 2,
            };
            cw(&mut v, t, f16, &NUMS, ar, false, false);
        }
        Sign => cw(&mut v, t, f16, &SIGNED, 1, false, false),
        Acos | Acosh | Asin | Asinh | Atan | Atanh | Ceil | Cos | Cosh | Degrees | Exp | Exp2 | Floor | Fract
        | InverseSqrt | Log | Log2 | Radians | Round | Saturate | Sin | Sinh | Sqrt | Tan | Tanh | Trunc => {
            cw(&mut v, t, f16, &FLOATS, 1, false, false)
        }
        Atan2 | Pow | Step => cw(&mut v, t, f16, &FLOATS, 2, false, false),
        Smoothstep | Fma => cw(&mut v, t, f16, &FLOATS, 3, false, false),
        Mix => {
            cw(&mut v, t, f16, &FLOATS, 3, false, false);
            for s in sets(f16, &FLOATS) {
                for n in 2..=4 {
                    let ty = t.vec(n, s);
                    v.push(Sig::new(&[ty, ty, s.id()], ty));
                }
            }
        }
        Distance => cw(&mut v, t, f16, &FLOATS, 2, true, false),
        Length => cw(&mut v, t, f16, &FLOATS, 1, true, false),
        Dot => {
            for s in sets(f16, &NUMS) {
                for n in 2..=4 {
                    let ty = t.vec(n, s);
                    v.push(Sig::new(&[ty, ty], s.id()));
                }
            }
        }
        Cross => {
            for s in sets(f16, &FLOATS) {
                let ty = t.vec(3, s);
                v.push(Sig::new(&[ty, ty], ty));
            }
        }
        Normalize => cw(&mut v, t, f16, &FLOATS, 1, false, true),
        FaceForward => cw(&mut v, t, f16, &FLOATS, 3, false, true),
        Reflect => cw(&mut v, t, f16, &FLOATS, 2, false, true),
        Refract => {
            for s in sets(f16, &FLOATS) {
                for n in 2..=4 {
                    let ty = t.vec(n, s);
                    v.push(Sig::new(&[ty, ty, s.id()], ty));
                }
            }
        }
        Determinant => {
            for s in sets(f16, &FLOATS) {
                for n in 2..=4 {
                    v.push(Sig::new(&[t.mat(n, n, s)], s.id()));
                }
            }
        }
        Transpose => {
            for s in sets(f16, &FLOATS) {
                for c in 2..=4 {
                    for r in 2..=4 {
                        v.push(Sig::new(&[t.mat(c, r, s)], t.mat(r, c, s)));
                    }
                }
            }
        }
        Ldexp => {
            for s in sets(f16, &FLOATS) {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    // T can only be abstract if I is also abstract and vice versa
                    let i = if s == Scalar::AbstractFloat { Scalar::AbstractInt } else { Scalar::I32 };
                    v.push(Sig::new(&[ty, t.vec_or_scalar(n, i)], ty));
                }
            }
        }
        Frexp | Modf => {
            for s in sets(f16, &FLOATS) {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    let r = t.builtin_struct(if b == Frexp { BuiltinStruct::Frexp(n, s) } else { BuiltinStruct::Modf(n, s) });
                    v.push(Sig::new(&[ty], r));
                }
            }
        }
        QuantizeToF16 => cw(&mut v, t, f16, &[Scalar::F32], 1, false, false),
        CountLeadingZeros | CountOneBits | CountTrailingZeros | ReverseBits | FirstLeadingBit | FirstTrailingBit => {
            cw(&mut v, t, f16, &INTS, 1, false, false)
        }
        ExtractBits => {
            for s in INTS {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    v.push(Sig::new(&[ty, U32, U32], ty));
                }
            }
        }
        InsertBits => {
            for s in INTS {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    v.push(Sig::new(&[ty, ty, U32, U32], ty));
                }
            }
        }
        Dot4U8Packed => v.push(Sig::new(&[U32, U32], U32)),
        Dot4I8Packed => v.push(Sig::new(&[U32, U32], I32)),
        Pack4x8snorm | Pack4x8unorm => v.push(Sig::new(&[t.vec(4, Scalar::F32)], U32)),
        Pack4xI8 | Pack4xI8Clamp => v.push(Sig::new(&[t.vec(4, Scalar::I32)], U32)),
        Pack4xU8 | Pack4xU8Clamp => v.push(Sig::new(&[t.vec(4, Scalar::U32)], U32)),
        Pack2x16snorm | Pack2x16unorm | Pack2x16float => v.push(Sig::new(&[t.vec(2, Scalar::F32)], U32)),
        Unpack4x8snorm | Unpack4x8unorm => v.push(Sig::new(&[U32], t.vec(4, Scalar::F32))),
        Unpack4xI8 => v.push(Sig::new(&[U32], t.vec(4, Scalar::I32))),
        Unpack4xU8 => v.push(Sig::new(&[U32], t.vec(4, Scalar::U32))),
        Unpack2x16snorm | Unpack2x16unorm | Unpack2x16float => v.push(Sig::new(&[U32], t.vec(2, Scalar::F32))),
        Dpdx | DpdxCoarse | DpdxFine | Dpdy | DpdyCoarse | DpdyFine | Fwidth | FwidthCoarse | FwidthFine => {
            cw(&mut v, t, f16, &[Scalar::F32], 1, false, false)
        }
        StorageBarrier | TextureBarrier | WorkgroupBarrier => v.push(Sig::new(&[], NONE)),
        SubgroupAdd | SubgroupExclusiveAdd | SubgroupInclusiveAdd | SubgroupMul | SubgroupExclusiveMul | SubgroupInclusiveMul | SubgroupMax
        | SubgroupMin | SubgroupBroadcastFirst | QuadSwapDiagonal | QuadSwapX | QuadSwapY => cw(&mut v, t, f16, &CONCRETE, 1, false, false),
        SubgroupAnd | SubgroupOr | SubgroupXor => cw(&mut v, t, f16, &INTS, 1, false, false),
        SubgroupAll | SubgroupAny => v.push(Sig::new(&[BOOL], BOOL)),
        SubgroupBallot => v.push(Sig::new(&[BOOL], t.vec(4, Scalar::U32))),
        SubgroupElect => v.push(Sig::new(&[], BOOL)),
        SubgroupBroadcast | SubgroupShuffle | QuadBroadcast => {
            for s in sets(f16, &CONCRETE) {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    v.push(Sig::new(&[ty, I32], ty));
                    v.push(Sig::new(&[ty, U32], ty));
                }
            }
        }
        SubgroupShuffleDown | SubgroupShuffleUp | SubgroupShuffleXor => {
            for s in sets(f16, &CONCRETE) {
                for n in 1..=4u8 {
                    let ty = t.vec_or_scalar(n, s);
                    v.push(Sig::new(&[ty, U32], ty));
                }
            }
        }
        _ => {}
    }
    v
}

/// the errors a built-in has when some of its arguments are constant (None: not constant):
/// Err((first argument, last argument, message)); the same for const-expressions and for
/// override-expressions at pipeline creation
pub fn arg_checks(t: &Types, b: Builtin, a: &[(Option<Value>, TypeId)]) -> Result<(), (usize, usize, String)> {
    let c = |i: usize| a.get(i).and_then(|x| x.0.as_ref());
    let ints = |v: &Value| -> Vec<i128> {
        match v {
            Value::Comp(cs) => cs.iter().filter_map(as_int).collect(),
            _ => as_int(v).into_iter().collect(),
        }
    };
    match b {
        Builtin::SubgroupBroadcast | Builtin::SubgroupShuffle | Builtin::SubgroupShuffleUp | Builtin::SubgroupShuffleDown | Builtin::SubgroupShuffleXor | Builtin::QuadBroadcast => {
            if let Some(v) = c(1).and_then(as_int) {
                let limit = if b == Builtin::QuadBroadcast { 4 } else { 128 };
                let arg = match b {
                    Builtin::SubgroupShuffleUp | Builtin::SubgroupShuffleDown => "delta",
                    Builtin::SubgroupShuffleXor => "mask",
                    _ => "id",
                };
                if v < 0 {
                    return Err((1, 1, format!("das Argument `{}` von `{}` darf nicht negativ sein ({})", arg, name(b), v)));
                }
                if v >= limit {
                    return Err((1, 1, format!("das Argument `{}` von `{}` muss kleiner als {} sein, nicht {}", arg, name(b), limit, v)));
                }
            }
        }
        Builtin::ExtractBits | Builtin::InsertBits => {
            let (oi, ci) = if b == Builtin::ExtractBits { (1, 2) } else { (2, 3) };
            if let (Some(o), Some(n)) = (c(oi), c(ci)) {
                let (o, n) = (as_int(o).unwrap_or(0), as_int(n).unwrap_or(0));
                if o + n > 32 {
                    return Err((oi, ci, format!("`offset` + `count` = {} ist größer als 32", o + n)));
                }
            }
        }
        Builtin::Clamp => {
            if let (Some(lo), Some(hi)) = (c(1), c(2)) {
                let (lv, hv) = (comps(t, lo, a[1].1), comps(t, hi, a[2].1));
                let bad = if t.is_scalar(a[1].1) { as_f64(lo) > as_f64(hi) } else { lv.iter().zip(&hv).any(|(x, y)| as_f64(x) > as_f64(y)) };
                if bad {
                    return Err((1, 2, String::from("`clamp`: die Untergrenze ist größer als die Obergrenze")));
                }
            }
        }
        Builtin::Smoothstep => {
            if let (Some(e0), Some(e1)) = (c(0), c(1)) {
                let eq = if t.is_scalar(a[0].1) { as_f64(e0) == as_f64(e1) } else { comps(t, e0, a[0].1).iter().zip(comps(t, e1, a[1].1)).any(|(x, y)| as_f64(x) == as_f64(&y)) };
                if eq {
                    return Err((0, 1, String::from("`smoothstep`: `low` und `high` dürfen nicht gleich sein")));
                }
            }
        }
        Builtin::Ldexp => {
            if let Some(e) = c(1) {
                let bias = match t.scalar_of(a[0].1) {
                    Some(Scalar::F32) => 127,
                    Some(Scalar::F16) => 15,
                    _ => 1023,
                };
                let exps = if t.is_scalar(a[1].1) { ints(e) } else { comps(t, e, a[1].1).iter().filter_map(as_int).collect() };
                if exps.iter().any(|&x| x > bias + 1) {
                    return Err((1, 1, format!("`ldexp`: der Exponent ist größer als {}", bias + 1)));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ---- texture overloads

/// a texture parameter pattern
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TP {
    /// a sampled texture of a dimension; Some(s): only that sampled type
    Samp(TexDim, Option<Scalar>),
    Ms,
    Depth(TexDim),
    DepthMs,
    External,
    /// a storage texture; true: readable (read, read_write), false: writable (write, read_write)
    Storage(TexDim, bool),
    /// any texture of a query function (textureDimensions & co.), see tex_any
    Query(u8),
}

/// parameter patterns
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum P {
    T(TP),
    Sampler,
    SamplerCmp,
    F32,
    /// vecN<f32>
    V(u8),
    /// vecN<i32>, a const-expression with components in [-8, 7]
    Offset(u8),
    /// i32 or u32 (coordinates, array index, level, sample index)
    Int,
    /// vecN<i32> or vecN<u32>
    IntV(u8),
    /// the gather component: a const i32 / u32 in [0, 3]
    Component,
    /// vec4 of the storage texture's channel type
    Texel,
}

/// result patterns
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TR {
    V4F,
    F32,
    /// vec4 of the sampled type
    V4S,
    /// vec4 of the storage format's channel type
    V4C,
    U32,
    /// u32, vec2<u32> or vec3<u32> by the texture's dimension
    Dims,
    Void,
}

// Any(mask) bits for the query functions
pub const Q_DIMS: u8 = 1;
pub const Q_DIMS_LEVEL: u8 = 2;
pub const Q_LAYERS: u8 = 3;
pub const Q_LEVELS: u8 = 4;
pub const Q_SAMPLES: u8 = 5;

/// whether texture type ty matches a query pattern
pub fn tex_any(mask: u8, dim: TexDim, kind: TexKind) -> bool {
    use TexKind::*;
    match mask {
        Q_DIMS => true,
        Q_DIMS_LEVEL => matches!(kind, Sampled(_) | Depth),
        Q_LAYERS => dim.arrayed() && !matches!(kind, External),
        Q_LEVELS => matches!(kind, Sampled(_) | Depth),
        Q_SAMPLES => matches!(kind, Multisampled(_) | DepthMultisampled),
        _ => false,
    }
}

pub fn texture_overloads(b: Builtin) -> &'static [(&'static [P], TR)] {
    use Builtin::*;
    use TexDim::*;
    use P::*;
    use TP::*;
    match b {
        TextureDimensions => &[(&[T(Query(Q_DIMS))], TR::Dims), (&[T(Query(Q_DIMS_LEVEL)), Int], TR::Dims)],
        TextureNumLayers => &[(&[T(Query(Q_LAYERS))], TR::U32)],
        TextureNumLevels => &[(&[T(Query(Q_LEVELS))], TR::U32)],
        TextureNumSamples => &[(&[T(Query(Q_SAMPLES))], TR::U32)],
        TextureGather => &[
            (&[Component, T(Samp(D2, None)), Sampler, V(2)], TR::V4S),
            (&[Component, T(Samp(D2, None)), Sampler, V(2), Offset(2)], TR::V4S),
            (&[Component, T(Samp(D2Array, None)), Sampler, V(2), Int], TR::V4S),
            (&[Component, T(Samp(D2Array, None)), Sampler, V(2), Int, Offset(2)], TR::V4S),
            (&[Component, T(Samp(Cube, None)), Sampler, V(3)], TR::V4S),
            (&[Component, T(Samp(CubeArray, None)), Sampler, V(3), Int], TR::V4S),
            (&[T(Depth(D2)), Sampler, V(2)], TR::V4F),
            (&[T(Depth(D2)), Sampler, V(2), Offset(2)], TR::V4F),
            (&[T(Depth(Cube)), Sampler, V(3)], TR::V4F),
            (&[T(Depth(D2Array)), Sampler, V(2), Int], TR::V4F),
            (&[T(Depth(D2Array)), Sampler, V(2), Int, Offset(2)], TR::V4F),
            (&[T(Depth(CubeArray)), Sampler, V(3), Int], TR::V4F),
        ],
        TextureGatherCompare => &[
            (&[T(Depth(D2)), SamplerCmp, V(2), F32], TR::V4F),
            (&[T(Depth(D2)), SamplerCmp, V(2), F32, Offset(2)], TR::V4F),
            (&[T(Depth(D2Array)), SamplerCmp, V(2), Int, F32], TR::V4F),
            (&[T(Depth(D2Array)), SamplerCmp, V(2), Int, F32, Offset(2)], TR::V4F),
            (&[T(Depth(Cube)), SamplerCmp, V(3), F32], TR::V4F),
            (&[T(Depth(CubeArray)), SamplerCmp, V(3), Int, F32], TR::V4F),
        ],
        TextureLoad => &[
            (&[T(Samp(D1, None)), Int, Int], TR::V4S),
            (&[T(Samp(D2, None)), IntV(2), Int], TR::V4S),
            (&[T(Samp(D2Array, None)), IntV(2), Int, Int], TR::V4S),
            (&[T(Samp(D3, None)), IntV(3), Int], TR::V4S),
            (&[T(Ms), IntV(2), Int], TR::V4S),
            (&[T(Depth(D2)), IntV(2), Int], TR::F32),
            (&[T(Depth(D2Array)), IntV(2), Int, Int], TR::F32),
            (&[T(DepthMs), IntV(2), Int], TR::F32),
            (&[T(External), IntV(2)], TR::V4F),
            (&[T(Storage(D1, true)), Int], TR::V4C),
            (&[T(Storage(D2, true)), IntV(2)], TR::V4C),
            (&[T(Storage(D2Array, true)), IntV(2), Int], TR::V4C),
            (&[T(Storage(D3, true)), IntV(3)], TR::V4C),
        ],
        TextureSample => &[
            (&[T(Samp(D1, Some(Scalar::F32))), Sampler, F32], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2)], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), Offset(2)], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, Offset(2)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3)], TR::V4F),
            (&[T(Samp(Cube, Some(Scalar::F32))), Sampler, V(3)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), Offset(3)], TR::V4F),
            (&[T(Samp(CubeArray, Some(Scalar::F32))), Sampler, V(3), Int], TR::V4F),
            (&[T(Depth(D2)), Sampler, V(2)], TR::F32),
            (&[T(Depth(D2)), Sampler, V(2), Offset(2)], TR::F32),
            (&[T(Depth(D2Array)), Sampler, V(2), Int], TR::F32),
            (&[T(Depth(D2Array)), Sampler, V(2), Int, Offset(2)], TR::F32),
            (&[T(Depth(Cube)), Sampler, V(3)], TR::F32),
            (&[T(Depth(CubeArray)), Sampler, V(3), Int], TR::F32),
        ],
        TextureSampleBias => &[
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), F32], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), F32, Offset(2)], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, F32], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, F32, Offset(2)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), F32], TR::V4F),
            (&[T(Samp(Cube, Some(Scalar::F32))), Sampler, V(3), F32], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), F32, Offset(3)], TR::V4F),
            (&[T(Samp(CubeArray, Some(Scalar::F32))), Sampler, V(3), Int, F32], TR::V4F),
        ],
        TextureSampleCompare | TextureSampleCompareLevel => &[
            (&[T(Depth(D2)), SamplerCmp, V(2), F32], TR::F32),
            (&[T(Depth(D2)), SamplerCmp, V(2), F32, Offset(2)], TR::F32),
            (&[T(Depth(D2Array)), SamplerCmp, V(2), Int, F32], TR::F32),
            (&[T(Depth(D2Array)), SamplerCmp, V(2), Int, F32, Offset(2)], TR::F32),
            (&[T(Depth(Cube)), SamplerCmp, V(3), F32], TR::F32),
            (&[T(Depth(CubeArray)), SamplerCmp, V(3), Int, F32], TR::F32),
        ],
        TextureSampleGrad => &[
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), V(2), V(2)], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), V(2), V(2), Offset(2)], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, V(2), V(2)], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, V(2), V(2), Offset(2)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), V(3), V(3)], TR::V4F),
            (&[T(Samp(Cube, Some(Scalar::F32))), Sampler, V(3), V(3), V(3)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), V(3), V(3), Offset(3)], TR::V4F),
            (&[T(Samp(CubeArray, Some(Scalar::F32))), Sampler, V(3), Int, V(3), V(3)], TR::V4F),
        ],
        TextureSampleLevel => &[
            (&[T(Samp(D1, Some(Scalar::F32))), Sampler, F32, F32], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), F32], TR::V4F),
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2), F32, Offset(2)], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, F32], TR::V4F),
            (&[T(Samp(D2Array, Some(Scalar::F32))), Sampler, V(2), Int, F32, Offset(2)], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), F32], TR::V4F),
            (&[T(Samp(Cube, Some(Scalar::F32))), Sampler, V(3), F32], TR::V4F),
            (&[T(Samp(D3, Some(Scalar::F32))), Sampler, V(3), F32, Offset(3)], TR::V4F),
            (&[T(Samp(CubeArray, Some(Scalar::F32))), Sampler, V(3), Int, F32], TR::V4F),
            (&[T(Depth(D2)), Sampler, V(2), Int], TR::F32),
            (&[T(Depth(D2)), Sampler, V(2), Int, Offset(2)], TR::F32),
            (&[T(Depth(D2Array)), Sampler, V(2), Int, Int], TR::F32),
            (&[T(Depth(D2Array)), Sampler, V(2), Int, Int, Offset(2)], TR::F32),
            (&[T(Depth(Cube)), Sampler, V(3), Int], TR::F32),
            (&[T(Depth(CubeArray)), Sampler, V(3), Int, Int], TR::F32),
        ],
        TextureSampleBaseClampToEdge => &[
            (&[T(Samp(D2, Some(Scalar::F32))), Sampler, V(2)], TR::V4F),
            (&[T(External), Sampler, V(2)], TR::V4F),
        ],
        TextureStore => &[
            (&[T(Storage(D1, false)), Int, Texel], TR::Void),
            (&[T(Storage(D2, false)), IntV(2), Texel], TR::Void),
            (&[T(Storage(D2Array, false)), IntV(2), Int, Texel], TR::Void),
            (&[T(Storage(D3, false)), IntV(3), Texel], TR::Void),
        ],
        _ => &[],
    }
}

pub fn is_texture_fn(b: Builtin) -> bool {
    !texture_overloads(b).is_empty()
}

// ---- constant evaluation

fn map1(t: &Types, v: &Value, ty: TypeId, f: &mut dyn FnMut(&Value) -> CR<Value>) -> CR<Value> {
    if t.is_scalar(ty) {
        return f(v);
    }
    let s = t.scalar_of(ty).unwrap_or(Scalar::F32);
    let cs = comps(t, v, ty);
    let mut out = Vec::with_capacity(cs.len());
    for c in &cs {
        out.push(map1(t, c, s.id(), f)?);
    }
    Ok(Value::Comp(out))
}

fn mapn(t: &Types, vs: &[Value], tys: &[TypeId], f: &mut dyn FnMut(&[Value]) -> CR<Value>) -> CR<Value> {
    let ty = tys[0];
    if t.is_scalar(ty) {
        return f(vs);
    }
    let n = t.width(ty).unwrap_or(1) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let cs: Vec<Value> = vs.iter().zip(tys).map(|(v, &tt)| if t.is_scalar(tt) { v.clone() } else { comp(t, v, tt, i) }).collect();
        out.push(f(&cs)?);
    }
    Ok(Value::Comp(out))
}

fn fl(f: fn(f64) -> f64, v: &Value) -> CR<Value> {
    float_result(f(as_f64(v)), v)
}

fn bits32(v: &Value) -> u32 {
    match *v {
        Value::I32(x) => x as u32,
        Value::U32(x) => x,
        _ => 0,
    }
}

fn from_bits32(like: &Value, b: u32) -> Value {
    match like {
        Value::I32(_) => Value::I32(b as i32),
        _ => Value::U32(b),
    }
}

fn f16_ok(x: f64) -> bool {
    x.is_finite() && x.abs() <= 65504.0
}

/// a float of the same type as `like` from f64, rounding (no range check)
fn same_float(like: &Value, x: f64) -> Value {
    match like {
        Value::F32(_) => Value::F32(x as f32),
        Value::F16(_) => Value::F16(crate::lex::round_f64(x, crate::lex::Fmt::F16).unwrap_or(0.0) as f32),
        _ => Value::AFloat(x),
    }
}

/// see the module comment; args: (value, type) after conversion to the overload's types
pub fn eval(t: &mut Types, b: Builtin, args: &[(Value, TypeId)]) -> CR<Value> {
    use Builtin::*;
    let a0 = &args[0].0;
    let t0 = args[0].1;
    let vals: Vec<Value> = args.iter().map(|a| a.0.clone()).collect();
    let tys: Vec<TypeId> = args.iter().map(|a| a.1).collect();
    let unf = |f: fn(f64) -> f64| move |v: &Value| fl(f, v);
    let t = &*t;
    match b {
        All | Any => {
            let cs = comps(t, a0, t0);
            let bs = cs.iter().map(|c| matches!(c, Value::Bool(true)));
            Ok(Value::Bool(if b == All { bs.fold(true, |a, x| a && x) } else { bs.fold(false, |a, x| a || x) }))
        }
        Select => {
            if t.is_scalar(tys[2]) {
                Ok(if matches!(vals[2], Value::Bool(true)) { vals[1].clone() } else { vals[0].clone() })
            } else {
                mapn(t, &vals, &tys, &mut |c| Ok(if matches!(c[2], Value::Bool(true)) { c[1].clone() } else { c[0].clone() }))
            }
        }
        Abs => map1(t, a0, t0, &mut |v| match *v {
            Value::AInt(x) => x.checked_abs().map(Value::AInt).ok_or_else(|| String::from("Überlauf: abs des kleinsten AbstractInt")),
            Value::I32(x) => Ok(Value::I32(x.wrapping_abs())),
            Value::U32(x) => Ok(Value::U32(x)),
            _ => Ok(same_float(v, as_f64(v).abs())),
        }),
        Acos => map1(t, a0, t0, &mut unf(math::acos)),
        Acosh => map1(t, a0, t0, &mut unf(math::acosh)),
        Asin => map1(t, a0, t0, &mut unf(math::asin)),
        Asinh => map1(t, a0, t0, &mut unf(math::asinh)),
        Atan => map1(t, a0, t0, &mut unf(math::atan)),
        Atanh => map1(t, a0, t0, &mut |v| {
            let x = as_f64(v);
            if x.abs() >= 1.0 {
                return Err(format!("`atanh` ist für {} nicht definiert (|x| < 1)", consteval::fmt(v)));
            }
            fl(math::atanh, v)
        }),
        Atan2 => mapn(t, &vals, &tys, &mut |c| float_result(math::atan2(as_f64(&c[0]), as_f64(&c[1])), &c[0])),
        Ceil => map1(t, a0, t0, &mut unf(math::ceil)),
        Floor => map1(t, a0, t0, &mut unf(math::floor)),
        Trunc => map1(t, a0, t0, &mut unf(math::trunc)),
        Round => map1(t, a0, t0, &mut unf(math::round)),
        Fract => map1(t, a0, t0, &mut |v| {
            // e - floor(e): exact in f64 for f32 / f16 inputs, rounded once (1.0 is a valid
            // result for tiny negative inputs)
            let x = as_f64(v);
            Ok(same_float(v, x - math::floor(x)))
        }),
        Clamp => mapn(t, &vals, &tys, &mut |c| {
            let lt = |a: &Value, b: &Value| matches!(consteval::binary_scalar(crate::ast::BinOp::Lt, a, b), Ok(Value::Bool(true)));
            if lt(&c[2], &c[1]) {
                return Err(format!("`clamp`: die Untergrenze {} ist größer als die Obergrenze {}", consteval::fmt(&c[1]), consteval::fmt(&c[2])));
            }
            let m = if lt(&c[0], &c[1]) { c[1].clone() } else { c[0].clone() };
            Ok(if lt(&c[2], &m) { c[2].clone() } else { m })
        }),
        Max | Min => mapn(t, &vals, &tys, &mut |c| {
            let lt = matches!(consteval::binary_scalar(crate::ast::BinOp::Lt, &c[0], &c[1]), Ok(Value::Bool(true)));
            Ok(if (b == Max) == lt { c[1].clone() } else { c[0].clone() })
        }),
        Cos => map1(t, a0, t0, &mut unf(math::cos)),
        Cosh => map1(t, a0, t0, &mut unf(math::cosh)),
        Sin => map1(t, a0, t0, &mut unf(math::sin)),
        Sinh => map1(t, a0, t0, &mut unf(math::sinh)),
        Tan => map1(t, a0, t0, &mut unf(math::tan)),
        Tanh => map1(t, a0, t0, &mut unf(math::tanh)),
        Exp => map1(t, a0, t0, &mut unf(math::exp)),
        Exp2 => map1(t, a0, t0, &mut unf(math::exp2)),
        Log => map1(t, a0, t0, &mut |v| {
            if as_f64(v) <= 0.0 {
                return Err(format!("`log` ist für {} nicht definiert (x > 0)", consteval::fmt(v)));
            }
            fl(math::log, v)
        }),
        Log2 => map1(t, a0, t0, &mut |v| {
            if as_f64(v) <= 0.0 {
                return Err(format!("`log2` ist für {} nicht definiert (x > 0)", consteval::fmt(v)));
            }
            fl(math::log2, v)
        }),
        Sqrt => map1(t, a0, t0, &mut |v| {
            if as_f64(v) < 0.0 {
                return Err(format!("`sqrt` ist für {} nicht definiert (x >= 0)", consteval::fmt(v)));
            }
            fl(math::sqrt, v)
        }),
        InverseSqrt => map1(t, a0, t0, &mut |v| {
            if as_f64(v) <= 0.0 {
                return Err(format!("`inverseSqrt` ist für {} nicht definiert (x > 0)", consteval::fmt(v)));
            }
            float_result(1.0 / math::sqrt(as_f64(v)), v)
        }),
        Pow => mapn(t, &vals, &tys, &mut |c| float_result(math::pow(as_f64(&c[0]), as_f64(&c[1])), &c[0])),
        Degrees => map1(t, a0, t0, &mut |v| float_result(as_f64(v) * (180.0 / math::PI), v)),
        Radians => map1(t, a0, t0, &mut |v| float_result(as_f64(v) * (math::PI / 180.0), v)),
        Saturate => map1(t, a0, t0, &mut |v| Ok(same_float(v, as_f64(v).clamp(0.0, 1.0)))),
        Sign => map1(t, a0, t0, &mut |v| {
            Ok(match *v {
                Value::AInt(x) => Value::AInt(x.signum()),
                Value::I32(x) => Value::I32(x.signum()),
                _ => {
                    let x = as_f64(v);
                    same_float(v, if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 })
                }
            })
        }),
        Step => mapn(t, &vals, &tys, &mut |c| Ok(same_float(&c[0], if as_f64(&c[1]) >= as_f64(&c[0]) { 1.0 } else { 0.0 }))),
        Smoothstep => mapn(t, &vals, &tys, &mut |c| {
            let (e0, e1, x) = (as_f64(&c[0]), as_f64(&c[1]), as_f64(&c[2]));
            if e0 == e1 {
                return Err(String::from("`smoothstep`: `low` und `high` dürfen nicht gleich sein"));
            }
            let tt = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            float_result(tt * tt * (3.0 - 2.0 * tt), &c[0])
        }),
        Fma => mapn(t, &vals, &tys, &mut |c| float_result(as_f64(&c[0]) * as_f64(&c[1]) + as_f64(&c[2]), &c[0])),
        Mix => mapn(t, &vals, &tys, &mut |c| {
            let (x, y, a) = (as_f64(&c[0]), as_f64(&c[1]), as_f64(&c[2]));
            float_result(x * (1.0 - a) + y * a, &c[0])
        }),
        Dot => {
            let (x, y) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]));
            consteval::dot(&x, &y)
        }
        Length => {
            let cs = comps(t, a0, t0);
            let s: f64 = cs.iter().map(|c| as_f64(c) * as_f64(c)).sum();
            float_result(math::sqrt(s), &cs[0])
        }
        Distance => {
            let (x, y) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]));
            let s: f64 = x.iter().zip(&y).map(|(a, b)| (as_f64(a) - as_f64(b)) * (as_f64(a) - as_f64(b))).sum();
            float_result(math::sqrt(s), &x[0])
        }
        Normalize => {
            let cs = comps(t, a0, t0);
            let l = math::sqrt(cs.iter().map(|c| as_f64(c) * as_f64(c)).sum());
            if l == 0.0 {
                return Err(String::from("`normalize` eines Nullvektors"));
            }
            let mut out = Vec::new();
            for c in &cs {
                out.push(float_result(as_f64(c) / l, c)?);
            }
            Ok(Value::Comp(out))
        }
        Cross => {
            let (a, c) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]));
            let f = |i: usize, j: usize| as_f64(&a[i]) * as_f64(&c[j]) - as_f64(&a[j]) * as_f64(&c[i]);
            Ok(Value::Comp(alloc::vec![float_result(f(1, 2), &a[0])?, float_result(f(2, 0), &a[0])?, float_result(f(0, 1), &a[0])?]))
        }
        FaceForward => {
            let (n, i, r) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]), comps(t, &vals[2], tys[2]));
            let d: f64 = r.iter().zip(&i).map(|(a, b)| as_f64(a) * as_f64(b)).sum();
            let mut out = Vec::new();
            for c in &n {
                out.push(float_result(if d < 0.0 { as_f64(c) } else { -as_f64(c) }, c)?);
            }
            Ok(Value::Comp(out))
        }
        Reflect => {
            let (i, n) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]));
            let d: f64 = n.iter().zip(&i).map(|(a, b)| as_f64(a) * as_f64(b)).sum();
            let mut out = Vec::new();
            for (a, b) in i.iter().zip(&n) {
                out.push(float_result(as_f64(a) - 2.0 * d * as_f64(b), a)?);
            }
            Ok(Value::Comp(out))
        }
        Refract => {
            let (i, n) = (comps(t, &vals[0], tys[0]), comps(t, &vals[1], tys[1]));
            let eta = as_f64(&vals[2]);
            let d: f64 = n.iter().zip(&i).map(|(a, b)| as_f64(a) * as_f64(b)).sum();
            let k = 1.0 - eta * eta * (1.0 - d * d);
            let mut out = Vec::new();
            for (a, b) in i.iter().zip(&n) {
                let r = if k < 0.0 { 0.0 } else { eta * as_f64(a) - (eta * d + math::sqrt(k)) * as_f64(b) };
                out.push(float_result(r, a)?);
            }
            Ok(Value::Comp(out))
        }
        Determinant => {
            let cols = consteval::mat_cols(t, a0, t0);
            let m: Vec<Vec<f64>> = cols.iter().map(|c| c.iter().map(as_f64).collect()).collect();
            let d = det(&m);
            float_result(d, &cols[0][0])
        }
        Transpose => {
            let cols = consteval::mat_cols(t, a0, t0);
            let (c, r) = (cols.len(), cols.first().map(|x| x.len()).unwrap_or(0));
            let out: Vec<Value> = (0..r).map(|i| Value::Comp((0..c).map(|j| cols[j][i].clone()).collect())).collect();
            Ok(Value::Comp(out))
        }
        Ldexp => mapn(t, &vals, &tys, &mut |c| {
            let e = as_int(&c[1]).unwrap_or(0);
            let bias = match c[0] {
                Value::F32(_) => 127,
                Value::F16(_) => 15,
                _ => 1023,
            };
            if e > bias + 1 {
                return Err(format!("`ldexp`: der Exponent {} ist größer als {}", e, bias + 1));
            }
            float_result(math::ldexp(as_f64(&c[0]), e as i64), &c[0])
        }),
        Frexp | Modf => {
            let n = t.width(t0).unwrap_or(1);
            let cs = comps(t, a0, t0);
            let (mut fr, mut wh) = (Vec::new(), Vec::new());
            for c in &cs {
                let x = as_f64(c);
                if b == Frexp {
                    let (f, e) = math::frexp(x);
                    fr.push(same_float(c, f));
                    wh.push(if matches!(c, Value::AFloat(_)) { Value::AInt(e) } else { Value::I32(e as i32) });
                } else {
                    let w = math::trunc(x);
                    fr.push(same_float(c, x - w));
                    wh.push(same_float(c, w));
                }
            }
            let pack = |v: Vec<Value>| if n > 1 { Value::Comp(v) } else { v.into_iter().next().unwrap_or(Value::Zero) };
            Ok(Value::Comp(alloc::vec![pack(fr), pack(wh)]))
        }
        QuantizeToF16 => map1(t, a0, t0, &mut |v| {
            let x = as_f64(v);
            if !f16_ok(x) {
                return Err(format!("`quantizeToF16`: {} liegt außerhalb des Bereichs von `f16`", consteval::fmt(v)));
            }
            Ok(Value::F32(crate::lex::round_f64(x, crate::lex::Fmt::F16).unwrap_or(0.0) as f32))
        }),
        CountLeadingZeros => map1(t, a0, t0, &mut |v| Ok(from_bits32(v, bits32(v).leading_zeros()))),
        CountOneBits => map1(t, a0, t0, &mut |v| Ok(from_bits32(v, bits32(v).count_ones()))),
        CountTrailingZeros => map1(t, a0, t0, &mut |v| Ok(from_bits32(v, bits32(v).trailing_zeros()))),
        ReverseBits => map1(t, a0, t0, &mut |v| Ok(from_bits32(v, bits32(v).reverse_bits()))),
        FirstTrailingBit => map1(t, a0, t0, &mut |v| {
            let x = bits32(v);
            Ok(from_bits32(v, if x == 0 { u32::MAX } else { x.trailing_zeros() }))
        }),
        FirstLeadingBit => map1(t, a0, t0, &mut |v| {
            let x = bits32(v);
            let r = match v {
                Value::I32(s) => {
                    let y = if *s < 0 { !x } else { x };
                    if y == 0 { u32::MAX } else { 31 - y.leading_zeros() }
                }
                _ => {
                    if x == 0 {
                        u32::MAX
                    } else {
                        31 - x.leading_zeros()
                    }
                }
            };
            Ok(from_bits32(v, r))
        }),
        ExtractBits | InsertBits => {
            let (o, c) = if b == ExtractBits { (bits32(&vals[1]), bits32(&vals[2])) } else { (bits32(&vals[2]), bits32(&vals[3])) };
            if o as u64 + c as u64 > 32 {
                return Err(format!("`{}`: `offset` + `count` = {} ist größer als 32", name(b), o as u64 + c as u64));
            }
            let mask: u32 = if c == 0 { 0 } else { (((1u64 << c) - 1) << o) as u32 };
            if b == ExtractBits {
                map1(t, a0, t0, &mut |v| {
                    let x = bits32(v);
                    if c == 0 {
                        return Ok(from_bits32(v, 0));
                    }
                    let mut r = ((x & mask) as u64 >> o) as u32;
                    if matches!(v, Value::I32(_)) && c < 32 && (r >> (c - 1)) & 1 == 1 {
                        r |= !((1u32 << c) - 1);
                    }
                    Ok(from_bits32(v, r))
                })
            } else {
                let sub = [vals[0].clone(), vals[1].clone()];
                mapn(t, &sub, &tys[..2], &mut |cs| {
                    let (e, nb) = (bits32(&cs[0]), bits32(&cs[1]));
                    Ok(from_bits32(&cs[0], (e & !mask) | (((nb as u64) << o) as u32 & mask)))
                })
            }
        }
        Dot4U8Packed | Dot4I8Packed => {
            let (x, y) = (bits32(&vals[0]), bits32(&vals[1]));
            let mut s: i64 = 0;
            for k in 0..4 {
                let (a, c) = ((x >> (8 * k)) & 255, (y >> (8 * k)) & 255);
                s += if b == Dot4I8Packed { (a as u8 as i8 as i64) * (c as u8 as i8 as i64) } else { a as i64 * c as i64 };
            }
            Ok(if b == Dot4I8Packed { Value::I32(s as i32) } else { Value::U32(s as u32) })
        }
        Pack4x8snorm | Pack4x8unorm | Pack2x16snorm | Pack2x16unorm => {
            let cs = comps(t, a0, t0);
            let (bits, snorm) = match b {
                Pack4x8snorm => (8, true),
                Pack4x8unorm => (8, false),
                Pack2x16snorm => (16, true),
                _ => (16, false),
            };
            let mut r: u32 = 0;
            for (k, c) in cs.iter().enumerate() {
                let x = as_f64(c);
                let q = if snorm {
                    let m = ((1u32 << (bits - 1)) - 1) as f64;
                    (math::floor(0.5 + m * x.clamp(-1.0, 1.0)) as i32) as u32 & ((1u32 << bits) - 1)
                } else {
                    let m = ((1u64 << bits) - 1) as f64;
                    math::floor(0.5 + m * x.clamp(0.0, 1.0)) as u32
                };
                r |= q << (bits * k as u32);
            }
            Ok(Value::U32(r))
        }
        Pack4xI8 | Pack4xU8 | Pack4xI8Clamp | Pack4xU8Clamp => {
            let cs = comps(t, a0, t0);
            let mut r: u32 = 0;
            for (k, c) in cs.iter().enumerate() {
                let q = match (b, c) {
                    (Pack4xI8, Value::I32(x)) => *x as u32 & 255,
                    (Pack4xU8, Value::U32(x)) => *x & 255,
                    (Pack4xI8Clamp, Value::I32(x)) => (*x).clamp(-128, 127) as u32 & 255,
                    (Pack4xU8Clamp, Value::U32(x)) => (*x).min(255),
                    _ => 0,
                };
                r |= q << (8 * k);
            }
            Ok(Value::U32(r))
        }
        Pack2x16float => {
            let cs = comps(t, a0, t0);
            let mut r: u32 = 0;
            for (k, c) in cs.iter().enumerate() {
                let x = as_f64(c);
                if !f16_ok(x) {
                    return Err(format!("`pack2x16float`: {} liegt außerhalb des Bereichs von `f16`", consteval::fmt(c)));
                }
                let h = crate::lex::round_f64(x, crate::lex::Fmt::F16).unwrap_or(0.0) as f32;
                r |= (f16_bits(h) as u32) << (16 * k);
            }
            Ok(Value::U32(r))
        }
        Unpack4x8snorm | Unpack4x8unorm | Unpack2x16snorm | Unpack2x16unorm => {
            let x = bits32(a0);
            let (bits, n, snorm) = match b {
                Unpack4x8snorm => (8, 4, true),
                Unpack4x8unorm => (8, 4, false),
                Unpack2x16snorm => (16, 2, true),
                _ => (16, 2, false),
            };
            let mut out = Vec::new();
            for k in 0..n {
                let raw = (x >> (bits * k)) & ((1u32 << bits) - 1);
                let v = if snorm {
                    let s = ((raw << (32 - bits)) as i32 >> (32 - bits)) as f64;
                    (s / ((1u32 << (bits - 1)) - 1) as f64).max(-1.0)
                } else {
                    raw as f64 / ((1u64 << bits) - 1) as f64
                };
                out.push(Value::F32(v as f32));
            }
            Ok(Value::Comp(out))
        }
        Unpack4xI8 | Unpack4xU8 => {
            let x = bits32(a0);
            let out = (0..4)
                .map(|k| {
                    let byte = (x >> (8 * k)) & 255;
                    if b == Unpack4xI8 { Value::I32(byte as u8 as i8 as i32) } else { Value::U32(byte) }
                })
                .collect();
            Ok(Value::Comp(out))
        }
        Unpack2x16float => {
            let x = bits32(a0);
            let out = (0..2).map(|k| Value::F32(f16_from_bits(((x >> (16 * k)) & 0xffff) as u16))).collect();
            Ok(Value::Comp(out))
        }
        _ => Err(format!("{} ist keine const-Funktion", name(b))),
    }
}

fn det(m: &[Vec<f64>]) -> f64 {
    match m.len() {
        2 => m[0][0] * m[1][1] - m[1][0] * m[0][1],
        3 => {
            m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
                + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2])
        }
        _ => {
            let mut d = 0.0;
            for c in 0..4 {
                let minor: Vec<Vec<f64>> = (0..4).filter(|&j| j != c).map(|j| (1..4).map(|i| m[j][i]).collect()).collect();
                let s = if c % 2 == 0 { 1.0 } else { -1.0 };
                d += s * m[c][0] * det(&minor);
            }
            d
        }
    }
}

/// an f16 bit pattern as f32
pub fn f16_from_bits(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 31) as i32;
    let m = (h & 1023) as f32;
    let v = if e == 0 {
        m * (1.0 / 16_777_216.0)
    } else if e == 31 {
        f32::INFINITY
    } else {
        (1.0 + m / 1024.0) * f32::from_bits(((e - 15 + 127) as u32) << 23)
    };
    sign * v
}
