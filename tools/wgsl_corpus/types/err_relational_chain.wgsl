// WGSL "Structure Types": relational operators do not chain
// expect: error
fn f(a: i32) -> bool { return 0 < a < 2; }
