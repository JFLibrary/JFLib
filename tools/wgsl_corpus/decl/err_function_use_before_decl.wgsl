// WGSL "Declaration and Scope": inside a function a name is in scope only after its declaration
// expect: error
fn f() -> i32 { let a = b; let b = 1; return a; }
