// WGSL "Array Types": the count must be an integer
// expect: error
fn f() { var a: array<i32, 4.0>; }
