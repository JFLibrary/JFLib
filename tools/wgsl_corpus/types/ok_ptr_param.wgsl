// WGSL "Pointer Types": a ptr<function, T> parameter
// expect: valid
fn g(p: ptr<function, i32>) { *p += 1; }
fn f() -> i32 { var a = 1; g(&a); return a; }
