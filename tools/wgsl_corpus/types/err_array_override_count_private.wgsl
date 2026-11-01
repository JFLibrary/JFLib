// WGSL "Array Types": an override-sized array only in workgroup memory
// expect: error
override n = 4;
var<private> a: array<f32, n>;
