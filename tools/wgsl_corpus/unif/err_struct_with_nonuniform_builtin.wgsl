// WGSL "Uniformity Analysis": one non-uniform member makes the whole input struct non-uniform
// expect: error
// uniformity
struct In { @builtin(workgroup_id) w: vec3u, @builtin(local_invocation_index) l: u32 }
@compute @workgroup_size(64) fn main(x: In) {
  if x.w.x == 0u { workgroupBarrier(); }
}
