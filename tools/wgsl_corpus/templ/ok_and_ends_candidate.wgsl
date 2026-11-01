// WGSL "Template Lists": && inside a candidate template list ends it: a < b && c > d compares
// expect: valid
fn f(a: i32, b: i32, c: i32, d: i32) -> bool { return a < b && c > d; }
