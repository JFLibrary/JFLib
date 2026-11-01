// WGSL "Indexing Expressions": a swizzle mixes no letter sets (xg)
// expect: error
fn f() { let v = vec4(1, 2, 3, 4); let x = v.xg; }
