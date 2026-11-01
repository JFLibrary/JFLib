// WGSL "Structure Types": no arithmetic on bool
// expect: error
fn f() { let a = true + false; }
