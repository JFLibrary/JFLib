// WGSL "Uniformity Analysis": subgroupAdd of a non-uniform value
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupAdd(i) == 0u { let a = subgroupAdd(1u); }
}
