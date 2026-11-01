// WGSL "Limits": 8192 bytes of private variables a shader uses
// expect: valid
var<private> v: array<u32, 2048>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; }
