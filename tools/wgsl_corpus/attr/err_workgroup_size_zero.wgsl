// WGSL "Attributes": workgroup sizes are at least 1
// expect: error
@compute @workgroup_size(0) fn main() {}
