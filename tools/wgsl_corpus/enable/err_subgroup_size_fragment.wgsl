// WGSL "Subgroups": @subgroup_size only on compute entry points
// expect: error
enable subgroup_size_control;
@subgroup_size(32) @fragment fn fs() -> @location(0) vec4f { return vec4f(); }
