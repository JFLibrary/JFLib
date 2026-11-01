// WGSL "Attributes": @workgroup_size takes 1 to 3 arguments
// expect: error
@compute @workgroup_size(1, 1, 1, 1) fn main() {}
