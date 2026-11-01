// WGSL "Declaration and Scope": module-scope declarations may be used before they are declared
// expect: valid
fn f() -> i32 { return K + g(); }
fn g() -> i32 { return 1; }
const K = 2;
