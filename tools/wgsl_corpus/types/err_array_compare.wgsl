// WGSL "Structure Types": arrays have no == operator
// expect: error
fn f() -> bool { return array(1, 2) == array(1, 2); }
