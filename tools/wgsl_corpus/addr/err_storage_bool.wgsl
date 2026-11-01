// WGSL "Address Spaces": bool is not host-shareable
// expect: error
@group(0) @binding(0) var<storage> b: array<bool>;
