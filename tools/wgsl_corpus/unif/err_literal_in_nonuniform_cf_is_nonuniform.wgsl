// WGSL "Uniformity Analysis": any value computed in non-uniform control flow is non-uniform there
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { var y = 1u; if y == 1u { workgroupBarrier(); } }
}
