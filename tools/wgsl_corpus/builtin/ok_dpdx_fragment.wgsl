// WGSL "Built-in Functions": derivatives in uniform control flow of a fragment shader
// expect: valid
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { return vec4f(dpdx(x), dpdyFine(x), fwidthCoarse(x), 1.0); }
