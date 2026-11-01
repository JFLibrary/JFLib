// WGSL "Comments": a comment separates tokens like blankspace
// expect: valid
const a = 1/**/+/* x */2;
const_assert a == 3;
