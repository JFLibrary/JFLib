// WGSL "Built-in Functions": mix(vecN, vecN, scalar) has its own overload
// expect: valid
fn f() -> vec3f { return mix(vec3f(), vec3f(1.0), 0.5); }
