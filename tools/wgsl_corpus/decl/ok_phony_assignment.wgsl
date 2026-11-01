// WGSL "Variable Declarations": the phony assignment _ = e evaluates and discards
// expect: valid
fn g() -> i32 { return 1; }
fn f() { _ = g(); _ = 1; _ = vec2(1, 2); }
