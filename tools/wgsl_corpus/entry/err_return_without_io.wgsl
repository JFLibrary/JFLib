// WGSL "Entry Points": an entry point result needs @builtin or @location
// expect: error
@fragment fn fs() -> vec4f { return vec4f(); }
