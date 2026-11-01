// WGSL "Attributes": calling a @must_use function as a statement
// expect: error
@must_use fn g() -> i32 { return 1; }
fn f() { g(); }
