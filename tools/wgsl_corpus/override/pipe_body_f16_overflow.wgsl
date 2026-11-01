// WGSL "Pipeline Creation": f32 to f16 out of range
// expect: pipeline-error
enable f16;
override a: f32 = 1e30;
@compute @workgroup_size(1) fn main() { var x = 1h; x = f16(a); }
