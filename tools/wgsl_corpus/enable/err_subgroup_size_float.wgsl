// WGSL "Subgroups": an integer
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(4.0) fn main() { }
