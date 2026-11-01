// WGSL "Array Types": a runtime-sized array must be the last struct member
// expect: error
struct S { a: array<f32>, n: u32 }
@group(0) @binding(0) var<storage> b: S;
