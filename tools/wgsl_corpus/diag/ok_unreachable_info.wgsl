// WGSL "Diagnostics": as an info
// expect: valid
diagnostic(info, chromium.unreachable_code);
fn f() -> i32 { return 1; let a = 2; }
