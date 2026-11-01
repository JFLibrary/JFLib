// WGSL "Attributes": @size smaller than the member type
// expect: error
struct S { @size(4) a: vec4f }
