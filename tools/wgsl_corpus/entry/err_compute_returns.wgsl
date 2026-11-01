// WGSL "Entry Points": a compute entry point returns nothing
// expect: error
@compute @workgroup_size(1) fn main() -> @location(0) u32 { return 1u; }
