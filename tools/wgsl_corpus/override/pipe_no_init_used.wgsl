// WGSL "override Declarations": an override without initializer that the entry point uses needs a pipeline constant
// expect: pipeline-error
override n: u32;
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
@compute @workgroup_size(1) fn main() { b[0] = n; }
