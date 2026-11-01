// WGSL "Built-in Functions": bitcast needs equal sizes (u32 to vec2f)
// expect: error
fn f() { _ = bitcast<vec2<f32>>(1u); }
