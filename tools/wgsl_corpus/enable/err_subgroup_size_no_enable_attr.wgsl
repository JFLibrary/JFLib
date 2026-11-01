// WGSL "Subgroups": @subgroup_size needs subgroup_size_control
// expect: error
enable subgroups;
@compute @workgroup_size(64) @subgroup_size(32) fn main() { }
