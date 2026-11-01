// WGSL "Diagnostics": and on a block
// expect: error
// uniformity
fn f() { @diagnostic(off, derivative_uniformity) @diagnostic(off, derivative_uniformity) { } }
