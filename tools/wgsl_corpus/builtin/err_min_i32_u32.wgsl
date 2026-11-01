// WGSL "Built-in Functions": min(1i, 2u): no overload
// expect: error
fn f() { _ = min(1i, 2u); }
