// WGSL "Built-in Functions": dot of bool vectors
// expect: error
fn f() { _ = dot(vec2(true), vec2(false)); }
