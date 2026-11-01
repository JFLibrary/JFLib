// WGSL "Uniformity Analysis": subgroup_size is uniform in a compute shader
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_size) sz: u32) { if sz == 32u { workgroupBarrier(); } }
