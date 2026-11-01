// WGSL "Built-in Functions": reflect and refract (vector, vector, scalar eta)
// expect: valid
fn f(i: vec3f, n: vec3f) -> vec3f { return reflect(i, n) + refract(i, n, 1.33) + faceForward(n, i, n); }
