// WGSL "Limits": a storage buffer may hold more
// expect: valid
@group(0) @binding(0) var<storage> v: array<u32, 100000>;
