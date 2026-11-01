// WGSL "Structure Types": mat3x2f * vec2f: the shapes do not fit
// expect: error
fn f(m: mat3x2f, v: vec2f) -> vec2f { return m * v; }
