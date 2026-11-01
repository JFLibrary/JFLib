// WGSL "Diagnostics": an info in one function, a warning in another
// expect: valid
// uniformity
var<private> pv: f32;
@diagnostic(info, derivative_uniformity) fn a() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }
@diagnostic(warning, derivative_uniformity) @fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = a(); if x > 0.0 { c += dpdy(x); } return vec4f(c); }
