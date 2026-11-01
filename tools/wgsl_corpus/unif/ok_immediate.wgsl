// WGSL "Uniformity Analysis": immediate data is uniform
// expect: valid
// uniformity
var<immediate> im: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if im == 0u { workgroupBarrier(); }
}
