// WGSL "Uniformity Analysis": after a while loop control flow is uniform again
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var k = 0u;
  while k < i { k++; }
  workgroupBarrier();
}
