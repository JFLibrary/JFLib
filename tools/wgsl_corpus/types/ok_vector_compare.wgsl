// WGSL "Structure Types": vector == gives a bool vector
// expect: valid
fn f() -> bool { return all(vec2(1, 2) == vec2(1, 2)); }
