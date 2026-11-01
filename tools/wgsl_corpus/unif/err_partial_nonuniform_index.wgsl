// WGSL "Uniformity Analysis": a non-uniform index in the assignment makes the whole array non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = array<u32, 4>();
  a[i % 4u] = 1u;
  if a[0] == 0u { workgroupBarrier(); }
}
