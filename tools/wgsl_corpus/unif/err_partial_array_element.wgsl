// WGSL "Uniformity Analysis": assigning an array element keeps the old value
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = array<u32, 2>(i, i);
  a[0] = 0u;
  if a[0] == 0u { workgroupBarrier(); }
}
