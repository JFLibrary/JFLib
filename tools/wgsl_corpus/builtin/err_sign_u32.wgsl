// WGSL "Built-in Functions": sign has no unsigned overload
// expect: error
fn f() { _ = sign(1u); }
