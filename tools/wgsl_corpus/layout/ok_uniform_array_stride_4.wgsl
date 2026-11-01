// WGSL "Memory Layout": array<f32, 4> in a uniform buffer: stride 4 (allowed with uniform_buffer_standard_layout)
// expect: valid
@group(0) @binding(0) var<uniform> u: array<f32, 4>;
@fragment fn fs() -> @location(0) vec4f { return vec4f(u[1]); }
