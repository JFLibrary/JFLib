// WGSL "Uniformity Analysis": the index of a let pointer is captured at the declaration
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = array<u32, 64>();
  var k = i;
  let p = &a[k];
  k = 0u;
  *p = 1u;
  if a[0] == 0u { workgroupBarrier(); }
}
