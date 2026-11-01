// WGSL "Declaration and Scope": a for initializer is scoped to the loop
// expect: error
fn f() -> i32 { for (var i = 0; i < 4; i++) {} return i; }
