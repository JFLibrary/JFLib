// WGSL "Uniformity Analysis": an increment of a uniform variable
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  x++;
  if x == 1u { workgroupBarrier(); }
}
