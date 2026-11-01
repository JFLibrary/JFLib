// WGSL "Limits": Chrome: at most 65535 elements in a function variable
// expect: error
fn f() { var v: array<u32, 65536>; }
