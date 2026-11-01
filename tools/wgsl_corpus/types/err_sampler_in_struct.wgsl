// WGSL "Pointer Types": a sampler is no struct member
// expect: error
struct S { s: sampler }
