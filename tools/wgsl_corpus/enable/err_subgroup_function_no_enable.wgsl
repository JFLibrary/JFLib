// WGSL "Subgroups": subgroupAdd needs enable subgroups
// expect: error
@compute @workgroup_size(64) fn main() { let a = subgroupAdd(1u); }
