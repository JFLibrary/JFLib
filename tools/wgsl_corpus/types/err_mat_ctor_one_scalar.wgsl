// WGSL "Matrix Types": there is no matrix constructor from one scalar
// expect: error
fn f() { let m = mat2x2f(1.0); }
