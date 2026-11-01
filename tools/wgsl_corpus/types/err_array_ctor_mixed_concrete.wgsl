// WGSL "Array Types": array(1i, 2.0): no common type
// expect: error
fn f() { let a = array(1i, 2.0); }
