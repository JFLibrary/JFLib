// WGSL "Built-in Functions": arrayLength of a runtime-sized array, directly and as a member
// expect: valid
struct S { n: u32, a: array<f32> }
@group(0) @binding(0) var<storage> b: array<u32>;
@group(0) @binding(1) var<storage> s: S;
@compute @workgroup_size(1) fn main() { _ = arrayLength(&b) + arrayLength(&s.a); }
