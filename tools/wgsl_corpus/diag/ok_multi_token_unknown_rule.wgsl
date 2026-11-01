// WGSL "Diagnostics": unknown rules with two names are accepted silently
// expect: valid
// uniformity
diagnostic(error, foo.bar);
@diagnostic(off, a.b) fn f() { }
