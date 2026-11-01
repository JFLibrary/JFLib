// WGSL "Abstract Numeric Types": -1 is not representable as u32
// expect: error
fn f() { let a: u32 = -1; }
