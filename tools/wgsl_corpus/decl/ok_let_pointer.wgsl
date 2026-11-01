// WGSL "Value Declarations": a let may hold a pointer
// expect: valid
fn f() -> i32 { var v = 1; let p = &v; *p = 2; return v; }
