// WGSL "Built-in Functions": transpose(mat2x3f) is a mat3x2f
// expect: valid
fn f(m: mat2x3f) -> mat3x2f { return transpose(m); }
