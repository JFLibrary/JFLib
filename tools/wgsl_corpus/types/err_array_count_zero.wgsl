// WGSL "Array Types": a fixed array has at least one element
// expect: error
fn f() { var a: array<i32, 0>; }
