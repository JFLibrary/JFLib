// WGSL "Uniformity Analysis": a result stored in non-uniform control flow
// expect: error
// uniformity
fn g() -> u32 { return 1u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  if i == 0u { x = g(); }
  if x == 0u { workgroupBarrier(); }
}
