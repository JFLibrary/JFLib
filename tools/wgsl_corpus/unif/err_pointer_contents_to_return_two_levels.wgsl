// WGSL "Uniformity Analysis": through two functions
// expect: error
// uniformity
fn g(p: ptr<function, u32>) -> u32 { return *p; }
fn h(p: ptr<function, u32>) -> u32 { return g(p); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  if h(&x) == 0u { workgroupBarrier(); }
}
