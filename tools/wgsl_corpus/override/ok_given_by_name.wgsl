// WGSL "override Declarations": the constant given by the override's name
// expect: valid
// constants: {"n": 7}
override n: u32;
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
@compute @workgroup_size(1) fn main() { b[0] = n; }
