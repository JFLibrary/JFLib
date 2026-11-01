// WGSL "Structure Types": a trailing comma after the last member is allowed
// expect: valid
struct S { a: i32, b: f32, }
const s = S(1, 2.0);
