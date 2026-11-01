// WGSL "Built-in Functions": min(1, 2u): the abstract argument converts
// expect: valid
fn f() -> u32 { return min(1, 2u); }
