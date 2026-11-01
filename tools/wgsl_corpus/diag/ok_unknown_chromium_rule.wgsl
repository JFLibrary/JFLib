// WGSL "Diagnostics": an unknown rule in the chromium namespace warns
// expect: valid
diagnostic(off, chromium.foo);
fn f() { }
