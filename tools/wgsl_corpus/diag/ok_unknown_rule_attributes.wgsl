// WGSL "Diagnostics": unknown rules with one name warn, also in attributes
// expect: valid
// uniformity
@diagnostic(warning, what_is_this) fn f() { }
fn g() { @diagnostic(off, not_a_rule) { } }
