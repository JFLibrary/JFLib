// WGSL "override Declarations": a pipeline constant that does not fit the override's type (u32 -1)
// expect: pipeline-error
// constants: {"n": -1}
override n: u32;
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
@compute @workgroup_size(1) fn main() { b[0] = n; }
