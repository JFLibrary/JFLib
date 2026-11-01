// WGSL "Uniformity Analysis": a while condition on a non-uniform value
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var k = 0u;
  while k < i { workgroupBarrier(); k++; }
}
