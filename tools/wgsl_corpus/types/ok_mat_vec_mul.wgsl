// WGSL "Structure Types": mat * vec, vec * mat, mat * mat
// expect: valid
fn f(m: mat3x2f, v: vec3f, w: vec2f, n: mat2x3f) -> f32 { let a = m * v; let b = w * m; let c = m * n; return a.x + b.y + c[0][0]; }
