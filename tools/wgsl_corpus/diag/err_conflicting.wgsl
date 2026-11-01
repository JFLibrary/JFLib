// WGSL "Diagnostics": two directives with different severities for one rule
// expect: error
diagnostic(off, derivative_uniformity);
diagnostic(error, derivative_uniformity);
