// WGSL "Uniformity Analysis": textureLoad of a sampled texture is uniform
// expect: valid
// uniformity
@group(0) @binding(0) var tt: texture_2d<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if textureLoad(tt, vec2u(), 0).x == 0u { workgroupBarrier(); }
}
