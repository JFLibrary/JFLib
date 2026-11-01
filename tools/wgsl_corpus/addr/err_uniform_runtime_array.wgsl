// WGSL "Address Spaces": a uniform buffer is not runtime-sized
// expect: error
@group(0) @binding(0) var<uniform> u: array<vec4f>;
