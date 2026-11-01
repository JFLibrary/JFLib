// WGSL "Diagnostics": before the body of a for loop
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  for (var k = 0; k < 4; k++) @diagnostic(off, derivative_uniformity) { if x > 0.5 { c += textureSample(t, s, vec2f(x)); } }
  return c;
}
