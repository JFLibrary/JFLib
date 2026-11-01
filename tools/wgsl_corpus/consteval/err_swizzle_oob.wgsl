// WGSL "Indexing Expressions": .w of a vec3
// expect: error
fn f() { let v = vec3(1, 2, 3); let x = v.w; }
