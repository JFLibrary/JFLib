// WGSL "Uniformity Analysis": a subgroup function in non-uniform control flow
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { let a = subgroupAdd(1u); }
}
