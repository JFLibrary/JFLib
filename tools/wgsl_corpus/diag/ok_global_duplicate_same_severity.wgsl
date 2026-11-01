// WGSL "Diagnostics": global filters may repeat without conflict
// expect: valid
// uniformity
diagnostic(off, derivative_uniformity);
diagnostic(off, derivative_uniformity);
fn f() { }
