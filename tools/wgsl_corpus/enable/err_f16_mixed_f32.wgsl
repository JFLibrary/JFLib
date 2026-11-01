// WGSL "Enable Extensions": f16 and f32 do not mix implicitly
// expect: error
enable f16;
fn f(x: f16, y: f32) -> f32 { return x + y; }
