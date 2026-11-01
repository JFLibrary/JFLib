// WGSL "Uniformity Analysis": subgroup_invocation_id is non-uniform
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_invocation_id) sii: u32) { if sii == 0u { let a = subgroupAdd(1u); } }
