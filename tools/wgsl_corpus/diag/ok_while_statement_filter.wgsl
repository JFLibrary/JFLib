// WGSL "Diagnostics": before a while statement: also its condition
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  @diagnostic(off, derivative_uniformity) while c.x < x { c += textureSample(t, s, vec2f(x)); }
  return c;
}
