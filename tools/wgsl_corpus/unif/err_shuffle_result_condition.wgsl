// WGSL "Uniformity Analysis": the result of a shuffle is not uniform
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupShuffle(1u, 0u) == 0u { let a = subgroupAdd(1u); }
}
