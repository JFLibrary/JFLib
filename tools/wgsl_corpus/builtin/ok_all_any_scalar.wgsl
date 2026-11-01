// WGSL "Built-in Functions": all / any of a bool scalar
// expect: valid
fn f(b: bool) -> bool { return all(b) || any(vec2(b)); }
