// WGSL "Diagnostics": before the body of a loop: the continuing block is inside the braces
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  loop @diagnostic(off, derivative_uniformity) { if c > x { break; } continuing { c += dpdx(x); } }
  return vec4f(c);
}
