// WGSL "Subgroups": subgroup_id is a language feature, not an extension
// expect: error
enable subgroup_id;
fn f() { }
