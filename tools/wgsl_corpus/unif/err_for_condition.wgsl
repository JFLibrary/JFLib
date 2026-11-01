// WGSL "Uniformity Analysis": a for condition on a non-uniform value: the body is non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < i; k++) { workgroupBarrier(); }
}
