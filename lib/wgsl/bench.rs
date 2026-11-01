// bench -- the six shaders of the GPU benchmark scene as WGSL (lib/wgsl/bench/*.wgsl): the
// same bindings and data contract as the SPIR-V shaders of lib/shader/bench.rs (docs/SHADER.md
// "Benchmark-Vertrag"); G2c compiles them and compares them with those.
//
//   MOVE, CULL, CULL_RESET, VS_INSTANCED, VS_SINGLE, FS_LAMBERT   the source texts
//   ALL                                                           (name, text) of all six
//
// The push constants are `var<immediate> pc: Push` (language extension
// immediate_address_space, which Chrome supports): 20 bytes at offset 0 as before, so
// vs_single keeps reading pc.object_index and move / cull keep their parameters.  Every
// entry point is `main`; each file is a complete module.

/// move.wgsl
pub const MOVE: &str = include_str!("bench/move.wgsl");
/// cull.wgsl
pub const CULL: &str = include_str!("bench/cull.wgsl");
/// cull_reset.wgsl
pub const CULL_RESET: &str = include_str!("bench/cull_reset.wgsl");
/// vs_instanced.wgsl
pub const VS_INSTANCED: &str = include_str!("bench/vs_instanced.wgsl");
/// vs_single.wgsl
pub const VS_SINGLE: &str = include_str!("bench/vs_single.wgsl");
/// fs_lambert.wgsl
pub const FS_LAMBERT: &str = include_str!("bench/fs_lambert.wgsl");

/// (name, source) of the six shaders
pub const ALL: [(&str, &str); 6] = [
    ("move", MOVE),
    ("cull", CULL),
    ("cull_reset", CULL_RESET),
    ("vs_instanced", VS_INSTANCED),
    ("vs_single", VS_SINGLE),
    ("fs_lambert", FS_LAMBERT),
];
