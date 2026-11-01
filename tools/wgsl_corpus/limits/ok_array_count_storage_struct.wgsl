// WGSL "Limits": also in a struct
// expect: valid
struct S { a: array<u32, 100000> }
@group(0) @binding(0) var<storage> v: S;
