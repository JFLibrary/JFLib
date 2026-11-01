// WGSL "Uniformity Analysis": the result of workgroupUniformLoad is uniform
// expect: valid
// uniformity
var<workgroup> w: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if workgroupUniformLoad(&w) == 0u { workgroupBarrier(); }
}
