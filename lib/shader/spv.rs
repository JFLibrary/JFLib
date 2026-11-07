// spv -- the numbers of SPIR-V 1.6 (unified1 grammar) that shader uses, with their names
//
//   op::*          opcodes (op::FAdd = 129), OPS / op_info(op): name and operand grammar
//   Cap, Sc (storage class), Dec (decoration), Bi (built-in), Model (execution model),
//   Mode (execution mode), Dim, Fmt (image format), Lang (source language): typed enums with
//   name tables (CAPS, SCS, ...) for the disassembler; name(table, v) -> Option<&str>
//   scope::*, sem::* (memory semantics), mask names (fn_ctl, sel_ctl, loop_ctl, mem_access,
//   img_ops), glsl::* (GLSL.std.450 instruction numbers, GLSL_NAMES)
//
// Operand grammar (OpInfo::ops), one character per operand, `*` repeats the last kind,
// `?` makes the rest optional:
//   i id   n literal word   s string   v literal number of the result type (OpConstant)
//   C capability   A addressing model   Y memory model   M execution model
//   E execution mode + literal args   G execution mode + id args   d decoration + args
//   S storage class   D dim   F image format   Q access qualifier   k source language
//   f function control   e selection control   l loop control (+ args)
//   m memory access (+ args)   I image operands (+ id args)   x extended instruction number
//   w switch targets: (literal, id) pairs
//   o an opcode (OpSpecConstantOp) followed by the operands of that opcode's grammar
#![allow(non_upper_case_globals)]

pub const MAGIC: u32 = 0x0723_0203;

/// the version word of SPIR-V major.minor
pub const fn version(major: u32, minor: u32) -> u32 {
    (major << 16) | (minor << 8)
}

/// result flags of OpInfo::res
pub const R_NONE: u8 = 0;
pub const R_ID: u8 = 1; // a result id
pub const R_TY: u8 = 2; // a result type and a result id

#[derive(Clone, Copy, Debug)]
pub struct OpInfo {
    pub op: u16,
    pub name: &'static str,
    pub res: u8,
    pub ops: &'static str,
}

macro_rules! ops {
    ($($name:ident = $n:expr, $res:ident, $ops:expr;)*) => {
        /// opcodes: op::FAdd and so on (the name without the "Op")
        pub mod op { $(pub const $name: u16 = $n;)* }
        /// every opcode shader knows, in opcode order
        pub const OPS: &[OpInfo] = &[$(OpInfo { op: $n, name: stringify!($name), res: $res, ops: $ops }),*];
    };
}

ops! {
    Nop = 0, R_NONE, "";
    Undef = 1, R_TY, "";
    Source = 3, R_NONE, "kni?s";
    Name = 5, R_NONE, "is";
    MemberName = 6, R_NONE, "ins";
    String = 7, R_ID, "s";
    Line = 8, R_NONE, "inn";
    Extension = 10, R_NONE, "s";
    ExtInstImport = 11, R_ID, "s";
    ExtInst = 12, R_TY, "ixi*";
    MemoryModel = 14, R_NONE, "AY";
    EntryPoint = 15, R_NONE, "Misi*";
    ExecutionMode = 16, R_NONE, "iE";
    Capability = 17, R_NONE, "C";
    TypeVoid = 19, R_ID, "";
    TypeBool = 20, R_ID, "";
    TypeInt = 21, R_ID, "nn";
    TypeFloat = 22, R_ID, "n";
    TypeVector = 23, R_ID, "in";
    TypeMatrix = 24, R_ID, "in";
    TypeImage = 25, R_ID, "iDnnnnF?Q";
    TypeSampler = 26, R_ID, "";
    TypeSampledImage = 27, R_ID, "i";
    TypeArray = 28, R_ID, "ii";
    TypeRuntimeArray = 29, R_ID, "i";
    TypeStruct = 30, R_ID, "i*";
    TypePointer = 32, R_ID, "Si";
    TypeFunction = 33, R_ID, "ii*";
    ConstantTrue = 41, R_TY, "";
    ConstantFalse = 42, R_TY, "";
    Constant = 43, R_TY, "v";
    ConstantComposite = 44, R_TY, "i*";
    ConstantNull = 46, R_TY, "";
    SpecConstantTrue = 48, R_TY, "";
    SpecConstantFalse = 49, R_TY, "";
    SpecConstant = 50, R_TY, "v";
    SpecConstantComposite = 51, R_TY, "i*";
    SpecConstantOp = 52, R_TY, "o";
    Function = 54, R_TY, "fi";
    FunctionParameter = 55, R_TY, "";
    FunctionEnd = 56, R_NONE, "";
    FunctionCall = 57, R_TY, "ii*";
    Variable = 59, R_TY, "S?i";
    ImageTexelPointer = 60, R_TY, "iii";
    Load = 61, R_TY, "i?m";
    Store = 62, R_NONE, "ii?m";
    CopyMemory = 63, R_NONE, "ii?m";
    AccessChain = 65, R_TY, "ii*";
    InBoundsAccessChain = 66, R_TY, "ii*";
    PtrAccessChain = 67, R_TY, "iii*";
    ArrayLength = 68, R_TY, "in";
    Decorate = 71, R_NONE, "id";
    MemberDecorate = 72, R_NONE, "ind";
    VectorExtractDynamic = 77, R_TY, "ii";
    VectorInsertDynamic = 78, R_TY, "iii";
    VectorShuffle = 79, R_TY, "iin*";
    CompositeConstruct = 80, R_TY, "i*";
    CompositeExtract = 81, R_TY, "in*";
    CompositeInsert = 82, R_TY, "iin*";
    CopyObject = 83, R_TY, "i";
    Transpose = 84, R_TY, "i";
    SampledImage = 86, R_TY, "ii";
    ImageSampleImplicitLod = 87, R_TY, "ii?I";
    ImageSampleExplicitLod = 88, R_TY, "iiI";
    ImageSampleDrefImplicitLod = 89, R_TY, "iii?I";
    ImageSampleDrefExplicitLod = 90, R_TY, "iiiI";
    ImageFetch = 95, R_TY, "ii?I";
    ImageGather = 96, R_TY, "iii?I";
    ImageDrefGather = 97, R_TY, "iii?I";
    ImageRead = 98, R_TY, "ii?I";
    ImageWrite = 99, R_NONE, "iii?I";
    Image = 100, R_TY, "i";
    ImageQuerySizeLod = 103, R_TY, "ii";
    ImageQuerySize = 104, R_TY, "i";
    ImageQueryLod = 105, R_TY, "ii";
    ImageQueryLevels = 106, R_TY, "i";
    ImageQuerySamples = 107, R_TY, "i";
    ConvertFToU = 109, R_TY, "i";
    ConvertFToS = 110, R_TY, "i";
    ConvertSToF = 111, R_TY, "i";
    ConvertUToF = 112, R_TY, "i";
    UConvert = 113, R_TY, "i";
    SConvert = 114, R_TY, "i";
    FConvert = 115, R_TY, "i";
    QuantizeToF16 = 116, R_TY, "i";
    Bitcast = 124, R_TY, "i";
    SNegate = 126, R_TY, "i";
    FNegate = 127, R_TY, "i";
    IAdd = 128, R_TY, "ii";
    FAdd = 129, R_TY, "ii";
    ISub = 130, R_TY, "ii";
    FSub = 131, R_TY, "ii";
    IMul = 132, R_TY, "ii";
    FMul = 133, R_TY, "ii";
    UDiv = 134, R_TY, "ii";
    SDiv = 135, R_TY, "ii";
    FDiv = 136, R_TY, "ii";
    UMod = 137, R_TY, "ii";
    SRem = 138, R_TY, "ii";
    SMod = 139, R_TY, "ii";
    FRem = 140, R_TY, "ii";
    FMod = 141, R_TY, "ii";
    VectorTimesScalar = 142, R_TY, "ii";
    MatrixTimesScalar = 143, R_TY, "ii";
    VectorTimesMatrix = 144, R_TY, "ii";
    MatrixTimesVector = 145, R_TY, "ii";
    MatrixTimesMatrix = 146, R_TY, "ii";
    OuterProduct = 147, R_TY, "ii";
    Dot = 148, R_TY, "ii";
    IAddCarry = 149, R_TY, "ii";
    ISubBorrow = 150, R_TY, "ii";
    UMulExtended = 151, R_TY, "ii";
    SMulExtended = 152, R_TY, "ii";
    Any = 154, R_TY, "i";
    All = 155, R_TY, "i";
    IsNan = 156, R_TY, "i";
    IsInf = 157, R_TY, "i";
    IsFinite = 158, R_TY, "i";
    IsNormal = 159, R_TY, "i";
    SignBitSet = 160, R_TY, "i";
    LogicalEqual = 164, R_TY, "ii";
    LogicalNotEqual = 165, R_TY, "ii";
    LogicalOr = 166, R_TY, "ii";
    LogicalAnd = 167, R_TY, "ii";
    LogicalNot = 168, R_TY, "i";
    Select = 169, R_TY, "iii";
    IEqual = 170, R_TY, "ii";
    INotEqual = 171, R_TY, "ii";
    UGreaterThan = 172, R_TY, "ii";
    SGreaterThan = 173, R_TY, "ii";
    UGreaterThanEqual = 174, R_TY, "ii";
    SGreaterThanEqual = 175, R_TY, "ii";
    ULessThan = 176, R_TY, "ii";
    SLessThan = 177, R_TY, "ii";
    ULessThanEqual = 178, R_TY, "ii";
    SLessThanEqual = 179, R_TY, "ii";
    FOrdEqual = 180, R_TY, "ii";
    FUnordEqual = 181, R_TY, "ii";
    FOrdNotEqual = 182, R_TY, "ii";
    FUnordNotEqual = 183, R_TY, "ii";
    FOrdLessThan = 184, R_TY, "ii";
    FUnordLessThan = 185, R_TY, "ii";
    FOrdGreaterThan = 186, R_TY, "ii";
    FUnordGreaterThan = 187, R_TY, "ii";
    FOrdLessThanEqual = 188, R_TY, "ii";
    FUnordLessThanEqual = 189, R_TY, "ii";
    FOrdGreaterThanEqual = 190, R_TY, "ii";
    FUnordGreaterThanEqual = 191, R_TY, "ii";
    ShiftRightLogical = 194, R_TY, "ii";
    ShiftRightArithmetic = 195, R_TY, "ii";
    ShiftLeftLogical = 196, R_TY, "ii";
    BitwiseOr = 197, R_TY, "ii";
    BitwiseXor = 198, R_TY, "ii";
    BitwiseAnd = 199, R_TY, "ii";
    Not = 200, R_TY, "i";
    BitFieldInsert = 201, R_TY, "iiii";
    BitFieldSExtract = 202, R_TY, "iii";
    BitFieldUExtract = 203, R_TY, "iii";
    BitReverse = 204, R_TY, "i";
    BitCount = 205, R_TY, "i";
    DPdx = 207, R_TY, "i";
    DPdy = 208, R_TY, "i";
    Fwidth = 209, R_TY, "i";
    DPdxFine = 210, R_TY, "i";
    DPdyFine = 211, R_TY, "i";
    FwidthFine = 212, R_TY, "i";
    DPdxCoarse = 213, R_TY, "i";
    DPdyCoarse = 214, R_TY, "i";
    FwidthCoarse = 215, R_TY, "i";
    ControlBarrier = 224, R_NONE, "iii";
    MemoryBarrier = 225, R_NONE, "ii";
    AtomicLoad = 227, R_TY, "iii";
    AtomicStore = 228, R_NONE, "iiii";
    AtomicExchange = 229, R_TY, "iiii";
    AtomicCompareExchange = 230, R_TY, "iiiiii";
    AtomicIIncrement = 232, R_TY, "iii";
    AtomicIDecrement = 233, R_TY, "iii";
    AtomicIAdd = 234, R_TY, "iiii";
    AtomicISub = 235, R_TY, "iiii";
    AtomicSMin = 236, R_TY, "iiii";
    AtomicUMin = 237, R_TY, "iiii";
    AtomicSMax = 238, R_TY, "iiii";
    AtomicUMax = 239, R_TY, "iiii";
    AtomicAnd = 240, R_TY, "iiii";
    AtomicOr = 241, R_TY, "iiii";
    AtomicXor = 242, R_TY, "iiii";
    Phi = 245, R_TY, "i*";
    LoopMerge = 246, R_NONE, "iil";
    SelectionMerge = 247, R_NONE, "ie";
    Label = 248, R_ID, "";
    Branch = 249, R_NONE, "i";
    BranchConditional = 250, R_NONE, "iii?n*";
    Switch = 251, R_NONE, "ii?w";
    Kill = 252, R_NONE, "";
    Return = 253, R_NONE, "";
    ReturnValue = 254, R_NONE, "i";
    Unreachable = 255, R_NONE, "";
    ModuleProcessed = 330, R_NONE, "s";
    ExecutionModeId = 331, R_NONE, "iG";
    GroupNonUniformElect = 333, R_TY, "i";
    GroupNonUniformAll = 334, R_TY, "ii";
    GroupNonUniformAny = 335, R_TY, "ii";
    GroupNonUniformBroadcast = 337, R_TY, "iii";
    GroupNonUniformBroadcastFirst = 338, R_TY, "ii";
    GroupNonUniformBallot = 339, R_TY, "ii";
    GroupNonUniformShuffle = 345, R_TY, "iii";
    GroupNonUniformShuffleXor = 346, R_TY, "iii";
    GroupNonUniformShuffleUp = 347, R_TY, "iii";
    GroupNonUniformShuffleDown = 348, R_TY, "iii";
    GroupNonUniformIAdd = 349, R_TY, "ini?";
    GroupNonUniformFAdd = 350, R_TY, "ini?";
    GroupNonUniformIMul = 351, R_TY, "ini?";
    GroupNonUniformFMul = 352, R_TY, "ini?";
    GroupNonUniformSMin = 353, R_TY, "ini?";
    GroupNonUniformUMin = 354, R_TY, "ini?";
    GroupNonUniformFMin = 355, R_TY, "ini?";
    GroupNonUniformSMax = 356, R_TY, "ini?";
    GroupNonUniformUMax = 357, R_TY, "ini?";
    GroupNonUniformFMax = 358, R_TY, "ini?";
    GroupNonUniformBitwiseAnd = 359, R_TY, "ini?";
    GroupNonUniformBitwiseOr = 360, R_TY, "ini?";
    GroupNonUniformBitwiseXor = 361, R_TY, "ini?";
    GroupNonUniformQuadBroadcast = 365, R_TY, "iii";
    GroupNonUniformQuadSwap = 366, R_TY, "iii";
    CopyLogical = 400, R_TY, "i";
    TerminateInvocation = 4416, R_NONE, "";
    DemoteToHelperInvocation = 5380, R_NONE, "";
    IsHelperInvocationEXT = 5381, R_TY, "";
}

/// the grammar of an opcode (None: unknown to shader)
pub fn op_info(op: u16) -> Option<&'static OpInfo> {
    OPS.binary_search_by_key(&op, |o| o.op).ok().map(|i| &OPS[i])
}

/// the name of v in a (value, name) table
pub fn name(table: &[(u32, &'static str)], v: u32) -> Option<&'static str> {
    table.iter().find(|e| e.0 == v).map(|e| e.1)
}

macro_rules! spv_enum {
    ($(#[$m:meta])* $ty:ident, $tab:ident { $($v:ident = $n:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        #[repr(u32)]
        pub enum $ty { $($v = $n),* }
        /// (value, name) of every variant
        pub const $tab: &[(u32, &str)] = &[$(($n, stringify!($v))),*];
    };
}

spv_enum!(/// capabilities (the ones shader knows by name)
Cap, CAPS {
    Matrix = 0, Shader = 1, Geometry = 2, Tessellation = 3, Addresses = 4, Linkage = 5, Kernel = 6,
    Float16Buffer = 8, Float16 = 9, Float64 = 10, Int64 = 11, Int64Atomics = 12, Int16 = 22,
    TessellationPointSize = 23, GeometryPointSize = 24, ImageGatherExtended = 25,
    StorageImageMultisample = 27, UniformBufferArrayDynamicIndexing = 28,
    SampledImageArrayDynamicIndexing = 29, StorageBufferArrayDynamicIndexing = 30,
    StorageImageArrayDynamicIndexing = 31, ClipDistance = 32, CullDistance = 33, ImageCubeArray = 34,
    SampleRateShading = 35, ImageRect = 36, SampledRect = 37, Int8 = 39, InputAttachment = 40,
    SparseResidency = 41, MinLod = 42, Sampled1D = 43, Image1D = 44, SampledCubeArray = 45,
    SampledBuffer = 46, ImageBuffer = 47, ImageMSArray = 48, StorageImageExtendedFormats = 49,
    ImageQuery = 50, DerivativeControl = 51, InterpolationFunction = 52, TransformFeedback = 53,
    GeometryStreams = 54, StorageImageReadWithoutFormat = 55, StorageImageWriteWithoutFormat = 56,
    MultiViewport = 57, GroupNonUniform = 61, GroupNonUniformVote = 62, GroupNonUniformArithmetic = 63,
    GroupNonUniformBallot = 64, GroupNonUniformShuffle = 65, GroupNonUniformShuffleRelative = 66,
    GroupNonUniformQuad = 68, ShaderLayer = 69, ShaderViewportIndex = 70,
    DrawParameters = 4427, StorageBuffer16BitAccess = 4433, UniformAndStorageBuffer16BitAccess = 4434,
    StoragePushConstant16 = 4435, StorageInputOutput16 = 4436, MultiView = 4439,
    StorageBuffer8BitAccess = 4448, ShaderNonUniform = 5301, RuntimeDescriptorArray = 5302,
    VulkanMemoryModel = 5345, PhysicalStorageBufferAddresses = 5347, DemoteToHelperInvocation = 5379,
    MeshShadingEXT = 5283,
});

spv_enum!(/// storage classes
Sc, SCS {
    UniformConstant = 0, Input = 1, Uniform = 2, Output = 3, Workgroup = 4, CrossWorkgroup = 5,
    Private = 6, Function = 7, Generic = 8, PushConstant = 9, AtomicCounter = 10, Image = 11,
    StorageBuffer = 12, PhysicalStorageBuffer = 5349,
});

spv_enum!(/// decorations
Dec, DECS {
    RelaxedPrecision = 0, SpecId = 1, Block = 2, BufferBlock = 3, RowMajor = 4, ColMajor = 5,
    ArrayStride = 6, MatrixStride = 7, GLSLShared = 8, GLSLPacked = 9, CPacked = 10, BuiltIn = 11,
    NoPerspective = 13, Flat = 14, Patch = 15, Centroid = 16, Sample = 17, Invariant = 18,
    Restrict = 19, Aliased = 20, Volatile = 21, Constant = 22, Coherent = 23, NonWritable = 24,
    NonReadable = 25, Uniform = 26, Location = 30, Component = 31, Index = 32, Binding = 33,
    DescriptorSet = 34, Offset = 35, XfbBuffer = 36, XfbStride = 37, NoContraction = 42,
    InputAttachmentIndex = 43, Alignment = 44, NonUniform = 5300,
});

spv_enum!(/// built-in variables
Bi, BIS {
    Position = 0, PointSize = 1, ClipDistance = 3, CullDistance = 4, VertexId = 5, InstanceId = 6,
    PrimitiveId = 7, InvocationId = 8, Layer = 9, ViewportIndex = 10, FragCoord = 15,
    PointCoord = 16, FrontFacing = 17, SampleId = 18, SamplePosition = 19, SampleMask = 20,
    FragDepth = 22, HelperInvocation = 23, NumWorkgroups = 24, WorkgroupSize = 25, WorkgroupId = 26,
    LocalInvocationId = 27, GlobalInvocationId = 28, LocalInvocationIndex = 29, SubgroupSize = 36,
    NumSubgroups = 38, SubgroupId = 40, SubgroupLocalInvocationId = 41, VertexIndex = 42,
    InstanceIndex = 43, BaseVertex = 4424, BaseInstance = 4425, DrawIndex = 4426, ViewIndex = 4440,
});

spv_enum!(/// execution models
Model, MODELS {
    Vertex = 0, TessellationControl = 1, TessellationEvaluation = 2, Geometry = 3, Fragment = 4,
    GLCompute = 5, Kernel = 6, TaskEXT = 5364, MeshEXT = 5365,
});

spv_enum!(/// execution modes
Mode, MODES {
    Invocations = 0, SpacingEqual = 1, VertexOrderCw = 4, VertexOrderCcw = 5, PixelCenterInteger = 6,
    OriginUpperLeft = 7, OriginLowerLeft = 8, EarlyFragmentTests = 9, PointMode = 10, Xfb = 11,
    DepthReplacing = 12, DepthGreater = 14, DepthLess = 15, DepthUnchanged = 16, LocalSize = 17,
    LocalSizeHint = 18, InputPoints = 19, InputLines = 20, Triangles = 22, Quads = 24, Isolines = 25,
    OutputVertices = 26, OutputPoints = 27, OutputLineStrip = 28, OutputTriangleStrip = 29,
    ContractionOff = 31, SubgroupSize = 35, LocalSizeId = 38, DenormPreserve = 4459,
    DenormFlushToZero = 4460, SignedZeroInfNanPreserve = 4461, RoundingModeRTE = 4462,
    RoundingModeRTZ = 4463,
});

spv_enum!(/// source languages (OpSource)
Lang, LANGS {
    Unknown = 0, ESSL = 1, GLSL = 2, HLSL = 5, WGSL = 10,
});

/// image dimensionality (OpTypeImage)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
#[non_exhaustive]
pub enum Dim {
    D1 = 0,
    D2 = 1,
    D3 = 2,
    Cube = 3,
    Rect = 4,
    Buffer = 5,
    SubpassData = 6,
}
pub const DIMS: &[(u32, &str)] =
    &[(0, "1D"), (1, "2D"), (2, "3D"), (3, "Cube"), (4, "Rect"), (5, "Buffer"), (6, "SubpassData")];

spv_enum!(/// image formats (OpTypeImage; Unknown for sampled images)
Fmt, FMTS {
    Unknown = 0, Rgba32f = 1, Rgba16f = 2, R32f = 3, Rgba8 = 4, Rgba8Snorm = 5, Rg32f = 6, Rg16f = 7,
    R11fG11fB10f = 8, R16f = 9, Rgba16 = 10, Rgb10A2 = 11, Rg16 = 12, Rg8 = 13, R16 = 14, R8 = 15,
    Rgba16Snorm = 16, Rg16Snorm = 17, Rg8Snorm = 18, R16Snorm = 19, R8Snorm = 20, Rgba32i = 21,
    Rgba16i = 22, Rgba8i = 23, R32i = 24, Rg32i = 25, Rg16i = 26, Rg8i = 27, R16i = 28, R8i = 29,
    Rgba32ui = 30, Rgba16ui = 31, Rgba8ui = 32, R32ui = 33, Rgb10a2ui = 34, Rg32ui = 35, Rg16ui = 36,
    Rg8ui = 37, R16ui = 38, R8ui = 39, R64ui = 40, R64i = 41,
});

pub const ADDRESSING: &[(u32, &str)] = &[(0, "Logical"), (1, "Physical32"), (2, "Physical64"), (5348, "PhysicalStorageBuffer64")];
pub const MEMORY_MODELS: &[(u32, &str)] = &[(0, "Simple"), (1, "GLSL450"), (2, "OpenCL"), (3, "Vulkan")];
pub const ACCESS: &[(u32, &str)] = &[(0, "ReadOnly"), (1, "WriteOnly"), (2, "ReadWrite")];

/// scopes (the value of a constant id operand)
pub mod scope {
    pub const CrossDevice: u32 = 0;
    pub const Device: u32 = 1;
    pub const Workgroup: u32 = 2;
    pub const Subgroup: u32 = 3;
    pub const Invocation: u32 = 4;
    pub const QueueFamily: u32 = 5;
}

/// the group operation literal of OpGroupNonUniform{IAdd, FAdd ...}
pub mod group_op {
    pub const Reduce: u32 = 0;
    pub const InclusiveScan: u32 = 1;
    pub const ExclusiveScan: u32 = 2;
}

/// memory semantics bits (the value of a constant id operand)
pub mod sem {
    pub const None: u32 = 0;
    pub const Acquire: u32 = 0x2;
    pub const Release: u32 = 0x4;
    pub const AcquireRelease: u32 = 0x8;
    pub const SequentiallyConsistent: u32 = 0x10;
    pub const UniformMemory: u32 = 0x40;
    pub const SubgroupMemory: u32 = 0x80;
    pub const WorkgroupMemory: u32 = 0x100;
    pub const CrossWorkgroupMemory: u32 = 0x200;
    pub const ImageMemory: u32 = 0x800;
}

// masks: (bit, name, operand words that follow when the bit is set: 'n' literal, 'i' id)
pub const FN_CTL: &[(u32, &str, &str)] = &[(1, "Inline", ""), (2, "DontInline", ""), (4, "Pure", ""), (8, "Const", "")];
pub const SEL_CTL: &[(u32, &str, &str)] = &[(1, "Flatten", ""), (2, "DontFlatten", "")];
pub const LOOP_CTL: &[(u32, &str, &str)] = &[
    (1, "Unroll", ""), (2, "DontUnroll", ""), (4, "DependencyInfinite", ""), (8, "DependencyLength", "n"),
    (0x10, "MinIterations", "n"), (0x20, "MaxIterations", "n"), (0x40, "IterationMultiple", "n"),
    (0x80, "PeelCount", "n"), (0x100, "PartialCount", "n"),
];
pub const MEM_ACCESS: &[(u32, &str, &str)] = &[
    (1, "Volatile", ""), (2, "Aligned", "n"), (4, "Nontemporal", ""), (8, "MakePointerAvailable", "i"),
    (0x10, "MakePointerVisible", "i"), (0x20, "NonPrivatePointer", ""),
];
pub const IMG_OPS: &[(u32, &str, &str)] = &[
    (1, "Bias", "i"), (2, "Lod", "i"), (4, "Grad", "ii"), (8, "ConstOffset", "i"), (0x10, "Offset", "i"),
    (0x20, "ConstOffsets", "i"), (0x40, "Sample", "i"), (0x80, "MinLod", "i"), (0x100, "MakeTexelAvailable", "i"),
    (0x200, "MakeTexelVisible", "i"), (0x400, "NonPrivateTexel", ""), (0x800, "VolatileTexel", ""),
    (0x1000, "SignExtend", ""), (0x2000, "ZeroExtend", ""), (0x4000, "Nontemporal", ""), (0x10000, "Offsets", "i"),
];

/// image operand bits (OpImageSample* and friends)
pub mod img {
    pub const Bias: u32 = 1;
    pub const Lod: u32 = 2;
    pub const Grad: u32 = 4;
    pub const ConstOffset: u32 = 8;
    pub const Sample: u32 = 0x40;
}

/// GLSL.std.450 extended instructions
pub mod glsl {
    pub const Round: u32 = 1;
    pub const RoundEven: u32 = 2;
    pub const Trunc: u32 = 3;
    pub const FAbs: u32 = 4;
    pub const SAbs: u32 = 5;
    pub const FSign: u32 = 6;
    pub const SSign: u32 = 7;
    pub const Floor: u32 = 8;
    pub const Ceil: u32 = 9;
    pub const Fract: u32 = 10;
    pub const Sin: u32 = 13;
    pub const Cos: u32 = 14;
    pub const Tan: u32 = 15;
    pub const Atan2: u32 = 25;
    pub const Pow: u32 = 26;
    pub const Exp: u32 = 27;
    pub const Log: u32 = 28;
    pub const Exp2: u32 = 29;
    pub const Log2: u32 = 30;
    pub const Sqrt: u32 = 31;
    pub const InverseSqrt: u32 = 32;
    pub const FMin: u32 = 37;
    pub const UMin: u32 = 38;
    pub const SMin: u32 = 39;
    pub const FMax: u32 = 40;
    pub const UMax: u32 = 41;
    pub const SMax: u32 = 42;
    pub const FClamp: u32 = 43;
    pub const UClamp: u32 = 44;
    pub const SClamp: u32 = 45;
    pub const FMix: u32 = 46;
    pub const Step: u32 = 48;
    pub const SmoothStep: u32 = 49;
    pub const Fma: u32 = 50;
    pub const Length: u32 = 66;
    pub const Distance: u32 = 67;
    pub const Cross: u32 = 68;
    pub const Normalize: u32 = 69;
    pub const Reflect: u32 = 71;
}

/// the names of GLSL.std.450 (index = instruction number)
pub const GLSL_NAMES: &[&str] = &[
    "", "Round", "RoundEven", "Trunc", "FAbs", "SAbs", "FSign", "SSign", "Floor", "Ceil", "Fract", "Radians",
    "Degrees", "Sin", "Cos", "Tan", "Asin", "Acos", "Atan", "Sinh", "Cosh", "Tanh", "Asinh", "Acosh", "Atanh",
    "Atan2", "Pow", "Exp", "Log", "Exp2", "Log2", "Sqrt", "InverseSqrt", "Determinant", "MatrixInverse", "Modf",
    "ModfStruct", "FMin", "UMin", "SMin", "FMax", "UMax", "SMax", "FClamp", "UClamp", "SClamp", "FMix", "IMix",
    "Step", "SmoothStep", "Fma", "Frexp", "FrexpStruct", "Ldexp", "PackSnorm4x8", "PackUnorm4x8",
    "PackSnorm2x16", "PackUnorm2x16", "PackHalf2x16", "PackDouble2x32", "UnpackSnorm2x16", "UnpackUnorm2x16",
    "UnpackHalf2x16", "UnpackSnorm4x8", "UnpackUnorm4x8", "UnpackDouble2x32", "Length", "Distance", "Cross",
    "Normalize", "FaceForward", "Reflect", "Refract", "FindILsb", "FindSMsb", "FindUMsb",
    "InterpolateAtCentroid", "InterpolateAtSample", "InterpolateAtOffset", "NMin", "NMax", "NClamp",
];
