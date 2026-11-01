// WGSL "Uniformity Analysis": global_invocation_index is non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(global_invocation_index) g: u32) {
  if g == 0u { workgroupBarrier(); }
}
