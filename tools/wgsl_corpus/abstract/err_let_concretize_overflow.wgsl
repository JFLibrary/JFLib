// WGSL "Abstract Numeric Types": let concretizes 1 << 31 to i32: 2147483648 does not fit
// expect: error
fn f() { let a = 1 << 31; }
