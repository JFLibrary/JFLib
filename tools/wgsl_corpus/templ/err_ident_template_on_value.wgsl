// WGSL "Template Lists": a<b>(c) is a template list after a value: an error
// expect: error
fn f(a: i32, b: i32, c: i32) -> i32 { return a<b>(c); }
