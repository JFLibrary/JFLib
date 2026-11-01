// WGSL "Indexing Expressions": a const index out of range of a const array
// expect: error
const a = array(1, 2, 3);
const b = a[3];
