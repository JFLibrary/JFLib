// WGSL "Declaration and Scope": the function body may not redeclare a parameter (same end of scope)
// expect: error
fn f(a: i32) -> i32 { let a = 2; return a; }
