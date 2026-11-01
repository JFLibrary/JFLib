// WGSL "Memory Layout": a runtime-sized array only as the last member of the outermost struct
// expect: error
struct Inner { a: array<u32> }
struct Outer { i: Inner }
@group(0) @binding(0) var<storage> b: Outer;
