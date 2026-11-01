// WGSL "Uniformity Analysis": the callee stores a parameter through the pointer
// expect: error
// uniformity
fn w(p: ptr<function, u32>, v: u32) { *p = v; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  w(&x, i);
  if x == 0u { workgroupBarrier(); }
}
