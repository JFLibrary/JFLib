// WGSL "Subgroups": not on a helper
// expect: error
enable subgroup_size_control;
@subgroup_size(32) fn f() { }
