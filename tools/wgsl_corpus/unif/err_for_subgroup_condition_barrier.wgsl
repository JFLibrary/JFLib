// WGSL "Uniformity Analysis": a for condition from a reduction: a barrier fails
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < subgroupMax(4u); k++) { workgroupBarrier(); }
}
