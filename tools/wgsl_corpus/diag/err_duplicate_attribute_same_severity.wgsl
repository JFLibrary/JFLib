// WGSL "Diagnostics": two attributes for the same rule on one function
// expect: error
// uniformity
@diagnostic(off, derivative_uniformity) @diagnostic(off, derivative_uniformity) fn f() { }
