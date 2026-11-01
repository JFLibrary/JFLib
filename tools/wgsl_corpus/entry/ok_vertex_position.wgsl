// WGSL "Entry Points": the smallest vertex shader
// expect: valid
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f { return vec4f(f32(i), 0.0, 0.0, 1.0); }
