// WGSL "Uniformity Analysis": workgroup_index is non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(workgroup_index) w: u32) {
  if w == 0u { workgroupBarrier(); }
}
