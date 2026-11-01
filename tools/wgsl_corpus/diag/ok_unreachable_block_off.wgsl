// WGSL "Diagnostics": off on a block
// expect: valid
fn f() -> i32 { @diagnostic(off, chromium.unreachable_code) { return 1; let a = 2; } }
