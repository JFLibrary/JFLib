// WGSL "Template Lists": after a literal < starts no template list: (1 < 2) > 3 compares bool with int
// expect: error
fn f() -> bool { return 1 < 2 > 3; }
