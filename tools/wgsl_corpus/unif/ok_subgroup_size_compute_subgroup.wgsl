// WGSL "Uniformity Analysis": at both scopes
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_size) sz: u32) { if sz == 32u { let a = subgroupAdd(1u); } }
