// WGSL "Attributes": @align(0)
// expect: error
struct S { @align(0) a: f32 }
