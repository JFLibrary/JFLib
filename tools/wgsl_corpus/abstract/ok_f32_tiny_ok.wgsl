// WGSL "Abstract Numeric Types": 1e-50 converts to f32 (rounded towards 0)
// expect: valid
fn f() -> f32 { let a: f32 = 1e-50; return a; }
