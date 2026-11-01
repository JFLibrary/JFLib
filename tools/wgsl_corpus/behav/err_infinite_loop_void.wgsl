// WGSL "Statements Behavior Analysis": a loop that never exits (its behavior is empty) is an error, also in a void function
// expect: error
fn f() { loop {} }
