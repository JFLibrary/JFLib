// WGSL "Uniformity Analysis": after the loop control flow is uniform again
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < i; k++) { }
  workgroupBarrier();
}
