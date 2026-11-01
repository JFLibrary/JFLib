// WGSL "Memory Layout": @size(8) on a vec3f member (12 bytes)
// expect: error
struct S { @size(8) v: vec3f }
