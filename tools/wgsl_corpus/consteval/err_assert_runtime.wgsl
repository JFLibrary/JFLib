// WGSL "const_assert Statement": const_assert needs a const expression
// expect: error
fn f(a: i32) { const_assert a > 0; }
