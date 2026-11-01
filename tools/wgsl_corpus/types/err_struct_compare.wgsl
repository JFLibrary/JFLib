// WGSL "Structure Types": structs have no == operator
// expect: error
struct S { a: i32 }
fn f() -> bool { return S(1) == S(1); }
