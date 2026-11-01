// WGSL "Uniformity Analysis": a call with a pointer to a part keeps the old value
// expect: error
// uniformity
fn w0(p: ptr<function, u32>) { *p = 0u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = array<u32, 2>(i, i);
  w0(&a[0]);
  if a[0] == 0u { workgroupBarrier(); }
}
