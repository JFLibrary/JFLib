// WGSL "Uniformity Analysis": and through a workgroup pointer
// expect: error
// uniformity
var<workgroup> w: u32;
fn g(p: ptr<workgroup, u32>) { if *p == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  g(&w);
}
