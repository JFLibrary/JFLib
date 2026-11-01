// WGSL "Diagnostics": before a loop statement: also the continuing block
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  @diagnostic(off, derivative_uniformity) loop { if x > c { break; } continuing { c += dpdx(x); } }
  return vec4f(c);
}
