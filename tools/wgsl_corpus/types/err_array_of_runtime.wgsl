// WGSL "Array Types": a runtime-sized array cannot be an element
// expect: error
@group(0) @binding(0) var<storage> b: array<array<i32>, 2>;
