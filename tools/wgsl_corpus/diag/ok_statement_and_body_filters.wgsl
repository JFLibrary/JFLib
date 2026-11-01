// WGSL "Diagnostics": the statement and its body are different ranges
// expect: valid
// uniformity
fn f() { @diagnostic(off, derivative_uniformity) if true @diagnostic(error, derivative_uniformity) { } }
