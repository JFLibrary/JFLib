// WGSL "Indexing Expressions": a negative const index
// expect: error
fn f() { let a = array(1, 2); let x = a[-1]; }
