// WGSL "Limits": behind a function pointer
// expect: error
fn f(p: ptr<function, array<u32, 65536>>) { }
