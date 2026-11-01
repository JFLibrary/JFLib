// WGSL "Attributes": @align must be a power of 2
// expect: error
struct S { @align(3) a: f32 }
