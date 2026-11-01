// WGSL "Value Declarations": a let cannot hold a runtime-sized array
// expect: error
@group(0) @binding(0) var<storage> b: array<f32>;
fn f() { let a = b; }
