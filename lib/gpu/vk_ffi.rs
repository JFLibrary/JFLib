// vk_ffi -- the Vulkan declarations of lib/gpu's backend, written by hand from vulkan_core.h,
// vulkan_win32.h and vulkan_android.h: only the handles, constants, structures and commands the backend uses.
//
//   constants      the C names without "VK_" (STRUCTURE_TYPE_BUFFER_CREATE_INFO, FORMAT_D32_SFLOAT)
//   structures     the C names without "Vk", fields in snake_case (s_type, p_next, ...);
//                  every one #[repr(C)], Copy, and Zero (all-zero is a valid value)
//   LoaderFns      vkGetInstanceProcAddr(NULL, ..): the commands before an instance exists
//   InstanceFns    vkGetInstanceProcAddr(instance, ..)
//   DeviceFns      vkGetDeviceProcAddr(device, ..)
//
// A table entry lists the command's names in the order they are tried (the core name first,
// then its KHR alias: vkCmdBeginRendering of Vulkan 1.3 / VK_KHR_dynamic_rendering); `req`
// entries must exist (load fails with the entry's name), `opt` entries are Option.
// Dispatchable handles (instance, physical device, device, queue, command buffer) are
// pointers (usize), the others 64-bit values (Handle).  The layout checks at the end compare
// sizes and offsets with the C headers (x86-64, values from clang on vulkan_core.h).
#![allow(dead_code)]

use core::ffi::c_void;

pub type VkResult = i32;
pub type Bool32 = u32;
pub type Instance = usize;
pub type PhysicalDevice = usize;
pub type Device = usize;
pub type Queue = usize;
pub type CommandBuffer = usize;
pub type Handle = u64;
pub type Pfn = usize; // a command's address (0 = missing)

pub const NULL_HANDLE: Handle = 0;
pub const WHOLE_SIZE: u64 = !0;
pub const QUEUE_FAMILY_IGNORED: u32 = !0;
pub const REMAINING_MIP_LEVELS: u32 = !0;
pub const REMAINING_ARRAY_LAYERS: u32 = !0;

pub const fn make_api_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}
pub const API_VERSION_1_1: u32 = make_api_version(1, 1, 0);
pub const API_VERSION_1_2: u32 = make_api_version(1, 2, 0);
pub const API_VERSION_1_3: u32 = make_api_version(1, 3, 0);
pub const fn api_major(v: u32) -> u32 {
    (v >> 22) & 0x7F
}
pub const fn api_minor(v: u32) -> u32 {
    (v >> 12) & 0x3FF
}
pub const fn api_patch(v: u32) -> u32 {
    v & 0xFFF
}

// ---------------------------------------------------------------- VkResult
pub const SUCCESS: VkResult = 0;
pub const NOT_READY: VkResult = 1;
pub const TIMEOUT: VkResult = 2;
pub const INCOMPLETE: VkResult = 5;
pub const ERROR_OUT_OF_HOST_MEMORY: VkResult = -1;
pub const ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub const ERROR_INITIALIZATION_FAILED: VkResult = -3;
pub const ERROR_DEVICE_LOST: VkResult = -4;
pub const ERROR_MEMORY_MAP_FAILED: VkResult = -5;
pub const ERROR_LAYER_NOT_PRESENT: VkResult = -6;
pub const ERROR_EXTENSION_NOT_PRESENT: VkResult = -7;
pub const ERROR_FEATURE_NOT_PRESENT: VkResult = -8;
pub const ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
pub const ERROR_TOO_MANY_OBJECTS: VkResult = -10;
pub const ERROR_FORMAT_NOT_SUPPORTED: VkResult = -11;
pub const ERROR_FRAGMENTED_POOL: VkResult = -12;
pub const ERROR_OUT_OF_POOL_MEMORY: VkResult = -1000069000;
pub const ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub const ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;
pub const SUBOPTIMAL_KHR: VkResult = 1000001003;
pub const ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;

// ---------------------------------------------------------------- dmabuf (vk_dmabuf.rs)
pub const STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO: u32 = 1000072001;
pub const STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO: u32 = 1000072002;
pub const STRUCTURE_TYPE_MEMORY_GET_FD_INFO_KHR: u32 = 1000074002;
pub const STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO: u32 = 1000127001;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO: u32 = 1000071000;
pub const STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES: u32 = 1000071001;
pub const STRUCTURE_TYPE_FORMAT_PROPERTIES_2: u32 = 1000059002;
pub const STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2: u32 = 1000059003;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2: u32 = 1000059004;
pub const STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT: u32 = 1000158000;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT: u32 = 1000158002;
pub const STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_LIST_CREATE_INFO_EXT: u32 = 1000158003;
pub const STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_PROPERTIES_EXT: u32 = 1000158005;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT: u32 = 1000353000;
pub const IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: u32 = 1000158000;
pub const EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT: u32 = 0x200;
pub const EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT: u32 = 2;
pub const IMAGE_ASPECT_MEMORY_PLANE_0_BIT_EXT: u32 = 0x80;
pub const QUEUE_FAMILY_EXTERNAL: u32 = !1;
pub const QUEUE_FAMILY_FOREIGN_EXT: u32 = !2;

// ---------------------------------------------------------------- VkStructureType
pub const STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
pub const STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
pub const STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: u32 = 2;
pub const STRUCTURE_TYPE_DEVICE_CREATE_INFO: u32 = 3;
pub const STRUCTURE_TYPE_SUBMIT_INFO: u32 = 4;
pub const STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: u32 = 5;
pub const STRUCTURE_TYPE_MAPPED_MEMORY_RANGE: u32 = 6;
pub const STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: u32 = 9;
pub const STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO: u32 = 11;
pub const STRUCTURE_TYPE_BUFFER_CREATE_INFO: u32 = 12;
pub const STRUCTURE_TYPE_IMAGE_CREATE_INFO: u32 = 14;
pub const STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: u32 = 15;
pub const STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: u32 = 16;
pub const STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: u32 = 18;
pub const STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: u32 = 19;
pub const STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: u32 = 20;
pub const STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: u32 = 22;
pub const STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: u32 = 23;
pub const STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: u32 = 24;
pub const STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: u32 = 25;
pub const STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: u32 = 26;
pub const STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: u32 = 27;
pub const STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: u32 = 28;
pub const STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: u32 = 29;
pub const STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: u32 = 30;
pub const STRUCTURE_TYPE_SAMPLER_CREATE_INFO: u32 = 31;
pub const STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: u32 = 32;
pub const STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: u32 = 33;
pub const STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: u32 = 34;
pub const STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: u32 = 35;
pub const STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: u32 = 39;
pub const STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: u32 = 40;
pub const STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: u32 = 42;
pub const STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: u32 = 45;
pub const STRUCTURE_TYPE_MEMORY_BARRIER: u32 = 46;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES: u32 = 49;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES: u32 = 51;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES: u32 = 53;
pub const STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: u32 = 1000001000;
pub const STRUCTURE_TYPE_PRESENT_INFO_KHR: u32 = 1000001001;
pub const STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR: u32 = 1000009000;
pub const STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR: u32 = 1000008000;
pub const STRUCTURE_TYPE_RENDERING_INFO: u32 = 1000044000;
pub const STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO: u32 = 1000044001;
pub const STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO: u32 = 1000044002;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES: u32 = 1000044003;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2: u32 = 1000059000;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2: u32 = 1000059001;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_PROPERTIES: u32 = 1000094000;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_PROPERTIES: u32 = 1000225000;
pub const STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO: u32 = 1000225001;
pub const STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CALLBACK_DATA_EXT: u32 = 1000128003;
pub const STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT: u32 = 1000128004;
pub const STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO: u32 = 1000161000;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_PROPERTIES: u32 = 1000161002;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES: u32 = 1000196000;
pub const STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES: u32 = 1000207000;
pub const STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO: u32 = 1000207002;
pub const STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO: u32 = 1000207003;
pub const STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO: u32 = 1000207004;
pub const STRUCTURE_TYPE_VALIDATION_FEATURES_EXT: u32 = 1000247000;
pub const STRUCTURE_TYPE_LAYER_SETTINGS_CREATE_INFO_EXT: u32 = 1000496000;

// ---------------------------------------------------------------- enums and flags
pub const PHYSICAL_DEVICE_TYPE_OTHER: u32 = 0;
pub const PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: u32 = 1;
pub const PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: u32 = 2;
pub const PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: u32 = 3;
pub const PHYSICAL_DEVICE_TYPE_CPU: u32 = 4;

pub const QUEUE_GRAPHICS_BIT: u32 = 1;
pub const QUEUE_COMPUTE_BIT: u32 = 2;

pub const MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 1;
pub const MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 2;
pub const MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 4;
pub const MEMORY_PROPERTY_HOST_CACHED_BIT: u32 = 8;
pub const MEMORY_PROPERTY_LAZILY_ALLOCATED_BIT: u32 = 0x10;
pub const MEMORY_PROPERTY_PROTECTED_BIT: u32 = 0x20;
pub const MEMORY_PROPERTY_DEVICE_COHERENT_BIT_AMD: u32 = 0x40;

pub const BUFFER_USAGE_TRANSFER_SRC_BIT: u32 = 1;
pub const BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 2;
pub const BUFFER_USAGE_UNIFORM_BUFFER_BIT: u32 = 0x10;
pub const BUFFER_USAGE_STORAGE_BUFFER_BIT: u32 = 0x20;
pub const BUFFER_USAGE_INDEX_BUFFER_BIT: u32 = 0x40;
pub const BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 0x80;
pub const BUFFER_USAGE_INDIRECT_BUFFER_BIT: u32 = 0x100;

pub const IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 1;
pub const IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 2;
pub const IMAGE_USAGE_SAMPLED_BIT: u32 = 4;
pub const IMAGE_USAGE_STORAGE_BIT: u32 = 8;
pub const IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 0x10;
pub const IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x20;

pub const IMAGE_CREATE_CUBE_COMPATIBLE_BIT: u32 = 0x10;
pub const IMAGE_TYPE_1D: u32 = 0;
pub const IMAGE_TYPE_2D: u32 = 1;
pub const IMAGE_TYPE_3D: u32 = 2;
pub const IMAGE_VIEW_TYPE_1D: u32 = 0;
pub const IMAGE_VIEW_TYPE_2D: u32 = 1;
pub const IMAGE_VIEW_TYPE_3D: u32 = 2;
pub const IMAGE_VIEW_TYPE_CUBE: u32 = 3;
pub const IMAGE_VIEW_TYPE_2D_ARRAY: u32 = 5;
pub const IMAGE_VIEW_TYPE_CUBE_ARRAY: u32 = 6;
pub const IMAGE_TILING_OPTIMAL: u32 = 0;
pub const SHARING_MODE_EXCLUSIVE: u32 = 0;
pub const SHARING_MODE_CONCURRENT: u32 = 1;
pub const SAMPLE_COUNT_1_BIT: u32 = 1;

pub const IMAGE_ASPECT_COLOR_BIT: u32 = 1;
pub const IMAGE_ASPECT_DEPTH_BIT: u32 = 2;
pub const IMAGE_ASPECT_STENCIL_BIT: u32 = 4;

pub const IMAGE_LAYOUT_UNDEFINED: u32 = 0;
pub const IMAGE_LAYOUT_GENERAL: u32 = 1;
pub const IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: u32 = 2;
pub const IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: u32 = 3;
pub const IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL: u32 = 4;
pub const IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: u32 = 5;
pub const IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: u32 = 6;
pub const IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: u32 = 7;
pub const IMAGE_LAYOUT_PRESENT_SRC_KHR: u32 = 1000001002;

pub const FORMAT_UNDEFINED: u32 = 0;
pub const FORMAT_R8_UNORM: u32 = 9;
pub const FORMAT_R8G8_UNORM: u32 = 16;
pub const FORMAT_R8G8B8A8_UNORM: u32 = 37;
pub const FORMAT_R8G8B8A8_SRGB: u32 = 43;
pub const FORMAT_B8G8R8A8_UNORM: u32 = 44;
pub const FORMAT_B8G8R8A8_SRGB: u32 = 50;
pub const FORMAT_A2B10G10R10_UNORM_PACK32: u32 = 64;
pub const FORMAT_R16_SFLOAT: u32 = 76;
pub const FORMAT_R16G16_SFLOAT: u32 = 83;
pub const FORMAT_R16G16B16A16_SFLOAT: u32 = 97;
pub const FORMAT_R32_UINT: u32 = 98;
pub const FORMAT_R32_SINT: u32 = 99;
pub const FORMAT_R32_SFLOAT: u32 = 100;
pub const FORMAT_R32G32_UINT: u32 = 101;
pub const FORMAT_R32G32_SFLOAT: u32 = 103;
pub const FORMAT_R32G32B32_SFLOAT: u32 = 106;
pub const FORMAT_R32G32B32A32_UINT: u32 = 107;
pub const FORMAT_R32G32B32A32_SFLOAT: u32 = 109;
pub const FORMAT_B10G11R11_UFLOAT_PACK32: u32 = 122;
pub const FORMAT_D16_UNORM: u32 = 124;
pub const FORMAT_D32_SFLOAT: u32 = 126;
pub const FORMAT_D24_UNORM_S8_UINT: u32 = 129;
pub const FORMAT_D32_SFLOAT_S8_UINT: u32 = 130;
pub const FORMAT_BC1_RGBA_UNORM_BLOCK: u32 = 133;
pub const FORMAT_BC1_RGBA_SRGB_BLOCK: u32 = 134;
pub const FORMAT_BC3_UNORM_BLOCK: u32 = 137;
pub const FORMAT_BC3_SRGB_BLOCK: u32 = 138;
pub const FORMAT_BC4_UNORM_BLOCK: u32 = 139;
pub const FORMAT_BC5_UNORM_BLOCK: u32 = 141;
pub const FORMAT_BC7_UNORM_BLOCK: u32 = 145;
pub const FORMAT_BC7_SRGB_BLOCK: u32 = 146;

pub const FORMAT_FEATURE_SAMPLED_IMAGE_BIT: u32 = 1;
pub const FORMAT_FEATURE_STORAGE_IMAGE_BIT: u32 = 2;
pub const FORMAT_FEATURE_COLOR_ATTACHMENT_BIT: u32 = 0x80;
pub const FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT: u32 = 0x100;
pub const FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x200;
pub const FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT: u32 = 0x1000;

pub const COMPONENT_SWIZZLE_IDENTITY: u32 = 0;

pub const FILTER_NEAREST: u32 = 0;
pub const FILTER_LINEAR: u32 = 1;
pub const SAMPLER_MIPMAP_MODE_NEAREST: u32 = 0;
pub const SAMPLER_MIPMAP_MODE_LINEAR: u32 = 1;
pub const SAMPLER_ADDRESS_MODE_REPEAT: u32 = 0;
pub const SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT: u32 = 1;
pub const SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: u32 = 2;
pub const BORDER_COLOR_FLOAT_TRANSPARENT_BLACK: u32 = 0;

pub const DESCRIPTOR_TYPE_SAMPLER: u32 = 0;
pub const DESCRIPTOR_TYPE_SAMPLED_IMAGE: u32 = 2;
pub const DESCRIPTOR_TYPE_STORAGE_IMAGE: u32 = 3;
pub const DESCRIPTOR_TYPE_UNIFORM_BUFFER: u32 = 6;
pub const DESCRIPTOR_TYPE_STORAGE_BUFFER: u32 = 7;

pub const DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: u32 = 1;
pub const DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT: u32 = 2;
pub const DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT: u32 = 2;
pub const DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT: u32 = 1;
pub const DESCRIPTOR_BINDING_UPDATE_UNUSED_WHILE_PENDING_BIT: u32 = 2;
pub const DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT: u32 = 4;

pub const SHADER_STAGE_VERTEX_BIT: u32 = 1;
pub const SHADER_STAGE_FRAGMENT_BIT: u32 = 0x10;
pub const SHADER_STAGE_COMPUTE_BIT: u32 = 0x20;
pub const SHADER_STAGE_ALL: u32 = 0x7FFF_FFFF;

pub const PIPELINE_BIND_POINT_GRAPHICS: u32 = 0;
pub const PIPELINE_BIND_POINT_COMPUTE: u32 = 1;

pub const PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 1;
pub const PIPELINE_STAGE_DRAW_INDIRECT_BIT: u32 = 2;
pub const PIPELINE_STAGE_VERTEX_INPUT_BIT: u32 = 4;
pub const PIPELINE_STAGE_VERTEX_SHADER_BIT: u32 = 8;
pub const PIPELINE_STAGE_FRAGMENT_SHADER_BIT: u32 = 0x80;
pub const PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: u32 = 0x100;
pub const PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: u32 = 0x200;
pub const PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 0x400;
pub const PIPELINE_STAGE_COMPUTE_SHADER_BIT: u32 = 0x800;
pub const PIPELINE_STAGE_TRANSFER_BIT: u32 = 0x1000;
pub const PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: u32 = 0x2000;
pub const PIPELINE_STAGE_HOST_BIT: u32 = 0x4000;
pub const PIPELINE_STAGE_ALL_COMMANDS_BIT: u32 = 0x10000;

pub const ACCESS_INDIRECT_COMMAND_READ_BIT: u32 = 1;
pub const ACCESS_INDEX_READ_BIT: u32 = 2;
pub const ACCESS_VERTEX_ATTRIBUTE_READ_BIT: u32 = 4;
pub const ACCESS_UNIFORM_READ_BIT: u32 = 8;
pub const ACCESS_SHADER_READ_BIT: u32 = 0x20;
pub const ACCESS_SHADER_WRITE_BIT: u32 = 0x40;
pub const ACCESS_COLOR_ATTACHMENT_READ_BIT: u32 = 0x80;
pub const ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 0x100;
pub const ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: u32 = 0x200;
pub const ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: u32 = 0x400;
pub const ACCESS_TRANSFER_READ_BIT: u32 = 0x800;
pub const ACCESS_TRANSFER_WRITE_BIT: u32 = 0x1000;
pub const ACCESS_HOST_READ_BIT: u32 = 0x2000;
pub const ACCESS_HOST_WRITE_BIT: u32 = 0x4000;
pub const ACCESS_MEMORY_READ_BIT: u32 = 0x8000;
pub const ACCESS_MEMORY_WRITE_BIT: u32 = 0x10000;

pub const COMMAND_POOL_CREATE_TRANSIENT_BIT: u32 = 1;
pub const COMMAND_BUFFER_LEVEL_PRIMARY: u32 = 0;
pub const COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 1;

pub const QUERY_TYPE_TIMESTAMP: u32 = 2;
pub const QUERY_RESULT_64_BIT: u32 = 1;
pub const QUERY_RESULT_WITH_AVAILABILITY_BIT: u32 = 4;

pub const SEMAPHORE_TYPE_TIMELINE: u32 = 1;

pub const PRIMITIVE_TOPOLOGY_POINT_LIST: u32 = 0;
pub const PRIMITIVE_TOPOLOGY_LINE_LIST: u32 = 1;
pub const PRIMITIVE_TOPOLOGY_LINE_STRIP: u32 = 2;
pub const PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: u32 = 3;
pub const PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP: u32 = 4;
pub const POLYGON_MODE_FILL: u32 = 0;
pub const CULL_MODE_NONE: u32 = 0;
pub const CULL_MODE_FRONT_BIT: u32 = 1;
pub const CULL_MODE_BACK_BIT: u32 = 2;
pub const FRONT_FACE_COUNTER_CLOCKWISE: u32 = 0;
pub const FRONT_FACE_CLOCKWISE: u32 = 1;
pub const COMPARE_OP_NEVER: u32 = 0;
pub const COMPARE_OP_LESS: u32 = 1;
pub const COMPARE_OP_EQUAL: u32 = 2;
pub const COMPARE_OP_LESS_OR_EQUAL: u32 = 3;
pub const COMPARE_OP_GREATER: u32 = 4;
pub const COMPARE_OP_NOT_EQUAL: u32 = 5;
pub const COMPARE_OP_GREATER_OR_EQUAL: u32 = 6;
pub const COMPARE_OP_ALWAYS: u32 = 7;
pub const BLEND_FACTOR_ZERO: u32 = 0;
pub const BLEND_FACTOR_ONE: u32 = 1;
pub const BLEND_FACTOR_SRC_COLOR: u32 = 2;
pub const BLEND_FACTOR_ONE_MINUS_SRC_COLOR: u32 = 3;
pub const BLEND_FACTOR_DST_COLOR: u32 = 4;
pub const BLEND_FACTOR_ONE_MINUS_DST_COLOR: u32 = 5;
pub const BLEND_FACTOR_SRC_ALPHA: u32 = 6;
pub const BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: u32 = 7;
pub const BLEND_FACTOR_DST_ALPHA: u32 = 8;
pub const BLEND_FACTOR_ONE_MINUS_DST_ALPHA: u32 = 9;
pub const BLEND_OP_ADD: u32 = 0;
pub const BLEND_OP_SUBTRACT: u32 = 1;
pub const BLEND_OP_REVERSE_SUBTRACT: u32 = 2;
pub const BLEND_OP_MIN: u32 = 3;
pub const BLEND_OP_MAX: u32 = 4;
pub const DYNAMIC_STATE_VIEWPORT: u32 = 0;
pub const DYNAMIC_STATE_SCISSOR: u32 = 1;
pub const VERTEX_INPUT_RATE_VERTEX: u32 = 0;
pub const VERTEX_INPUT_RATE_INSTANCE: u32 = 1;
pub const INDEX_TYPE_UINT16: u32 = 0;
pub const INDEX_TYPE_UINT32: u32 = 1;
pub const ATTACHMENT_LOAD_OP_LOAD: u32 = 0;
pub const ATTACHMENT_LOAD_OP_CLEAR: u32 = 1;
pub const ATTACHMENT_LOAD_OP_DONT_CARE: u32 = 2;
pub const ATTACHMENT_STORE_OP_STORE: u32 = 0;
pub const ATTACHMENT_STORE_OP_DONT_CARE: u32 = 1;

pub const PRESENT_MODE_IMMEDIATE_KHR: u32 = 0;
pub const PRESENT_MODE_MAILBOX_KHR: u32 = 1;
pub const PRESENT_MODE_FIFO_KHR: u32 = 2;
pub const COLOR_SPACE_SRGB_NONLINEAR_KHR: u32 = 0;
pub const COMPOSITE_ALPHA_OPAQUE_BIT_KHR: u32 = 1;
pub const COMPOSITE_ALPHA_INHERIT_BIT_KHR: u32 = 8;
pub const SURFACE_TRANSFORM_IDENTITY_BIT_KHR: u32 = 1;

pub const SUBGROUP_FEATURE_BASIC_BIT: u32 = 1;
pub const SUBGROUP_FEATURE_VOTE_BIT: u32 = 2;
pub const SUBGROUP_FEATURE_ARITHMETIC_BIT: u32 = 4;
pub const SUBGROUP_FEATURE_BALLOT_BIT: u32 = 8;
pub const SUBGROUP_FEATURE_SHUFFLE_BIT: u32 = 0x10;
pub const SUBGROUP_FEATURE_SHUFFLE_RELATIVE_BIT: u32 = 0x20;
pub const SUBGROUP_FEATURE_QUAD_BIT: u32 = 0x80;

pub const DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT: u32 = 1;
pub const DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT: u32 = 0x10;
pub const DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: u32 = 0x100;
pub const DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: u32 = 0x1000;
pub const DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT: u32 = 1;
pub const DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT: u32 = 2;
pub const DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT: u32 = 4;

pub const VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT: u32 = 4;
pub const LAYER_SETTING_TYPE_BOOL32_EXT: u32 = 0;

// indices into the VkBool32 arrays of the feature structures (their fields in header order)
pub const F10_FULL_DRAW_INDEX_UINT32: usize = 1;
pub const F10_IMAGE_CUBE_ARRAY: usize = 2;
pub const F10_INDEPENDENT_BLEND: usize = 3;
pub const F10_GEOMETRY_SHADER: usize = 4;
pub const F10_SAMPLE_RATE_SHADING: usize = 6;
pub const F10_DUAL_SRC_BLEND: usize = 7;
pub const F10_MULTI_DRAW_INDIRECT: usize = 9;
pub const F10_DRAW_INDIRECT_FIRST_INSTANCE: usize = 10;
pub const F10_DEPTH_CLAMP: usize = 11;
pub const F10_FILL_MODE_NON_SOLID: usize = 13;
pub const F10_SAMPLER_ANISOTROPY: usize = 19;
pub const F10_TEXTURE_COMPRESSION_BC: usize = 22;
pub const F10_VERTEX_PIPELINE_STORES_AND_ATOMICS: usize = 25;
pub const F10_FRAGMENT_STORES_AND_ATOMICS: usize = 26;
pub const F10_SHADER_STORAGE_IMAGE_EXTENDED_FORMATS: usize = 29;
pub const F10_SHADER_STORAGE_IMAGE_READ_WITHOUT_FORMAT: usize = 31;
pub const F10_SHADER_STORAGE_IMAGE_WRITE_WITHOUT_FORMAT: usize = 32;
pub const F10_SHADER_UNIFORM_BUFFER_ARRAY_DYNAMIC_INDEXING: usize = 33;
pub const F10_SHADER_SAMPLED_IMAGE_ARRAY_DYNAMIC_INDEXING: usize = 34;
pub const F10_SHADER_STORAGE_BUFFER_ARRAY_DYNAMIC_INDEXING: usize = 35;
pub const F10_SHADER_STORAGE_IMAGE_ARRAY_DYNAMIC_INDEXING: usize = 36;
pub const F10_SHADER_CLIP_DISTANCE: usize = 37;
pub const F10_SHADER_FLOAT64: usize = 39;
pub const F10_SHADER_INT64: usize = 40;
pub const F10_SHADER_INT16: usize = 41;
pub const F10_COUNT: usize = 55;
pub const F11_STORAGE_BUFFER_16BIT_ACCESS: usize = 0;
pub const F11_UNIFORM_AND_STORAGE_BUFFER_16BIT_ACCESS: usize = 1;
pub const F11_STORAGE_PUSH_CONSTANT16: usize = 2;
pub const F11_SHADER_DRAW_PARAMETERS: usize = 11;
pub const F11_COUNT: usize = 12;
pub const F12_DRAW_INDIRECT_COUNT: usize = 1;
pub const F12_STORAGE_BUFFER_8BIT_ACCESS: usize = 2;
pub const F12_SHADER_FLOAT16: usize = 7;
pub const F12_SHADER_INT8: usize = 8;
pub const F12_DESCRIPTOR_INDEXING: usize = 9;
pub const F12_SHADER_SAMPLED_IMAGE_ARRAY_NON_UNIFORM_INDEXING: usize = 14;
pub const F12_SHADER_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING: usize = 15;
pub const F12_SHADER_STORAGE_IMAGE_ARRAY_NON_UNIFORM_INDEXING: usize = 16;
pub const F12_DESCRIPTOR_BINDING_SAMPLED_IMAGE_UPDATE_AFTER_BIND: usize = 21;
pub const F12_DESCRIPTOR_BINDING_STORAGE_IMAGE_UPDATE_AFTER_BIND: usize = 22;
pub const F12_DESCRIPTOR_BINDING_STORAGE_BUFFER_UPDATE_AFTER_BIND: usize = 23;
pub const F12_DESCRIPTOR_BINDING_UPDATE_UNUSED_WHILE_PENDING: usize = 26;
pub const F12_DESCRIPTOR_BINDING_PARTIALLY_BOUND: usize = 27;
pub const F12_RUNTIME_DESCRIPTOR_ARRAY: usize = 29;
pub const F12_SCALAR_BLOCK_LAYOUT: usize = 31;
pub const F12_UNIFORM_BUFFER_STANDARD_LAYOUT: usize = 33;
pub const F12_SHADER_SUBGROUP_EXTENDED_TYPES: usize = 34;
pub const F12_HOST_QUERY_RESET: usize = 36;
pub const F12_TIMELINE_SEMAPHORE: usize = 37;
pub const F12_BUFFER_DEVICE_ADDRESS: usize = 38;
pub const F12_VULKAN_MEMORY_MODEL: usize = 41;
pub const F12_VULKAN_MEMORY_MODEL_DEVICE_SCOPE: usize = 42;
pub const F12_COUNT: usize = 47;
pub const F13_SHADER_DEMOTE_TO_HELPER_INVOCATION: usize = 5;
pub const F13_SUBGROUP_SIZE_CONTROL: usize = 7;
pub const F13_SYNCHRONIZATION2: usize = 9;
pub const F13_DYNAMIC_RENDERING: usize = 12;
pub const F13_MAINTENANCE4: usize = 14;
pub const F13_COUNT: usize = 15;
// VkPhysicalDeviceDescriptorIndexingProperties as an array of uint32_t (fields in header order)
pub const DIP_MAX_PER_STAGE_UAB_SAMPLERS: usize = 8;
pub const DIP_MAX_PER_STAGE_UAB_STORAGE_BUFFERS: usize = 10;
pub const DIP_MAX_PER_STAGE_UAB_SAMPLED_IMAGES: usize = 11;
pub const DIP_MAX_PER_STAGE_UAB_STORAGE_IMAGES: usize = 12;
pub const DIP_MAX_PER_STAGE_UAB_RESOURCES: usize = 14;
pub const DIP_MAX_SET_UAB_SAMPLERS: usize = 15;
pub const DIP_MAX_SET_UAB_STORAGE_BUFFERS: usize = 18;
pub const DIP_MAX_SET_UAB_SAMPLED_IMAGES: usize = 20;
pub const DIP_MAX_SET_UAB_STORAGE_IMAGES: usize = 21;
pub const DIP_COUNT: usize = 23;

// ---------------------------------------------------------------- structures

/// all-zero is a valid value of the type (no references, fn pointers only as Option)
///
/// # Safety
/// Implemented only for the plain C structures below.
pub unsafe trait Zero: Sized {
    fn zero() -> Self {
        // SAFETY: the implementors are C structures of integers, floats, raw pointers and
        // Option<fn>, for which the all-zero bit pattern is valid.
        unsafe { core::mem::zeroed() }
    }
}

macro_rules! vk_structs {
    ($( $(#[$m:meta])* pub struct $name:ident { $( pub $f:ident : $t:ty ),* $(,)? } )*) => {
        $(
            $(#[$m])*
            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct $name { $( pub $f: $t ),* }
            // SAFETY: a C structure of plain fields (see Zero).
            unsafe impl Zero for $name {}
        )*
    };
}

pub type DebugCallback = unsafe extern "system" fn(u32, u32, *const DebugUtilsMessengerCallbackDataEXT, *mut c_void) -> Bool32;

vk_structs! {
    pub struct ApplicationInfo {
        pub s_type: u32, pub p_next: *const c_void, pub p_application_name: *const u8,
        pub application_version: u32, pub p_engine_name: *const u8, pub engine_version: u32,
        pub api_version: u32,
    }
    pub struct InstanceCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32,
        pub p_application_info: *const ApplicationInfo, pub enabled_layer_count: u32,
        pub pp_enabled_layer_names: *const *const u8, pub enabled_extension_count: u32,
        pub pp_enabled_extension_names: *const *const u8,
    }
    pub struct ExtensionProperties { pub extension_name: [u8; 256], pub spec_version: u32 }
    pub struct LayerProperties {
        pub layer_name: [u8; 256], pub spec_version: u32, pub implementation_version: u32,
        pub description: [u8; 256],
    }
    pub struct ValidationFeaturesEXT {
        pub s_type: u32, pub p_next: *const c_void, pub enabled_validation_feature_count: u32,
        pub p_enabled_validation_features: *const u32, pub disabled_validation_feature_count: u32,
        pub p_disabled_validation_features: *const u32,
    }
    pub struct LayerSettingEXT {
        pub p_layer_name: *const u8, pub p_setting_name: *const u8, pub type_: u32,
        pub value_count: u32, pub p_values: *const c_void,
    }
    pub struct LayerSettingsCreateInfoEXT {
        pub s_type: u32, pub p_next: *const c_void, pub setting_count: u32,
        pub p_settings: *const LayerSettingEXT,
    }
    pub struct PhysicalDeviceLimits {
        pub max_image_dimension1_d: u32, pub max_image_dimension2_d: u32,
        pub max_image_dimension3_d: u32, pub max_image_dimension_cube: u32,
        pub max_image_array_layers: u32, pub max_texel_buffer_elements: u32,
        pub max_uniform_buffer_range: u32, pub max_storage_buffer_range: u32,
        pub max_push_constants_size: u32, pub max_memory_allocation_count: u32,
        pub max_sampler_allocation_count: u32, pub buffer_image_granularity: u64,
        pub sparse_address_space_size: u64, pub max_bound_descriptor_sets: u32,
        pub max_per_stage_descriptor_samplers: u32, pub max_per_stage_descriptor_uniform_buffers: u32,
        pub max_per_stage_descriptor_storage_buffers: u32, pub max_per_stage_descriptor_sampled_images: u32,
        pub max_per_stage_descriptor_storage_images: u32, pub max_per_stage_descriptor_input_attachments: u32,
        pub max_per_stage_resources: u32, pub max_descriptor_set_samplers: u32,
        pub max_descriptor_set_uniform_buffers: u32, pub max_descriptor_set_uniform_buffers_dynamic: u32,
        pub max_descriptor_set_storage_buffers: u32, pub max_descriptor_set_storage_buffers_dynamic: u32,
        pub max_descriptor_set_sampled_images: u32, pub max_descriptor_set_storage_images: u32,
        pub max_descriptor_set_input_attachments: u32, pub max_vertex_input_attributes: u32,
        pub max_vertex_input_bindings: u32, pub max_vertex_input_attribute_offset: u32,
        pub max_vertex_input_binding_stride: u32, pub max_vertex_output_components: u32,
        pub max_tessellation_generation_level: u32, pub max_tessellation_patch_size: u32,
        pub max_tessellation_control_per_vertex_input_components: u32,
        pub max_tessellation_control_per_vertex_output_components: u32,
        pub max_tessellation_control_per_patch_output_components: u32,
        pub max_tessellation_control_total_output_components: u32,
        pub max_tessellation_evaluation_input_components: u32,
        pub max_tessellation_evaluation_output_components: u32,
        pub max_geometry_shader_invocations: u32, pub max_geometry_input_components: u32,
        pub max_geometry_output_components: u32, pub max_geometry_output_vertices: u32,
        pub max_geometry_total_output_components: u32, pub max_fragment_input_components: u32,
        pub max_fragment_output_attachments: u32, pub max_fragment_dual_src_attachments: u32,
        pub max_fragment_combined_output_resources: u32, pub max_compute_shared_memory_size: u32,
        pub max_compute_work_group_count: [u32; 3], pub max_compute_work_group_invocations: u32,
        pub max_compute_work_group_size: [u32; 3], pub sub_pixel_precision_bits: u32,
        pub sub_texel_precision_bits: u32, pub mipmap_precision_bits: u32,
        pub max_draw_indexed_index_value: u32, pub max_draw_indirect_count: u32,
        pub max_sampler_lod_bias: f32, pub max_sampler_anisotropy: f32, pub max_viewports: u32,
        pub max_viewport_dimensions: [u32; 2], pub viewport_bounds_range: [f32; 2],
        pub viewport_sub_pixel_bits: u32, pub min_memory_map_alignment: usize,
        pub min_texel_buffer_offset_alignment: u64, pub min_uniform_buffer_offset_alignment: u64,
        pub min_storage_buffer_offset_alignment: u64, pub min_texel_offset: i32,
        pub max_texel_offset: u32, pub min_texel_gather_offset: i32, pub max_texel_gather_offset: u32,
        pub min_interpolation_offset: f32, pub max_interpolation_offset: f32,
        pub sub_pixel_interpolation_offset_bits: u32, pub max_framebuffer_width: u32,
        pub max_framebuffer_height: u32, pub max_framebuffer_layers: u32,
        pub framebuffer_color_sample_counts: u32, pub framebuffer_depth_sample_counts: u32,
        pub framebuffer_stencil_sample_counts: u32, pub framebuffer_no_attachments_sample_counts: u32,
        pub max_color_attachments: u32, pub sampled_image_color_sample_counts: u32,
        pub sampled_image_integer_sample_counts: u32, pub sampled_image_depth_sample_counts: u32,
        pub sampled_image_stencil_sample_counts: u32, pub storage_image_sample_counts: u32,
        pub max_sample_mask_words: u32, pub timestamp_compute_and_graphics: Bool32,
        pub timestamp_period: f32, pub max_clip_distances: u32, pub max_cull_distances: u32,
        pub max_combined_clip_and_cull_distances: u32, pub discrete_queue_priorities: u32,
        pub point_size_range: [f32; 2], pub line_width_range: [f32; 2],
        pub point_size_granularity: f32, pub line_width_granularity: f32, pub strict_lines: Bool32,
        pub standard_sample_locations: Bool32, pub optimal_buffer_copy_offset_alignment: u64,
        pub optimal_buffer_copy_row_pitch_alignment: u64, pub non_coherent_atom_size: u64,
    }
    pub struct PhysicalDeviceProperties {
        pub api_version: u32, pub driver_version: u32, pub vendor_id: u32, pub device_id: u32,
        pub device_type: u32, pub device_name: [u8; 256], pub pipeline_cache_uuid: [u8; 16],
        pub limits: PhysicalDeviceLimits, pub sparse_properties: [u32; 5],
    }
    pub struct PhysicalDeviceProperties2 {
        pub s_type: u32, pub p_next: *mut c_void, pub properties: PhysicalDeviceProperties,
    }
    pub struct PhysicalDeviceDriverProperties {
        pub s_type: u32, pub p_next: *mut c_void, pub driver_id: u32, pub driver_name: [u8; 256],
        pub driver_info: [u8; 256], pub conformance_version: [u8; 4],
    }
    pub struct PhysicalDeviceSubgroupProperties {
        pub s_type: u32, pub p_next: *mut c_void, pub subgroup_size: u32, pub supported_stages: u32,
        pub supported_operations: u32, pub quad_operations_in_all_stages: Bool32,
    }
    pub struct PhysicalDeviceSubgroupSizeControlProperties {
        pub s_type: u32, pub p_next: *mut c_void, pub min_subgroup_size: u32, pub max_subgroup_size: u32,
        pub max_compute_workgroup_subgroups: u32, pub required_subgroup_size_stages: u32,
    }
    pub struct PipelineShaderStageRequiredSubgroupSizeCreateInfo {
        pub s_type: u32, pub p_next: *mut c_void, pub required_subgroup_size: u32,
    }
    /// VkPhysicalDeviceDescriptorIndexingProperties (its 23 uint32_t / VkBool32 fields: DIP_*)
    pub struct PhysicalDeviceDescriptorIndexingProperties {
        pub s_type: u32, pub p_next: *mut c_void, pub v: [u32; DIP_COUNT],
    }
    /// VkPhysicalDeviceFeatures2 (the 55 VkBool32 of VkPhysicalDeviceFeatures: F10_*)
    pub struct PhysicalDeviceFeatures2 { pub s_type: u32, pub p_next: *mut c_void, pub f: [Bool32; F10_COUNT] }
    /// VkPhysicalDeviceVulkan11Features (F11_*)
    pub struct PhysicalDeviceVulkan11Features { pub s_type: u32, pub p_next: *mut c_void, pub f: [Bool32; F11_COUNT] }
    /// VkPhysicalDeviceVulkan12Features (F12_*)
    pub struct PhysicalDeviceVulkan12Features { pub s_type: u32, pub p_next: *mut c_void, pub f: [Bool32; F12_COUNT] }
    /// VkPhysicalDeviceVulkan13Features (F13_*)
    pub struct PhysicalDeviceVulkan13Features { pub s_type: u32, pub p_next: *mut c_void, pub f: [Bool32; F13_COUNT] }
    /// VkPhysicalDeviceTimelineSemaphoreFeatures / VkPhysicalDeviceDynamicRenderingFeatures:
    /// one VkBool32
    pub struct PhysicalDeviceOneFeature { pub s_type: u32, pub p_next: *mut c_void, pub f: Bool32 }
    pub struct QueueFamilyProperties {
        pub queue_flags: u32, pub queue_count: u32, pub timestamp_valid_bits: u32,
        pub min_image_transfer_granularity: Extent3D,
    }
    pub struct MemoryType { pub property_flags: u32, pub heap_index: u32 }
    pub struct MemoryHeap { pub size: u64, pub flags: u32 }
    pub struct PhysicalDeviceMemoryProperties {
        pub memory_type_count: u32, pub memory_types: [MemoryType; 32], pub memory_heap_count: u32,
        pub memory_heaps: [MemoryHeap; 16],
    }
    pub struct FormatProperties {
        pub linear_tiling_features: u32, pub optimal_tiling_features: u32, pub buffer_features: u32,
    }
    pub struct ExternalMemoryImageCreateInfo { pub s_type: u32, pub p_next: *const c_void, pub handle_types: u32 }
    pub struct ExportMemoryAllocateInfo { pub s_type: u32, pub p_next: *const c_void, pub handle_types: u32 }
    pub struct MemoryDedicatedAllocateInfo { pub s_type: u32, pub p_next: *const c_void, pub image: Handle, pub buffer: Handle }
    pub struct MemoryGetFdInfoKHR { pub s_type: u32, pub p_next: *const c_void, pub memory: Handle, pub handle_type: u32 }
    pub struct DrmFormatModifierPropertiesEXT { pub modifier: u64, pub plane_count: u32, pub tiling_features: u32 }
    pub struct DrmFormatModifierPropertiesListEXT {
        pub s_type: u32, pub p_next: *mut c_void, pub count: u32, pub p_properties: *mut DrmFormatModifierPropertiesEXT,
    }
    pub struct FormatProperties2 { pub s_type: u32, pub p_next: *mut c_void, pub format_properties: FormatProperties }
    pub struct ImageDrmFormatModifierListCreateInfoEXT {
        pub s_type: u32, pub p_next: *const c_void, pub count: u32, pub p_modifiers: *const u64,
    }
    pub struct ImageDrmFormatModifierPropertiesEXT { pub s_type: u32, pub p_next: *mut c_void, pub modifier: u64 }
    pub struct ImageSubresource { pub aspect_mask: u32, pub mip_level: u32, pub array_layer: u32 }
    pub struct SubresourceLayout { pub offset: u64, pub size: u64, pub row_pitch: u64, pub array_pitch: u64, pub depth_pitch: u64 }
    pub struct PhysicalDeviceImageFormatInfo2 {
        pub s_type: u32, pub p_next: *const c_void, pub format: u32, pub image_type: u32, pub tiling: u32,
        pub usage: u32, pub flags: u32,
    }
    pub struct PhysicalDeviceExternalImageFormatInfo { pub s_type: u32, pub p_next: *const c_void, pub handle_type: u32 }
    pub struct PhysicalDeviceImageDrmFormatModifierInfoEXT {
        pub s_type: u32, pub p_next: *const c_void, pub modifier: u64, pub sharing_mode: u32,
        pub queue_family_index_count: u32, pub p_queue_family_indices: *const u32,
    }
    pub struct ImageFormatProperties {
        pub max_extent: Extent3D, pub max_mip_levels: u32, pub max_array_layers: u32, pub sample_counts: u32,
        pub max_resource_size: u64,
    }
    pub struct ImageFormatProperties2 { pub s_type: u32, pub p_next: *mut c_void, pub image_format_properties: ImageFormatProperties }
    pub struct ExternalMemoryProperties {
        pub external_memory_features: u32, pub export_from_imported_handle_types: u32, pub compatible_handle_types: u32,
    }
    pub struct ExternalImageFormatProperties { pub s_type: u32, pub p_next: *mut c_void, pub external_memory_properties: ExternalMemoryProperties }
    pub struct PhysicalDeviceDrmPropertiesEXT {
        pub s_type: u32, pub p_next: *mut c_void, pub has_primary: u32, pub has_render: u32,
        pub primary_major: i64, pub primary_minor: i64, pub render_major: i64, pub render_minor: i64,
    }
    pub struct DeviceQueueCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub queue_family_index: u32,
        pub queue_count: u32, pub p_queue_priorities: *const f32,
    }
    pub struct DeviceCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub queue_create_info_count: u32,
        pub p_queue_create_infos: *const DeviceQueueCreateInfo, pub enabled_layer_count: u32,
        pub pp_enabled_layer_names: *const *const u8, pub enabled_extension_count: u32,
        pub pp_enabled_extension_names: *const *const u8, pub p_enabled_features: *const c_void,
    }
    pub struct MemoryAllocateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub allocation_size: u64, pub memory_type_index: u32,
    }
    pub struct MemoryRequirements { pub size: u64, pub alignment: u64, pub memory_type_bits: u32 }
    pub struct MappedMemoryRange {
        pub s_type: u32, pub p_next: *const c_void, pub memory: Handle, pub offset: u64, pub size: u64,
    }
    pub struct BufferCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub size: u64, pub usage: u32,
        pub sharing_mode: u32, pub queue_family_index_count: u32, pub p_queue_family_indices: *const u32,
    }
    pub struct Extent2D { pub width: u32, pub height: u32 }
    pub struct Extent3D { pub width: u32, pub height: u32, pub depth: u32 }
    pub struct Offset2D { pub x: i32, pub y: i32 }
    pub struct Offset3D { pub x: i32, pub y: i32, pub z: i32 }
    pub struct Rect2D { pub offset: Offset2D, pub extent: Extent2D }
    pub struct ImageCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub image_type: u32,
        pub format: u32, pub extent: Extent3D, pub mip_levels: u32, pub array_layers: u32,
        pub samples: u32, pub tiling: u32, pub usage: u32, pub sharing_mode: u32,
        pub queue_family_index_count: u32, pub p_queue_family_indices: *const u32,
        pub initial_layout: u32,
    }
    pub struct ImageSubresourceRange {
        pub aspect_mask: u32, pub base_mip_level: u32, pub level_count: u32,
        pub base_array_layer: u32, pub layer_count: u32,
    }
    pub struct ComponentMapping { pub r: u32, pub g: u32, pub b: u32, pub a: u32 }
    pub struct ImageViewCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub image: Handle,
        pub view_type: u32, pub format: u32, pub components: ComponentMapping,
        pub subresource_range: ImageSubresourceRange,
    }
    pub struct SamplerCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub mag_filter: u32,
        pub min_filter: u32, pub mipmap_mode: u32, pub address_mode_u: u32, pub address_mode_v: u32,
        pub address_mode_w: u32, pub mip_lod_bias: f32, pub anisotropy_enable: Bool32,
        pub max_anisotropy: f32, pub compare_enable: Bool32, pub compare_op: u32, pub min_lod: f32,
        pub max_lod: f32, pub border_color: u32, pub unnormalized_coordinates: Bool32,
    }
    pub struct ShaderModuleCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub code_size: usize,
        pub p_code: *const u32,
    }
    pub struct DescriptorSetLayoutBinding {
        pub binding: u32, pub descriptor_type: u32, pub descriptor_count: u32, pub stage_flags: u32,
        pub p_immutable_samplers: *const Handle,
    }
    pub struct DescriptorSetLayoutCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub binding_count: u32,
        pub p_bindings: *const DescriptorSetLayoutBinding,
    }
    pub struct DescriptorSetLayoutBindingFlagsCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub binding_count: u32, pub p_binding_flags: *const u32,
    }
    pub struct PushConstantRange { pub stage_flags: u32, pub offset: u32, pub size: u32 }
    pub struct PipelineLayoutCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub set_layout_count: u32,
        pub p_set_layouts: *const Handle, pub push_constant_range_count: u32,
        pub p_push_constant_ranges: *const PushConstantRange,
    }
    pub struct DescriptorPoolSize { pub type_: u32, pub descriptor_count: u32 }
    pub struct DescriptorPoolCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub max_sets: u32,
        pub pool_size_count: u32, pub p_pool_sizes: *const DescriptorPoolSize,
    }
    pub struct DescriptorSetAllocateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub descriptor_pool: Handle,
        pub descriptor_set_count: u32, pub p_set_layouts: *const Handle,
    }
    pub struct DescriptorImageInfo { pub sampler: Handle, pub image_view: Handle, pub image_layout: u32 }
    pub struct DescriptorBufferInfo { pub buffer: Handle, pub offset: u64, pub range: u64 }
    pub struct WriteDescriptorSet {
        pub s_type: u32, pub p_next: *const c_void, pub dst_set: Handle, pub dst_binding: u32,
        pub dst_array_element: u32, pub descriptor_count: u32, pub descriptor_type: u32,
        pub p_image_info: *const DescriptorImageInfo, pub p_buffer_info: *const DescriptorBufferInfo,
        pub p_texel_buffer_view: *const Handle,
    }
    pub struct SpecializationMapEntry { pub constant_id: u32, pub offset: u32, pub size: usize }
    pub struct SpecializationInfo {
        pub map_entry_count: u32, pub p_map_entries: *const SpecializationMapEntry, pub data_size: usize,
        pub p_data: *const c_void,
    }
    pub struct PipelineShaderStageCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub stage: u32, pub module: Handle,
        pub p_name: *const u8, pub p_specialization_info: *const c_void,
    }
    pub struct ComputePipelineCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32,
        pub stage: PipelineShaderStageCreateInfo, pub layout: Handle, pub base_pipeline_handle: Handle,
        pub base_pipeline_index: i32,
    }
    pub struct VertexInputBindingDescription { pub binding: u32, pub stride: u32, pub input_rate: u32 }
    pub struct VertexInputAttributeDescription {
        pub location: u32, pub binding: u32, pub format: u32, pub offset: u32,
    }
    pub struct PipelineVertexInputStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32,
        pub vertex_binding_description_count: u32,
        pub p_vertex_binding_descriptions: *const VertexInputBindingDescription,
        pub vertex_attribute_description_count: u32,
        pub p_vertex_attribute_descriptions: *const VertexInputAttributeDescription,
    }
    pub struct PipelineInputAssemblyStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub topology: u32,
        pub primitive_restart_enable: Bool32,
    }
    pub struct PipelineViewportStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub viewport_count: u32,
        pub p_viewports: *const Viewport, pub scissor_count: u32, pub p_scissors: *const Rect2D,
    }
    pub struct PipelineRasterizationStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub depth_clamp_enable: Bool32,
        pub rasterizer_discard_enable: Bool32, pub polygon_mode: u32, pub cull_mode: u32,
        pub front_face: u32, pub depth_bias_enable: Bool32, pub depth_bias_constant_factor: f32,
        pub depth_bias_clamp: f32, pub depth_bias_slope_factor: f32, pub line_width: f32,
    }
    pub struct PipelineMultisampleStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub rasterization_samples: u32,
        pub sample_shading_enable: Bool32, pub min_sample_shading: f32, pub p_sample_mask: *const u32,
        pub alpha_to_coverage_enable: Bool32, pub alpha_to_one_enable: Bool32,
    }
    pub struct StencilOpState {
        pub fail_op: u32, pub pass_op: u32, pub depth_fail_op: u32, pub compare_op: u32,
        pub compare_mask: u32, pub write_mask: u32, pub reference: u32,
    }
    pub struct PipelineDepthStencilStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub depth_test_enable: Bool32,
        pub depth_write_enable: Bool32, pub depth_compare_op: u32, pub depth_bounds_test_enable: Bool32,
        pub stencil_test_enable: Bool32, pub front: StencilOpState, pub back: StencilOpState,
        pub min_depth_bounds: f32, pub max_depth_bounds: f32,
    }
    pub struct PipelineColorBlendAttachmentState {
        pub blend_enable: Bool32, pub src_color_blend_factor: u32, pub dst_color_blend_factor: u32,
        pub color_blend_op: u32, pub src_alpha_blend_factor: u32, pub dst_alpha_blend_factor: u32,
        pub alpha_blend_op: u32, pub color_write_mask: u32,
    }
    pub struct PipelineColorBlendStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub logic_op_enable: Bool32,
        pub logic_op: u32, pub attachment_count: u32,
        pub p_attachments: *const PipelineColorBlendAttachmentState, pub blend_constants: [f32; 4],
    }
    pub struct PipelineDynamicStateCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub dynamic_state_count: u32,
        pub p_dynamic_states: *const u32,
    }
    pub struct PipelineRenderingCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub view_mask: u32,
        pub color_attachment_count: u32, pub p_color_attachment_formats: *const u32,
        pub depth_attachment_format: u32, pub stencil_attachment_format: u32,
    }
    pub struct GraphicsPipelineCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub stage_count: u32,
        pub p_stages: *const PipelineShaderStageCreateInfo,
        pub p_vertex_input_state: *const PipelineVertexInputStateCreateInfo,
        pub p_input_assembly_state: *const PipelineInputAssemblyStateCreateInfo,
        pub p_tessellation_state: *const c_void,
        pub p_viewport_state: *const PipelineViewportStateCreateInfo,
        pub p_rasterization_state: *const PipelineRasterizationStateCreateInfo,
        pub p_multisample_state: *const PipelineMultisampleStateCreateInfo,
        pub p_depth_stencil_state: *const PipelineDepthStencilStateCreateInfo,
        pub p_color_blend_state: *const PipelineColorBlendStateCreateInfo,
        pub p_dynamic_state: *const PipelineDynamicStateCreateInfo, pub layout: Handle,
        pub render_pass: Handle, pub subpass: u32, pub base_pipeline_handle: Handle,
        pub base_pipeline_index: i32,
    }
    pub struct CommandPoolCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub queue_family_index: u32,
    }
    pub struct CommandBufferAllocateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub command_pool: Handle, pub level: u32,
        pub command_buffer_count: u32,
    }
    pub struct CommandBufferBeginInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub p_inheritance_info: *const c_void,
    }
    pub struct MemoryBarrier {
        pub s_type: u32, pub p_next: *const c_void, pub src_access_mask: u32, pub dst_access_mask: u32,
    }
    pub struct ImageMemoryBarrier {
        pub s_type: u32, pub p_next: *const c_void, pub src_access_mask: u32, pub dst_access_mask: u32,
        pub old_layout: u32, pub new_layout: u32, pub src_queue_family_index: u32,
        pub dst_queue_family_index: u32, pub image: Handle, pub subresource_range: ImageSubresourceRange,
    }
    pub struct BufferCopy { pub src_offset: u64, pub dst_offset: u64, pub size: u64 }
    pub struct ImageSubresourceLayers {
        pub aspect_mask: u32, pub mip_level: u32, pub base_array_layer: u32, pub layer_count: u32,
    }
    pub struct BufferImageCopy {
        pub buffer_offset: u64, pub buffer_row_length: u32, pub buffer_image_height: u32,
        pub image_subresource: ImageSubresourceLayers, pub image_offset: Offset3D,
        pub image_extent: Extent3D,
    }
    /// VkRenderingAttachmentInfo; clear_value: the VkClearValue union as 4 words (RGBA floats /
    /// depth float + stencil)
    pub struct RenderingAttachmentInfo {
        pub s_type: u32, pub p_next: *const c_void, pub image_view: Handle, pub image_layout: u32,
        pub resolve_mode: u32, pub resolve_image_view: Handle, pub resolve_image_layout: u32,
        pub load_op: u32, pub store_op: u32, pub clear_value: [u32; 4],
    }
    pub struct RenderingInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub render_area: Rect2D,
        pub layer_count: u32, pub view_mask: u32, pub color_attachment_count: u32,
        pub p_color_attachments: *const RenderingAttachmentInfo,
        pub p_depth_attachment: *const RenderingAttachmentInfo,
        pub p_stencil_attachment: *const RenderingAttachmentInfo,
    }
    pub struct Viewport {
        pub x: f32, pub y: f32, pub width: f32, pub height: f32, pub min_depth: f32, pub max_depth: f32,
    }
    pub struct QueryPoolCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub query_type: u32,
        pub query_count: u32, pub pipeline_statistics: u32,
    }
    pub struct SemaphoreCreateInfo { pub s_type: u32, pub p_next: *const c_void, pub flags: u32 }
    pub struct SemaphoreTypeCreateInfo {
        pub s_type: u32, pub p_next: *const c_void, pub semaphore_type: u32, pub initial_value: u64,
    }
    pub struct TimelineSemaphoreSubmitInfo {
        pub s_type: u32, pub p_next: *const c_void, pub wait_semaphore_value_count: u32,
        pub p_wait_semaphore_values: *const u64, pub signal_semaphore_value_count: u32,
        pub p_signal_semaphore_values: *const u64,
    }
    pub struct SemaphoreWaitInfo {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub semaphore_count: u32,
        pub p_semaphores: *const Handle, pub p_values: *const u64,
    }
    pub struct SubmitInfo {
        pub s_type: u32, pub p_next: *const c_void, pub wait_semaphore_count: u32,
        pub p_wait_semaphores: *const Handle, pub p_wait_dst_stage_mask: *const u32,
        pub command_buffer_count: u32, pub p_command_buffers: *const CommandBuffer,
        pub signal_semaphore_count: u32, pub p_signal_semaphores: *const Handle,
    }
    pub struct Win32SurfaceCreateInfoKHR {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub hinstance: isize, pub hwnd: isize,
    }
    pub struct AndroidSurfaceCreateInfoKHR {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub window: usize,
    }
    pub struct SurfaceCapabilitiesKHR {
        pub min_image_count: u32, pub max_image_count: u32, pub current_extent: Extent2D,
        pub min_image_extent: Extent2D, pub max_image_extent: Extent2D,
        pub max_image_array_layers: u32, pub supported_transforms: u32, pub current_transform: u32,
        pub supported_composite_alpha: u32, pub supported_usage_flags: u32,
    }
    pub struct SurfaceFormatKHR { pub format: u32, pub color_space: u32 }
    pub struct SwapchainCreateInfoKHR {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub surface: Handle,
        pub min_image_count: u32, pub image_format: u32, pub image_color_space: u32,
        pub image_extent: Extent2D, pub image_array_layers: u32, pub image_usage: u32,
        pub image_sharing_mode: u32, pub queue_family_index_count: u32,
        pub p_queue_family_indices: *const u32, pub pre_transform: u32, pub composite_alpha: u32,
        pub present_mode: u32, pub clipped: Bool32, pub old_swapchain: Handle,
    }
    pub struct PresentInfoKHR {
        pub s_type: u32, pub p_next: *const c_void, pub wait_semaphore_count: u32,
        pub p_wait_semaphores: *const Handle, pub swapchain_count: u32, pub p_swapchains: *const Handle,
        pub p_image_indices: *const u32, pub p_results: *mut VkResult,
    }
    pub struct DebugUtilsMessengerCreateInfoEXT {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub message_severity: u32,
        pub message_type: u32, pub pfn_user_callback: Option<DebugCallback>, pub p_user_data: *mut c_void,
    }
    /// the leading fields of VkDebugUtilsMessengerCallbackDataEXT (the rest is never read)
    pub struct DebugUtilsMessengerCallbackDataEXT {
        pub s_type: u32, pub p_next: *const c_void, pub flags: u32, pub p_message_id_name: *const u8,
        pub message_id_number: i32, pub p_message: *const u8,
    }
}

// ---------------------------------------------------------------- command tables

macro_rules! fn_type {
    (req ($($a:ty),*) $(-> $r:ty)?) => { unsafe extern "system" fn($($a),*) $(-> $r)? };
    (opt ($($a:ty),*) $(-> $r:ty)?) => { Option<unsafe extern "system" fn($($a),*) $(-> $r)?> };
}

macro_rules! fn_value {
    (req $p:expr, $f:ident, ($($a:ty),*) $(-> $r:ty)?) => {{
        let p: Pfn = $p;
        if p == 0 {
            return Err(stringify!($f));
        }
        // SAFETY: the loader returned this command's address; the type mirrors its C prototype.
        unsafe { core::mem::transmute::<Pfn, unsafe extern "system" fn($($a),*) $(-> $r)?>(p) }
    }};
    (opt $p:expr, $f:ident, ($($a:ty),*) $(-> $r:ty)?) => {{
        let p: Pfn = $p;
        // SAFETY: as above.
        if p == 0 { None } else { Some(unsafe { core::mem::transmute::<Pfn, unsafe extern "system" fn($($a),*) $(-> $r)?>(p) }) }
    }};
}

macro_rules! fn_table {
    ($(#[$m:meta])* pub struct $name:ident { $( $f:ident $kind:ident [$($n:literal),+] ($($a:ty),* $(,)?) $(-> $r:ty)? ; )* }) => {
        $(#[$m])*
        pub struct $name { $( pub $f: fn_type!($kind ($($a),*) $(-> $r)?), )* }
        impl $name {
            /// every command through get (a NUL-terminated name -> its address, 0 = missing):
            /// Err(entry) when a required one is missing
            pub fn load(get: &dyn Fn(&[u8]) -> Pfn) -> Result<$name, &'static str> {
                Ok($name { $( $f: fn_value!($kind { let mut p = 0; $( if p == 0 { p = get($n); } )+ p }, $f, ($($a),*) $(-> $r)?), )* })
            }
        }
    };
}

type A = *const c_void; // VkAllocationCallbacks* (always null)

fn_table! {
    /// the commands of the loader itself
    pub struct LoaderFns {
        enumerate_instance_version opt [b"vkEnumerateInstanceVersion\0"] (*mut u32) -> VkResult;
        enumerate_instance_extension_properties req [b"vkEnumerateInstanceExtensionProperties\0"]
            (*const u8, *mut u32, *mut ExtensionProperties) -> VkResult;
        enumerate_instance_layer_properties req [b"vkEnumerateInstanceLayerProperties\0"]
            (*mut u32, *mut LayerProperties) -> VkResult;
        create_instance req [b"vkCreateInstance\0"] (*const InstanceCreateInfo, A, *mut Instance) -> VkResult;
    }
}

fn_table! {
    /// the instance-level commands
    pub struct InstanceFns {
        destroy_instance req [b"vkDestroyInstance\0"] (Instance, A);
        enumerate_physical_devices req [b"vkEnumeratePhysicalDevices\0"] (Instance, *mut u32, *mut PhysicalDevice) -> VkResult;
        get_physical_device_properties2 req [b"vkGetPhysicalDeviceProperties2\0", b"vkGetPhysicalDeviceProperties2KHR\0"]
            (PhysicalDevice, *mut PhysicalDeviceProperties2);
        get_physical_device_features2 req [b"vkGetPhysicalDeviceFeatures2\0", b"vkGetPhysicalDeviceFeatures2KHR\0"]
            (PhysicalDevice, *mut PhysicalDeviceFeatures2);
        get_physical_device_queue_family_properties req [b"vkGetPhysicalDeviceQueueFamilyProperties\0"]
            (PhysicalDevice, *mut u32, *mut QueueFamilyProperties);
        get_physical_device_memory_properties req [b"vkGetPhysicalDeviceMemoryProperties\0"]
            (PhysicalDevice, *mut PhysicalDeviceMemoryProperties);
        get_physical_device_format_properties req [b"vkGetPhysicalDeviceFormatProperties\0"]
            (PhysicalDevice, u32, *mut FormatProperties);
        get_physical_device_format_properties2 opt [b"vkGetPhysicalDeviceFormatProperties2\0", b"vkGetPhysicalDeviceFormatProperties2KHR\0"]
            (PhysicalDevice, u32, *mut FormatProperties2);
        get_physical_device_image_format_properties2 opt [b"vkGetPhysicalDeviceImageFormatProperties2\0", b"vkGetPhysicalDeviceImageFormatProperties2KHR\0"]
            (PhysicalDevice, *const PhysicalDeviceImageFormatInfo2, *mut ImageFormatProperties2) -> VkResult;
        enumerate_device_extension_properties req [b"vkEnumerateDeviceExtensionProperties\0"]
            (PhysicalDevice, *const u8, *mut u32, *mut ExtensionProperties) -> VkResult;
        create_device req [b"vkCreateDevice\0"] (PhysicalDevice, *const DeviceCreateInfo, A, *mut Device) -> VkResult;
        get_device_proc_addr req [b"vkGetDeviceProcAddr\0"] (Device, *const u8) -> Pfn;
        destroy_surface_khr opt [b"vkDestroySurfaceKHR\0"] (Instance, Handle, A);
        get_physical_device_surface_support_khr opt [b"vkGetPhysicalDeviceSurfaceSupportKHR\0"]
            (PhysicalDevice, u32, Handle, *mut Bool32) -> VkResult;
        get_physical_device_surface_capabilities_khr opt [b"vkGetPhysicalDeviceSurfaceCapabilitiesKHR\0"]
            (PhysicalDevice, Handle, *mut SurfaceCapabilitiesKHR) -> VkResult;
        get_physical_device_surface_formats_khr opt [b"vkGetPhysicalDeviceSurfaceFormatsKHR\0"]
            (PhysicalDevice, Handle, *mut u32, *mut SurfaceFormatKHR) -> VkResult;
        get_physical_device_surface_present_modes_khr opt [b"vkGetPhysicalDeviceSurfacePresentModesKHR\0"]
            (PhysicalDevice, Handle, *mut u32, *mut u32) -> VkResult;
        create_win32_surface_khr opt [b"vkCreateWin32SurfaceKHR\0"]
            (Instance, *const Win32SurfaceCreateInfoKHR, A, *mut Handle) -> VkResult;
        create_android_surface_khr opt [b"vkCreateAndroidSurfaceKHR\0"]
            (Instance, *const AndroidSurfaceCreateInfoKHR, A, *mut Handle) -> VkResult;
        create_debug_utils_messenger_ext opt [b"vkCreateDebugUtilsMessengerEXT\0"]
            (Instance, *const DebugUtilsMessengerCreateInfoEXT, A, *mut Handle) -> VkResult;
        destroy_debug_utils_messenger_ext opt [b"vkDestroyDebugUtilsMessengerEXT\0"] (Instance, Handle, A);
    }
}

fn_table! {
    /// the device-level commands
    pub struct DeviceFns {
        destroy_device req [b"vkDestroyDevice\0"] (Device, A);
        get_device_queue req [b"vkGetDeviceQueue\0"] (Device, u32, u32, *mut Queue);
        device_wait_idle req [b"vkDeviceWaitIdle\0"] (Device) -> VkResult;
        queue_submit req [b"vkQueueSubmit\0"] (Queue, u32, *const SubmitInfo, Handle) -> VkResult;
        queue_wait_idle req [b"vkQueueWaitIdle\0"] (Queue) -> VkResult;
        allocate_memory req [b"vkAllocateMemory\0"] (Device, *const MemoryAllocateInfo, A, *mut Handle) -> VkResult;
        free_memory req [b"vkFreeMemory\0"] (Device, Handle, A);
        map_memory req [b"vkMapMemory\0"] (Device, Handle, u64, u64, u32, *mut *mut c_void) -> VkResult;
        invalidate_mapped_memory_ranges req [b"vkInvalidateMappedMemoryRanges\0"]
            (Device, u32, *const MappedMemoryRange) -> VkResult;
        flush_mapped_memory_ranges req [b"vkFlushMappedMemoryRanges\0"] (Device, u32, *const MappedMemoryRange) -> VkResult;
        create_buffer req [b"vkCreateBuffer\0"] (Device, *const BufferCreateInfo, A, *mut Handle) -> VkResult;
        destroy_buffer req [b"vkDestroyBuffer\0"] (Device, Handle, A);
        get_buffer_memory_requirements req [b"vkGetBufferMemoryRequirements\0"] (Device, Handle, *mut MemoryRequirements);
        bind_buffer_memory req [b"vkBindBufferMemory\0"] (Device, Handle, Handle, u64) -> VkResult;
        create_image req [b"vkCreateImage\0"] (Device, *const ImageCreateInfo, A, *mut Handle) -> VkResult;
        destroy_image req [b"vkDestroyImage\0"] (Device, Handle, A);
        get_image_memory_requirements req [b"vkGetImageMemoryRequirements\0"] (Device, Handle, *mut MemoryRequirements);
        bind_image_memory req [b"vkBindImageMemory\0"] (Device, Handle, Handle, u64) -> VkResult;
        create_image_view req [b"vkCreateImageView\0"] (Device, *const ImageViewCreateInfo, A, *mut Handle) -> VkResult;
        destroy_image_view req [b"vkDestroyImageView\0"] (Device, Handle, A);
        create_sampler req [b"vkCreateSampler\0"] (Device, *const SamplerCreateInfo, A, *mut Handle) -> VkResult;
        destroy_sampler req [b"vkDestroySampler\0"] (Device, Handle, A);
        create_shader_module req [b"vkCreateShaderModule\0"] (Device, *const ShaderModuleCreateInfo, A, *mut Handle) -> VkResult;
        destroy_shader_module req [b"vkDestroyShaderModule\0"] (Device, Handle, A);
        create_descriptor_set_layout req [b"vkCreateDescriptorSetLayout\0"]
            (Device, *const DescriptorSetLayoutCreateInfo, A, *mut Handle) -> VkResult;
        destroy_descriptor_set_layout req [b"vkDestroyDescriptorSetLayout\0"] (Device, Handle, A);
        create_pipeline_layout req [b"vkCreatePipelineLayout\0"] (Device, *const PipelineLayoutCreateInfo, A, *mut Handle) -> VkResult;
        destroy_pipeline_layout req [b"vkDestroyPipelineLayout\0"] (Device, Handle, A);
        create_descriptor_pool req [b"vkCreateDescriptorPool\0"] (Device, *const DescriptorPoolCreateInfo, A, *mut Handle) -> VkResult;
        destroy_descriptor_pool req [b"vkDestroyDescriptorPool\0"] (Device, Handle, A);
        allocate_descriptor_sets req [b"vkAllocateDescriptorSets\0"] (Device, *const DescriptorSetAllocateInfo, *mut Handle) -> VkResult;
        free_descriptor_sets req [b"vkFreeDescriptorSets\0"] (Device, Handle, u32, *const Handle) -> VkResult;
        update_descriptor_sets req [b"vkUpdateDescriptorSets\0"] (Device, u32, *const WriteDescriptorSet, u32, *const c_void);
        create_compute_pipelines req [b"vkCreateComputePipelines\0"]
            (Device, Handle, u32, *const ComputePipelineCreateInfo, A, *mut Handle) -> VkResult;
        create_graphics_pipelines req [b"vkCreateGraphicsPipelines\0"]
            (Device, Handle, u32, *const GraphicsPipelineCreateInfo, A, *mut Handle) -> VkResult;
        destroy_pipeline req [b"vkDestroyPipeline\0"] (Device, Handle, A);
        create_command_pool req [b"vkCreateCommandPool\0"] (Device, *const CommandPoolCreateInfo, A, *mut Handle) -> VkResult;
        destroy_command_pool req [b"vkDestroyCommandPool\0"] (Device, Handle, A);
        reset_command_pool req [b"vkResetCommandPool\0"] (Device, Handle, u32) -> VkResult;
        allocate_command_buffers req [b"vkAllocateCommandBuffers\0"]
            (Device, *const CommandBufferAllocateInfo, *mut CommandBuffer) -> VkResult;
        begin_command_buffer req [b"vkBeginCommandBuffer\0"] (CommandBuffer, *const CommandBufferBeginInfo) -> VkResult;
        end_command_buffer req [b"vkEndCommandBuffer\0"] (CommandBuffer) -> VkResult;
        cmd_pipeline_barrier req [b"vkCmdPipelineBarrier\0"]
            (CommandBuffer, u32, u32, u32, u32, *const MemoryBarrier, u32, *const c_void, u32, *const ImageMemoryBarrier);
        cmd_copy_buffer req [b"vkCmdCopyBuffer\0"] (CommandBuffer, Handle, Handle, u32, *const BufferCopy);
        cmd_copy_buffer_to_image req [b"vkCmdCopyBufferToImage\0"]
            (CommandBuffer, Handle, Handle, u32, u32, *const BufferImageCopy);
        cmd_copy_image_to_buffer req [b"vkCmdCopyImageToBuffer\0"]
            (CommandBuffer, Handle, u32, Handle, u32, *const BufferImageCopy);
        cmd_fill_buffer req [b"vkCmdFillBuffer\0"] (CommandBuffer, Handle, u64, u64, u32);
        cmd_bind_pipeline req [b"vkCmdBindPipeline\0"] (CommandBuffer, u32, Handle);
        cmd_bind_descriptor_sets req [b"vkCmdBindDescriptorSets\0"]
            (CommandBuffer, u32, Handle, u32, u32, *const Handle, u32, *const u32);
        cmd_push_constants req [b"vkCmdPushConstants\0"] (CommandBuffer, Handle, u32, u32, u32, *const c_void);
        cmd_dispatch req [b"vkCmdDispatch\0"] (CommandBuffer, u32, u32, u32);
        cmd_dispatch_indirect req [b"vkCmdDispatchIndirect\0"] (CommandBuffer, Handle, u64);
        cmd_set_viewport req [b"vkCmdSetViewport\0"] (CommandBuffer, u32, u32, *const Viewport);
        cmd_set_scissor req [b"vkCmdSetScissor\0"] (CommandBuffer, u32, u32, *const Rect2D);
        cmd_bind_vertex_buffers req [b"vkCmdBindVertexBuffers\0"] (CommandBuffer, u32, u32, *const Handle, *const u64);
        cmd_bind_index_buffer req [b"vkCmdBindIndexBuffer\0"] (CommandBuffer, Handle, u64, u32);
        cmd_draw req [b"vkCmdDraw\0"] (CommandBuffer, u32, u32, u32, u32);
        cmd_draw_indexed req [b"vkCmdDrawIndexed\0"] (CommandBuffer, u32, u32, u32, i32, u32);
        cmd_draw_indirect req [b"vkCmdDrawIndirect\0"] (CommandBuffer, Handle, u64, u32, u32);
        cmd_draw_indexed_indirect req [b"vkCmdDrawIndexedIndirect\0"] (CommandBuffer, Handle, u64, u32, u32);
        cmd_draw_indexed_indirect_count opt [b"vkCmdDrawIndexedIndirectCount\0", b"vkCmdDrawIndexedIndirectCountKHR\0"]
            (CommandBuffer, Handle, u64, Handle, u64, u32, u32);
        cmd_begin_rendering req [b"vkCmdBeginRendering\0", b"vkCmdBeginRenderingKHR\0"] (CommandBuffer, *const RenderingInfo);
        cmd_end_rendering req [b"vkCmdEndRendering\0", b"vkCmdEndRenderingKHR\0"] (CommandBuffer);
        create_query_pool req [b"vkCreateQueryPool\0"] (Device, *const QueryPoolCreateInfo, A, *mut Handle) -> VkResult;
        destroy_query_pool req [b"vkDestroyQueryPool\0"] (Device, Handle, A);
        cmd_reset_query_pool req [b"vkCmdResetQueryPool\0"] (CommandBuffer, Handle, u32, u32);
        reset_query_pool opt [b"vkResetQueryPool\0", b"vkResetQueryPoolEXT\0"] (Device, Handle, u32, u32);
        cmd_write_timestamp req [b"vkCmdWriteTimestamp\0"] (CommandBuffer, u32, Handle, u32);
        get_query_pool_results req [b"vkGetQueryPoolResults\0"]
            (Device, Handle, u32, u32, usize, *mut c_void, u64, u32) -> VkResult;
        create_semaphore req [b"vkCreateSemaphore\0"] (Device, *const SemaphoreCreateInfo, A, *mut Handle) -> VkResult;
        destroy_semaphore req [b"vkDestroySemaphore\0"] (Device, Handle, A);
        wait_semaphores req [b"vkWaitSemaphores\0", b"vkWaitSemaphoresKHR\0"] (Device, *const SemaphoreWaitInfo, u64) -> VkResult;
        get_semaphore_counter_value req [b"vkGetSemaphoreCounterValue\0", b"vkGetSemaphoreCounterValueKHR\0"]
            (Device, Handle, *mut u64) -> VkResult;
        get_memory_fd_khr opt [b"vkGetMemoryFdKHR\0"] (Device, *const MemoryGetFdInfoKHR, *mut i32) -> VkResult;
        get_image_drm_format_modifier_properties_ext opt [b"vkGetImageDrmFormatModifierPropertiesEXT\0"]
            (Device, Handle, *mut ImageDrmFormatModifierPropertiesEXT) -> VkResult;
        get_image_subresource_layout opt [b"vkGetImageSubresourceLayout\0"] (Device, Handle, *const ImageSubresource, *mut SubresourceLayout);
        create_swapchain_khr opt [b"vkCreateSwapchainKHR\0"] (Device, *const SwapchainCreateInfoKHR, A, *mut Handle) -> VkResult;
        destroy_swapchain_khr opt [b"vkDestroySwapchainKHR\0"] (Device, Handle, A);
        get_swapchain_images_khr opt [b"vkGetSwapchainImagesKHR\0"] (Device, Handle, *mut u32, *mut Handle) -> VkResult;
        acquire_next_image_khr opt [b"vkAcquireNextImageKHR\0"] (Device, Handle, u64, Handle, Handle, *mut u32) -> VkResult;
        queue_present_khr opt [b"vkQueuePresentKHR\0"] (Queue, *const PresentInfoKHR) -> VkResult;
    }
}

/// a NUL-terminated name field (extension, layer, device names) as text
pub fn cstr(b: &[u8]) -> &str {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..n]).unwrap_or("?")
}

// ---------------------------------------------------------------- layout checks
// sizes and offsets of vulkan_core.h on x86-64 (tools: clang -ffreestanding over the header)
#[cfg(target_pointer_width = "64")]
mod layout {
    use super::*;
    use core::mem::{offset_of, size_of};
    macro_rules! check {
        ($t:ident, $size:expr $(, $f:ident = $o:expr)*) => {
            const _: () = assert!(size_of::<$t>() == $size);
            $( const _: () = assert!(offset_of!($t, $f) == $o); )*
        };
    }
    check!(ApplicationInfo, 48, s_type = 0, p_next = 8, p_application_name = 16, application_version = 24, p_engine_name = 32, engine_version = 40, api_version = 44);
    check!(InstanceCreateInfo, 64, s_type = 0, p_next = 8, flags = 16, p_application_info = 24, enabled_layer_count = 32, pp_enabled_layer_names = 40, enabled_extension_count = 48, pp_enabled_extension_names = 56);
    check!(ExtensionProperties, 260, extension_name = 0, spec_version = 256);
    check!(LayerProperties, 520, layer_name = 0, spec_version = 256, implementation_version = 260, description = 264);
    check!(ValidationFeaturesEXT, 48, s_type = 0, p_next = 8, enabled_validation_feature_count = 16, p_enabled_validation_features = 24, disabled_validation_feature_count = 32, p_disabled_validation_features = 40);
    check!(LayerSettingEXT, 32, p_layer_name = 0, p_setting_name = 8, type_ = 16, value_count = 20, p_values = 24);
    check!(LayerSettingsCreateInfoEXT, 32, s_type = 0, p_next = 8, setting_count = 16, p_settings = 24);
    check!(PhysicalDeviceLimits, 504, max_image_dimension1_d = 0, max_image_dimension2_d = 4, max_image_dimension3_d = 8, max_image_dimension_cube = 12, max_image_array_layers = 16, max_texel_buffer_elements = 20, max_uniform_buffer_range = 24, max_storage_buffer_range = 28, max_push_constants_size = 32, max_memory_allocation_count = 36, max_sampler_allocation_count = 40, buffer_image_granularity = 48, sparse_address_space_size = 56, max_bound_descriptor_sets = 64, max_per_stage_descriptor_samplers = 68, max_per_stage_descriptor_uniform_buffers = 72, max_per_stage_descriptor_storage_buffers = 76, max_per_stage_descriptor_sampled_images = 80, max_per_stage_descriptor_storage_images = 84, max_per_stage_descriptor_input_attachments = 88, max_per_stage_resources = 92, max_descriptor_set_samplers = 96, max_descriptor_set_uniform_buffers = 100, max_descriptor_set_uniform_buffers_dynamic = 104, max_descriptor_set_storage_buffers = 108, max_descriptor_set_storage_buffers_dynamic = 112, max_descriptor_set_sampled_images = 116, max_descriptor_set_storage_images = 120, max_descriptor_set_input_attachments = 124, max_vertex_input_attributes = 128, max_vertex_input_bindings = 132, max_vertex_input_attribute_offset = 136, max_vertex_input_binding_stride = 140, max_vertex_output_components = 144, max_tessellation_generation_level = 148, max_tessellation_patch_size = 152, max_tessellation_control_per_vertex_input_components = 156, max_tessellation_control_per_vertex_output_components = 160, max_tessellation_control_per_patch_output_components = 164, max_tessellation_control_total_output_components = 168, max_tessellation_evaluation_input_components = 172, max_tessellation_evaluation_output_components = 176, max_geometry_shader_invocations = 180, max_geometry_input_components = 184, max_geometry_output_components = 188, max_geometry_output_vertices = 192, max_geometry_total_output_components = 196, max_fragment_input_components = 200, max_fragment_output_attachments = 204, max_fragment_dual_src_attachments = 208, max_fragment_combined_output_resources = 212, max_compute_shared_memory_size = 216, max_compute_work_group_count = 220, max_compute_work_group_invocations = 232, max_compute_work_group_size = 236, sub_pixel_precision_bits = 248, sub_texel_precision_bits = 252, mipmap_precision_bits = 256, max_draw_indexed_index_value = 260, max_draw_indirect_count = 264, max_sampler_lod_bias = 268, max_sampler_anisotropy = 272, max_viewports = 276, max_viewport_dimensions = 280, viewport_bounds_range = 288, viewport_sub_pixel_bits = 296, min_memory_map_alignment = 304, min_texel_buffer_offset_alignment = 312, min_uniform_buffer_offset_alignment = 320, min_storage_buffer_offset_alignment = 328, min_texel_offset = 336, max_texel_offset = 340, min_texel_gather_offset = 344, max_texel_gather_offset = 348, min_interpolation_offset = 352, max_interpolation_offset = 356, sub_pixel_interpolation_offset_bits = 360, max_framebuffer_width = 364, max_framebuffer_height = 368, max_framebuffer_layers = 372, framebuffer_color_sample_counts = 376, framebuffer_depth_sample_counts = 380, framebuffer_stencil_sample_counts = 384, framebuffer_no_attachments_sample_counts = 388, max_color_attachments = 392, sampled_image_color_sample_counts = 396, sampled_image_integer_sample_counts = 400, sampled_image_depth_sample_counts = 404, sampled_image_stencil_sample_counts = 408, storage_image_sample_counts = 412, max_sample_mask_words = 416, timestamp_compute_and_graphics = 420, timestamp_period = 424, max_clip_distances = 428, max_cull_distances = 432, max_combined_clip_and_cull_distances = 436, discrete_queue_priorities = 440, point_size_range = 444, line_width_range = 452, point_size_granularity = 460, line_width_granularity = 464, strict_lines = 468, standard_sample_locations = 472, optimal_buffer_copy_offset_alignment = 480, optimal_buffer_copy_row_pitch_alignment = 488, non_coherent_atom_size = 496);
    check!(PhysicalDeviceProperties, 824, api_version = 0, driver_version = 4, vendor_id = 8, device_id = 12, device_type = 16, device_name = 20, pipeline_cache_uuid = 276, limits = 296, sparse_properties = 800);
    check!(PhysicalDeviceProperties2, 840, s_type = 0, p_next = 8, properties = 16);
    check!(PhysicalDeviceDriverProperties, 536, s_type = 0, p_next = 8, driver_id = 16, driver_name = 20, driver_info = 276, conformance_version = 532);
    check!(PhysicalDeviceSubgroupProperties, 32, s_type = 0, p_next = 8, subgroup_size = 16, supported_stages = 20, supported_operations = 24, quad_operations_in_all_stages = 28);
    check!(PhysicalDeviceSubgroupSizeControlProperties, 32, s_type = 0, p_next = 8, min_subgroup_size = 16, max_subgroup_size = 20, max_compute_workgroup_subgroups = 24, required_subgroup_size_stages = 28);
    check!(PipelineShaderStageRequiredSubgroupSizeCreateInfo, 24, s_type = 0, p_next = 8, required_subgroup_size = 16);
    check!(PhysicalDeviceDescriptorIndexingProperties, 112);
    check!(PhysicalDeviceFeatures2, 240);
    check!(PhysicalDeviceVulkan11Features, 64);
    check!(PhysicalDeviceVulkan12Features, 208);
    check!(PhysicalDeviceVulkan13Features, 80);
    check!(PhysicalDeviceOneFeature, 24);
    check!(QueueFamilyProperties, 24, queue_flags = 0, queue_count = 4, timestamp_valid_bits = 8, min_image_transfer_granularity = 12);
    check!(MemoryType, 8, property_flags = 0, heap_index = 4);
    check!(MemoryHeap, 16, size = 0, flags = 8);
    check!(PhysicalDeviceMemoryProperties, 520, memory_type_count = 0, memory_types = 4, memory_heap_count = 260, memory_heaps = 264);
    check!(FormatProperties, 12, linear_tiling_features = 0, optimal_tiling_features = 4, buffer_features = 8);
    check!(DeviceQueueCreateInfo, 40, s_type = 0, p_next = 8, flags = 16, queue_family_index = 20, queue_count = 24, p_queue_priorities = 32);
    check!(DeviceCreateInfo, 72, s_type = 0, p_next = 8, flags = 16, queue_create_info_count = 20, p_queue_create_infos = 24, enabled_layer_count = 32, pp_enabled_layer_names = 40, enabled_extension_count = 48, pp_enabled_extension_names = 56, p_enabled_features = 64);
    check!(MemoryAllocateInfo, 32, s_type = 0, p_next = 8, allocation_size = 16, memory_type_index = 24);
    check!(MemoryRequirements, 24, size = 0, alignment = 8, memory_type_bits = 16);
    check!(MappedMemoryRange, 40, s_type = 0, p_next = 8, memory = 16, offset = 24, size = 32);
    check!(BufferCreateInfo, 56, s_type = 0, p_next = 8, flags = 16, size = 24, usage = 32, sharing_mode = 36, queue_family_index_count = 40, p_queue_family_indices = 48);
    check!(Extent2D, 8, width = 0, height = 4);
    check!(Extent3D, 12, width = 0, height = 4, depth = 8);
    check!(Offset2D, 8, x = 0, y = 4);
    check!(Offset3D, 12, x = 0, y = 4, z = 8);
    check!(Rect2D, 16, offset = 0, extent = 8);
    check!(ImageCreateInfo, 88, s_type = 0, p_next = 8, flags = 16, image_type = 20, format = 24, extent = 28, mip_levels = 40, array_layers = 44, samples = 48, tiling = 52, usage = 56, sharing_mode = 60, queue_family_index_count = 64, p_queue_family_indices = 72, initial_layout = 80);
    check!(ImageSubresourceRange, 20, aspect_mask = 0, base_mip_level = 4, level_count = 8, base_array_layer = 12, layer_count = 16);
    check!(ComponentMapping, 16, r = 0, g = 4, b = 8, a = 12);
    check!(ImageViewCreateInfo, 80, s_type = 0, p_next = 8, flags = 16, image = 24, view_type = 32, format = 36, components = 40, subresource_range = 56);
    check!(SamplerCreateInfo, 80, s_type = 0, p_next = 8, flags = 16, mag_filter = 20, min_filter = 24, mipmap_mode = 28, address_mode_u = 32, address_mode_v = 36, address_mode_w = 40, mip_lod_bias = 44, anisotropy_enable = 48, max_anisotropy = 52, compare_enable = 56, compare_op = 60, min_lod = 64, max_lod = 68, border_color = 72, unnormalized_coordinates = 76);
    check!(ShaderModuleCreateInfo, 40, s_type = 0, p_next = 8, flags = 16, code_size = 24, p_code = 32);
    check!(DescriptorSetLayoutBinding, 24, binding = 0, descriptor_type = 4, descriptor_count = 8, stage_flags = 12, p_immutable_samplers = 16);
    check!(DescriptorSetLayoutCreateInfo, 32, s_type = 0, p_next = 8, flags = 16, binding_count = 20, p_bindings = 24);
    check!(DescriptorSetLayoutBindingFlagsCreateInfo, 32, s_type = 0, p_next = 8, binding_count = 16, p_binding_flags = 24);
    check!(PushConstantRange, 12, stage_flags = 0, offset = 4, size = 8);
    check!(PipelineLayoutCreateInfo, 48, s_type = 0, p_next = 8, flags = 16, set_layout_count = 20, p_set_layouts = 24, push_constant_range_count = 32, p_push_constant_ranges = 40);
    check!(DescriptorPoolSize, 8, type_ = 0, descriptor_count = 4);
    check!(DescriptorPoolCreateInfo, 40, s_type = 0, p_next = 8, flags = 16, max_sets = 20, pool_size_count = 24, p_pool_sizes = 32);
    check!(DescriptorSetAllocateInfo, 40, s_type = 0, p_next = 8, descriptor_pool = 16, descriptor_set_count = 24, p_set_layouts = 32);
    check!(DescriptorImageInfo, 24, sampler = 0, image_view = 8, image_layout = 16);
    check!(DescriptorBufferInfo, 24, buffer = 0, offset = 8, range = 16);
    check!(WriteDescriptorSet, 64, s_type = 0, p_next = 8, dst_set = 16, dst_binding = 24, dst_array_element = 28, descriptor_count = 32, descriptor_type = 36, p_image_info = 40, p_buffer_info = 48, p_texel_buffer_view = 56);
    check!(SpecializationMapEntry, 16, constant_id = 0, offset = 4, size = 8);
    check!(SpecializationInfo, 32, map_entry_count = 0, p_map_entries = 8, data_size = 16, p_data = 24);
    check!(PipelineShaderStageCreateInfo, 48, s_type = 0, p_next = 8, flags = 16, stage = 20, module = 24, p_name = 32, p_specialization_info = 40);
    check!(ComputePipelineCreateInfo, 96, s_type = 0, p_next = 8, flags = 16, stage = 24, layout = 72, base_pipeline_handle = 80, base_pipeline_index = 88);
    check!(VertexInputBindingDescription, 12, binding = 0, stride = 4, input_rate = 8);
    check!(VertexInputAttributeDescription, 16, location = 0, binding = 4, format = 8, offset = 12);
    check!(PipelineVertexInputStateCreateInfo, 48, s_type = 0, p_next = 8, flags = 16, vertex_binding_description_count = 20, p_vertex_binding_descriptions = 24, vertex_attribute_description_count = 32, p_vertex_attribute_descriptions = 40);
    check!(PipelineInputAssemblyStateCreateInfo, 32, s_type = 0, p_next = 8, flags = 16, topology = 20, primitive_restart_enable = 24);
    check!(PipelineViewportStateCreateInfo, 48, s_type = 0, p_next = 8, flags = 16, viewport_count = 20, p_viewports = 24, scissor_count = 32, p_scissors = 40);
    check!(PipelineRasterizationStateCreateInfo, 64, s_type = 0, p_next = 8, flags = 16, depth_clamp_enable = 20, rasterizer_discard_enable = 24, polygon_mode = 28, cull_mode = 32, front_face = 36, depth_bias_enable = 40, depth_bias_constant_factor = 44, depth_bias_clamp = 48, depth_bias_slope_factor = 52, line_width = 56);
    check!(PipelineMultisampleStateCreateInfo, 48, s_type = 0, p_next = 8, flags = 16, rasterization_samples = 20, sample_shading_enable = 24, min_sample_shading = 28, p_sample_mask = 32, alpha_to_coverage_enable = 40, alpha_to_one_enable = 44);
    check!(StencilOpState, 28, fail_op = 0, pass_op = 4, depth_fail_op = 8, compare_op = 12, compare_mask = 16, write_mask = 20, reference = 24);
    check!(PipelineDepthStencilStateCreateInfo, 104, s_type = 0, p_next = 8, flags = 16, depth_test_enable = 20, depth_write_enable = 24, depth_compare_op = 28, depth_bounds_test_enable = 32, stencil_test_enable = 36, front = 40, back = 68, min_depth_bounds = 96, max_depth_bounds = 100);
    check!(PipelineColorBlendAttachmentState, 32, blend_enable = 0, src_color_blend_factor = 4, dst_color_blend_factor = 8, color_blend_op = 12, src_alpha_blend_factor = 16, dst_alpha_blend_factor = 20, alpha_blend_op = 24, color_write_mask = 28);
    check!(PipelineColorBlendStateCreateInfo, 56, s_type = 0, p_next = 8, flags = 16, logic_op_enable = 20, logic_op = 24, attachment_count = 28, p_attachments = 32, blend_constants = 40);
    check!(PipelineDynamicStateCreateInfo, 32, s_type = 0, p_next = 8, flags = 16, dynamic_state_count = 20, p_dynamic_states = 24);
    check!(PipelineRenderingCreateInfo, 40, s_type = 0, p_next = 8, view_mask = 16, color_attachment_count = 20, p_color_attachment_formats = 24, depth_attachment_format = 32, stencil_attachment_format = 36);
    check!(GraphicsPipelineCreateInfo, 144, s_type = 0, p_next = 8, flags = 16, stage_count = 20, p_stages = 24, p_vertex_input_state = 32, p_input_assembly_state = 40, p_tessellation_state = 48, p_viewport_state = 56, p_rasterization_state = 64, p_multisample_state = 72, p_depth_stencil_state = 80, p_color_blend_state = 88, p_dynamic_state = 96, layout = 104, render_pass = 112, subpass = 120, base_pipeline_handle = 128, base_pipeline_index = 136);
    check!(CommandPoolCreateInfo, 24, s_type = 0, p_next = 8, flags = 16, queue_family_index = 20);
    check!(CommandBufferAllocateInfo, 32, s_type = 0, p_next = 8, command_pool = 16, level = 24, command_buffer_count = 28);
    check!(CommandBufferBeginInfo, 32, s_type = 0, p_next = 8, flags = 16, p_inheritance_info = 24);
    check!(MemoryBarrier, 24, s_type = 0, p_next = 8, src_access_mask = 16, dst_access_mask = 20);
    check!(ImageMemoryBarrier, 72, s_type = 0, p_next = 8, src_access_mask = 16, dst_access_mask = 20, old_layout = 24, new_layout = 28, src_queue_family_index = 32, dst_queue_family_index = 36, image = 40, subresource_range = 48);
    check!(BufferCopy, 24, src_offset = 0, dst_offset = 8, size = 16);
    check!(ImageSubresourceLayers, 16, aspect_mask = 0, mip_level = 4, base_array_layer = 8, layer_count = 12);
    check!(BufferImageCopy, 56, buffer_offset = 0, buffer_row_length = 8, buffer_image_height = 12, image_subresource = 16, image_offset = 32, image_extent = 44);
    check!(RenderingAttachmentInfo, 72, s_type = 0, p_next = 8, image_view = 16, image_layout = 24, resolve_mode = 28, resolve_image_view = 32, resolve_image_layout = 40, load_op = 44, store_op = 48, clear_value = 52);
    check!(RenderingInfo, 72, s_type = 0, p_next = 8, flags = 16, render_area = 20, layer_count = 36, view_mask = 40, color_attachment_count = 44, p_color_attachments = 48, p_depth_attachment = 56, p_stencil_attachment = 64);
    check!(Viewport, 24, x = 0, y = 4, width = 8, height = 12, min_depth = 16, max_depth = 20);
    check!(QueryPoolCreateInfo, 32, s_type = 0, p_next = 8, flags = 16, query_type = 20, query_count = 24, pipeline_statistics = 28);
    check!(SemaphoreCreateInfo, 24, s_type = 0, p_next = 8, flags = 16);
    check!(SemaphoreTypeCreateInfo, 32, s_type = 0, p_next = 8, semaphore_type = 16, initial_value = 24);
    check!(TimelineSemaphoreSubmitInfo, 48, s_type = 0, p_next = 8, wait_semaphore_value_count = 16, p_wait_semaphore_values = 24, signal_semaphore_value_count = 32, p_signal_semaphore_values = 40);
    check!(SemaphoreWaitInfo, 40, s_type = 0, p_next = 8, flags = 16, semaphore_count = 20, p_semaphores = 24, p_values = 32);
    check!(SubmitInfo, 72, s_type = 0, p_next = 8, wait_semaphore_count = 16, p_wait_semaphores = 24, p_wait_dst_stage_mask = 32, command_buffer_count = 40, p_command_buffers = 48, signal_semaphore_count = 56, p_signal_semaphores = 64);
    check!(Win32SurfaceCreateInfoKHR, 40, s_type = 0, p_next = 8, flags = 16, hinstance = 24, hwnd = 32);
    check!(AndroidSurfaceCreateInfoKHR, 32, s_type = 0, p_next = 8, flags = 16, window = 24);
    check!(SurfaceCapabilitiesKHR, 52, min_image_count = 0, max_image_count = 4, current_extent = 8, min_image_extent = 16, max_image_extent = 24, max_image_array_layers = 32, supported_transforms = 36, current_transform = 40, supported_composite_alpha = 44, supported_usage_flags = 48);
    check!(SurfaceFormatKHR, 8, format = 0, color_space = 4);
    check!(SwapchainCreateInfoKHR, 104, s_type = 0, p_next = 8, flags = 16, surface = 24, min_image_count = 32, image_format = 36, image_color_space = 40, image_extent = 44, image_array_layers = 52, image_usage = 56, image_sharing_mode = 60, queue_family_index_count = 64, p_queue_family_indices = 72, pre_transform = 80, composite_alpha = 84, present_mode = 88, clipped = 92, old_swapchain = 96);
    check!(PresentInfoKHR, 64, s_type = 0, p_next = 8, wait_semaphore_count = 16, p_wait_semaphores = 24, swapchain_count = 32, p_swapchains = 40, p_image_indices = 48, p_results = 56);
    check!(DebugUtilsMessengerCreateInfoEXT, 48, s_type = 0, p_next = 8, flags = 16, message_severity = 20, message_type = 24, pfn_user_callback = 32, p_user_data = 40);
    const _: () = { assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, s_type) == 0); assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, p_next) == 8); assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, flags) == 16); assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, p_message_id_name) == 24); assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, message_id_number) == 32); assert!(offset_of!(DebugUtilsMessengerCallbackDataEXT, p_message) == 40); };
}
