// WGSL "Address Spaces": parameters are values: no assignment
// expect: error
fn f(a: i32) { a = 2; }
