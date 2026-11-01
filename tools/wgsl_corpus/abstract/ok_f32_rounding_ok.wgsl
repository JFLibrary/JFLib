// WGSL "Abstract Numeric Types": 16777217 converts to f32 (rounded), it is in range
// expect: valid
fn f() -> f32 { let a: f32 = 16777217; return a; }
