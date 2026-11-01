// WGSL "Uniformity Analysis": a counter started and advanced uniformly stays uniform
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < 8u; k++) { workgroupBarrier(); }
}
