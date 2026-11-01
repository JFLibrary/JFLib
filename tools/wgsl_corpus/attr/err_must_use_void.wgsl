// WGSL "Attributes": @must_use on a function without a result
// expect: error
@must_use fn g() {}
