// WGSL "Limits": a struct alone neither
// expect: valid
struct S { a: array<u32, 100000> }
