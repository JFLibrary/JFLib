// WGSL "Declaration and Scope": two declarations of one name in one scope
// expect: error
fn f() { let a = 1; let a = 2; }
