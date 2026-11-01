// WGSL "Diagnostics": a warning in a helper, an error in the next function
// expect: error
// uniformity
var<private> pv: f32;
@diagnostic(warning, derivative_uniformity) fn w() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }
fn e() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }
@fragment fn fs() -> @location(0) vec4f { return vec4f(w() + e()); }
