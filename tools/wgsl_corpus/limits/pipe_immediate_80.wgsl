// WGSL "Limits": over: a pipeline error
// expect: pipeline-error
struct S { a: vec4u, b: vec4u, c: vec4u, d: vec4u, e: u32 }
var<immediate> v: S;
@compute @workgroup_size(1) fn main() { _ = v.a; }
