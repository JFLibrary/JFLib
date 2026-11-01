// WGSL "Built-in Functions": bitcast<vec2<f16>>(u32) with f16
// expect: valid
enable f16;
fn f(x: u32) -> vec2h { return bitcast<vec2<f16>>(x); }
