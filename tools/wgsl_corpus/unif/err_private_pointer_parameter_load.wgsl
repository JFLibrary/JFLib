// WGSL "Uniformity Analysis": and through a private pointer
// expect: error
// uniformity
var<private> pv: u32;
fn g(p: ptr<private, u32>) -> u32 { return *p; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if g(&pv) == 0u { workgroupBarrier(); }
}
