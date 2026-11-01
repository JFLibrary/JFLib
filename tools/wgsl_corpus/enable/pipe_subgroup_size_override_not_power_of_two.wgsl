// WGSL "Subgroups": an override's value is checked at the pipeline
// expect: pipeline-error
enable subgroup_size_control;
override o = 33;
@compute @workgroup_size(66) @subgroup_size(o) fn main() { }
