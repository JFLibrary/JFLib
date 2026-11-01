// WGSL "Indexing Expressions": a const index out of range of a vector
// expect: error
fn f() { let v = vec3(1, 2, 3); let x = v[3]; }
