// WGSL "Diagnostics": two attributes with different severities
// expect: error
// uniformity
@diagnostic(off, derivative_uniformity) @diagnostic(error, derivative_uniformity) fn f() { }
