// WGSL "Address Spaces": pointers to storage memory as parameters (unrestricted_pointer_parameters)
// expect: valid
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
fn g(p: ptr<storage, array<u32>, read_write>) { (*p)[0] = 1u; }
@compute @workgroup_size(1) fn main() { g(&b); }
