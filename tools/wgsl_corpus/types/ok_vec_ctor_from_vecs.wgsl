// WGSL "Constructible Types": vector constructors take vectors whose components add up
// expect: valid
fn f() -> vec4f { return vec4f(vec2f(1.0), vec2f(2.0)) + vec4f(1.0, vec3f()); }
