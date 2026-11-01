// WGSL "Uniformity Analysis": with a uniform argument the pointee is uniform after the call
// expect: valid
// uniformity
fn w(p: ptr<function, u32>, v: u32) { *p = v; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  w(&x, 1u);
  if x == 0u { workgroupBarrier(); }
}
