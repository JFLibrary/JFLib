// WGSL "Abstract Numeric Types": a concrete i32 does not convert to f32 implicitly
// expect: error
fn f() { let a: f32 = 1i; }
