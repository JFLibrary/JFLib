// WGSL "Template Lists": a < b without a closing > is a comparison
// expect: valid
fn f(a: i32, b: i32) -> bool { return a < b; }
