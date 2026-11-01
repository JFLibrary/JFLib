// WGSL "override Declarations": the workgroup size from an override
// expect: valid
override wg = 64;
@compute @workgroup_size(wg) fn main() {}
