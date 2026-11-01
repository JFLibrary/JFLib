// WGSL "Subgroups": at least 1
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(0) fn main() { }
