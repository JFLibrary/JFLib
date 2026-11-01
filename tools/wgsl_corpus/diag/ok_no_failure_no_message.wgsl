// WGSL "Diagnostics": a filter without a failure says nothing
// expect: valid
// uniformity
diagnostic(warning, derivative_uniformity);
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  return vec4f(dpdx(x));
}
