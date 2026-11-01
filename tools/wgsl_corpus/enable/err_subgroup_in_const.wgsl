// WGSL "Subgroups": not in const-expressions
// expect: error
enable subgroups;
const c = subgroupAdd(1u);
