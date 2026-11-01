// WGSL "Uniformity Analysis": a constant stored in non-uniform control flow
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  const c = 1u;
  var x = 0u;
  if i == 0u { x = c; }
  if x == 0u { workgroupBarrier(); }
}
