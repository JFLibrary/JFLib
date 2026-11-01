// WGSL "Uniformity Analysis": even right after a write
// expect: error
// uniformity
var<private> pv: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  pv = 0u;
  if pv == 0u { workgroupBarrier(); }
}
