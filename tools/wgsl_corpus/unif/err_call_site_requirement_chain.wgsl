// WGSL "Uniformity Analysis": the requirement passes through a chain of functions
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
fn h() -> vec4f { return textureSample(t, s, vec2f()); }
fn g() -> vec4f { return h(); }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  if x > 0.5 { c = g(); }
  return c;
}
