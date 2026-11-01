// WGSL "Uniformity Analysis": subgroup_id is uniform at subgroup scope
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) sid: u32) { if sid == 0u { let a = subgroupAdd(1u); } }
