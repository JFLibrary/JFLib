// WGSL "Subgroups": one argument
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(4, 8) fn main() { }
