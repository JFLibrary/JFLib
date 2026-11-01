// WGSL "Uniformity Analysis": the analysis does not track components: one non-uniform component taints the vector
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var v = vec2u(0u, i);
  if v.x == 0u { workgroupBarrier(); }
}
