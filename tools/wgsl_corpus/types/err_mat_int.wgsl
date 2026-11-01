// WGSL "Matrix Types": matrix components are floating point
// expect: error
fn f() { let m = mat2x2<i32>(); }
