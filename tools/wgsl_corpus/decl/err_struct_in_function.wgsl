// WGSL "Structure Types": structs are declared at module scope only
// expect: error
fn f() { struct S { a: i32 } }
