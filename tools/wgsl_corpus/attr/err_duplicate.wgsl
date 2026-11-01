// WGSL "Attributes": an attribute appears at most once per object
// expect: error
@fragment fn main() -> @location(0) @location(1) vec4f { return vec4f(); }
