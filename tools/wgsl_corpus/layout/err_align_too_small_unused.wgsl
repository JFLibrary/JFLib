// WGSL "Memory Layout": @align(8) on a vec4f member, even of a struct not used for a buffer (a multiple of AlignOf = 16)
// expect: error
struct S { a: f32, @align(8) v: vec4f }
fn f() -> vec4f { let s = S(1.0, vec4f()); return s.v; }
