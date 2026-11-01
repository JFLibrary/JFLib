// WGSL "Limits": 64 bytes of immediate data
// expect: valid
struct S { a: vec4u, b: vec4u, c: vec4u, d: vec4u }
var<immediate> v: S;
@compute @workgroup_size(1) fn main() { _ = v.a; }
