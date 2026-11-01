// WGSL "Subgroups": also with an override workgroup size
// expect: pipeline-error
enable subgroup_size_control;
override w = 30;
@compute @workgroup_size(w) @subgroup_size(32) fn main() { }
