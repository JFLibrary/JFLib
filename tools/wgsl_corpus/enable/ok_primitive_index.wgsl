// WGSL "Enable Extensions": enable primitive_index: the primitive_index fragment input
// expect: valid
enable primitive_index;
@fragment fn fs(@builtin(primitive_index) p: u32) -> @location(0) vec4f { return vec4f(f32(p)); }
