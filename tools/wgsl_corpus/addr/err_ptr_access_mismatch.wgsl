// WGSL "Address Spaces": a read_write pointer from a read-only buffer
// expect: error
@group(0) @binding(0) var<storage, read> b: array<u32>;
fn g(p: ptr<storage, array<u32>, read_write>) {}
@compute @workgroup_size(1) fn main() { g(&b); }
