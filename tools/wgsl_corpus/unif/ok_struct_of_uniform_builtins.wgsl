// WGSL "Uniformity Analysis": a struct of uniform built-ins is uniform
// expect: valid
// uniformity
struct In { @builtin(workgroup_id) w: vec3u, @builtin(num_workgroups) n: vec3u }
@compute @workgroup_size(64) fn main(x: In) {
  if x.w.x == 0u { workgroupBarrier(); }
}
