// WGSL "Built-in Functions": ldexp(f32, i32)
// expect: valid
fn f(e: i32) -> f32 { return ldexp(1.0, e); }
