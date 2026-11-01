// WGSL "Abstract Numeric Types": 2147483648 is not representable as i32
// expect: error
fn f() { let a: i32 = 2147483648; }
