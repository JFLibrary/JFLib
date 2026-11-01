// WGSL "Language Extensions": requires uniform_buffer_standard_layout: array<f32, 4> in a uniform buffer
// expect: valid
requires uniform_buffer_standard_layout;
@group(0) @binding(0) var<uniform> u: array<f32, 4>;
@fragment fn fs() -> @location(0) vec4f { return vec4f(u[3]); }
