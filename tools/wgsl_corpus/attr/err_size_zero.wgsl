// WGSL "Attributes": @size(0)
// expect: error
struct S { @size(0) a: f32 }
