// WGSL "Limits": in a uniform buffer
// expect: error
@group(0) @binding(0) var<uniform> v: array<vec4u, 65536>;
