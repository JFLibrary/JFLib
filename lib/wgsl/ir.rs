// ir -- the checked module: what the back end (G2c) lowers to SPIR-V without re-deriving
// anything (docs/WGSL.md, "Das IR")
//
//   Module            types (interned: equal types have equal TypeIds), structs with their
//                     layout, globals, overrides, functions (callees before callers), entry
//                     points with their interface, the module-scope expression arena, enabled
//                     extensions, diagnostic filters (read by the uniformity analysis)
//   Type              Scalar / Vector / Matrix / Atomic / Array (count, stride) / Struct /
//                     Pointer / Reference / SwizzleView / Sampler / Texture
//   Expr / ExprKind   one arena per function (Function::exprs) and one for module scope
//                     (Module::exprs); every node has its concrete type
//   Stmt              structured statements; for and while are desugared to Loop
//   Value             a folded constant (the value of every const-expression)
//
// Conventions:
//   * Every const-expression is folded into ExprKind::Const with a concrete type; abstract
//     types (AbstractInt, AbstractFloat) never appear in function or module arenas.
//   * References are explicit: Global(g) and Local(var) have Reference type; Load(e) reads
//     one; AddrOf / Deref convert between Reference and Pointer.  Index and Member on a
//     reference give a reference; on a value they give a value.  p.x on a pointer
//     (pointer_composite_access) is written Member(Deref(p), 0).
//   * Swizzle(base, comps, n): on a value: a vector value; on a reference with read_write
//     access (swizzle_assignment) the node has SwizzleView type -- Load reads the vector,
//     Assign / Compound write the listed components.  A single-letter swizzle is Member(base,
//     component).
//   * Binary operands have equal types except: matrix * scalar, scalar * matrix, matrix *
//     vector, vector * matrix, matrix * matrix, float vector * float scalar (both orders, for
//     OpVectorTimesScalar), the shifts (right operand u32 / vecN<u32>), and comparisons
//     (bool result).  Other mixed scalar / vector arithmetic gets an explicit Splat.
//   * Builtin(f, args): the arguments already have the types of the chosen overload; the
//     overload is determined by f and the argument types.
//   * Construct: vectors from scalars and vectors (as written), matrices from column vectors
//     (scalar arguments are grouped into Construct columns), arrays and structs from
//     elements / members.  Convert: numeric conversion (scalar, vector or matrix); identity
//     conversions are removed.
//   * Overrides stay symbolic: Override(i) leaves inside expressions; override-expressions
//     at module scope (override initializers, private var initializers, workgroup sizes,
//     override-sized array counts) live in Module::exprs.
//   * Compound assignment and ++ / -- evaluate the left side once: Compound { lhs, op, rhs }
//     (++ is Compound with op Add and the constant 1).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{BinOp, Span};
use crate::Diagnostic;

/// an index into Module::types (equal types have equal ids)
pub type TypeId = u32;
/// an index into an expression arena (Function::exprs or Module::exprs)
pub type ExprId = u32;

// the scalar TypeIds (interned first, in this order)
pub const BOOL: TypeId = 0;
/// AbstractInt (never in a checked module's arenas)
pub const AINT: TypeId = 1;
/// AbstractFloat (never in a checked module's arenas)
pub const AFLOAT: TypeId = 2;
/// i32
pub const I32: TypeId = 3;
/// u32
pub const U32: TypeId = 4;
/// f32
pub const F32: TypeId = 5;
/// f16
pub const F16: TypeId = 6;
/// the type of a call without a result (Call / Builtin nodes of void functions)
pub const VOID: TypeId = 7;

/// the scalar types (AbstractInt / AbstractFloat only for analysis)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum Scalar {
    Bool,
    AbstractInt,
    AbstractFloat,
    I32,
    U32,
    F32,
    F16,
}

impl Scalar {
    /// every scalar in TypeId order
    pub const ALL: [Scalar; 7] = [Scalar::Bool, Scalar::AbstractInt, Scalar::AbstractFloat, Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16];
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        match self {
            Scalar::Bool => "bool",
            Scalar::AbstractInt => "AbstractInt",
            Scalar::AbstractFloat => "AbstractFloat",
            Scalar::I32 => "i32",
            Scalar::U32 => "u32",
            Scalar::F32 => "f32",
            Scalar::F16 => "f16",
        }
    }
    /// the TypeId of the scalar
    pub fn id(self) -> TypeId {
        self as TypeId
    }
    /// AbstractInt or AbstractFloat
    pub fn is_abstract(self) -> bool {
        matches!(self, Scalar::AbstractInt | Scalar::AbstractFloat)
    }
    /// AbstractFloat, f32, f16
    pub fn is_float(self) -> bool {
        matches!(self, Scalar::AbstractFloat | Scalar::F32 | Scalar::F16)
    }
    /// AbstractInt, i32, u32
    pub fn is_int(self) -> bool {
        matches!(self, Scalar::AbstractInt | Scalar::I32 | Scalar::U32)
    }
    /// not bool
    pub fn is_numeric(self) -> bool {
        self != Scalar::Bool
    }
    /// byte size in memory (bool: 4)
    pub fn size(self) -> u32 {
        match self {
            Scalar::F16 => 2,
            Scalar::AbstractInt | Scalar::AbstractFloat => 8,
            _ => 4,
        }
    }
}

/// the address spaces (Handle: textures and samplers)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum AddressSpace {
    Function,
    Private,
    Workgroup,
    Uniform,
    Storage,
    Immediate,
    Handle,
}

impl AddressSpace {
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        match self {
            AddressSpace::Function => "function",
            AddressSpace::Private => "private",
            AddressSpace::Workgroup => "workgroup",
            AddressSpace::Uniform => "uniform",
            AddressSpace::Storage => "storage",
            AddressSpace::Immediate => "immediate",
            AddressSpace::Handle => "handle",
        }
    }
}

/// access modes of memory views
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

impl Access {
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
            Access::ReadWrite => "read_write",
        }
    }
    /// read or read_write
    pub fn readable(self) -> bool {
        self != Access::Write
    }
    /// write or read_write
    pub fn writable(self) -> bool {
        self != Access::Read
    }
}

/// the element count of an array type
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum ArrayCount {
    Fixed(u32),
    /// an identifier naming an override (Module::overrides index)
    Override(u32),
    /// another override-expression (Module::exprs; every such type is distinct)
    Expr(ExprId),
    Runtime,
}

/// texture dimensionalities
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum TexDim {
    D1,
    D2,
    D2Array,
    D3,
    Cube,
    CubeArray,
}

impl TexDim {
    /// the type-name suffix ("2d_array")
    pub fn suffix(self) -> &'static str {
        match self {
            TexDim::D1 => "1d",
            TexDim::D2 => "2d",
            TexDim::D2Array => "2d_array",
            TexDim::D3 => "3d",
            TexDim::Cube => "cube",
            TexDim::CubeArray => "cube_array",
        }
    }
    /// coordinate components (cube: the direction vector)
    pub fn coords(self) -> u8 {
        match self {
            TexDim::D1 => 1,
            TexDim::D2 | TexDim::D2Array => 2,
            TexDim::D3 | TexDim::Cube | TexDim::CubeArray => 3,
        }
    }
    /// 2d_array or cube_array
    pub fn arrayed(self) -> bool {
        matches!(self, TexDim::D2Array | TexDim::CubeArray)
    }
}

/// the texel formats of storage textures (spec 6.6.1)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum TexelFormat {
    Rgba8unorm, Rgba8snorm, Rgba8uint, Rgba8sint, Rgba16unorm, Rgba16snorm, Rgba16uint, Rgba16sint,
    Rgba16float, Rg8unorm, Rg8snorm, Rg8uint, Rg8sint, Rg16unorm, Rg16snorm, Rg16uint, Rg16sint,
    Rg16float, R32uint, R32sint, R32float, Rg32uint, Rg32sint, Rg32float, Rgba32uint, Rgba32sint,
    Rgba32float, Bgra8unorm, R8unorm, R8snorm, R8uint, R8sint, R16unorm, R16snorm, R16uint,
    R16sint, R16float, Rgb10a2unorm, Rgb10a2uint, Rg11b10ufloat,
}

/// (name, format, needs texture_formats_tier1)
pub const TEXEL_FORMATS: [(&str, TexelFormat, bool); 40] = [
    ("rgba8unorm", TexelFormat::Rgba8unorm, false), ("rgba8snorm", TexelFormat::Rgba8snorm, false),
    ("rgba8uint", TexelFormat::Rgba8uint, false), ("rgba8sint", TexelFormat::Rgba8sint, false),
    ("rgba16unorm", TexelFormat::Rgba16unorm, true), ("rgba16snorm", TexelFormat::Rgba16snorm, true),
    ("rgba16uint", TexelFormat::Rgba16uint, false), ("rgba16sint", TexelFormat::Rgba16sint, false),
    ("rgba16float", TexelFormat::Rgba16float, false), ("rg8unorm", TexelFormat::Rg8unorm, true),
    ("rg8snorm", TexelFormat::Rg8snorm, true), ("rg8uint", TexelFormat::Rg8uint, true),
    ("rg8sint", TexelFormat::Rg8sint, true), ("rg16unorm", TexelFormat::Rg16unorm, true),
    ("rg16snorm", TexelFormat::Rg16snorm, true), ("rg16uint", TexelFormat::Rg16uint, true),
    ("rg16sint", TexelFormat::Rg16sint, true), ("rg16float", TexelFormat::Rg16float, true),
    ("r32uint", TexelFormat::R32uint, false), ("r32sint", TexelFormat::R32sint, false),
    ("r32float", TexelFormat::R32float, false), ("rg32uint", TexelFormat::Rg32uint, false),
    ("rg32sint", TexelFormat::Rg32sint, false), ("rg32float", TexelFormat::Rg32float, false),
    ("rgba32uint", TexelFormat::Rgba32uint, false), ("rgba32sint", TexelFormat::Rgba32sint, false),
    ("rgba32float", TexelFormat::Rgba32float, false), ("bgra8unorm", TexelFormat::Bgra8unorm, false),
    ("r8unorm", TexelFormat::R8unorm, true), ("r8snorm", TexelFormat::R8snorm, true),
    ("r8uint", TexelFormat::R8uint, true), ("r8sint", TexelFormat::R8sint, true),
    ("r16unorm", TexelFormat::R16unorm, true), ("r16snorm", TexelFormat::R16snorm, true),
    ("r16uint", TexelFormat::R16uint, true), ("r16sint", TexelFormat::R16sint, true),
    ("r16float", TexelFormat::R16float, true), ("rgb10a2unorm", TexelFormat::Rgb10a2unorm, true),
    ("rgb10a2uint", TexelFormat::Rgb10a2uint, true), ("rg11b10ufloat", TexelFormat::Rg11b10ufloat, true),
];

impl TexelFormat {
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        TEXEL_FORMATS.iter().find(|f| f.1 == self).map(|f| f.0).unwrap_or("?")
    }
    /// the channel type in the shader: f32, i32 or u32
    pub fn channel(self) -> Scalar {
        let n = self.name();
        if n.ends_with("sint") {
            Scalar::I32
        } else if n.ends_with("uint") {
            Scalar::U32
        } else {
            Scalar::F32
        }
    }
}

/// texture kinds (with the sampled type, format and access where they have one)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum TexKind {
    /// texture_1d<T> ... texture_cube_array<T>; T is f32, i32 or u32
    Sampled(Scalar),
    Multisampled(Scalar),
    Depth,
    DepthMultisampled,
    Storage(TexelFormat, Access),
    External,
}

/// a type (see the module comment)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum Type {
    Scalar(Scalar),
    Vector(u8, Scalar),
    /// columns, rows, element (f32, f16 or AbstractFloat)
    Matrix(u8, u8, Scalar),
    /// atomic<i32>, atomic<u32> or atomic<vec2<u32>> (the TypeId of the inner type)
    Atomic(TypeId),
    /// stride: roundUp(align, size) of the element (0 for abstract elements)
    Array { elem: TypeId, count: ArrayCount, stride: u32 },
    /// Module::structs index
    Struct(u32),
    Pointer { space: AddressSpace, store: TypeId, access: Access },
    Reference { space: AddressSpace, store: TypeId, access: Access },
    /// swizzle<AS, S, N, K>: K components of a vecN<S> in memory (swizzle_assignment)
    SwizzleView { space: AddressSpace, elem: Scalar, n: u8, k: u8 },
    Sampler { comparison: bool },
    Texture(TexDim, TexKind),
    /// no value: the type of a call to a function without result type (TypeId VOID)
    Void,
}

/// the predeclared structures of built-in results
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum BuiltinStruct {
    /// frexp: (vector width or 1, fract scalar type)
    Frexp(u8, Scalar),
    /// modf: (vector width or 1, scalar type)
    Modf(u8, Scalar),
    /// atomicCompareExchangeWeak: old_value type (i32 / u32)
    AtomicCompareExchange(Scalar),
}

/// the built-in values of @builtin(...)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum BuiltinValue {
    VertexIndex,
    InstanceIndex,
    ClipDistances,
    Position,
    FrontFacing,
    FragDepth,
    PrimitiveIndex,
    SampleIndex,
    SampleMask,
    LocalInvocationId,
    LocalInvocationIndex,
    GlobalInvocationId,
    GlobalInvocationIndex,
    WorkgroupId,
    WorkgroupIndex,
    NumWorkgroups,
    SubgroupInvocationId,
    SubgroupSize,
    SubgroupId,
    NumSubgroups,
}

/// (name, value) of every built-in value
pub const BUILTIN_VALUES: [(&str, BuiltinValue); 20] = [
    ("vertex_index", BuiltinValue::VertexIndex), ("instance_index", BuiltinValue::InstanceIndex),
    ("clip_distances", BuiltinValue::ClipDistances), ("position", BuiltinValue::Position),
    ("front_facing", BuiltinValue::FrontFacing), ("frag_depth", BuiltinValue::FragDepth),
    ("primitive_index", BuiltinValue::PrimitiveIndex), ("sample_index", BuiltinValue::SampleIndex),
    ("sample_mask", BuiltinValue::SampleMask), ("local_invocation_id", BuiltinValue::LocalInvocationId),
    ("local_invocation_index", BuiltinValue::LocalInvocationIndex),
    ("global_invocation_id", BuiltinValue::GlobalInvocationId),
    ("global_invocation_index", BuiltinValue::GlobalInvocationIndex), ("workgroup_id", BuiltinValue::WorkgroupId),
    ("workgroup_index", BuiltinValue::WorkgroupIndex), ("num_workgroups", BuiltinValue::NumWorkgroups),
    ("subgroup_invocation_id", BuiltinValue::SubgroupInvocationId), ("subgroup_size", BuiltinValue::SubgroupSize),
    ("subgroup_id", BuiltinValue::SubgroupId), ("num_subgroups", BuiltinValue::NumSubgroups),
];

impl BuiltinValue {
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        BUILTIN_VALUES.iter().find(|b| b.1 == self).map(|b| b.0).unwrap_or("?")
    }
}

/// @interpolate types
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum InterpType {
    Perspective,
    Linear,
    Flat,
}

/// @interpolate samplings
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum InterpSampling {
    Center,
    Centroid,
    Sample,
    First,
    Either,
}

/// the depth mode of @builtin(frag_depth, less | greater) (fragment_depth)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum DepthMode {
    Less,
    Greater,
}

/// the IO attributes of a parameter, return type or struct member
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Io {
    pub builtin: Option<BuiltinValue>,
    pub depth_mode: Option<DepthMode>,
    pub location: Option<u32>,
    pub blend_src: Option<u32>,
    /// as written (None: not written)
    pub interpolate: Option<(InterpType, Option<InterpSampling>)>,
    pub invariant: bool,
}

/// a member with its layout and IO attributes
#[derive(Clone, PartialEq, Debug)]
pub struct StructMember {
    pub name: String,
    pub ty: TypeId,
    /// byte offset, size (@size or the type's), alignment (@align or the type's)
    pub offset: u32,
    pub size: u32,
    pub align: u32,
    pub io: Io,
    pub span: Span,
}

/// a structure with its layout (the same in every address space)
#[derive(Clone, PartialEq, Debug)]
pub struct Struct {
    pub name: String,
    pub members: Vec<StructMember>,
    pub align: u32,
    /// roundUp(align, end of the last member); with a runtime-sized last member: the size
    /// without its elements
    pub size: u32,
    pub runtime_sized: bool,
    pub builtin: Option<BuiltinStruct>,
    pub span: Span,
}

/// a folded constant; Zero is the zero value of whatever type the node has
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum Value {
    Bool(bool),
    AInt(i64),
    AFloat(f64),
    I32(i32),
    U32(u32),
    F32(f32),
    /// an f16 value (exactly representable in f16), held as f32
    F16(f32),
    /// vector, matrix (columns), array or struct components
    Comp(Vec<Value>),
    Zero,
}

/// the bits of an f16 value held as f32
pub fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let e = ((b >> 23) & 0xff) as i32;
    let m = b & 0x7f_ffff;
    if e == 0 && m == 0 {
        return sign;
    }
    let ue = e - 127;
    if ue < -14 {
        // subnormal f16: x = m16 * 2^-24
        let full = m | 0x80_0000;
        let shift = (-14 - ue) as u32 + 13;
        return sign | (full >> shift) as u16;
    }
    sign | (((ue + 15) as u16) << 10) | (m >> 13) as u16
}

/// unary operators (&, * are AddrOf / Deref)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum UnaryOp {
    Neg,
    /// logical not (bool, vecN<bool>)
    Not,
    BitNot,
}

/// the built-in functions (spec section 17; no buffer views)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[non_exhaustive]
pub enum Builtin {
    // logical, array
    All, Any, Select, ArrayLength,
    // numeric
    Abs, Acos, Acosh, Asin, Asinh, Atan, Atanh, Atan2, Ceil, Clamp, Cos, Cosh, CountLeadingZeros,
    CountOneBits, CountTrailingZeros, Cross, Degrees, Determinant, Distance, Dot, Dot4U8Packed,
    Dot4I8Packed, Exp, Exp2, ExtractBits, FaceForward, FirstLeadingBit, FirstTrailingBit, Floor,
    Fma, Fract, Frexp, InsertBits, InverseSqrt, Ldexp, Length, Log, Log2, Max, Min, Mix, Modf,
    Normalize, Pow, QuantizeToF16, Radians, Reflect, Refract, ReverseBits, Round, Saturate, Sign,
    Sin, Sinh, Smoothstep, Sqrt, Step, Tan, Tanh, Transpose, Trunc,
    // derivatives
    Dpdx, DpdxCoarse, DpdxFine, Dpdy, DpdyCoarse, DpdyFine, Fwidth, FwidthCoarse, FwidthFine,
    // textures
    TextureDimensions, TextureGather, TextureGatherCompare, TextureLoad, TextureNumLayers,
    TextureNumLevels, TextureNumSamples, TextureSample, TextureSampleBias, TextureSampleCompare,
    TextureSampleCompareLevel, TextureSampleGrad, TextureSampleLevel, TextureSampleBaseClampToEdge,
    TextureStore,
    // atomics
    AtomicLoad, AtomicStore, AtomicAdd, AtomicSub, AtomicMax, AtomicMin, AtomicAnd, AtomicOr,
    AtomicXor, AtomicExchange, AtomicCompareExchangeWeak, AtomicStoreMin, AtomicStoreMax,
    // packing
    Pack4x8snorm, Pack4x8unorm, Pack4xI8, Pack4xU8, Pack4xI8Clamp, Pack4xU8Clamp, Pack2x16snorm,
    Pack2x16unorm, Pack2x16float, Unpack4x8snorm, Unpack4x8unorm, Unpack4xI8, Unpack4xU8,
    Unpack2x16snorm, Unpack2x16unorm, Unpack2x16float,
    // synchronization
    StorageBarrier, TextureBarrier, WorkgroupBarrier, WorkgroupUniformLoad,
    // subgroups
    SubgroupAdd, SubgroupExclusiveAdd, SubgroupInclusiveAdd, SubgroupAll, SubgroupAnd, SubgroupAny,
    SubgroupBallot, SubgroupBroadcast, SubgroupBroadcastFirst, SubgroupElect, SubgroupMax, SubgroupMin,
    SubgroupMul, SubgroupExclusiveMul, SubgroupInclusiveMul, SubgroupOr, SubgroupShuffle,
    SubgroupShuffleDown, SubgroupShuffleUp, SubgroupShuffleXor, SubgroupXor,
    // quad operations
    QuadBroadcast, QuadSwapDiagonal, QuadSwapX, QuadSwapY,
}

/// expression nodes (see the module comment for the conventions)
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum ExprKind {
    Const(Value),
    Override(u32),
    Global(u32),
    Local(u32),
    Param(u32),
    Load(ExprId),
    AddrOf(ExprId),
    Deref(ExprId),
    Index(ExprId, ExprId),
    /// struct member or vector component (constant index)
    Member(ExprId, u32),
    /// base, components, count (2..4)
    Swizzle(ExprId, [u8; 4], u8),
    Unary(UnaryOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    Construct(Vec<ExprId>),
    Splat(ExprId),
    Convert(ExprId),
    Bitcast(ExprId),
    /// Module::functions index, arguments
    Call(u32, Vec<ExprId>),
    Builtin(Builtin, Vec<ExprId>),
}

/// an expression node with its concrete type and source range
#[derive(Clone, PartialEq, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: TypeId,
    pub span: Span,
}

/// a switch clause
#[derive(Clone, PartialEq, Debug)]
pub struct Case {
    /// the selector values (i32 or u32, as i64)
    pub selectors: Vec<i64>,
    pub default: bool,
    pub body: Vec<Stmt>,
}

/// statements (for and while are desugared to Loop)
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum Stmt {
    Block(Vec<Stmt>),
    Var { local: u32, init: Option<ExprId> },
    Let { local: u32, init: ExprId },
    Assign { lhs: ExprId, rhs: ExprId },
    Compound { lhs: ExprId, op: BinOp, rhs: ExprId },
    /// `_ = e`: evaluate and drop
    Phony(ExprId),
    /// a call whose result (if any) is dropped
    Call(ExprId),
    If { cond: ExprId, then: Vec<Stmt>, else_: Vec<Stmt> },
    Switch { selector: ExprId, cases: Vec<Case> },
    Loop { body: Vec<Stmt>, continuing: Vec<Stmt>, break_if: Option<ExprId> },
    Break,
    Continue,
    Return(Option<ExprId>),
    Discard,
}

/// what a local is
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum LocalKind {
    /// a function-scope variable: Local(i) is a reference to ty
    Var,
    /// a let: Local(i) is the value (ty may be a pointer, texture or sampler)
    Let,
}

/// a function-scope var or let
#[derive(Clone, PartialEq, Debug)]
pub struct Local {
    pub name: String,
    pub ty: TypeId,
    pub kind: LocalKind,
    pub span: Span,
}

/// a function parameter (io: the IO attributes of an entry point parameter)
#[derive(Clone, PartialEq, Debug)]
pub struct Param {
    pub name: String,
    pub ty: TypeId,
    pub io: Io,
    pub span: Span,
}

/// shader stages
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
}

impl Stage {
    /// the WGSL spelling
    pub fn name(self) -> &'static str {
        match self {
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
            Stage::Compute => "compute",
        }
    }
}

/// a checked function
#[derive(Clone, PartialEq, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeId>,
    pub ret_io: Io,
    pub stage: Option<Stage>,
    pub must_use: bool,
    pub locals: Vec<Local>,
    pub exprs: Vec<Expr>,
    pub body: Vec<Stmt>,
    /// the functions called directly
    pub calls: Vec<u32>,
    /// module-scope variables and overrides used, including by callees (ascending)
    pub globals: Vec<u32>,
    pub overrides: Vec<u32>,
    /// discard reachable from here (callees included)
    pub uses_discard: bool,
    pub span: Span,
}

/// @group / @binding of a resource
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binding {
    pub group: u32,
    pub binding: u32,
}

/// a module-scope variable
#[derive(Clone, PartialEq, Debug)]
pub struct Global {
    pub name: String,
    pub space: AddressSpace,
    pub access: Access,
    /// the store type
    pub ty: TypeId,
    pub binding: Option<Binding>,
    /// Module::exprs (private variables: a Const or an override-expression)
    pub init: Option<ExprId>,
    /// uniform buffers: the type also satisfies the strict uniform layout rules (16-byte
    /// array strides and struct alignment, std140-compatible); always true elsewhere
    pub std140: bool,
    pub span: Span,
}

/// a pipeline-overridable constant
#[derive(Clone, PartialEq, Debug)]
pub struct Override {
    pub name: String,
    /// @id(n)
    pub id: Option<u16>,
    /// a concrete scalar type
    pub ty: TypeId,
    /// the default (Module::exprs)
    pub init: Option<ExprId>,
    pub span: Span,
}

/// a module-scope const (folded; kept for tools, the IR never refers to it)
#[derive(Clone, PartialEq, Debug)]
pub struct Const {
    pub name: String,
    pub ty: TypeId,
    pub value: Value,
    pub span: Span,
}

/// one workgroup_size dimension
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum WgSize {
    Const(u32),
    /// an override-expression in Module::exprs
    Override(ExprId),
}

/// how a stage input / output is bound
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum IoKind {
    Builtin(BuiltinValue),
    /// location, blend_src, interpolation with the defaults applied (None for integer
    /// vertex inputs and fragment outputs, where it does not apply)
    Location { location: u32, blend_src: Option<u32>, interp: Option<(InterpType, InterpSampling)> },
}

/// one shader stage input or output
#[derive(Clone, PartialEq, Debug)]
pub struct IoVar {
    /// "param" or "param.member" (outputs: "return" or "return.member")
    pub name: String,
    pub ty: TypeId,
    /// the parameter (inputs; None for outputs)
    pub param: Option<u32>,
    /// the struct member when the parameter / return type is a struct
    pub member: Option<u32>,
    pub kind: IoKind,
    pub invariant: bool,
    pub depth_mode: Option<DepthMode>,
}

/// an entry point and its interface
#[derive(Clone, PartialEq, Debug)]
pub struct EntryPoint {
    pub name: String,
    /// Module::functions index
    pub func: u32,
    pub stage: Stage,
    /// compute only
    pub workgroup_size: Option<[WgSize; 3]>,
    /// @subgroup_size (compute only, subgroup_size_control)
    pub subgroup_size: Option<WgSize>,
    pub inputs: Vec<IoVar>,
    pub outputs: Vec<IoVar>,
    /// every module-scope variable statically accessed (ascending)
    pub globals: Vec<u32>,
    /// the resource variables among them with their (group, binding)
    pub resources: Vec<(u32, Binding)>,
    /// the overrides statically accessed (also through initializers, array sizes and the
    /// workgroup size)
    pub overrides: Vec<u32>,
    /// the immediate data variable, if one is used
    pub immediate: Option<u32>,
}

/// diagnostic filter severities
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Severity {
    Error,
    Warning,
    Info,
    Off,
}

/// a diagnostic filter: the affected range (the whole module for global ones)
#[derive(Clone, PartialEq, Debug)]
pub struct DiagnosticFilter {
    pub severity: Severity,
    /// "derivative_uniformity", "a.b"
    pub rule: String,
    pub range: Span,
    pub global: bool,
}

/// the checked module
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Module {
    pub types: Vec<Type>,
    pub structs: Vec<Struct>,
    pub globals: Vec<Global>,
    pub overrides: Vec<Override>,
    pub consts: Vec<Const>,
    pub functions: Vec<Function>,
    pub entry_points: Vec<EntryPoint>,
    /// module-scope expressions (override-expressions and folded initializers)
    pub exprs: Vec<Expr>,
    /// feature::* bits enabled with `enable` and named in `requires`
    pub enables: u32,
    pub requires: u32,
    pub diagnostic_filters: Vec<DiagnosticFilter>,
    pub warnings: Vec<Diagnostic>,
}

impl Module {
    /// the type of a TypeId
    pub fn ty(&self, t: TypeId) -> &Type {
        &self.types[t as usize]
    }
    /// the WGSL spelling of a type ("vec3<f32>", "ptr<storage, S, read>")
    pub fn type_name(&self, t: TypeId) -> String {
        type_name(&self.types, &self.structs, t)
    }
    /// a function by name
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.name == name)
    }
    /// an entry point by name
    pub fn entry_point(&self, name: &str) -> Option<&EntryPoint> {
        self.entry_points.iter().find(|e| e.name == name)
    }
    /// a module-scope variable by name
    pub fn global(&self, name: &str) -> Option<&Global> {
        self.globals.iter().find(|g| g.name == name)
    }
    /// a structure by name
    pub fn struct_named(&self, name: &str) -> Option<&Struct> {
        self.structs.iter().find(|s| s.name == name)
    }
}

/// see Module::type_name
pub fn type_name(types: &[Type], structs: &[Struct], t: TypeId) -> String {
    let Some(ty) = types.get(t as usize) else { return String::from("?") };
    match *ty {
        Type::Scalar(s) => String::from(s.name()),
        Type::Vector(n, s) => format!("vec{}<{}>", n, s.name()),
        Type::Matrix(c, r, s) => format!("mat{}x{}<{}>", c, r, s.name()),
        Type::Atomic(i) => format!("atomic<{}>", type_name(types, structs, i)),
        Type::Array { elem, count, .. } => match count {
            ArrayCount::Fixed(n) => format!("array<{}, {}>", type_name(types, structs, elem), n),
            ArrayCount::Runtime => format!("array<{}>", type_name(types, structs, elem)),
            _ => format!("array<{}, override>", type_name(types, structs, elem)),
        },
        Type::Struct(i) => structs.get(i as usize).map(|s| s.name.clone()).unwrap_or_else(|| String::from("?")),
        Type::Pointer { space, store, access } => {
            format!("ptr<{}, {}, {}>", space.name(), type_name(types, structs, store), access.name())
        }
        Type::Reference { space, store, access } => {
            format!("ref<{}, {}, {}>", space.name(), type_name(types, structs, store), access.name())
        }
        Type::SwizzleView { space, elem, n, k } => format!("swizzle<{}, {}, {}, {}>", space.name(), elem.name(), n, k),
        Type::Sampler { comparison } => String::from(if comparison { "sampler_comparison" } else { "sampler" }),
        Type::Texture(dim, kind) => match kind {
            TexKind::Sampled(s) => format!("texture_{}<{}>", dim.suffix(), s.name()),
            TexKind::Multisampled(s) => format!("texture_multisampled_2d<{}>", s.name()),
            TexKind::Depth => format!("texture_depth_{}", dim.suffix()),
            TexKind::DepthMultisampled => String::from("texture_depth_multisampled_2d"),
            TexKind::Storage(f, a) => format!("texture_storage_{}<{}, {}>", dim.suffix(), f.name(), a.name()),
            TexKind::External => String::from("texture_external"),
        },
        Type::Void => String::from("void"),
    }
}
