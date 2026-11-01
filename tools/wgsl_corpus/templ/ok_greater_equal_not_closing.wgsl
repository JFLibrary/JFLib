// WGSL "Template Lists": >= after a template-like list: vec2<f32>(1.0).x >= 0.0
// expect: valid
fn f() -> bool { return vec2<f32>(1.0).x >= 0.0; }
