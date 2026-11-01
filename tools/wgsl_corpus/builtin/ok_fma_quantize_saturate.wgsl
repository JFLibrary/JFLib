// WGSL "Built-in Functions": fma, quantizeToF16, saturate
// expect: valid
fn f(x: f32) -> f32 { return fma(x, 2.0, 3.0) + quantizeToF16(x) + saturate(x); }
