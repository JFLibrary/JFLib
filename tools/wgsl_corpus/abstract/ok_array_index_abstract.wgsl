// WGSL "Abstract Numeric Types": an abstract index into a fixed array
// expect: valid
fn f() -> i32 { let a = array(1, 2, 3); return a[2]; }
