// WGSL "Uniformity Analysis": after a loop with behavior {Next} control flow is uniform again
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  loop { if x > 0.5 { break; } c += vec4f(1.0); if c.x > 3.0 { break; } }
  return textureSample(t, s, vec2f()) + c;
}
