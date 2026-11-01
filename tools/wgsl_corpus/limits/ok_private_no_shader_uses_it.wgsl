// WGSL "Limits": a variable no shader uses does not count
// expect: valid
var<private> v: array<u32, 2049>;
@compute @workgroup_size(1) fn main() { }
