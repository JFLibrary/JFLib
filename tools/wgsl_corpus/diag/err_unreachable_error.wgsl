// WGSL "Diagnostics": chromium.unreachable_code as an error
// expect: error
diagnostic(error, chromium.unreachable_code);
fn f() -> i32 { return 1; let a = 2; }
