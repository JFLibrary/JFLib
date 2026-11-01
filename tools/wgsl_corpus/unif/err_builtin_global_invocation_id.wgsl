// WGSL "Uniformity Analysis": global_invocation_id is non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3u) {
  if g.x == 0u { workgroupBarrier(); }
}
