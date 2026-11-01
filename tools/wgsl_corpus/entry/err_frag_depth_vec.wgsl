// WGSL "Built-in Values": frag_depth is f32
// expect: error
@fragment fn fs() -> @builtin(frag_depth) vec2f { return vec2f(); }
