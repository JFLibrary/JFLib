// WGSL "Limits": nested
// expect: error
fn f() { var v: array<array<u32, 65536>, 2>; }
