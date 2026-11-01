// WGSL "Statements Behavior Analysis": if without else may fall through: a function result is missing
// expect: error
fn f(c: bool) -> i32 { if c { return 1; } }
