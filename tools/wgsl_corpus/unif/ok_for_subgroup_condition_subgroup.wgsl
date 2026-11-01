// WGSL "Uniformity Analysis": a subgroup function does not
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < subgroupMax(4u); k++) { let a = subgroupAdd(1u); }
}
