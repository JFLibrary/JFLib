// WGSL "Attributes": @const is not for user functions
// expect: error
@const fn g() -> i32 { return 1; }
