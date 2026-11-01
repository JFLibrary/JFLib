// WGSL "Attributes": built-in value functions are must_use (abs as a statement)
// expect: error
fn f() { abs(1); }
