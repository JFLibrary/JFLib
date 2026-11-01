// WGSL "Uniformity Analysis": a swizzle view over some components is partial
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var v = vec3u(i);
  v.yx = vec2u(0u);
  if v.x == 0u { workgroupBarrier(); }
}
