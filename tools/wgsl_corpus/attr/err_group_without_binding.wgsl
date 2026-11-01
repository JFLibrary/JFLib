// WGSL "Attributes": a resource needs both @group and @binding
// expect: error
@group(0) var<storage> b: array<f32>;
@compute @workgroup_size(1) fn main() { _ = b[0]; }
