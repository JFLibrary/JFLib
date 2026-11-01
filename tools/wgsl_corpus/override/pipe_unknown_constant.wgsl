// WGSL "override Declarations": a pipeline constant the module does not declare
// expect: pipeline-error
// constants: {"m": 1}
override n: u32 = 1;
@compute @workgroup_size(1) fn main() { _ = n; }
