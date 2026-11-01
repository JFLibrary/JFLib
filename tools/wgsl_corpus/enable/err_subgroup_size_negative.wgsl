// WGSL "Subgroups": not negative
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(-4) fn main() { }
