// WGSL "Attributes": 256 invocations is the default limit
// expect: valid
@compute @workgroup_size(16, 16) fn main() {}
