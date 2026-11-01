// WGSL "Uniformity Analysis": with a uniform argument it decides subgroup-uniform control flow
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupBroadcastFirst(1u) == 0u { let a = subgroupAdd(1u); }
}
