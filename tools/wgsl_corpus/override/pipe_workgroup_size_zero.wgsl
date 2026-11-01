// WGSL "override Declarations": a workgroup size override of 0 fails at pipeline creation
// expect: pipeline-error
// constants: {"wg": 0}
override wg: u32;
@compute @workgroup_size(wg) fn main() {}
