// WGSL "Structure Types": (*p)[0] indexes through a dereference, p[1] directly (pointer_composite_access)
// expect: valid
fn f() -> i32 { var a = array(1, 2); let p = &a; return (*p)[0] + p[1]; }
