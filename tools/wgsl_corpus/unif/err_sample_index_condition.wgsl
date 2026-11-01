// WGSL "Uniformity Analysis": sample_index is non-uniform
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@builtin(sample_index) si: u32) -> @location(0) vec4f {
  var c = vec4f();
  if si == 0u { c = textureSample(t, s, vec2f()); }
  return c;
}
