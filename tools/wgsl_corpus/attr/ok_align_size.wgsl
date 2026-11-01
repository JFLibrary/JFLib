// WGSL "Attributes": @align (power of 2) and @size (at least the type's size) on members
// expect: valid
struct S { @align(16) a: f32, @size(32) b: vec4f, c: i32 }
@group(0) @binding(0) var<storage> s: S;
@compute @workgroup_size(1) fn main() { _ = s.c; }
