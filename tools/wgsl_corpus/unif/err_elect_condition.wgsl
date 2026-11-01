// WGSL "Uniformity Analysis": subgroupElect differs in the subgroup
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupElect() { let a = subgroupAdd(1u); }
}
