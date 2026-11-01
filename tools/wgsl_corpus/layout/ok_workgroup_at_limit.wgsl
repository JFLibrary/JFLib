// WGSL "Memory Layout": exactly 16384 bytes of workgroup storage
// expect: valid
var<workgroup> w: array<f32, 4096>;
@compute @workgroup_size(1) fn main() { w[0] = 1.0; }
