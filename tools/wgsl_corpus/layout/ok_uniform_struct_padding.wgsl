// WGSL "Memory Layout": a member after a struct member in a uniform buffer without 16-byte padding (allowed with uniform_buffer_standard_layout)
// expect: valid
struct In { x: f32 }
struct U { a: In, b: f32 }
@group(0) @binding(0) var<uniform> u: U;
@fragment fn fs() -> @location(0) vec4f { return vec4f(u.b); }
