// WGSL "Subgroups": once
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(32) @subgroup_size(32) fn main() { }
