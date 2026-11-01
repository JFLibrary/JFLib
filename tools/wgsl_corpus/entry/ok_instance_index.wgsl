// WGSL "Built-in Values": instance_index as a vertex input
// expect: valid
@vertex fn vs(@builtin(instance_index) i: u32) -> @builtin(position) vec4f { return vec4f(f32(i)); }
