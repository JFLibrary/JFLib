// WGSL "Uniformity Analysis": select is not control flow, but its value depends on the condition
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let k = select(0u, 1u, i == 0u);
  if k == 0u { workgroupBarrier(); }
}
