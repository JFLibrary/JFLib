// WGSL "Pointer Types": function pointers have access read_write only
// expect: error
fn g(p: ptr<function, i32, read>) {}
