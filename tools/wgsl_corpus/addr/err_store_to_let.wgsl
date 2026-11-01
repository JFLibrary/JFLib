// WGSL "Address Spaces": a let is no reference
// expect: error
fn f() { let a = 1; a = 2; }
