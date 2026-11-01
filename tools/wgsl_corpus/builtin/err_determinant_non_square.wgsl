// WGSL "Built-in Functions": determinant only of square matrices
// expect: error
fn f() { _ = determinant(mat2x3f()); }
