// WGSL "Subgroups": @subgroup_size is a power of two
// expect: error
enable subgroup_size_control;
@compute @workgroup_size(64) @subgroup_size(3) fn main() { }
