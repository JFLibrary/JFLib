// WGSL "Attributes": an unknown built-in value name
// expect: error
@fragment fn main(@builtin(foo) x: u32) -> @location(0) vec4f { return vec4f(); }
