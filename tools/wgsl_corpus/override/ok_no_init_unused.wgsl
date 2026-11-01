// WGSL "override Declarations": an override without initializer the entry point does not use needs no constant
// expect: valid
override n: i32;
@compute @workgroup_size(1) fn main() {}
