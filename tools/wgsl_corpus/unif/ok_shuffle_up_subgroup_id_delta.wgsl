// WGSL "Uniformity Analysis": subgroup_id as delta
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) sid: u32) { let a = subgroupShuffleUp(1u, sid); }
