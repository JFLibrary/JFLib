// WGSL "Subgroups": not in vertex shaders
// expect: error
enable subgroups;
@vertex fn vs(@builtin(subgroup_size) a: u32) -> @builtin(position) vec4f { return vec4f(); }
