// WGSL "Diagnostics": a namespaced rule (chromium.unreachable_code)
// expect: valid
diagnostic(off, chromium.unreachable_code);
fn f() -> i32 { return 1; let a = 2; }
