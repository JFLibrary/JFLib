// WGSL "Built-in Values": there is no workgroup_size built-in value in WGSL
// expect: error
@compute @workgroup_size(64) fn main(@builtin(workgroup_size) s: vec3u) {}
