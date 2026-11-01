// WGSL "Indexing Expressions": a run-time index into a const array is allowed
// expect: valid
const a = array(1, 2, 3);
fn f(i: u32) -> i32 { return a[i]; }
