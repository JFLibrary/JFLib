// WGSL "Template Lists": a parenthesized comparison inside a template list is one argument (bool: not an array count)
// expect: error
fn f() { var a: array<i32, (2 < 3)>; }
