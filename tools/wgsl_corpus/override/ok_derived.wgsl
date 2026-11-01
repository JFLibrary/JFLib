// WGSL "override Declarations": an override may be computed from another, with built-ins
// expect: valid
override a: f32 = 4.0;
override b = sqrt(a) * 2.0;
@group(0) @binding(0) var<storage, read_write> o: array<f32>;
@compute @workgroup_size(1) fn main() { o[0] = b; }
