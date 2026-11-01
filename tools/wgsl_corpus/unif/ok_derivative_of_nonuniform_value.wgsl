// WGSL "Uniformity Analysis": a derivative of a non-uniform value in uniform control flow is fine
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { return vec4f(dpdx(x * x)); }
