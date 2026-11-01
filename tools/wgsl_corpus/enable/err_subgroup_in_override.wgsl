// WGSL "Subgroups": nor in override-expressions
// expect: error
enable subgroups;
override o = subgroupAdd(1u);
