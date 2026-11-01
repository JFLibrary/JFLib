// WGSL "Diagnostics": and before the body of an if
// expect: error
// uniformity
fn f() { if true @diagnostic(off, derivative_uniformity) @diagnostic(error, derivative_uniformity) { } }
