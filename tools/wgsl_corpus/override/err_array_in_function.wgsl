// WGSL "override Declarations": an override-sized array only in workgroup memory, not in a function
// expect: error
override n = 4;
fn f() { var a: array<f32, n>; }
