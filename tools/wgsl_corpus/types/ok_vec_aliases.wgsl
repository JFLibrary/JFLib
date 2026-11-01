// WGSL "Vector Types": the predeclared aliases vec2i, vec3u, vec4f, mat4x4f
// expect: valid
fn f() -> vec4f { let a = vec2i(1); let b = vec3u(2u); let m = mat4x4f(); return m[0] + vec4f(f32(a.x), f32(b.y), 0.0, 1.0); }
