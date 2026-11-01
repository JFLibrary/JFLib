// spirv_builtin -- the built-in functions of WGSL (spec section 17) in SPIR-V (spirv_func.rs
// calls builtin() for every ExprKind::Builtin)
//
// Most numeric functions are GLSL.std.450 instructions.  Where WGSL defines more than SPIR-V:
//   min / max / clamp / saturate on floats: NMin / NMax (the other operand when one is NaN, as
//        WGSL says); clamp = min(max(e, low), high), also for low > high (GLSL's clamp is
//        undefined there)
//   extractBits / insertBits: offset and count clamped (o = min(offset, w), c = min(count, w - o))
//   countLeadingZeros = 31 - FindUMsb (32 for 0), countTrailingZeros = min(FindILsb, 32)
//   dot4I8Packed / dot4U8Packed, pack4xI8 ... unpack4xU8, integer dot: written out
//   round = RoundEven; atomics relaxed, Device scope (Workgroup for workgroup memory)
//   workgroupUniformLoad = barrier, load, barrier
// Textures (robust): textureLoad clamps level, coordinates, array index and sample index to
// what the texture has (OpImageQuery*); textureStore skips out-of-bounds texels;
// textureDimensions clamps the level; textureSampleBaseClampToEdge clamps to half a texel.

use alloc::vec::Vec;

use shader::{op, spv, Cap, Id, Sc};

use crate::ir::*;
use crate::spirv_func::{Fx, V};

mod g {
    // GLSL.std.450 numbers lib/shader's glsl module does not name
    pub const RADIANS: u32 = 11;
    pub const DEGREES: u32 = 12;
    pub const ASIN: u32 = 16;
    pub const ACOS: u32 = 17;
    pub const ATAN: u32 = 18;
    pub const SINH: u32 = 19;
    pub const COSH: u32 = 20;
    pub const TANH: u32 = 21;
    pub const ASINH: u32 = 22;
    pub const ACOSH: u32 = 23;
    pub const ATANH: u32 = 24;
    pub const DETERMINANT: u32 = 33;
    pub const MODF_STRUCT: u32 = 36;
    pub const FREXP_STRUCT: u32 = 52;
    pub const LDEXP: u32 = 53;
    pub const PACK_HALF2X16: u32 = 58;
    pub const UNPACK_SNORM2X16: u32 = 60;
    pub const UNPACK_UNORM2X16: u32 = 61;
    pub const UNPACK_HALF2X16: u32 = 62;
    pub const UNPACK_SNORM4X8: u32 = 63;
    pub const UNPACK_UNORM4X8: u32 = 64;
    pub const FACE_FORWARD: u32 = 70;
    pub const REFRACT: u32 = 72;
    pub const FIND_ILSB: u32 = 73;
    pub const FIND_SMSB: u32 = 74;
    pub const FIND_UMSB: u32 = 75;
    pub const NMIN: u32 = 79;
    pub const NMAX: u32 = 80;
}

/// a GLSL.std.450 instruction with the result type of its first argument
fn gl(fx: &mut Fx, inst: u32, args: &[Id]) -> Id {
    fx.g.b.glsl(inst, args)
}

fn min_max(fx: &mut Fx, max: bool, a: Id, b: Id, t: TypeId) -> Id {
    let inst = match (fx.scalar(t), max) {
        (Scalar::I32, false) => spv::glsl::SMin,
        (Scalar::I32, true) => spv::glsl::SMax,
        (Scalar::U32, false) => spv::glsl::UMin,
        (Scalar::U32, true) => spv::glsl::UMax,
        (_, false) => g::NMIN,
        (_, true) => g::NMAX,
    };
    gl(fx, inst, &[a, b])
}

/// the u32 type of n components
fn uvec(fx: &mut Fx, n: u8) -> Id {
    let u = fx.g.b.ty_u32();
    if n == 1 { u } else { fx.g.b.ty_vec(u, n as u32) }
}

/// an integer value (scalar or vector) as u32 components
fn as_u32(fx: &mut Fx, v: Id, t: TypeId) -> Id {
    if fx.scalar(t) == Scalar::I32 {
        let n = fx.width(t);
        let ut = uvec(fx, n);
        return fx.g.b.bitcast(ut, v);
    }
    v
}

/// the lane or quad index of a broadcast: SPIR-V wants an unsigned constant, WGSL allows i32
fn const_index(fx: &mut Fx, e: ExprId) -> Id {
    let idx = match fx.exprs.get(e as usize).map(|x| &x.kind) {
        Some(ExprKind::Const(v)) => crate::consteval::as_int(v).unwrap_or(0) as u32,
        _ => 0,
    };
    fx.g.b.const_u32(idx)
}

/// a subgroup or quad function: OpGroupNonUniform*, scope Subgroup
fn subgroup(fx: &mut Fx, b: Builtin, args: &[ExprId], ty: TypeId) -> V {
    use Builtin::*;
    use spv::group_op::{ExclusiveScan, InclusiveScan, Reduce};
    let a: Vec<Id> = args.iter().map(|&x| fx.val(x)).collect();
    let rt = fx.sty(ty);
    let scope = fx.g.b.const_u32(spv::scope::Subgroup);
    fx.g.b.capability(Cap::GroupNonUniform);
    let t0 = args.first().map(|&x| fx.tyof(x)).unwrap_or(ty);
    let s = fx.scalar(t0);
    if s == Scalar::F16 {
        fx.g.needs.subgroup_extended_types = true;
    }
    // the arithmetic and bit operations: (opcode, group operation)
    let arith = |int: u16, sint: u16, float: u16| -> u16 {
        match s {
            Scalar::I32 => sint,
            Scalar::U32 => int,
            _ => float,
        }
    };
    let reduction = match b {
        SubgroupAdd => Some((arith(op::GroupNonUniformIAdd, op::GroupNonUniformIAdd, op::GroupNonUniformFAdd), Reduce)),
        SubgroupExclusiveAdd => Some((arith(op::GroupNonUniformIAdd, op::GroupNonUniformIAdd, op::GroupNonUniformFAdd), ExclusiveScan)),
        SubgroupInclusiveAdd => Some((arith(op::GroupNonUniformIAdd, op::GroupNonUniformIAdd, op::GroupNonUniformFAdd), InclusiveScan)),
        SubgroupMul => Some((arith(op::GroupNonUniformIMul, op::GroupNonUniformIMul, op::GroupNonUniformFMul), Reduce)),
        SubgroupExclusiveMul => Some((arith(op::GroupNonUniformIMul, op::GroupNonUniformIMul, op::GroupNonUniformFMul), ExclusiveScan)),
        SubgroupInclusiveMul => Some((arith(op::GroupNonUniformIMul, op::GroupNonUniformIMul, op::GroupNonUniformFMul), InclusiveScan)),
        SubgroupMin => Some((arith(op::GroupNonUniformUMin, op::GroupNonUniformSMin, op::GroupNonUniformFMin), Reduce)),
        SubgroupMax => Some((arith(op::GroupNonUniformUMax, op::GroupNonUniformSMax, op::GroupNonUniformFMax), Reduce)),
        SubgroupAnd => Some((op::GroupNonUniformBitwiseAnd, Reduce)),
        SubgroupOr => Some((op::GroupNonUniformBitwiseOr, Reduce)),
        SubgroupXor => Some((op::GroupNonUniformBitwiseXor, Reduce)),
        _ => None,
    };
    if let Some((opc, gop)) = reduction {
        fx.g.b.capability(Cap::GroupNonUniformArithmetic);
        fx.g.needs.subgroup_ops |= 4;
        return V::Id(fx.g.b.emit(opc, Some(rt), &[scope.0, gop, a[0].0]));
    }
    fx.g.needs.subgroup_ops |= 1;
    let r = match b {
        SubgroupElect => fx.g.b.emit(op::GroupNonUniformElect, Some(rt), &[scope.0]),
        SubgroupAll | SubgroupAny => {
            fx.g.b.capability(Cap::GroupNonUniformVote);
            fx.g.needs.subgroup_ops |= 2;
            let opc = if b == SubgroupAll { op::GroupNonUniformAll } else { op::GroupNonUniformAny };
            fx.g.b.emit(opc, Some(rt), &[scope.0, a[0].0])
        }
        SubgroupBallot | SubgroupBroadcastFirst => {
            fx.g.b.capability(Cap::GroupNonUniformBallot);
            fx.g.needs.subgroup_ops |= 8;
            let opc = if b == SubgroupBallot { op::GroupNonUniformBallot } else { op::GroupNonUniformBroadcastFirst };
            fx.g.b.emit(opc, Some(rt), &[scope.0, a[0].0])
        }
        SubgroupBroadcast => {
            // (before SPIR-V 1.5 the id must be a constant: it is, the front end checks it)
            fx.g.b.capability(Cap::GroupNonUniformBallot);
            fx.g.needs.subgroup_ops |= 8;
            let id = const_index(fx, args[1]);
            fx.g.b.emit(op::GroupNonUniformBroadcast, Some(rt), &[scope.0, a[0].0, id.0])
        }
        SubgroupShuffle | SubgroupShuffleXor => {
            fx.g.b.capability(Cap::GroupNonUniformShuffle);
            fx.g.needs.subgroup_ops |= 16;
            let id = as_u32(fx, a[1], fx.tyof(args[1]));
            let opc = if b == SubgroupShuffle { op::GroupNonUniformShuffle } else { op::GroupNonUniformShuffleXor };
            fx.g.b.emit(opc, Some(rt), &[scope.0, a[0].0, id.0])
        }
        SubgroupShuffleUp | SubgroupShuffleDown => {
            fx.g.b.capability(Cap::GroupNonUniformShuffleRelative);
            fx.g.needs.subgroup_ops |= 32;
            let opc = if b == SubgroupShuffleUp { op::GroupNonUniformShuffleUp } else { op::GroupNonUniformShuffleDown };
            fx.g.b.emit(opc, Some(rt), &[scope.0, a[0].0, a[1].0])
        }
        QuadBroadcast => {
            fx.g.b.capability(Cap::GroupNonUniformQuad);
            fx.g.needs.subgroup_ops |= 128;
            let c = const_index(fx, args[1]);
            fx.g.b.emit(op::GroupNonUniformQuadBroadcast, Some(rt), &[scope.0, a[0].0, c.0])
        }
        QuadSwapX | QuadSwapY | QuadSwapDiagonal => {
            fx.g.b.capability(Cap::GroupNonUniformQuad);
            fx.g.needs.subgroup_ops |= 128;
            let dir = match b {
                QuadSwapX => 0,
                QuadSwapY => 1,
                _ => 2,
            };
            let c = fx.g.b.const_u32(dir);
            fx.g.b.emit(op::GroupNonUniformQuadSwap, Some(rt), &[scope.0, a[0].0, c.0])
        }
        _ => return V::None,
    };
    V::Id(r)
}

pub(crate) fn builtin(fx: &mut Fx, b: Builtin, args: &[ExprId], ty: TypeId) -> V {
    use Builtin::*;
    if crate::builtins::is_texture_fn(b) {
        return texture(fx, b, args);
    }
    match b {
        AtomicLoad | AtomicStore | AtomicAdd | AtomicSub | AtomicMax | AtomicMin | AtomicAnd | AtomicOr | AtomicXor
        | AtomicExchange | AtomicCompareExchangeWeak | AtomicStoreMin | AtomicStoreMax => return atomic(fx, b, args, ty),
        WorkgroupBarrier => {
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::WorkgroupMemory);
            return V::None;
        }
        StorageBarrier => {
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::UniformMemory);
            return V::None;
        }
        TextureBarrier => {
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::ImageMemory);
            return V::None;
        }
        WorkgroupUniformLoad => {
            let p = match fx.expr(args[0]) {
                V::Ptr(p) => p,
                _ => return V::None,
            };
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::WorkgroupMemory);
            let v = if let Type::Atomic(_) = *fx.m().ty(p.ty) {
                let ptr = fx.ptr_id(&p);
                fx.g.b.atomic_load(ptr, spv::scope::Workgroup, 0)
            } else {
                fx.load_ptr(&p)
            };
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::WorkgroupMemory);
            return V::Id(v);
        }
        ArrayLength => {
            let p = match fx.expr(args[0]) {
                V::Ptr(p) => p,
                _ => return V::None,
            };
            return V::Id(fx.array_length(&p));
        }
        _ if crate::builtins::feature(b) == crate::feature::SUBGROUPS => return subgroup(fx, b, args, ty),
        _ => {}
    }
    let t0 = fx.tyof(args[0]);
    let a: Vec<Id> = args.iter().map(|&x| fx.val(x)).collect();
    let s = fx.scalar(t0);
    let float = s.is_float();
    let rt = fx.sty(ty);
    let r = match b {
        All | Any => {
            if fx.width(t0) == 1 {
                a[0]
            } else if b == All {
                fx.g.b.all(a[0])
            } else {
                fx.g.b.any(a[0])
            }
        }
        Select => {
            // select(f, t, cond): a vector of conditions for vector values
            let c = if fx.width(fx.tyof(args[2])) == 1 && fx.width(t0) > 1 {
                let bt = fx.bool_t(t0);
                let bty = fx.sty(bt);
                let n = fx.width(t0) as usize;
                fx.g.b.construct(bty, &alloc::vec![a[2]; n])
            } else {
                a[2]
            };
            fx.g.b.select(c, a[1], a[0])
        }
        Abs => match s {
            Scalar::U32 => a[0],
            Scalar::I32 => gl(fx, spv::glsl::SAbs, &[a[0]]),
            _ => gl(fx, spv::glsl::FAbs, &[a[0]]),
        },
        Acos => gl(fx, g::ACOS, &[a[0]]),
        Acosh => gl(fx, g::ACOSH, &[a[0]]),
        Asin => gl(fx, g::ASIN, &[a[0]]),
        Asinh => gl(fx, g::ASINH, &[a[0]]),
        Atan => gl(fx, g::ATAN, &[a[0]]),
        Atanh => gl(fx, g::ATANH, &[a[0]]),
        Atan2 => gl(fx, spv::glsl::Atan2, &[a[0], a[1]]),
        Ceil => gl(fx, spv::glsl::Ceil, &[a[0]]),
        Floor => gl(fx, spv::glsl::Floor, &[a[0]]),
        Trunc => gl(fx, spv::glsl::Trunc, &[a[0]]),
        Round => gl(fx, spv::glsl::RoundEven, &[a[0]]),
        Fract => gl(fx, spv::glsl::Fract, &[a[0]]),
        Cos => gl(fx, spv::glsl::Cos, &[a[0]]),
        Cosh => gl(fx, g::COSH, &[a[0]]),
        Sin => gl(fx, spv::glsl::Sin, &[a[0]]),
        Sinh => gl(fx, g::SINH, &[a[0]]),
        Tan => gl(fx, spv::glsl::Tan, &[a[0]]),
        Tanh => gl(fx, g::TANH, &[a[0]]),
        Exp => gl(fx, spv::glsl::Exp, &[a[0]]),
        Exp2 => gl(fx, spv::glsl::Exp2, &[a[0]]),
        Log => gl(fx, spv::glsl::Log, &[a[0]]),
        Log2 => gl(fx, spv::glsl::Log2, &[a[0]]),
        Pow => gl(fx, spv::glsl::Pow, &[a[0], a[1]]),
        Sqrt => gl(fx, spv::glsl::Sqrt, &[a[0]]),
        InverseSqrt => gl(fx, spv::glsl::InverseSqrt, &[a[0]]),
        Degrees => gl(fx, g::DEGREES, &[a[0]]),
        Radians => gl(fx, g::RADIANS, &[a[0]]),
        Fma => gl(fx, spv::glsl::Fma, &[a[0], a[1], a[2]]),
        Sign => {
            if float {
                gl(fx, spv::glsl::FSign, &[a[0]])
            } else {
                gl(fx, spv::glsl::SSign, &[a[0]])
            }
        }
        Step => gl(fx, spv::glsl::Step, &[a[0], a[1]]),
        Smoothstep => gl(fx, spv::glsl::SmoothStep, &[a[0], a[1], a[2]]),
        Mix => {
            let w = if fx.width(fx.tyof(args[2])) == 1 && fx.width(t0) > 1 { fx.splat_v(a[2], t0) } else { a[2] };
            gl(fx, spv::glsl::FMix, &[a[0], a[1], w])
        }
        Clamp => {
            let x = min_max(fx, true, a[0], a[1], t0);
            min_max(fx, false, x, a[2], t0)
        }
        Max => min_max(fx, true, a[0], a[1], t0),
        Min => min_max(fx, false, a[0], a[1], t0),
        Saturate => {
            let (z, o) = (fx.num(t0, 0.0), fx.num(t0, 1.0));
            let x = gl(fx, g::NMAX, &[a[0], z]);
            gl(fx, g::NMIN, &[x, o])
        }
        Cross => gl(fx, spv::glsl::Cross, &[a[0], a[1]]),
        Distance => {
            let st = fx.sty(ty);
            fx.g.b.ext(st, spv::glsl::Distance, &[a[0], a[1]])
        }
        Length => {
            let st = fx.sty(ty);
            fx.g.b.ext(st, spv::glsl::Length, &[a[0]])
        }
        Normalize => gl(fx, spv::glsl::Normalize, &[a[0]]),
        Reflect => gl(fx, spv::glsl::Reflect, &[a[0], a[1]]),
        Refract => gl(fx, g::REFRACT, &[a[0], a[1], a[2]]),
        FaceForward => gl(fx, g::FACE_FORWARD, &[a[0], a[1], a[2]]),
        Dot => {
            if float {
                fx.g.b.dot(a[0], a[1])
            } else {
                let p = fx.g.b.imul(a[0], a[1]);
                let n = fx.width(t0);
                let mut acc = fx.g.b.extract(p, &[0]);
                for k in 1..n as u32 {
                    let x = fx.g.b.extract(p, &[k]);
                    acc = fx.g.b.iadd(acc, x);
                }
                acc
            }
        }
        Determinant => fx.g.b.ext(rt, g::DETERMINANT, &[a[0]]),
        Transpose => fx.g.b.transpose(a[0]),
        CountOneBits => fx.g.b.unary(op::BitCount, a[0]),
        ReverseBits => fx.g.b.unary(op::BitReverse, a[0]),
        CountLeadingZeros => {
            let n = fx.width(t0);
            let ut = uvec(fx, n);
            let u = as_u32(fx, a[0], t0);
            let msb = fx.g.b.ext(ut, g::FIND_UMSB, &[u]);
            let c31 = fx.num(fx.g.vec_tid(n, Scalar::U32), 31.0);
            let r = fx.g.b.isub(c31, msb);
            if s == Scalar::I32 { fx.g.b.bitcast(rt, r) } else { r }
        }
        CountTrailingZeros => {
            let n = fx.width(t0);
            let ut = uvec(fx, n);
            let u = as_u32(fx, a[0], t0);
            let lsb = fx.g.b.ext(ut, g::FIND_ILSB, &[u]);
            let c32 = fx.num(fx.g.vec_tid(n, Scalar::U32), 32.0);
            let r = fx.g.b.ext(ut, spv::glsl::UMin, &[lsb, c32]);
            if s == Scalar::I32 { fx.g.b.bitcast(rt, r) } else { r }
        }
        FirstLeadingBit => {
            if s == Scalar::I32 {
                fx.g.b.ext(rt, g::FIND_SMSB, &[a[0]])
            } else {
                fx.g.b.ext(rt, g::FIND_UMSB, &[a[0]])
            }
        }
        FirstTrailingBit => fx.g.b.ext(rt, g::FIND_ILSB, &[a[0]]),
        ExtractBits => {
            let (o, c) = bit_range(fx, a[1], a[2]);
            let opc = if s == Scalar::I32 { op::BitFieldSExtract } else { op::BitFieldUExtract };
            fx.g.b.emit(opc, Some(rt), &[a[0].0, o.0, c.0])
        }
        InsertBits => {
            let (o, c) = bit_range(fx, a[2], a[3]);
            fx.g.b.emit(op::BitFieldInsert, Some(rt), &[a[0].0, a[1].0, o.0, c.0])
        }
        Dot4U8Packed | Dot4I8Packed => {
            let signed = b == Dot4I8Packed;
            let it = if signed { fx.g.b.ty_i32() } else { fx.g.b.ty_u32() };
            let (x, y) = if signed { (fx.g.b.bitcast(it, a[0]), fx.g.b.bitcast(it, a[1])) } else { (a[0], a[1]) };
            let opc = if signed { op::BitFieldSExtract } else { op::BitFieldUExtract };
            let eight = fx.cu(8);
            let mut acc: Option<Id> = None;
            for k in 0..4u32 {
                let off = fx.cu(8 * k);
                let bx = fx.g.b.emit(opc, Some(it), &[x.0, off.0, eight.0]);
                let by = fx.g.b.emit(opc, Some(it), &[y.0, off.0, eight.0]);
                let p = fx.g.b.imul(bx, by);
                acc = Some(match acc {
                    None => p,
                    Some(s) => fx.g.b.iadd(s, p),
                });
            }
            acc.unwrap_or(a[0])
        }
        Frexp => fx.g.b.ext(rt, g::FREXP_STRUCT, &[a[0]]),
        Modf => fx.g.b.ext(rt, g::MODF_STRUCT, &[a[0]]),
        Ldexp => fx.g.b.ext(rt, g::LDEXP, &[a[0], a[1]]),
        QuantizeToF16 => fx.g.b.unary(op::QuantizeToF16, a[0]),
        // WGSL: floor(0.5 + k * clamp(e, lo, 1)) per component (GLSL's packSnorm4x8 & co.
        // round ties away from zero on some drivers)
        Pack4x8snorm => pack_norm(fx, a[0], t0, 4, true),
        Pack4x8unorm => pack_norm(fx, a[0], t0, 4, false),
        Pack2x16snorm => pack_norm(fx, a[0], t0, 2, true),
        Pack2x16unorm => pack_norm(fx, a[0], t0, 2, false),
        Pack2x16float => fx.g.b.ext(rt, g::PACK_HALF2X16, &[a[0]]),
        Unpack4x8snorm => fx.g.b.ext(rt, g::UNPACK_SNORM4X8, &[a[0]]),
        Unpack4x8unorm => fx.g.b.ext(rt, g::UNPACK_UNORM4X8, &[a[0]]),
        Unpack2x16snorm => fx.g.b.ext(rt, g::UNPACK_SNORM2X16, &[a[0]]),
        Unpack2x16unorm => fx.g.b.ext(rt, g::UNPACK_UNORM2X16, &[a[0]]),
        Unpack2x16float => fx.g.b.ext(rt, g::UNPACK_HALF2X16, &[a[0]]),
        Pack4xI8 | Pack4xU8 | Pack4xI8Clamp | Pack4xU8Clamp => {
            let mut v = a[0];
            if b == Pack4xI8Clamp {
                let (lo, hi) = (fx.num(t0, -128.0), fx.num(t0, 127.0));
                let x = gl(fx, spv::glsl::SMax, &[v, lo]);
                v = gl(fx, spv::glsl::SMin, &[x, hi]);
            } else if b == Pack4xU8Clamp {
                let hi = fx.num(t0, 255.0);
                v = gl(fx, spv::glsl::UMin, &[v, hi]);
            }
            let u = as_u32(fx, v, t0);
            let ff = fx.cu(255);
            let mut acc: Option<Id> = None;
            for k in 0..4u32 {
                let c = fx.g.b.extract(u, &[k]);
                let c = fx.g.b.band(c, ff);
                let sh = fx.cu(8 * k);
                let c = if k == 0 { c } else { fx.g.b.shl(c, sh) };
                acc = Some(match acc {
                    None => c,
                    Some(s) => fx.g.b.bor(s, c),
                });
            }
            acc.unwrap_or(a[0])
        }
        Unpack4xI8 | Unpack4xU8 => {
            let signed = b == Unpack4xI8;
            let it = if signed { fx.g.b.ty_i32() } else { fx.g.b.ty_u32() };
            let x = if signed { fx.g.b.bitcast(it, a[0]) } else { a[0] };
            let opc = if signed { op::BitFieldSExtract } else { op::BitFieldUExtract };
            let eight = fx.cu(8);
            let parts: Vec<Id> = (0..4u32).map(|k| {
                let off = fx.cu(8 * k);
                fx.g.b.emit(opc, Some(it), &[x.0, off.0, eight.0])
            }).collect();
            fx.g.b.construct(rt, &parts)
        }
        Dpdx => fx.g.b.deriv(op::DPdx, a[0]),
        DpdxCoarse => fx.g.b.deriv(op::DPdxCoarse, a[0]),
        DpdxFine => fx.g.b.deriv(op::DPdxFine, a[0]),
        Dpdy => fx.g.b.deriv(op::DPdy, a[0]),
        DpdyCoarse => fx.g.b.deriv(op::DPdyCoarse, a[0]),
        DpdyFine => fx.g.b.deriv(op::DPdyFine, a[0]),
        Fwidth => fx.g.b.deriv(op::Fwidth, a[0]),
        FwidthCoarse => fx.g.b.deriv(op::FwidthCoarse, a[0]),
        FwidthFine => fx.g.b.deriv(op::FwidthFine, a[0]),
        _ => a[0],
    };
    V::Id(r)
}

/// pack4x8snorm / unorm, pack2x16snorm / unorm as WGSL defines them: component i becomes
/// floor(0.5 + k * clamp(e[i], -1 or 0, 1)) (k = 127, 255, 32767 or 65535) in bits n * i ..
fn pack_norm(fx: &mut Fx, v: Id, t: TypeId, n: u32, signed: bool) -> Id {
    let bits = 32 / n;
    let k = ((1u64 << (bits - signed as u32)) - 1) as f64;
    let lo = fx.num(t, if signed { -1.0 } else { 0.0 });
    let hi = fx.num(t, 1.0);
    let x = gl(fx, g::NMAX, &[v, lo]);
    let x = gl(fx, g::NMIN, &[x, hi]);
    let kv = fx.num(t, k);
    let x = fx.g.b.fmul(x, kv);
    let half = fx.num(t, 0.5);
    let x = fx.g.b.fadd(x, half);
    let x = gl(fx, spv::glsl::Floor, &[x]);
    let it = fx.g.vec_tid(n as u8, Scalar::I32);
    let iv = fx.sty(it);
    let x = fx.g.b.f2s(iv, x);
    let ut = uvec(fx, n as u8);
    let x = fx.g.b.bitcast(ut, x);
    let mask = fx.cu(((1u64 << bits) - 1) as u32);
    let mut acc: Option<Id> = None;
    for i in 0..n {
        let c = fx.g.b.extract(x, &[i]);
        let c = fx.g.b.band(c, mask);
        let c = if i == 0 {
            c
        } else {
            let sh = fx.cu(bits * i);
            fx.g.b.shl(c, sh)
        };
        acc = Some(match acc {
            None => c,
            Some(s) => fx.g.b.bor(s, c),
        });
    }
    acc.unwrap_or(v)
}

/// extractBits / insertBits: o = min(offset, 32), c = min(count, 32 - o)
fn bit_range(fx: &mut Fx, offset: Id, count: Id) -> (Id, Id) {
    let c32 = fx.cu(32);
    let u = fx.g.b.ty_u32();
    let o = fx.g.b.ext(u, spv::glsl::UMin, &[offset, c32]);
    let rest = fx.g.b.isub(c32, o);
    let c = fx.g.b.ext(u, spv::glsl::UMin, &[count, rest]);
    (o, c)
}

fn atomic(fx: &mut Fx, b: Builtin, args: &[ExprId], ty: TypeId) -> V {
    use Builtin::*;
    let p = match fx.expr(args[0]) {
        V::Ptr(p) => p,
        _ => return V::None,
    };
    let scope = if p.sc == Sc::Workgroup { spv::scope::Workgroup } else { spv::scope::Device };
    let inner = match *fx.m().ty(p.ty) {
        Type::Atomic(i) => i,
        _ => p.ty,
    };
    let s = fx.scalar(inner);
    let vals: Vec<Id> = args[1..].iter().map(|&x| fx.val(x)).collect();
    let storage = p.sc == Sc::StorageBuffer;
    let ptr = fx.ptr_id(&p);
    match b {
        AtomicLoad => V::Id(fx.g.b.atomic_load(ptr, scope, 0)),
        AtomicStore => {
            let v = vals[0];
            if storage {
                fx.guarded(|fx| fx.g.b.atomic_store(ptr, scope, 0, v));
            } else {
                fx.g.b.atomic_store(ptr, scope, 0, v);
            }
            V::None
        }
        AtomicStoreMin | AtomicStoreMax => {
            let u64t = fx.g.b.ty_int(64, false);
            let v = fx.g.b.bitcast(u64t, vals[0]);
            let opc = if b == AtomicStoreMin { op::AtomicUMin } else { op::AtomicUMax };
            fx.guarded(|fx| {
                fx.g.b.atomic(opc, ptr, scope, 0, v);
            });
            V::None
        }
        AtomicCompareExchangeWeak => {
            let (cmp, v) = (vals[0], vals[1]);
            let st = fx.sty(inner);
            let old = if storage {
                fx.guarded_value(st, |fx| fx.g.b.atomic_cas(ptr, scope, 0, 0, v, cmp))
            } else {
                fx.g.b.atomic_cas(ptr, scope, 0, 0, v, cmp)
            };
            let ex = fx.g.b.ieq(old, cmp);
            let rt = fx.sty(ty);
            V::Id(fx.g.b.construct(rt, &[old, ex]))
        }
        _ => {
            let opc = match (b, s) {
                (AtomicAdd, _) => op::AtomicIAdd,
                (AtomicSub, _) => op::AtomicISub,
                (AtomicMax, Scalar::I32) => op::AtomicSMax,
                (AtomicMax, _) => op::AtomicUMax,
                (AtomicMin, Scalar::I32) => op::AtomicSMin,
                (AtomicMin, _) => op::AtomicUMin,
                (AtomicAnd, _) => op::AtomicAnd,
                (AtomicOr, _) => op::AtomicOr,
                (AtomicXor, _) => op::AtomicXor,
                _ => op::AtomicExchange,
            };
            let v = vals[0];
            let st = fx.sty(inner);
            let r = if storage {
                fx.guarded_value(st, |fx| fx.g.b.atomic(opc, ptr, scope, 0, v))
            } else {
                fx.g.b.atomic(opc, ptr, scope, 0, v)
            };
            V::Id(r)
        }
    }
}

// ---------------------------------------------------------------- textures

/// the texture's dimension and kind
fn tex_type(fx: &Fx, g: u32) -> (TexDim, TexKind) {
    match *fx.m().ty(fx.m().globals[g as usize].ty) {
        Type::Texture(d, k) => (d, k),
        _ => (TexDim::D2, TexKind::Sampled(Scalar::F32)),
    }
}

/// the size query of an image: n = coordinate components + 1 when arrayed (u32 components)
fn query_size(fx: &mut Fx, img: Id, dim: TexDim, kind: TexKind, level: Option<Id>) -> (Id, u8) {
    let n = match dim {
        TexDim::D1 => 1,
        TexDim::D2 | TexDim::Cube => 2,
        TexDim::D2Array | TexDim::CubeArray => 3,
        TexDim::D3 => 3,
    };
    let ut = uvec(fx, n);
    let lod = matches!(kind, TexKind::Sampled(_) | TexKind::Depth | TexKind::External);
    let v = if lod {
        let l = match level {
            Some(l) => l,
            None => fx.cu(0),
        };
        fx.g.b.image_size_lod(ut, img, l)
    } else {
        fx.g.b.image_size(ut, img)
    };
    (v, n)
}

/// the components 0..k of a vector (k = 1: a scalar)
fn first(fx: &mut Fx, v: Id, n: u8, k: u8) -> Id {
    if k == n {
        return v;
    }
    if k == 1 {
        return fx.g.b.extract(v, &[0]);
    }
    let comps: Vec<u32> = (0..k as u32).collect();
    fx.g.b.shuffle(v, v, &comps)
}

/// the number of mip levels - 1 clamped level (u32)
fn clamp_level(fx: &mut Fx, img: Id, level: Id) -> Id {
    let u = fx.g.b.ty_u32();
    let n = fx.g.b.image_levels(u, img);
    let one = fx.cu(1);
    let hi = fx.g.b.isub(n, one);
    fx.g.b.ext(u, spv::glsl::UMin, &[level, hi])
}

/// coordinates (and layer) as one vector of the coordinate type plus a float layer when
/// sampling
fn float_coords(fx: &mut Fx, c: Id, ct: TypeId, layer: Option<(Id, TypeId)>) -> Id {
    let Some((l, lt)) = layer else { return c };
    let f = fx.g.b.ty_f32();
    let lf = match fx.scalar(lt) {
        Scalar::I32 => fx.g.b.s2f(f, l),
        _ => fx.g.b.u2f(f, l),
    };
    let n = fx.width(ct) as u32;
    let vt = fx.g.b.ty_vec(f, n + 1);
    fx.g.b.construct(vt, &[c, lf])
}

fn texture(fx: &mut Fx, b: Builtin, args: &[ExprId]) -> V {
    use Builtin::*;
    let m = fx.m();
    let has_component = b == TextureGather && !matches!(*m.ty(fx.tyof(args[0])), Type::Texture(..) | Type::Reference { .. });
    let ti = if has_component { 1 } else { 0 };
    let tg = fx.handle_of(args[ti]);
    let (dim, kind) = tex_type(fx, tg);
    let robust = fx.g.robust;
    let depth = matches!(kind, TexKind::Depth | TexKind::DepthMultisampled);
    match b {
        TextureDimensions => {
            let img = fx.handle(tg);
            let level = if args.len() > 1 {
                let (l, _) = fx.index_u32(args[1]);
                Some(if robust { clamp_level(fx, img, l) } else { l })
            } else {
                None
            };
            let (v, n) = query_size(fx, img, dim, kind, level);
            let k = match dim {
                TexDim::D1 => 1,
                TexDim::D3 => 3,
                _ => 2,
            };
            V::Id(first(fx, v, n, k))
        }
        TextureNumLayers => {
            let img = fx.handle(tg);
            let (v, n) = query_size(fx, img, dim, kind, None);
            V::Id(fx.g.b.extract(v, &[n as u32 - 1]))
        }
        TextureNumLevels => {
            let img = fx.handle(tg);
            let u = fx.g.b.ty_u32();
            V::Id(fx.g.b.image_levels(u, img))
        }
        TextureNumSamples => {
            let img = fx.handle(tg);
            let u = fx.g.b.ty_u32();
            fx.g.b.capability(Cap::ImageQuery);
            V::Id(fx.g.b.emit(op::ImageQuerySamples, Some(u), &[img.0]))
        }
        TextureLoad | TextureStore => {
            let ct = fx.tyof(args[1]);
            let nc = fx.width(ct);
            let coords = fx.val(args[1]);
            let mut i = 2;
            let layer = if dim.arrayed() {
                i += 1;
                Some(fx.val(args[2]))
            } else {
                None
            };
            let lt = if dim.arrayed() { fx.tyof(args[2]) } else { U32 };
            let storage = matches!(kind, TexKind::Storage(..));
            let ms = matches!(kind, TexKind::Multisampled(_) | TexKind::DepthMultisampled);
            let level_or_sample = if b == TextureLoad && !storage && kind != TexKind::External { Some(fx.index_u32(args[i]).0) } else { None };
            let texel = if b == TextureStore { Some(fx.val(args[i])) } else { None };
            let img = fx.handle(tg);
            // coordinates (and layer) as u32 components
            let cu = as_u32(fx, coords, ct);
            let lu = layer.map(|l| as_u32(fx, l, lt));
            let level = match level_or_sample {
                Some(l) if !ms => Some(if robust { clamp_level(fx, img, l) } else { l }),
                _ => None,
            };
            let mut sample = if ms { level_or_sample } else { None };
            let (mut cu, mut lu) = (cu, lu);
            let mut inside: Option<Id> = None;
            if robust {
                let (size, n) = query_size(fx, img, dim, kind, level);
                let dims = first(fx, size, n, nc);
                let one = fx.num(fx.g.vec_tid(nc, Scalar::U32), 1.0);
                if b == TextureLoad {
                    let hi = fx.g.b.isub(dims, one);
                    let ut = uvec(fx, nc);
                    cu = fx.g.b.ext(ut, spv::glsl::UMin, &[cu, hi]);
                    if let Some(l) = lu {
                        let layers = fx.g.b.extract(size, &[n as u32 - 1]);
                        let one = fx.cu(1);
                        let hi = fx.g.b.isub(layers, one);
                        let u = fx.g.b.ty_u32();
                        lu = Some(fx.g.b.ext(u, spv::glsl::UMin, &[l, hi]));
                    }
                    if let Some(s) = sample {
                        let u = fx.g.b.ty_u32();
                        fx.g.b.capability(Cap::ImageQuery);
                        let ns = fx.g.b.emit(op::ImageQuerySamples, Some(u), &[img.0]);
                        let one = fx.cu(1);
                        let hi = fx.g.b.isub(ns, one);
                        sample = Some(fx.g.b.ext(u, spv::glsl::UMin, &[s, hi]));
                    }
                } else {
                    let lt_ = fx.g.b.ult(cu, dims);
                    let mut ok = if nc > 1 { fx.g.b.all(lt_) } else { lt_ };
                    if let Some(l) = lu {
                        let layers = fx.g.b.extract(size, &[n as u32 - 1]);
                        let c = fx.g.b.ult(l, layers);
                        ok = fx.g.b.land(ok, c);
                    }
                    inside = Some(ok);
                }
            }
            let coord = match lu {
                Some(l) => {
                    let vt = uvec(fx, nc + 1);
                    fx.g.b.construct(vt, &[cu, l])
                }
                None => cu,
            };
            if b == TextureStore {
                let texel = texel.unwrap_or(coord);
                let write = move |fx: &mut Fx| {
                    // the image again: the load and the write in one block
                    let img = fx.handle(tg);
                    fx.g.b.image_write(img, coord, texel);
                };
                match inside {
                    Some(ok) => fx.if_then(ok, |fx| fx.guarded(write)),
                    None => fx.guarded(write),
                }
                return V::None;
            }
            let v = if storage {
                fx.g.b.image_read(img, coord)
            } else if let Some(s) = sample {
                let t4 = vec4_of(fx, kind);
                fx.g.b.emit(op::ImageFetch, Some(t4), &[img.0, coord.0, spv::img::Sample, s.0])
            } else {
                let l = match level {
                    Some(l) => l,
                    None => fx.cu(0),
                };
                fx.g.b.fetch(img, coord, l)
            };
            V::Id(if depth { fx.g.b.extract(v, &[0]) } else { v })
        }
        _ => {
            // the sampling functions: texture, sampler, coordinates [, array index] ...
            let si_arg = ti + 1;
            let ci = ti + 2;
            let ct = fx.tyof(args[ci]);
            let coords = fx.val(args[ci]);
            let mut i = ci + 1;
            let layer = if dim.arrayed() {
                i += 1;
                let lt = fx.tyof(args[ci + 1]);
                Some((fx.val(args[ci + 1]), lt))
            } else {
                None
            };
            let coord = float_coords(fx, coords, ct, layer);
            let comp = if has_component {
                match &fx.exprs[args[0] as usize].kind {
                    ExprKind::Const(Value::I32(x)) => *x as u32,
                    ExprKind::Const(Value::U32(x)) => *x,
                    _ => 0,
                }
            } else {
                0
            };
            // the further operands by function
            let mut dref = None;
            let mut bias = None;
            let mut lod = None;
            let mut grad = None;
            match b {
                TextureSampleBias => {
                    bias = Some(fx.val(args[i]));
                    i += 1;
                }
                TextureSampleLevel => {
                    let lt = fx.tyof(args[i]);
                    let l = fx.val(args[i]);
                    let f = fx.g.b.ty_f32();
                    lod = Some(match fx.scalar(lt) {
                        Scalar::I32 => fx.g.b.s2f(f, l),
                        Scalar::U32 => fx.g.b.u2f(f, l),
                        _ => l,
                    });
                    i += 1;
                }
                TextureSampleGrad => {
                    let dx = fx.val(args[i]);
                    let dy = fx.val(args[i + 1]);
                    grad = Some((dx, dy));
                    i += 2;
                }
                TextureSampleCompare | TextureSampleCompareLevel | TextureGatherCompare => {
                    dref = Some(fx.val(args[i]));
                    i += 1;
                }
                _ => {}
            }
            let offset = args.get(i).map(|&o| fx.val(o));
            let base_clamp = b == TextureSampleBaseClampToEdge;
            let img = fx.handle(tg);
            let coord = if base_clamp {
                let (size, n) = query_size(fx, img, dim, kind, None);
                let size = first(fx, size, n, 2);
                let f = fx.g.b.ty_f32();
                let v2 = fx.g.b.ty_vec(f, 2);
                let sf = fx.g.b.u2f(v2, size);
                let half = fx.g.b.const_vec_f32(&[0.5, 0.5]);
                let ht = fx.g.b.fdiv(half, sf);
                let one = fx.g.b.const_vec_f32(&[1.0, 1.0]);
                let hi = fx.g.b.fsub(one, ht);
                let x = fx.g.b.glsl(g::NMAX, &[coord, ht]);
                fx.g.b.glsl(g::NMIN, &[x, hi])
            } else {
                coord
            };
            if base_clamp {
                lod = Some(fx.g.b.const_f32(0.0));
            }
            let sg = fx.handle_of(args[si_arg]);
            let smp = fx.handle(sg);
            let si = fx.g.b.sampled_image(img, smp);
            // image operands in bit order: Bias, Lod, Grad, ConstOffset
            let mut mask = 0u32;
            let mut ops: Vec<u32> = Vec::new();
            if let Some(x) = bias {
                mask |= spv::img::Bias;
                ops.push(x.0);
            }
            if let Some(x) = lod {
                mask |= spv::img::Lod;
                ops.push(x.0);
            }
            if b == TextureSampleCompareLevel {
                mask |= spv::img::Lod;
                let z = fx.g.b.const_f32(0.0);
                ops.push(z.0);
            }
            if let Some((dx, dy)) = grad {
                mask |= spv::img::Grad;
                ops.push(dx.0);
                ops.push(dy.0);
            }
            if let Some(o) = offset {
                mask |= spv::img::ConstOffset;
                ops.push(o.0);
            }
            let f = fx.g.b.ty_f32();
            let t4 = vec4_of(fx, kind);
            let mut w = alloc::vec![si.0, coord.0];
            let (opc, res) = match b {
                TextureSampleCompare => {
                    w.push(dref.unwrap_or(coord).0);
                    (op::ImageSampleDrefImplicitLod, f)
                }
                TextureSampleCompareLevel => {
                    w.push(dref.unwrap_or(coord).0);
                    (op::ImageSampleDrefExplicitLod, f)
                }
                TextureGatherCompare => {
                    w.push(dref.unwrap_or(coord).0);
                    (op::ImageDrefGather, t4)
                }
                TextureGather => {
                    let c = fx.g.b.const_u32(comp);
                    w.push(c.0);
                    (op::ImageGather, t4)
                }
                TextureSampleLevel | TextureSampleGrad | TextureSampleBaseClampToEdge => (op::ImageSampleExplicitLod, t4),
                _ => (op::ImageSampleImplicitLod, t4),
            };
            if mask != 0 {
                w.push(mask);
                w.extend(ops);
            }
            let v = fx.g.b.emit(opc, Some(res), &w);
            let scalar_result = depth && !matches!(b, TextureGather | TextureGatherCompare | TextureSampleCompare | TextureSampleCompareLevel);
            V::Id(if scalar_result { fx.g.b.extract(v, &[0]) } else { v })
        }
    }
}

/// vec4 of a texture's sampled type
fn vec4_of(fx: &mut Fx, kind: TexKind) -> Id {
    let s = match kind {
        TexKind::Sampled(s) | TexKind::Multisampled(s) => s,
        TexKind::Storage(f, _) => f.channel(),
        _ => Scalar::F32,
    };
    let st = fx.g.scalar_ty(s);
    fx.g.b.ty_vec(st, 4)
}
