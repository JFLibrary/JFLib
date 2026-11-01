// WGSL "Array Types": an override-sized array in workgroup memory
// expect: valid
override n = 4;
var<workgroup> a: array<f32, n>;
@compute @workgroup_size(1) fn main() { a[0] = 1.0; }
