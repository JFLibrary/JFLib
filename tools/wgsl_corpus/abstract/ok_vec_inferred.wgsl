// WGSL "Abstract Numeric Types": vec2(1, 2) converts to vec2<f32> where needed
// expect: valid
fn f() -> vec2<f32> { let v: vec2<f32> = vec2(1, 2); return v; }
