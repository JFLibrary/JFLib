// WGSL "Declaration and Scope": a function must not call itself
// expect: error
fn f(n: i32) -> i32 { if n <= 0 { return 0; } return f(n - 1); }
