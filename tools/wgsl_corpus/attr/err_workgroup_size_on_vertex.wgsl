// WGSL "Attributes": @workgroup_size only on compute entry points
// expect: error
@vertex @workgroup_size(1) fn main() -> @builtin(position) vec4f { return vec4f(); }
