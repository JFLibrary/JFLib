// WGSL "Uniformity Analysis": textureSampleLevel, textureSampleGrad and textureGather do not
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  if x > 0.5 { c = textureSampleLevel(t, s, vec2f(x), 0.0) + textureSampleGrad(t, s, vec2f(), vec2f(), vec2f()) + textureGather(0, t, s, vec2f()); }
  return c;
}
