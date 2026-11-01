// WGSL "Memory Layout": f16 vectors in a uniform buffer
// expect: valid
enable f16;
struct U { a: vec3h, b: f16 }
@group(0) @binding(0) var<uniform> u: U;
@fragment fn fs() -> @location(0) vec4f { return vec4f(f32(u.b)); }
