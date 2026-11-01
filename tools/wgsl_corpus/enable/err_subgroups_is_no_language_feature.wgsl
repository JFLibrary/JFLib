// WGSL "Subgroups": subgroups is an extension, not a language feature
// expect: error
requires subgroups;
fn f() { }
