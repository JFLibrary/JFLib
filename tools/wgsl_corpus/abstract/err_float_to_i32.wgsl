// WGSL "Abstract Numeric Types": an abstract float does not convert to i32
// expect: error
fn f() { let a: i32 = 1.0; }
