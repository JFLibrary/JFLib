// WGSL "Uniformity Analysis": the result depends on the pointee
// expect: error
// uniformity
fn f(p: ptr<function, u32>) -> u32 { return *p; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  if f(&x) == 0u { workgroupBarrier(); }
}
