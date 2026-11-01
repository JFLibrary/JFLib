// WGSL "Uniformity Analysis": num_workgroups is uniform
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(num_workgroups) n: vec3u) {
  if n.x == 1u { workgroupBarrier(); }
}
