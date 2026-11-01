// WGSL "Diagnostics": barriers cannot be filtered
// expect: error
// uniformity
diagnostic(off, derivative_uniformity);
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { workgroupBarrier(); }
}
