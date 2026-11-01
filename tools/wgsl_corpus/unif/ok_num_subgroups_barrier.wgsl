// WGSL "Uniformity Analysis": num_subgroups is uniform
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(num_subgroups) n: u32) { if n == 1u { workgroupBarrier(); } }
