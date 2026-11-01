// WGSL "Address Spaces": uniform takes no access mode
// expect: error
@group(0) @binding(0) var<uniform, read> u: vec4f;
