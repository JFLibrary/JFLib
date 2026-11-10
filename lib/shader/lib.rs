// shader -- SPIR-V for JFLib's GPU code: a module builder, a disassembler, a structural
// validator, std140 / std430 layouts and the shaders of the GPU benchmark scene.  The back
// end of the WGSL compiler (step G2), usable directly from Rust.  No GPU needed: everything
// here produces or reads words (docs/SHADER.md).
//
//   Module::new(minor)        a SPIR-V 1.minor module (1.3: Vulkan 1.1 "Basis", 1.6: Vulkan
//                             1.3 "Voll"); types, constants, globals, functions, structured
//                             control flow, instructions; finish() -> Vec<u32>
//   layout                    Rule::Std140 / Std430: align, size, stride, matrix_stride, offsets
//   dis::disassemble(words)   text close to spirv-dis
//   val::validate(words)      the structural checks (val module comment lists the rules)
//   bench                     the benchmark shaders (move, cull, cull_reset, vs_instanced,
//                             vs_single, fs_lambert), their data contract and CPU references
//   reflect                   entry points, specialization constants and their values once a
//                             pipeline sets some (OpSpecConstantOp evaluated), workgroup sizes
//   samples                   small shaders for the tests: specialization constants, textures
//   spv                       opcodes, enums and their names; parse: words -> instructions
//   msl::translate(words)     the module in the Metal Shading Language (lib/gpu on Apple)
//   to_bytes / from_bytes     words <-> the bytes of a .spv file
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bench;
pub mod dis;
pub mod layout;
mod module;
pub mod msl;
mod msl_fn;
pub mod parse;
pub mod reflect;
pub mod samples;
pub mod spv;
pub mod val;

pub use layout::Rule;
pub use module::{from_bytes, to_bytes, Id, Loop, Module, Ty, Types};
pub use spv::{op, Bi, Cap, Dec, Dim, Fmt, Lang, Mode, Model, Sc};

/// the SPIR-V minor version of the feature level "Basis" (Vulkan 1.1: SPIR-V 1.3)
pub const BASIS: u32 = 3;
/// the SPIR-V minor version of the feature level "Voll" (Vulkan 1.3: SPIR-V 1.6)
pub const VOLL: u32 = 6;
