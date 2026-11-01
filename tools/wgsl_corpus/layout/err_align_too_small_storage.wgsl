// WGSL "Memory Layout": @align(8) on a vec4f member of a storage struct (needs 16)
// expect: error
struct S { a: f32, @align(8) v: vec4f }
@group(0) @binding(0) var<storage> s: S;
@compute @workgroup_size(1) fn main() { _ = s.v; }
