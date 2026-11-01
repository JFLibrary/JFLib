// WGSL "Entry Points": one built-in input twice
// expect: error
@fragment fn fs(@builtin(position) a: vec4f, @builtin(position) b: vec4f) -> @location(0) vec4f { return a + b; }
