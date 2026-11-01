// WGSL "Built-in Values": global_invocation_id is no fragment input
// expect: error
@fragment fn fs(@builtin(global_invocation_id) g: vec3u) -> @location(0) vec4f { return vec4f(); }
