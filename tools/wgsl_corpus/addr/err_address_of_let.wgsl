// WGSL "Address Spaces": & takes a reference: not of a let
// expect: error
fn f() { let a = 1; let p = &a; }
