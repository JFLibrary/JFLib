// WGSL "Built-in Functions": normalize only of vectors
// expect: error
fn f() { _ = normalize(2.0); }
