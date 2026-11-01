// WGSL "Uniformity Analysis": nor a sampled value
// expect: error
// uniformity
enable subgroups;
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if textureSample(t, s, vec2f()).x > 0.5 { c = subgroupAdd(1.0); }
  return vec4f(c);
}
