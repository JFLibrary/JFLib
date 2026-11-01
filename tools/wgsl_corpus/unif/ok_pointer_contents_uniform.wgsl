// WGSL "Uniformity Analysis": with a uniform pointee
// expect: valid
// uniformity
fn g(p: ptr<function, u32>) { if *p == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  g(&x);
}
