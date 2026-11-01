// WGSL "Uniformity Analysis": the pointee of one argument flows into the other
// expect: error
// uniformity
fn w(p: ptr<function, u32>, q: ptr<function, u32>) { *p = *q; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = i;
  var b = 0u;
  w(&b, &a);
  if b == 0u { workgroupBarrier(); }
}
