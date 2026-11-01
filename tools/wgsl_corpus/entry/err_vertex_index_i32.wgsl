// WGSL "Built-in Values": vertex_index is u32
// expect: error
@vertex fn vs(@builtin(vertex_index) i: i32) -> @builtin(position) vec4f { return vec4f(); }
