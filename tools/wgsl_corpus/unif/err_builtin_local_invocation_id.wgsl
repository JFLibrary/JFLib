// WGSL "Uniformity Analysis": local_invocation_id is non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_id) l: vec3u) {
  if l.x == 0u { workgroupBarrier(); }
}
