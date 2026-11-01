// WGSL "Built-in Values": position is vec4<f32>
// expect: error
@fragment fn fs(@builtin(position) p: vec3f) -> @location(0) vec4f { return vec4f(p, 1.0); }
