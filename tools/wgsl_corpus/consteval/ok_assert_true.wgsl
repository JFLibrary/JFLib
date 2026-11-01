// WGSL "const_assert Statement": a true const_assert
// expect: valid
const_assert 1 < 2;
fn f() { const_assert 2 > 1; }
