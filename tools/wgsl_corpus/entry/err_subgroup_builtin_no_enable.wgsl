// WGSL "Built-in Values": subgroup_invocation_id needs enable subgroups
// expect: error
@compute @workgroup_size(64) fn main(@builtin(subgroup_invocation_id) i: u32) {}
