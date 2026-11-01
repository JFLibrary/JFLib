// WGSL "Entry Points": an entry point is not callable
// expect: error
@compute @workgroup_size(1) fn main() {}
fn f() { main(); }
