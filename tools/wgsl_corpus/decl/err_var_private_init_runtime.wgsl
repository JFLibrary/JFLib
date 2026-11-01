// WGSL "Variable Declarations": a module-scope initializer must be a const or override expression
// expect: error
var<private> a: i32 = 1;
var<private> b: i32 = a;
