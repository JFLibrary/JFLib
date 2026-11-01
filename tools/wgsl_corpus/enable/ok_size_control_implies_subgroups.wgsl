// WGSL "Subgroups": it turns subgroups on
// expect: valid
enable subgroup_size_control;
@compute @workgroup_size(64) fn main() { let a = subgroupAdd(1u); }
