// WGSL "override Declarations": a bool override from a number (non-zero is true)
// expect: valid
// constants: {"flag": 2}
override flag: bool;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(1) fn main() { o[0] = select(0u, 1u, flag); }
