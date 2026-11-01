// WGSL "Uniformity Analysis": a scan differs in the subgroup
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupInclusiveAdd(1u) == 0u { let a = subgroupAdd(1u); }
}
