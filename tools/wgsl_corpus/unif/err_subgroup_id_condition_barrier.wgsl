// WGSL "Uniformity Analysis": not at workgroup scope
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) sid: u32) { if sid == 0u { workgroupBarrier(); } }
