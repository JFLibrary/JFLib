// WGSL "Variable Declarations": a function var without initializer is zero
// expect: valid
fn f() -> i32 { var a: i32; return a; }
