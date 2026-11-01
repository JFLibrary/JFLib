// WGSL "Limits": as a result
// expect: error
fn f() -> array<u32, 65536> { return array<u32, 65536>(); }
