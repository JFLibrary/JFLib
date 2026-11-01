// WGSL "Address Spaces": storage buffers are read or read_write, not write
// expect: error
@group(0) @binding(0) var<storage, write> b: array<f32>;
