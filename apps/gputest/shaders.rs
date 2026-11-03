// shaders -- the SPIR-V modules of gputest, written by hand (SPIR-V 1.3, the Vulkan 1.1 level;
// one comment per instruction: the assembly it encodes, ids numbered in order of appearance).
// Checked with spirv-val --target-env vulkan1.1 and vulkan1.3 (Vulkan SDK); lib/gpu does not
// need the SDK.  Conventions of lib/gpu: DescriptorSet = bind group, Binding = binding, one
// push-constant block from offset 0, NDC y up (the backend flips the viewport).
/// compute, 64 threads per workgroup: v[i] = (i * mul + add) ^ (i >> 5) for i < n
/// set 0 binding 0: buffer { uint v[]; }; push constants { uint n, mul, add; }
pub const CS_FILL: &[u32] = &[
    0x07230203, 0x00010300, 0, 39, 0, // magic, SPIR-V 1.3, generator, id bound, schema
    0x20011, 1, // OpCapability Shader
    0x3000e, 0, 1, // OpMemoryModel Logical GLSL450
    0x6000f, 5, 1, 1852399981, 0, 2, // OpEntryPoint GLCompute %main "main" %gid
    0x60010, 1, 17, 64, 1, 1, // OpExecutionMode %main LocalSize 64 1 1
    0x40047, 2, 11, 28, // OpDecorate %gid BuiltIn GlobalInvocationId
    0x40047, 3, 6, 4, // OpDecorate %rta ArrayStride 4
    0x50048, 4, 0, 35, 0, // OpMemberDecorate %B 0 Offset 0
    0x30047, 4, 2, // OpDecorate %B Block
    0x40047, 5, 34, 0, // OpDecorate %buf DescriptorSet 0
    0x40047, 5, 33, 0, // OpDecorate %buf Binding 0
    0x50048, 6, 0, 35, 0, // OpMemberDecorate %P 0 Offset 0
    0x50048, 6, 1, 35, 4, // OpMemberDecorate %P 1 Offset 4
    0x50048, 6, 2, 35, 8, // OpMemberDecorate %P 2 Offset 8
    0x30047, 6, 2, // OpDecorate %P Block
    0x20013, 7, // %void = OpTypeVoid
    0x30021, 8, 7, // %fn = OpTypeFunction %void
    0x40015, 9, 32, 0, // %uint = OpTypeInt 32 0
    0x20014, 10, // %bool = OpTypeBool
    0x40017, 11, 9, 3, // %v3uint = OpTypeVector %uint 3
    0x40020, 12, 1, 11, // %ptr_in_v3 = OpTypePointer Input %v3uint
    0x4003b, 12, 2, 1, // %gid = OpVariable %ptr_in_v3 Input
    0x3001d, 3, 9, // %rta = OpTypeRuntimeArray %uint
    0x3001e, 4, 3, // %B = OpTypeStruct %rta
    0x40020, 13, 12, 4, // %ptr_sb_B = OpTypePointer StorageBuffer %B
    0x4003b, 13, 5, 12, // %buf = OpVariable %ptr_sb_B StorageBuffer
    0x40020, 14, 12, 9, // %ptr_sb_u = OpTypePointer StorageBuffer %uint
    0x5001e, 6, 9, 9, 9, // %P = OpTypeStruct %uint %uint %uint
    0x40020, 15, 9, 6, // %ptr_pc_P = OpTypePointer PushConstant %P
    0x4003b, 15, 16, 9, // %pc = OpVariable %ptr_pc_P PushConstant
    0x40020, 17, 9, 9, // %ptr_pc_u = OpTypePointer PushConstant %uint
    0x4002b, 9, 18, 0, // %c0 = OpConstant %uint 0
    0x4002b, 9, 19, 1, // %c1 = OpConstant %uint 1
    0x4002b, 9, 20, 2, // %c2 = OpConstant %uint 2
    0x4002b, 9, 21, 5, // %c5 = OpConstant %uint 5
    0x50036, 7, 1, 0, 8, // %main = OpFunction %void None %fn
    0x200f8, 22, // %entry = OpLabel
    0x4003d, 11, 23, 2, // %g = OpLoad %v3uint %gid
    0x50051, 9, 24, 23, 0, // %i = OpCompositeExtract %uint %g 0
    0x50041, 17, 25, 16, 18, // %pn = OpAccessChain %ptr_pc_u %pc %c0
    0x4003d, 9, 26, 25, // %n = OpLoad %uint %pn
    0x500b0, 10, 27, 24, 26, // %in = OpULessThan %bool %i %n
    0x300f7, 28, 0, // OpSelectionMerge %end None
    0x400fa, 27, 29, 28, // OpBranchConditional %in %body %end
    0x200f8, 29, // %body = OpLabel
    0x50041, 17, 30, 16, 19, // %pm = OpAccessChain %ptr_pc_u %pc %c1
    0x4003d, 9, 31, 30, // %mul = OpLoad %uint %pm
    0x50041, 17, 32, 16, 20, // %pa = OpAccessChain %ptr_pc_u %pc %c2
    0x4003d, 9, 33, 32, // %add = OpLoad %uint %pa
    0x50084, 9, 34, 24, 31, // %t1 = OpIMul %uint %i %mul
    0x50080, 9, 35, 34, 33, // %t2 = OpIAdd %uint %t1 %add
    0x500c2, 9, 36, 24, 21, // %t3 = OpShiftRightLogical %uint %i %c5
    0x500c6, 9, 37, 35, 36, // %val = OpBitwiseXor %uint %t2 %t3
    0x60041, 14, 38, 5, 18, 24, // %pv = OpAccessChain %ptr_sb_u %buf %c0 %i
    0x3003e, 38, 37, // OpStore %pv %val
    0x200f9, 28, // OpBranch %end
    0x200f8, 28, // %end = OpLabel
    0x100fd, // OpReturn
    0x10038, // OpFunctionEnd
];

/// compute, one thread: v[i] = w -- one word of the indirect arguments from push constants
/// set 0 binding 0: buffer { uint v[]; }; push constants { uint i; uint w; }
/// (one word per dispatch: an index that varies into a push-constant array, v[i] = w[i] with
/// i = gl_GlobalInvocationID.x, reads w[0] on the Samsung Xclipse 540, driver 24.0.560)
pub const CS_ARGS: &[u32] = &[
    0x07230203, 0x00010300, 0, 26, 0, // magic, SPIR-V 1.3, generator, id bound, schema
    0x20011, 1, // OpCapability Shader
    0x3000e, 0, 1, // OpMemoryModel Logical GLSL450
    0x5000f, 5, 1, 1852399981, 0, // OpEntryPoint GLCompute %main "main"
    0x60010, 1, 17, 1, 1, 1, // OpExecutionMode %main LocalSize 1 1 1
    0x40047, 3, 6, 4, // OpDecorate %rta ArrayStride 4
    0x50048, 4, 0, 35, 0, // OpMemberDecorate %B 0 Offset 0
    0x30047, 4, 2, // OpDecorate %B Block
    0x40047, 5, 34, 0, // OpDecorate %buf DescriptorSet 0
    0x40047, 5, 33, 0, // OpDecorate %buf Binding 0
    0x50048, 7, 0, 35, 0, // OpMemberDecorate %P 0 Offset 0
    0x50048, 7, 1, 35, 4, // OpMemberDecorate %P 1 Offset 4
    0x30047, 7, 2, // OpDecorate %P Block
    0x20013, 8, // %void = OpTypeVoid
    0x30021, 9, 8, // %fn = OpTypeFunction %void
    0x40015, 10, 32, 0, // %uint = OpTypeInt 32 0
    0x3001d, 3, 10, // %rta = OpTypeRuntimeArray %uint
    0x3001e, 4, 3, // %B = OpTypeStruct %rta
    0x40020, 13, 12, 4, // %ptr_sb_B = OpTypePointer StorageBuffer %B
    0x4003b, 13, 5, 12, // %buf = OpVariable %ptr_sb_B StorageBuffer
    0x40020, 14, 12, 10, // %ptr_sb_u = OpTypePointer StorageBuffer %uint
    0x4002b, 10, 15, 0, // %c0 = OpConstant %uint 0
    0x4002b, 10, 16, 1, // %c1 = OpConstant %uint 1
    0x4001e, 7, 10, 10, // %P = OpTypeStruct %uint %uint
    0x40020, 17, 9, 7, // %ptr_pc_P = OpTypePointer PushConstant %P
    0x4003b, 17, 18, 9, // %pc = OpVariable %ptr_pc_P PushConstant
    0x40020, 19, 9, 10, // %ptr_pc_u = OpTypePointer PushConstant %uint
    0x50036, 8, 1, 0, 9, // %main = OpFunction %void None %fn
    0x200f8, 20, // %entry = OpLabel
    0x50041, 19, 21, 18, 15, // %pi = OpAccessChain %ptr_pc_u %pc %c0
    0x4003d, 10, 22, 21, // %i = OpLoad %uint %pi
    0x50041, 19, 23, 18, 16, // %pw = OpAccessChain %ptr_pc_u %pc %c1
    0x4003d, 10, 24, 23, // %w = OpLoad %uint %pw
    0x60041, 14, 25, 5, 15, 22, // %pv = OpAccessChain %ptr_sb_u %buf %c0 %i
    0x3003e, 25, 24, // OpStore %pv %w
    0x100fd, // OpReturn
    0x10038, // OpFunctionEnd
];

/// vertex, vertex pulling: gl_Position = v[2 * i], colour (location 0) = v[2 * i + 1], i = gl_VertexIndex
/// set 0 binding 0: readonly buffer { vec4 v[]; }; no vertex input
pub const VS_PULL: &[u32] = &[
    0x07230203, 0x00010300, 0, 28, 0, // magic, SPIR-V 1.3, generator, id bound, schema
    0x20011, 1, // OpCapability Shader
    0x3000e, 0, 1, // OpMemoryModel Logical GLSL450
    0x8000f, 0, 1, 1852399981, 0, 2, 3, 4, // OpEntryPoint Vertex %main "main" %vid %pos %col
    0x40047, 2, 11, 42, // OpDecorate %vid BuiltIn VertexIndex
    0x40047, 3, 11, 0, // OpDecorate %pos BuiltIn Position
    0x40047, 4, 30, 0, // OpDecorate %col Location 0
    0x40047, 5, 6, 16, // OpDecorate %rta ArrayStride 16
    0x40048, 6, 0, 24, // OpMemberDecorate %V 0 NonWritable
    0x50048, 6, 0, 35, 0, // OpMemberDecorate %V 0 Offset 0
    0x30047, 6, 2, // OpDecorate %V Block
    0x40047, 7, 34, 0, // OpDecorate %buf DescriptorSet 0
    0x40047, 7, 33, 0, // OpDecorate %buf Binding 0
    0x20013, 8, // %void = OpTypeVoid
    0x30021, 9, 8, // %fn = OpTypeFunction %void
    0x40015, 10, 32, 0, // %uint = OpTypeInt 32 0
    0x30016, 11, 32, // %float = OpTypeFloat 32
    0x40017, 12, 11, 4, // %v4 = OpTypeVector %float 4
    0x40020, 13, 1, 10, // %ptr_in_u = OpTypePointer Input %uint
    0x4003b, 13, 2, 1, // %vid = OpVariable %ptr_in_u Input
    0x40020, 14, 3, 12, // %ptr_out_v4 = OpTypePointer Output %v4
    0x4003b, 14, 3, 3, // %pos = OpVariable %ptr_out_v4 Output
    0x4003b, 14, 4, 3, // %col = OpVariable %ptr_out_v4 Output
    0x3001d, 5, 12, // %rta = OpTypeRuntimeArray %v4
    0x3001e, 6, 5, // %V = OpTypeStruct %rta
    0x40020, 15, 12, 6, // %ptr_sb_V = OpTypePointer StorageBuffer %V
    0x4003b, 15, 7, 12, // %buf = OpVariable %ptr_sb_V StorageBuffer
    0x40020, 16, 12, 12, // %ptr_sb_v4 = OpTypePointer StorageBuffer %v4
    0x4002b, 10, 17, 0, // %c0 = OpConstant %uint 0
    0x4002b, 10, 18, 1, // %c1 = OpConstant %uint 1
    0x4002b, 10, 19, 2, // %c2 = OpConstant %uint 2
    0x50036, 8, 1, 0, 9, // %main = OpFunction %void None %fn
    0x200f8, 20, // %entry = OpLabel
    0x4003d, 10, 21, 2, // %i = OpLoad %uint %vid
    0x50084, 10, 22, 21, 19, // %i2 = OpIMul %uint %i %c2
    0x60041, 16, 23, 7, 17, 22, // %pp = OpAccessChain %ptr_sb_v4 %buf %c0 %i2
    0x4003d, 12, 24, 23, // %p = OpLoad %v4 %pp
    0x3003e, 3, 24, // OpStore %pos %p
    0x50080, 10, 25, 22, 18, // %i3 = OpIAdd %uint %i2 %c1
    0x60041, 16, 26, 7, 17, 25, // %pk = OpAccessChain %ptr_sb_v4 %buf %c0 %i3
    0x4003d, 12, 27, 26, // %k = OpLoad %v4 %pk
    0x3003e, 4, 27, // OpStore %col %k
    0x100fd, // OpReturn
    0x10038, // OpFunctionEnd
];

/// fragment: out colour (location 0) = in colour (location 0)
pub const FS_COLOR: &[u32] = &[
    0x07230203, 0x00010300, 0, 12, 0, // magic, SPIR-V 1.3, generator, id bound, schema
    0x20011, 1, // OpCapability Shader
    0x3000e, 0, 1, // OpMemoryModel Logical GLSL450
    0x7000f, 4, 1, 1852399981, 0, 2, 3, // OpEntryPoint Fragment %main "main" %in %out
    0x30010, 1, 7, // OpExecutionMode %main OriginUpperLeft
    0x40047, 2, 30, 0, // OpDecorate %in Location 0
    0x40047, 3, 30, 0, // OpDecorate %out Location 0
    0x20013, 4, // %void = OpTypeVoid
    0x30021, 5, 4, // %fn = OpTypeFunction %void
    0x30016, 6, 32, // %float = OpTypeFloat 32
    0x40017, 7, 6, 4, // %v4 = OpTypeVector %float 4
    0x40020, 8, 1, 7, // %ptr_in_v4 = OpTypePointer Input %v4
    0x4003b, 8, 2, 1, // %in = OpVariable %ptr_in_v4 Input
    0x40020, 9, 3, 7, // %ptr_out_v4 = OpTypePointer Output %v4
    0x4003b, 9, 3, 3, // %out = OpVariable %ptr_out_v4 Output
    0x50036, 4, 1, 0, 5, // %main = OpFunction %void None %fn
    0x200f8, 10, // %entry = OpLabel
    0x4003d, 7, 11, 2, // %c = OpLoad %v4 %in
    0x3003e, 3, 11, // OpStore %out %c
    0x100fd, // OpReturn
    0x10038, // OpFunctionEnd
];

/// compute, one thread per workgroup k (Voll): o[2k] = bufs[k].v[0], o[2k + 1] = texelFetch(texs[k], (0, 0), 0).x
/// set 0 = the Bindless table (binding 0: utexture2D texs[], binding 2: buffer { uint v[]; } bufs[]);
/// set 1 binding 0: buffer { uint o[]; }
pub const CS_BINDLESS: &[u32] = &[
    0x07230203, 0x00010300, 0, 42, 0, // magic, SPIR-V 1.3, generator, id bound, schema
    0x20011, 1, // OpCapability Shader
    0x20011, 29, // OpCapability SampledImageArrayDynamicIndexing
    0x20011, 30, // OpCapability StorageBufferArrayDynamicIndexing
    0x20011, 5302, // OpCapability RuntimeDescriptorArray
    0x8000a, 1599492179, 1599363141, 1668506980, 1953524082, 1767862895, 2019910766, 6778473, // OpExtension "SPV_EXT_descriptor_indexing"
    0x3000e, 0, 1, // OpMemoryModel Logical GLSL450
    0x6000f, 5, 1, 1852399981, 0, 2, // OpEntryPoint GLCompute %main "main" %wg
    0x60010, 1, 17, 1, 1, 1, // OpExecutionMode %main LocalSize 1 1 1
    0x40047, 2, 11, 26, // OpDecorate %wg BuiltIn WorkgroupId
    0x40047, 3, 6, 4, // OpDecorate %rta ArrayStride 4
    0x50048, 4, 0, 35, 0, // OpMemberDecorate %B 0 Offset 0
    0x30047, 4, 2, // OpDecorate %B Block
    0x40047, 5, 34, 0, // OpDecorate %bufs DescriptorSet 0
    0x40047, 5, 33, 2, // OpDecorate %bufs Binding 2
    0x40047, 6, 34, 0, // OpDecorate %texs DescriptorSet 0
    0x40047, 6, 33, 0, // OpDecorate %texs Binding 0
    0x40047, 7, 34, 1, // OpDecorate %out DescriptorSet 1
    0x40047, 7, 33, 0, // OpDecorate %out Binding 0
    0x20013, 8, // %void = OpTypeVoid
    0x30021, 9, 8, // %fn = OpTypeFunction %void
    0x40015, 10, 32, 0, // %uint = OpTypeInt 32 0
    0x40015, 11, 32, 1, // %int = OpTypeInt 32 1
    0x40017, 12, 11, 2, // %v2int = OpTypeVector %int 2
    0x40017, 13, 10, 3, // %v3uint = OpTypeVector %uint 3
    0x40017, 14, 10, 4, // %v4uint = OpTypeVector %uint 4
    0x40020, 15, 1, 13, // %ptr_in_v3 = OpTypePointer Input %v3uint
    0x4003b, 15, 2, 1, // %wg = OpVariable %ptr_in_v3 Input
    0x3001d, 3, 10, // %rta = OpTypeRuntimeArray %uint
    0x3001e, 4, 3, // %B = OpTypeStruct %rta
    0x3001d, 16, 4, // %rta_B = OpTypeRuntimeArray %B
    0x40020, 17, 12, 16, // %ptr_sb_rtaB = OpTypePointer StorageBuffer %rta_B
    0x4003b, 17, 5, 12, // %bufs = OpVariable %ptr_sb_rtaB StorageBuffer
    0x40020, 18, 12, 4, // %ptr_sb_B = OpTypePointer StorageBuffer %B
    0x4003b, 18, 7, 12, // %out = OpVariable %ptr_sb_B StorageBuffer
    0x40020, 19, 12, 10, // %ptr_sb_u = OpTypePointer StorageBuffer %uint
    0x90019, 20, 10, 1, 0, 0, 0, 1, 0, // %img = OpTypeImage %uint 2D 0 0 0 1 Unknown
    0x3001d, 21, 20, // %rta_img = OpTypeRuntimeArray %img
    0x40020, 22, 0, 21, // %ptr_uc_rtai = OpTypePointer UniformConstant %rta_img
    0x4003b, 22, 6, 0, // %texs = OpVariable %ptr_uc_rtai UniformConstant
    0x40020, 23, 0, 20, // %ptr_uc_img = OpTypePointer UniformConstant %img
    0x4002b, 10, 24, 0, // %c0 = OpConstant %uint 0
    0x4002b, 10, 25, 1, // %c1 = OpConstant %uint 1
    0x4002b, 10, 26, 2, // %c2 = OpConstant %uint 2
    0x4002b, 11, 27, 0, // %ci0 = OpConstant %int 0
    0x5002c, 12, 28, 27, 27, // %coord = OpConstantComposite %v2int %ci0 %ci0
    0x50036, 8, 1, 0, 9, // %main = OpFunction %void None %fn
    0x200f8, 29, // %entry = OpLabel
    0x4003d, 13, 30, 2, // %w = OpLoad %v3uint %wg
    0x50051, 10, 31, 30, 0, // %k = OpCompositeExtract %uint %w 0
    0x70041, 19, 32, 5, 31, 24, 24, // %pa = OpAccessChain %ptr_sb_u %bufs %k %c0 %c0
    0x4003d, 10, 33, 32, // %a = OpLoad %uint %pa
    0x50084, 10, 34, 31, 26, // %k2 = OpIMul %uint %k %c2
    0x60041, 19, 35, 7, 24, 34, // %po = OpAccessChain %ptr_sb_u %out %c0 %k2
    0x3003e, 35, 33, // OpStore %po %a
    0x50041, 23, 36, 6, 31, // %pt = OpAccessChain %ptr_uc_img %texs %k
    0x4003d, 20, 37, 36, // %t = OpLoad %img %pt
    0x7005f, 14, 38, 37, 28, 2, 27, // %f = OpImageFetch %v4uint %t %coord Lod %ci0
    0x50051, 10, 39, 38, 0, // %fx = OpCompositeExtract %uint %f 0
    0x50080, 10, 40, 34, 25, // %k3 = OpIAdd %uint %k2 %c1
    0x60041, 19, 41, 7, 24, 40, // %po2 = OpAccessChain %ptr_sb_u %out %c0 %k3
    0x3003e, 41, 39, // OpStore %po2 %fx
    0x100fd, // OpReturn
    0x10038, // OpFunctionEnd
];
