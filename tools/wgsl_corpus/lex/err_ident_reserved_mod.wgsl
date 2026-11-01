// WGSL "Identifiers": reserved words are not identifiers (mod)
// expect: error
fn f() { let mod = 1; }
