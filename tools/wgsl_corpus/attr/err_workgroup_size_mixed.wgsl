// WGSL "Attributes": workgroup size arguments of different concrete types
// expect: error
@compute @workgroup_size(2i, 2u) fn main() {}
