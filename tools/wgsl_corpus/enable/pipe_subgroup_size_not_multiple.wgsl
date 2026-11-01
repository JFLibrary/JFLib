// WGSL "Subgroups": the workgroup size in x is a multiple of it
// expect: pipeline-error
enable subgroup_size_control;
@compute @workgroup_size(30) @subgroup_size(32) fn main() { }
