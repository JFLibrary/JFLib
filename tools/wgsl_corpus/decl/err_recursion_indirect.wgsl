// WGSL "Declaration and Scope": recursion through another function is an error too
// expect: error
fn a() { b(); }
fn b() { a(); }
