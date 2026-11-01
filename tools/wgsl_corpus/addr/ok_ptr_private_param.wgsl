// WGSL "Address Spaces": pointers to private memory as parameters
// expect: valid
var<private> a: i32;
fn g(p: ptr<private, i32>) { *p = 3; }
fn f() { g(&a); }
