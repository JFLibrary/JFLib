// WGSL "Declaration and Scope": a nested block may shadow an outer name
// expect: valid
fn f() -> i32 { let a = 1; { let a = 2u; _ = a; } return a; }
