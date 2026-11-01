// WGSL "Attributes": attribute arguments may end with a comma
// expect: valid
@compute @workgroup_size(8, 8,) fn main() {}
@fragment fn fs() -> @location(0,) vec4f { return vec4f(); }
