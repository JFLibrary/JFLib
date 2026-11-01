// WGSL "Attributes": a @must_use result discarded with _ =
// expect: valid
@must_use fn g() -> i32 { return 1; }
fn f() { _ = g(); }
