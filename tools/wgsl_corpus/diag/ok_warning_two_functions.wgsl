// WGSL "Diagnostics": each function reports its own failure
// expect: valid
// uniformity
diagnostic(warning, derivative_uniformity);
var<private> pv: f32;
fn a() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }
fn b() -> f32 { var c = 0.0; if pv > 1.0 { c = dpdy(1.0); } return c; }
@fragment fn fs() -> @location(0) vec4f { return vec4f(a() + b()); }
