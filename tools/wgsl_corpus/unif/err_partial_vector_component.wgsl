// WGSL "Uniformity Analysis": assigning a vector component keeps the old value
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var v = vec2u(i);
  v.x = 0u;
  v.y = 0u;
  if v.x == 0u { workgroupBarrier(); }
}
