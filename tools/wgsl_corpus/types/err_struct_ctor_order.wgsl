// WGSL "Structure Types": a struct constructor takes the members in order and type
// expect: error
struct S { a: i32, b: f32 }
fn f() { let s = S(1.0, 2); }
