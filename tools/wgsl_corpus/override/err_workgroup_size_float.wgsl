// WGSL "override Declarations": a workgroup size must be an integer
// expect: error
override wg: f32 = 4.0;
@compute @workgroup_size(wg) fn main() {}
