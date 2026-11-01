// WGSL "Subgroups": an input only
// expect: error
enable subgroups;
@fragment fn fs() -> @builtin(subgroup_size) u32 { return 0u; }
