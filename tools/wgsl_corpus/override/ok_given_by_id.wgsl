// WGSL "override Declarations": an override with @id is given by its number
// expect: valid
// constants: {"5": 7}
@id(5) override n: u32;
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
@compute @workgroup_size(1) fn main() { b[0] = n; }
