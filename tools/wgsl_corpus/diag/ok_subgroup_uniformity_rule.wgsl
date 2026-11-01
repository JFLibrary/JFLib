// WGSL "Diagnostics": diagnostic(off, subgroup_uniformity) (subgroup_uniformity)
// expect: valid
// uniformity
diagnostic(off, subgroup_uniformity);
enable subgroups;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i > 3u { o[i] = subgroupAdd(i); } }
