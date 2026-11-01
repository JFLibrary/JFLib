// WGSL "Built-in Functions": cross only of vec3
// expect: error
fn f() { _ = cross(vec2f(), vec2f()); }
