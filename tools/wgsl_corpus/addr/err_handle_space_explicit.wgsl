// WGSL "Address Spaces": the handle address space cannot be written
// expect: error
@group(0) @binding(0) var<handle> s: sampler;
