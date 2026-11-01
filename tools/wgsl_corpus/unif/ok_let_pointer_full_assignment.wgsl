// WGSL "Uniformity Analysis": an assignment through a full pointer is a full assignment
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  let p = &x;
  *p = 0u;
  if x == 0u { workgroupBarrier(); }
}
