// WGSL "Built-in Functions": abs(bool) does not
// expect: error
fn f() { _ = abs(true); }
