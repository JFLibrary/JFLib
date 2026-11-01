// WGSL "Uniformity Analysis": even when the callee only reads it
// expect: error
// uniformity
fn r(p: ptr<function, u32>) -> u32 { return *p; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  if i == 0u { _ = r(&x); }
  if x == 0u { workgroupBarrier(); }
}
