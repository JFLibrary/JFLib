// WGSL "Entry Points": a vertex shader must output @builtin(position)
// expect: error
@vertex fn vs() -> @location(0) vec4f { return vec4f(); }
