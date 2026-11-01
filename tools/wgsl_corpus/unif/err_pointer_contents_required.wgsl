// WGSL "Uniformity Analysis": the pointee must be uniform
// expect: error
// uniformity
fn g(p: ptr<function, u32>) { if *p == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  g(&x);
}
