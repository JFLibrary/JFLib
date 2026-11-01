// WGSL "Uniformity Analysis": a sampled value is not uniform
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  let a = textureSample(t, s, vec2f());
  var c = vec4f();
  if a.x > 0.5 { c = textureSample(t, s, vec2f()); }
  return c;
}
