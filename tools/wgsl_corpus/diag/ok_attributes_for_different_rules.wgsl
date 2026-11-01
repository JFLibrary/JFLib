// WGSL "Diagnostics": two attributes for different rules
// expect: valid
// uniformity
@diagnostic(off, derivative_uniformity) @diagnostic(error, foo.bar) fn f() { }
