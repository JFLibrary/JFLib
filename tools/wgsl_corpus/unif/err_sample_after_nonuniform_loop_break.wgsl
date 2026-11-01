// WGSL "Uniformity Analysis": textureSample after a loop with a non-uniform break
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  for (var i = 0; i < 4; i++) { if x > f32(i) { break; } c += textureSample(t, s, vec2f(x)); }
  return c;
}
