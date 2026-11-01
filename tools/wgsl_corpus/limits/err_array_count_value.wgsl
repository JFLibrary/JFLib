// WGSL "Limits": as a value
// expect: error
fn f() { let a = array<u32, 65536>(); }
