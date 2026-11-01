// WGSL "Uniformity Analysis": assigning the whole vector forgets the old value
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var v = vec2u(i);
  v = vec2u(0u);
  if v.x == 0u { workgroupBarrier(); }
}
