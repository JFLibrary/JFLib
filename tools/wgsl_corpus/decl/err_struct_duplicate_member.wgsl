// WGSL "Structure Types": member names of a struct are unique
// expect: error
struct S { a: i32, a: f32 }
