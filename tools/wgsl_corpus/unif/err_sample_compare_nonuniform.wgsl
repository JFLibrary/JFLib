// WGSL "Uniformity Analysis": textureSampleCompare computes a derivative
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_depth_2d;
@group(0) @binding(1) var s: sampler_comparison;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = textureSampleCompare(t, s, vec2f(x), 0.5); }
  return vec4f(c);
}
