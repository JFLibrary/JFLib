// WGSL "Template Lists": a < inside brackets starts no template list across the bracket
// expect: valid
fn f(a: array<i32, 4>, i: i32, j: i32) -> i32 { return a[u32(i < j)] + i; }
