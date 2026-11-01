// WGSL "Attributes": an abstract argument converts to the concrete one
// expect: valid
@compute @workgroup_size(2, 2u) fn main() {}
