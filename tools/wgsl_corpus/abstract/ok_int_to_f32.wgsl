// WGSL "Abstract Numeric Types": an abstract int converts to f32
// expect: valid
fn f() -> f32 { let a: f32 = 1; return a; }
