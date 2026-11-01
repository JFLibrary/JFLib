// WGSL "Uniformity Analysis": with a uniform source the target is uniform
// expect: valid
// uniformity
fn w(p: ptr<function, u32>, q: ptr<function, u32>) { *p = *q; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = 0u;
  var b = i;
  w(&b, &a);
  if b == 0u { workgroupBarrier(); }
}
